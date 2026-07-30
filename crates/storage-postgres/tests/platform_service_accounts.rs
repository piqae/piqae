#![allow(clippy::expect_used)]

use chrono::{Duration, Utc};
use piqae_auth::{
    generate_platform_service_account_key, rotate_platform_service_account_key,
    verify_platform_service_account_key,
};
use piqae_domain::{EnvironmentId, WorkspaceId};
use piqae_storage_postgres::{PostgresStore, StorageError};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::env;
use uuid::Uuid;

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

async fn insert_tenant(pool: &PgPool, suffix: &str) -> (WorkspaceId, EnvironmentId) {
    let workspace_id = WorkspaceId::new();
    let environment_id = EnvironmentId::new();
    sqlx::query(
        "INSERT INTO workspaces (id, name, slug)
         VALUES ($1, $2, $3)",
    )
    .bind(workspace_id.to_string())
    .bind(format!("Platform {suffix}"))
    .bind(format!(
        "platform-{}-{}",
        suffix.to_ascii_lowercase(),
        ulid::Ulid::new()
    ))
    .execute(pool)
    .await
    .expect("workspace fixture");
    sqlx::query(
        "INSERT INTO environments (id, workspace_id, kind, name)
         VALUES ($1, $2, 'live', 'Live')",
    )
    .bind(environment_id.to_string())
    .bind(workspace_id.to_string())
    .execute(pool)
    .await
    .expect("environment fixture");
    (workspace_id, environment_id)
}

