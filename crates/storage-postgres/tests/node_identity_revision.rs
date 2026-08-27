#![allow(clippy::expect_used, clippy::too_many_lines)]

use piqae_domain::{AgentId, EnvironmentId, WorkspaceId};
use piqae_protocol::agent::NodeDisplayIdentity;
use piqae_storage_postgres::{PostgresStore, StorageError};
use sqlx::{PgPool, migrate::Migrator, postgres::PgPoolOptions};
use std::{borrow::Cow, env};

async fn schema_pool(database_url: &str, schema: &str) -> PgPool {
    let schema = schema.to_owned();
    PgPoolOptions::new()
        .max_connections(4)
        .after_connect(move |connection, _metadata| {
            let statement = format!("SET search_path TO {schema}");
            Box::pin(async move {
                sqlx::query(&statement).execute(connection).await?;
                Ok(())
            })
        })
        .connect(database_url)
        .await
        .expect("connect disposable PostgreSQL schema")
}

#[tokio::test]
async fn node_identity_upgrades_43_and_fences_tenants_revisions_and_invalid_rows() {
    let Some(database_url) = env::var("PIQAE_TEST_DATABASE_URL").ok() else {
        eprintln!("skipped: set PIQAE_TEST_DATABASE_URL for node identity migration evidence");
        return;
    };
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .expect("connect PostgreSQL test database");
    let schema = format!("piqae_node_identity_{}", ulid::Ulid::new()).to_ascii_lowercase();
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .expect("create exact disposable schema");
    let pool = schema_pool(&database_url, &schema).await;
    let all = sqlx::migrate!("../../migrations/postgres");
    let previous = Migrator {
        migrations: Cow::Owned(
            all.iter()
                .filter(|migration| migration.version < 44)
                .cloned()
                .collect(),
        ),
        ignore_missing: false,
        locking: true,
        no_tx: false,
    };
    previous
        .run(&pool)
        .await
        .expect("apply exact version 43 schema");

    let workspace = WorkspaceId::new();
    let environment = EnvironmentId::new();
    let other_workspace = WorkspaceId::new();
    let other_environment = EnvironmentId::new();
    let agent = AgentId::new();
    for (workspace_id, environment_id, suffix) in [
        (workspace, environment, "owner"),
        (other_workspace, other_environment, "other"),
    ] {
        sqlx::query("INSERT INTO workspaces (id,name,slug) VALUES ($1,$2,$3)")
            .bind(workspace_id.to_string())
            .bind(format!("Workspace {suffix}"))
            .bind(format!("node-identity-{suffix}-{}", ulid::Ulid::new()).to_ascii_lowercase())
            .execute(&pool)
            .await
            .expect("workspace fixture");
        sqlx::query(
            "INSERT INTO environments (id,workspace_id,kind,name) VALUES ($1,$2,'test','Test')",
        )
        .bind(environment_id.to_string())
        .bind(workspace_id.to_string())
        .execute(&pool)
        .await
        .expect("environment fixture");
    }
    sqlx::query("INSERT INTO agents (id,workspace_id,environment_id,name,installation_id,os,architecture,version,protocol_version) VALUES ($1,$2,$3,'Node','identity-install','test','test','1',1)")
        .bind(agent.to_string())
        .bind(workspace.to_string())
        .bind(environment.to_string())
        .execute(&pool)
        .await
        .expect("version 43 agent fixture");

    let store = PostgresStore::from_pool(pool.clone());
    store.migrate().await.expect("upgrade 43 to 44");
    let upgraded = store
        .get_agent(workspace, environment, agent)
        .await
        .expect("upgraded agent");
    assert_eq!(upgraded.identity_revision, 1);
    assert!(upgraded.labels.is_empty());

    let invalid_site = sqlx::query("UPDATE agents SET identity_site=$2 WHERE id=$1")
        .bind(agent.to_string())
        .bind("é".repeat(61))
        .execute(&pool)
        .await;
    assert!(invalid_site.is_err(), "site bound must use UTF-8 bytes");
    let control_site = sqlx::query("UPDATE agents SET identity_site=$2 WHERE id=$1")
        .bind(agent.to_string())
        .bind("Warehouse\nAdmin")
        .execute(&pool)
        .await;
    assert!(control_site.is_err(), "site control characters must fail");
    let control_location = sqlx::query("UPDATE agents SET identity_location=$2 WHERE id=$1")
        .bind(agent.to_string())
        .bind("Desk\u{0007}")
        .execute(&pool)
        .await;
    assert!(
        control_location.is_err(),
        "location control characters must fail"
    );
    let invalid_labels = sqlx::query(
        "UPDATE agents SET identity_labels='[\" duplicate \" , \"duplicate\", \"duplicate\"]'::jsonb WHERE id=$1",
    )
    .bind(agent.to_string())
    .execute(&pool)
    .await;
    assert!(invalid_labels.is_err(), "direct invalid labels must fail");
    let control_label = sqlx::query(
        "UPDATE agents SET identity_labels='[\"shipping\\nadmin\"]'::jsonb WHERE id=$1",
    )
    .bind(agent.to_string())
    .execute(&pool)
    .await;
    assert!(control_label.is_err(), "label control characters must fail");
    let too_many_labels = sqlx::query(
        "UPDATE agents SET identity_labels=(SELECT jsonb_agg('label-' || value) FROM generate_series(1,17) AS value) WHERE id=$1",
    )
    .bind(agent.to_string())
    .execute(&pool)
    .await;
    assert!(too_many_labels.is_err(), "more than 16 labels must fail");

    let first = NodeDisplayIdentity {
        display_name: "First writer".into(),
        site: Some("Warehouse".into()),
        location: None,
        labels: vec!["shipping".into()],
    };
    let second = NodeDisplayIdentity {
        display_name: "Second writer".into(),
        site: None,
        location: Some("Desk".into()),
        labels: vec!["dispatch".into()],
    };
    let (left, right) = tokio::join!(
        store.update_agent_identity(workspace, environment, agent, Some(1), &first),
        store.update_agent_identity(workspace, environment, agent, Some(1), &second),
    );
    assert_ne!(
        left.is_ok(),
        right.is_ok(),
        "exactly one CAS writer succeeds"
    );
    let (winner, loser) = if left.is_ok() {
        (&first, right.expect_err("second writer conflicts"))
    } else {
        (&second, left.expect_err("first writer conflicts"))
    };
    assert!(matches!(
        loser,
        StorageError::NodeIdentityRevisionConflict(2)
    ));
    let replay = store
        .update_agent_identity(workspace, environment, agent, Some(1), winner)
        .await
        .expect("exact response-loss replay is idempotent");
    assert_eq!(replay.identity_revision, 2);
    assert_eq!(replay.name, winner.display_name);

    let cross_tenant = store
        .update_agent_identity(other_workspace, other_environment, agent, Some(2), winner)
        .await;
    assert!(matches!(cross_tenant, Err(StorageError::NotFound)));
    let latest: i64 = sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations WHERE success")
        .fetch_one(&pool)
        .await
        .expect("schema version");
    assert_eq!(latest, 44);

    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .expect("drop exact disposable schema");

    let fresh_schema =
        format!("piqae_node_identity_fresh_{}", ulid::Ulid::new()).to_ascii_lowercase();
    sqlx::query(&format!("CREATE SCHEMA {fresh_schema}"))
        .execute(&admin)
        .await
        .expect("create fresh schema");
    let fresh = schema_pool(&database_url, &fresh_schema).await;
    all.run(&fresh).await.expect("migrate empty database to 44");
    let fresh_latest: i64 =
        sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations WHERE success")
            .fetch_one(&fresh)
            .await
            .expect("fresh schema version");
    assert_eq!(fresh_latest, 44);
    fresh.close().await;
    sqlx::query(&format!("DROP SCHEMA {fresh_schema} CASCADE"))
        .execute(&admin)
        .await
        .expect("drop fresh schema");
}
