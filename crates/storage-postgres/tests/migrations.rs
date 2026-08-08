#![allow(clippy::expect_used)]

use piqae_storage_postgres::PostgresStore;
use sqlx::{PgPool, migrate::Migrator, postgres::PgPoolOptions};
use std::{borrow::Cow, env};

async fn schema_pool(database_url: &str, schema: &str) -> PgPool {
    let schema = schema.to_owned();
    PgPoolOptions::new()
        .max_connections(2)
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
async fn postgres_reported_complete_billing_upgrades_from_previous_schema() {
    let Some(database_url) = env::var("PIQAE_TEST_DATABASE_URL").ok() else {
        eprintln!("skipped: set PIQAE_TEST_DATABASE_URL to run PostgreSQL migration evidence");
        return;
    };

    let schema = format!("piqae_migration_{}", ulid::Ulid::new()).to_ascii_lowercase();
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .expect("connect PostgreSQL test database");
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .expect("create disposable schema");
    let pool = schema_pool(&database_url, &schema).await;

    let all = sqlx::migrate!("../../migrations/postgres");
    let previous = Migrator {
        migrations: Cow::Owned(
            all.iter()
                .filter(|migration| migration.version < 17)
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
        .expect("apply previous schema version");

    let store = PostgresStore::from_pool(pool.clone());
    store.migrate().await.expect("upgrade to latest schema");

    let latest: i64 = sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .expect("read latest schema version");
    assert_eq!(latest, 26);
    let billable_index: Option<String> =
        sqlx::query_scalar("SELECT to_regclass('usage_one_billable_print_per_job_idx')::text")
            .fetch_one(&pool)
            .await
            .expect("inspect billable usage index");
    assert_eq!(
        billable_index.as_deref(),
        Some("usage_one_billable_print_per_job_idx")
    );

    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .expect("drop exact disposable schema");
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn node_connector_upgrade_backfills_without_cross_tenant_merging() {
    let Some(database_url) = env::var("PIQAE_TEST_DATABASE_URL").ok() else {
        eprintln!("skipped: set PIQAE_TEST_DATABASE_URL to run PostgreSQL migration evidence");
        return;
    };
    let schema = format!("piqae_connector_migration_{}", ulid::Ulid::new()).to_ascii_lowercase();
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .expect("connect PostgreSQL test database");
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .expect("create disposable schema");
    let pool = schema_pool(&database_url, &schema).await;
    let all = sqlx::migrate!("../../migrations/postgres");
    let previous = Migrator {
        migrations: Cow::Owned(
            all.iter()
                .filter(|migration| migration.version < 19)
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
        .expect("apply schema through 0018");
    for suffix in ["a", "b"] {
        sqlx::query("INSERT INTO workspaces (id,name,slug) VALUES ($1,$2,$3)")
            .bind(format!("wsp_{suffix}"))
            .bind(suffix)
            .bind(format!("migration-{suffix}"))
            .execute(&pool)
            .await
            .expect("workspace");
        sqlx::query(
            "INSERT INTO environments (id,workspace_id,kind,name) VALUES ($1,$2,'live','Live')",
        )
        .bind(format!("env_{suffix}"))
        .bind(format!("wsp_{suffix}"))
        .execute(&pool)
        .await
        .expect("environment");
        sqlx::query("INSERT INTO agents (id,workspace_id,environment_id,name,installation_id,public_key,os,architecture,version,protocol_version) VALUES ($1,$2,$3,'Node','historical-collision',$4,'test','test','1',1)")
            .bind(format!("agt_{suffix}")).bind(format!("wsp_{suffix}"))
            .bind(format!("env_{suffix}")).bind(vec![suffix.as_bytes()[0]])
            .execute(&pool).await.expect("legacy agent");
    }
    PostgresStore::from_pool(pool.clone())
        .migrate()
        .await
        .expect("upgrade through 0022");
    let installations: i64 = sqlx::query_scalar("SELECT count(*) FROM node_installations")
        .fetch_one(&pool)
        .await
        .expect("installation count");
    let connectors: i64 = sqlx::query_scalar("SELECT count(*) FROM node_connectors")
        .fetch_one(&pool)
        .await
        .expect("connector count");
    assert_eq!((installations, connectors), (2, 2));
    let connector_mappings: Vec<(String, String, String, String, String)> = sqlx::query_as(
        "SELECT connector.agent_id,
                connector.workspace_id,
                connector.environment_id,
                connector.installation_id,
                installation.installation_key
         FROM node_connectors connector
         JOIN node_installations installation
           ON installation.id = connector.installation_id
         ORDER BY connector.agent_id",
    )
    .fetch_all(&pool)
    .await
    .expect("inspect tenant connector installation mapping");
    assert_eq!(
        connector_mappings,
        vec![
            (
                "agt_a".into(),
                "wsp_a".into(),
                "env_a".into(),
                "ninst_agt_a".into(),
                "legacy:agt_a".into(),
            ),
            (
                "agt_b".into(),
                "wsp_b".into(),
                "env_b".into(),
                "ninst_agt_b".into(),
                "legacy:agt_b".into(),
            ),
        ],
        "colliding tenant-local legacy installation IDs must remain distinct"
    );
    let connector_columns: Vec<String> = sqlx::query_scalar(
        "SELECT column_name
         FROM information_schema.columns
         WHERE table_schema = current_schema()
           AND table_name = 'node_connectors'
           AND column_name IN (
             'installation_id', 'workspace_id', 'environment_id', 'agent_id'
           )
         ORDER BY column_name",
    )
    .fetch_all(&pool)
    .await
    .expect("inspect connector tenant columns");
    assert_eq!(
        connector_columns,
        vec![
            "agent_id",
            "environment_id",
            "installation_id",
            "workspace_id"
        ]
    );
    let cross_tenant: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM node_connectors
         WHERE (workspace_id = 'wsp_a' AND agent_id = 'agt_b')
            OR (workspace_id = 'wsp_b' AND agent_id = 'agt_a')",
    )
    .fetch_one(&pool)
    .await
    .expect("tenant isolation query");
    assert_eq!(cross_tenant, 0);
    sqlx::query("INSERT INTO node_content_encryption_keys (workspace_id,environment_id,agent_id,key_id,algorithm,public_key_spki) VALUES ('wsp_a','env_a','agt_a','cek_a','RSA-OAEP-256',$1)")
        .bind("A".repeat(128)).execute(&pool).await.expect("tenant key");
    let cross_tenant_key = sqlx::query("INSERT INTO node_content_encryption_keys (workspace_id,environment_id,agent_id,key_id,algorithm,public_key_spki) VALUES ('wsp_a','env_a','agt_b','cek_probe','RSA-OAEP-256',$1)")
        .bind("B".repeat(128)).execute(&pool).await;
    assert!(
        cross_tenant_key.is_err(),
        "composite agent foreign key must reject cross-tenant key registration"
    );
    sqlx::query("INSERT INTO node_content_encryption_keys (workspace_id,environment_id,agent_id,key_id,algorithm,public_key_spki) VALUES ('wsp_b','env_b','agt_b','cek_ecdh','ECDH-P256-HKDF-SHA256',$1)")
        .bind("C".repeat(122))
        .execute(&pool)
        .await
        .expect("tenant-scoped P-256 encryption key");
    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .expect("drop exact disposable schema");
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn agent_health_migrates_empty_and_previous_schemas_with_tenant_fencing() {
    let Some(database_url) = env::var("PIQAE_TEST_DATABASE_URL").ok() else {
        eprintln!("skipped: set PIQAE_TEST_DATABASE_URL to run PostgreSQL migration evidence");
        return;
    };
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .expect("connect PostgreSQL test database");
    let all = sqlx::migrate!("../../migrations/postgres");

    let empty_schema = format!("piqae_health_empty_{}", ulid::Ulid::new()).to_ascii_lowercase();
    sqlx::query(&format!("CREATE SCHEMA {empty_schema}"))
        .execute(&admin)
        .await
        .expect("create empty-database schema");
    let empty_pool = schema_pool(&database_url, &empty_schema).await;
    all.run(&empty_pool).await.expect("migrate empty database");
    let latest: i64 = sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations")
        .fetch_one(&empty_pool)
        .await
        .expect("read empty-database schema version");
    assert_eq!(latest, 26);
    empty_pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {empty_schema} CASCADE"))
        .execute(&admin)
        .await
        .expect("drop exact empty-database schema");

    let upgrade_schema = format!("piqae_health_upgrade_{}", ulid::Ulid::new()).to_ascii_lowercase();
    sqlx::query(&format!("CREATE SCHEMA {upgrade_schema}"))
        .execute(&admin)
        .await
        .expect("create upgrade schema");
    let upgrade_pool = schema_pool(&database_url, &upgrade_schema).await;
    let previous = Migrator {
        migrations: Cow::Owned(
            all.iter()
                .filter(|migration| migration.version < 26)
                .cloned()
                .collect(),
        ),
        ignore_missing: false,
        locking: true,
        no_tx: false,
    };
    previous.run(&upgrade_pool).await.expect("apply schema 25");
    for suffix in ["a", "b"] {
        sqlx::query("INSERT INTO workspaces (id,name,slug) VALUES ($1,$2,$3)")
            .bind(format!("wsp_health_{suffix}"))
            .bind(suffix)
            .bind(format!("health-{suffix}"))
            .execute(&upgrade_pool)
            .await
            .expect("workspace");
        sqlx::query(
            "INSERT INTO environments (id,workspace_id,kind,name) VALUES ($1,$2,'live','Live')",
        )
        .bind(format!("env_health_{suffix}"))
        .bind(format!("wsp_health_{suffix}"))
        .execute(&upgrade_pool)
        .await
        .expect("environment");
        sqlx::query("INSERT INTO agents (id,workspace_id,environment_id,name,installation_id,os,architecture,version,protocol_version) VALUES ($1,$2,$3,'Node',$4,'test','test','1',1)")
            .bind(format!("agt_health_{suffix}"))
            .bind(format!("wsp_health_{suffix}"))
            .bind(format!("env_health_{suffix}"))
            .bind(format!("installation-health-{suffix}"))
            .execute(&upgrade_pool).await.expect("legacy agent");
    }
    all.run(&upgrade_pool)
        .await
        .expect("upgrade schema 25 to 26");
    let own_update = sqlx::query(
        "UPDATE agents SET executor_crashes = 2, last_error_code = 'executor_crashed'
         WHERE id = 'agt_health_a' AND workspace_id = 'wsp_health_a'
           AND environment_id = 'env_health_a'",
    )
    .execute(&upgrade_pool)
    .await
    .expect("tenant health update");
    assert_eq!(own_update.rows_affected(), 1);
    let cross_tenant_probe = sqlx::query(
        "UPDATE agents SET executor_crashes = 99
         WHERE id = 'agt_health_b' AND workspace_id = 'wsp_health_a'
           AND environment_id = 'env_health_a'",
    )
    .execute(&upgrade_pool)
    .await
    .expect("cross-tenant probe");
    assert_eq!(cross_tenant_probe.rows_affected(), 0);
    let other_count: i64 =
        sqlx::query_scalar("SELECT executor_crashes FROM agents WHERE id = 'agt_health_b'")
            .fetch_one(&upgrade_pool)
            .await
            .expect("other tenant health");
    assert_eq!(other_count, 0);
    sqlx::query("INSERT INTO node_diagnostics (request_id, workspace_id, environment_id, agent_id, state) VALUES ('diag_a','wsp_health_a','env_health_a','agt_health_a','requested')")
        .execute(&upgrade_pool).await.expect("tenant diagnostic request");
    let cross_tenant_report = sqlx::query("UPDATE node_diagnostics SET state = 'complete', report = '{}'::jsonb WHERE request_id = 'diag_a' AND workspace_id = 'wsp_health_b' AND environment_id = 'env_health_b' AND agent_id = 'agt_health_b'")
        .execute(&upgrade_pool).await.expect("cross-tenant diagnostic probe");
    assert_eq!(cross_tenant_report.rows_affected(), 0);

    upgrade_pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {upgrade_schema} CASCADE"))
        .execute(&admin)
        .await
        .expect("drop exact upgrade schema");
}