fn assert_not_found(result: &Result<impl Sized, StorageError>) {
    assert!(
        matches!(result, Err(StorageError::NotFound)),
        "tenant/grant mismatch must be indistinguishable from an unknown resource"
    );
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn postgres_platform_grants_are_exact_scoped_and_revocable() {
    let Some(database_url) = env::var("PIQAE_TEST_DATABASE_URL").ok() else {
        eprintln!(
            "skipped: set PIQAE_TEST_DATABASE_URL to run PostgreSQL platform service-account evidence"
        );
        return;
    };

    let schema = format!("piqae_platform_test_{}", ulid::Ulid::new()).to_ascii_lowercase();
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
    let store = PostgresStore::from_pool(pool.clone());
    store.migrate().await.expect("apply migrations");

    let (granted_workspace, granted_environment) = insert_tenant(&pool, "Granted").await;
    let (other_workspace, other_environment) = insert_tenant(&pool, "Other").await;
    let credential = generate_platform_service_account_key().expect("generate synthetic key");
    let platform_id = credential.id.to_string();
    let scopes = vec![
        "printers_read".to_owned(),
        "jobs_read".to_owned(),
        "jobs_write".to_owned(),
    ];
    store
        .create_platform_service_account_with_grant(
            &platform_id,
            "SaaS integration",
            &credential.password_hash,
            granted_workspace,
            granted_environment,
            &scopes,
            None,
        )
        .await
        .expect("create exact platform grant");

    let granted = store
        .platform_grant_for_authentication(&platform_id, granted_workspace, granted_environment)
        .await
        .expect("exact workspace/environment grant");
    assert_eq!(granted.secret_hash, credential.password_hash);
    assert_ne!(granted.secret_hash, credential.plaintext);
    verify_platform_service_account_key(&credential.plaintext, &granted.secret_hash)
        .expect("synthetic platform credential verifies");
    assert_eq!(granted.scopes, scopes);
    store
        .record_platform_service_account_use(
            &platform_id,
            granted_workspace,
            granted_environment,
            "printers_read",
            true,
            "req_platform_postgres_test",
        )
        .await
        .expect("record authenticated platform request");
    let audit_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_events
         WHERE workspace_id = $1 AND environment_id = $2
           AND actor_type = 'platform_service_account' AND actor_id = $3
           AND action = 'platform_service_account.authenticated'
           AND request_id = 'req_platform_postgres_test'
           AND safe_metadata =
               '{\"required_scope\":\"printers_read\",\"scope_granted\":true}'::jsonb",
    )
    .bind(granted_workspace.to_string())
    .bind(granted_environment.to_string())
    .bind(&platform_id)
    .fetch_one(&pool)
    .await
    .expect("query platform authentication audit evidence");
    assert_eq!(audit_count, 1);

    assert_not_found(
        &store
            .platform_grant_for_authentication(&platform_id, other_workspace, other_environment)
            .await,
    );
    assert_not_found(
        &store
            .platform_grant_for_authentication(&platform_id, granted_workspace, other_environment)
            .await,
    );

    let ordinary_key_id = Uuid::now_v7().to_string();
    let ordinary_lookup_prefix = format!("piq_live_{}", &ordinary_key_id[..8]);
    store
        .create_api_key(
            granted_workspace,
            granted_environment,
            &ordinary_key_id,
            "Ordinary integration",
            &ordinary_lookup_prefix,
            "$argon2id$test-ordinary-secret-hash",
            &["printers_read".to_owned()],
            None,
        )
        .await
        .expect("create ordinary tenant API key");
    let ordinary = store
        .api_key_for_authentication(&ordinary_lookup_prefix)
        .await
        .expect("ordinary key remains tenant-bound");
    assert_eq!(ordinary.workspace_id, granted_workspace);
    assert_eq!(ordinary.environment_id, granted_environment);
    assert_not_found(
        &store
            .platform_grant_for_authentication(&ordinary_key_id, other_workspace, other_environment)
            .await,
    );

    store
        .upsert_platform_workspace_grant(
            &platform_id,
            other_workspace,
            other_environment,
            &["printers_read".to_owned()],
            None,
        )
        .await
        .expect("create independent second grant");
    store
        .revoke_platform_workspace_grant(&platform_id, granted_workspace, granted_environment)
        .await
        .expect("revoke exact workspace/environment grant");
    assert_not_found(
        &store
            .platform_grant_for_authentication(&platform_id, granted_workspace, granted_environment)
            .await,
    );
    store
        .platform_grant_for_authentication(&platform_id, other_workspace, other_environment)
        .await
        .expect("revoking one grant leaves the other grant active");

    store
        .upsert_platform_workspace_grant(
            &platform_id,
            granted_workspace,
            granted_environment,
            &scopes,
            None,
        )
        .await
        .expect("restore grant before account-level revocation");
    let rotated =
        rotate_platform_service_account_key(credential.id).expect("rotate synthetic credential");
    store
        .rotate_platform_service_account(&platform_id, &rotated.password_hash)
        .await
        .expect("persist credential rotation");
    let rotated_grant = store
        .platform_grant_for_authentication(&platform_id, granted_workspace, granted_environment)
        .await
        .expect("grant remains available after rotation");
    verify_platform_service_account_key(&rotated.plaintext, &rotated_grant.secret_hash)
        .expect("rotated credential verifies");
    assert!(
        verify_platform_service_account_key(&credential.plaintext, &rotated_grant.secret_hash)
            .is_err(),
        "the original credential must stop verifying immediately after rotation"
    );

    store
        .upsert_platform_workspace_grant(
            &platform_id,
            granted_workspace,
            granted_environment,
            &scopes,
            Some(Utc::now() - Duration::seconds(1)),
        )
        .await
        .expect("expire the restored grant");
    assert_not_found(
        &store
            .platform_grant_for_authentication(&platform_id, granted_workspace, granted_environment)
            .await,
    );
    store
        .upsert_platform_workspace_grant(
            &platform_id,
            granted_workspace,
            granted_environment,
            &scopes,
            None,
        )
        .await
        .expect("restore grant after expiry evidence");

    store
        .revoke_platform_service_account(&platform_id)
        .await
        .expect("revoke platform service account");
    assert_not_found(
        &store
            .platform_grant_for_authentication(&platform_id, granted_workspace, granted_environment)
            .await,
    );
    assert_not_found(
        &store
            .platform_grant_for_authentication(&platform_id, other_workspace, other_environment)
            .await,
    );

    let secret_leaks: i64 = sqlx::query_scalar(
        "SELECT
             (SELECT count(*) FROM platform_service_accounts
              WHERE secret_hash = $1 OR secret_hash = $2)
           + (SELECT count(*) FROM audit_events
              WHERE safe_metadata::text LIKE '%' || $1 || '%'
                 OR safe_metadata::text LIKE '%' || $2 || '%')",
    )
    .bind(&credential.plaintext)
    .bind(&rotated.plaintext)
    .fetch_one(&pool)
    .await
    .expect("scan database and audit metadata for plaintext credentials");
    assert_eq!(
        secret_leaks, 0,
        "plaintext credentials must never be persisted"
    );
    store
        .delete_platform_service_account(&platform_id)
        .await
        .expect("delete revoked platform service account");

    let lifecycle_actions: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT action FROM audit_events
         WHERE resource_type = 'platform_service_account' AND resource_id = $1",
    )
    .bind(&platform_id)
    .fetch_all(&pool)
    .await
    .expect("read lifecycle audit actions");
    for expected in [
        "platform_service_account.created",
        "platform_service_account.grant_updated",
        "platform_service_account.grant_revoked",
        "platform_service_account.rotated",
        "platform_service_account.revoked",
        "platform_service_account.deleted",
        "platform_service_account.authenticated",
    ] {
        assert!(
            lifecycle_actions.iter().any(|action| action == expected),
            "missing lifecycle audit action {expected}"
        );
    }

    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .expect("drop disposable schema");
}
