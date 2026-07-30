#![allow(clippy::expect_used)]

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use piqae_auth::generate_platform_service_account_key;
use piqae_control_plane::{
    AppState, authentication::PostgresAuthenticator, repository::MemoryRepository, router,
};
use piqae_domain::{EnvironmentId, WorkspaceId};
use piqae_storage_postgres::PostgresStore;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{env, sync::Arc};
use tower::ServiceExt;

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

async fn insert_tenant(pool: &PgPool, name: &str) -> (WorkspaceId, EnvironmentId) {
    let workspace_id = WorkspaceId::new();
    let environment_id = EnvironmentId::new();
    sqlx::query("INSERT INTO workspaces (id, name, slug) VALUES ($1,$2,$3)")
        .bind(workspace_id.to_string())
        .bind(name)
        .bind(format!(
            "platform-http-{}",
            ulid::Ulid::new().to_string().to_ascii_lowercase()
        ))
        .execute(pool)
        .await
        .expect("workspace fixture");
    sqlx::query(
        "INSERT INTO environments (id, workspace_id, kind, name)
         VALUES ($1,$2,'live','Live')",
    )
    .bind(environment_id.to_string())
    .bind(workspace_id.to_string())
    .execute(pool)
    .await
    .expect("environment fixture");
    (workspace_id, environment_id)
}

fn platform_request(
    credential: &str,
    workspace_id: WorkspaceId,
    environment_id: EnvironmentId,
    request_id: &str,
) -> Request<Body> {
    Request::builder()
        .uri("/v1/jobs")
        .header("authorization", format!("Bearer {credential}"))
        .header("x-piqae-workspace-id", workspace_id.to_string())
        .header("x-piqae-environment-id", environment_id.to_string())
        .header("x-request-id", request_id)
        .body(Body::empty())
        .expect("valid platform request")
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn postgres_platform_http_auth_is_tenant_scoped_audited_and_revocable() {
    let Some(database_url) = env::var("PIQAE_TEST_DATABASE_URL").ok() else {
        eprintln!("skipped: set PIQAE_TEST_DATABASE_URL to run PostgreSQL platform HTTP evidence");
        return;
    };

    let schema = format!("piqae_platform_http_{}", ulid::Ulid::new()).to_ascii_lowercase();
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

    let (workspace_id, environment_id) = insert_tenant(&pool, "Granted").await;
    let (other_workspace_id, other_environment_id) = insert_tenant(&pool, "Other").await;
    let credential = generate_platform_service_account_key().expect("generate platform key");
    store
        .create_platform_service_account_with_grant(
            &credential.id.to_string(),
            "HTTP integration",
            &credential.password_hash,
            workspace_id,
            environment_id,
            &["jobs_read".to_owned()],
            None,
        )
        .await
        .expect("create platform account");
    store
        .upsert_platform_workspace_grant(
            &credential.id.to_string(),
            other_workspace_id,
            other_environment_id,
            &["printers_read".to_owned()],
            None,
        )
        .await
        .expect("create scope-limited second grant");

    let application = router(AppState::new(
        Arc::new(MemoryRepository::default()),
        Arc::new(PostgresAuthenticator::new(store.clone())),
    ));

    let granted = application
        .clone()
        .oneshot(platform_request(
            &credential.plaintext,
            workspace_id,
            environment_id,
            "req_platform_http_granted",
        ))
        .await
        .expect("granted response");
    assert_eq!(granted.status(), StatusCode::OK);

    let scope_denied = application
        .clone()
        .oneshot(platform_request(
            &credential.plaintext,
            other_workspace_id,
            other_environment_id,
            "req_platform_http_scope_denied",
        ))
        .await
        .expect("scope-denied response");
    assert_eq!(scope_denied.status(), StatusCode::FORBIDDEN);

    let unknown_tenant = application
        .clone()
        .oneshot(platform_request(
            &credential.plaintext,
            WorkspaceId::new(),
            EnvironmentId::new(),
            "req_platform_http_unknown",
        ))
        .await
        .expect("unknown-tenant response");
    assert_eq!(unknown_tenant.status(), StatusCode::UNAUTHORIZED);

    store
        .revoke_platform_workspace_grant(&credential.id.to_string(), workspace_id, environment_id)
        .await
        .expect("revoke exact grant");
    let revoked = application
        .oneshot(platform_request(
            &credential.plaintext,
            workspace_id,
            environment_id,
            "req_platform_http_revoked",
        ))
        .await
        .expect("revoked response");
    assert_eq!(revoked.status(), StatusCode::UNAUTHORIZED);

    let results: Vec<(String, bool)> = sqlx::query_as(
        "SELECT request_id, (safe_metadata->>'scope_granted')::boolean
         FROM audit_events
         WHERE actor_id = $1
           AND action = 'platform_service_account.authenticated'
         ORDER BY request_id",
    )
    .bind(credential.id.to_string())
    .fetch_all(&pool)
    .await
    .expect("read platform request audit");
    assert_eq!(
        results,
        vec![
            ("req_platform_http_granted".to_owned(), true),
            ("req_platform_http_scope_denied".to_owned(), false),
        ]
    );

    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .expect("drop disposable schema");
}
