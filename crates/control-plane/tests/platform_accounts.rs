#![allow(clippy::expect_used, clippy::similar_names, clippy::too_many_lines)]

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use chrono::Utc;
use http_body_util::BodyExt;
use piqae_auth::{
    Environment, Scope, generate_api_key, generate_local_owner_credential,
    generate_local_owner_session, generate_platform_service_account_key,
};
use piqae_control_plane::{
    AppState,
    authentication::{CombinedAuthenticator, LocalSessionAuthenticator, PostgresAuthenticator},
    repository::Repository,
    router,
};
use piqae_domain::{
    AgentId, ContentKind, ContentSource, EnvironmentId, Job, JobId, JobOptions, JobState,
    PrinterId, WorkspaceId,
};
use piqae_object_store::MemoryObjectStore;
use piqae_storage_postgres::PostgresStore;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{collections::BTreeMap, env, sync::Arc};
use tower::ServiceExt;

async fn schema_pool(database_url: &str, schema: &str) -> PgPool {
    let schema = schema.to_owned();
    PgPoolOptions::new()
        .max_connections(8)
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

async fn insert_owner_tenant(pool: &PgPool, suffix: &str) -> (WorkspaceId, EnvironmentId) {
    let workspace_id = WorkspaceId::new();
    let environment_id = EnvironmentId::new();
    sqlx::query(
        "INSERT INTO workspaces (id, name, slug)
         VALUES ($1,$2,$3)",
    )
    .bind(workspace_id.to_string())
    .bind(format!("Platform owner {suffix}"))
    .bind(format!("platform-owner-{suffix}-{}", ulid::Ulid::new()).to_ascii_lowercase())
    .execute(pool)
    .await
    .expect("owner workspace");
    sqlx::query(
        "INSERT INTO environments (id, workspace_id, kind, name)
         VALUES ($1,$2,'live','Live')",
    )
    .bind(environment_id.to_string())
    .bind(workspace_id.to_string())
    .execute(pool)
    .await
    .expect("owner environment");
    (workspace_id, environment_id)
}

fn request(method: &str, path: &str, bearer: &str, body: Option<&str>) -> Request<Body> {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {bearer}"));
    if body.is_some() {
        request = request.header("content-type", "application/json");
    }
    request
        .body(body.map_or_else(Body::empty, |value| Body::from(value.to_owned())))
        .expect("HTTP request")
}

async fn response_json(response: axum::response::Response) -> serde_json::Value {
    serde_json::from_slice(
        &response
            .into_body()
            .collect()
            .await
            .expect("response body")
            .to_bytes(),
    )
    .expect("response JSON")
}

async fn insert_durable_job(
    pool: &PgPool,
    store: &PostgresStore,
    workspace_id: WorkspaceId,
    environment_id: EnvironmentId,
) -> JobId {
    let agent_id = AgentId::new();
    let printer_id = PrinterId::new();
    sqlx::query(
        "INSERT INTO agents (
            id, workspace_id, environment_id, name, installation_id,
            os, architecture, version, protocol_version, state
         ) VALUES ($1,$2,$3,'Archive fixture','archive-fixture',
                   'test','test','0.1.0',1,'offline')",
    )
    .bind(agent_id.to_string())
    .bind(workspace_id.to_string())
    .bind(environment_id.to_string())
    .execute(pool)
    .await
    .expect("job agent");
    sqlx::query(
        "INSERT INTO printers (
            id, workspace_id, environment_id, agent_id, native_id, name
         ) VALUES ($1,$2,$3,$4,'archive-printer','Archive printer')",
    )
    .bind(printer_id.to_string())
    .bind(workspace_id.to_string())
    .bind(environment_id.to_string())
    .bind(agent_id.to_string())
    .execute(pool)
    .await
    .expect("job printer");
    let now = Utc::now();
    let job = Job {
        id: JobId::new(),
        workspace_id,
        environment_id,
        printer_id,
        title: "Durable archive fixture".into(),
        source: None,
        content_kind: ContentKind::Pdf,
        content: ContentSource::Base64 {
            data: "JVBERi0=".into(),
        },
        options: JobOptions::default(),
        metadata: BTreeMap::new(),
        deliveries: 1,
        state: JobState::WaitingForAgent,
        created_at: now,
        expires_at: now + chrono::Duration::hours(1),
    };
    store
        .create_job(&job, agent_id, None, b"durable archive fixture")
        .await
        .expect("durable job");
    job.id
}

#[tokio::test]
async fn postgres_http_platform_accounts_are_owned_idempotent_and_archive_safely() {
    let Some(database_url) = env::var("PIQAE_TEST_DATABASE_URL").ok() else {
        eprintln!(
            "skipped: set PIQAE_TEST_DATABASE_URL to run PostgreSQL platform account HTTP evidence"
        );
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
    let (owner_a, owner_a_environment) = insert_owner_tenant(&pool, "a").await;
    let (owner_b, owner_b_environment) = insert_owner_tenant(&pool, "b").await;
    let platform_a = generate_platform_service_account_key().expect("platform A key");
    let platform_b = generate_platform_service_account_key().expect("platform B key");
    store
        .create_platform_service_account_with_grant(
            &platform_a.id.to_string(),
            "Platform A",
            &platform_a.password_hash,
            owner_a,
            owner_a_environment,
            &[Scope::ApiKeysWrite.as_str().into()],
            None,
        )
        .await
        .expect("platform A");
    store
        .create_platform_service_account_with_grant(
            &platform_b.id.to_string(),
            "Platform B",
            &platform_b.password_hash,
            owner_b,
            owner_b_environment,
            &[Scope::ApiKeysWrite.as_str().into()],
            None,
        )
        .await
        .expect("platform B");

    let owner_credential = generate_local_owner_credential().expect("owner credential");
    let owner_session = generate_local_owner_session().expect("owner session");
    sqlx::query(
        "INSERT INTO local_owner_credentials (id, workspace_id, key_hash)
         VALUES ($1,$2,$3)",
    )
    .bind(owner_credential.id.to_string())
    .bind(owner_a.to_string())
    .bind(owner_credential.password_hash)
    .execute(&pool)
    .await
    .expect("owner credential row");
    store
        .create_local_owner_session(
            &owner_session.id.to_string(),
            owner_a,
            &owner_credential.id.to_string(),
            &owner_session.password_hash,
            Utc::now() + chrono::Duration::hours(1),
        )
        .await
        .expect("owner session row");
    let ordinary_key = generate_api_key(Environment::Live).expect("ordinary API key");
    store
        .create_api_key(
            owner_a,
            owner_a_environment,
            &ordinary_key.id.to_string(),
            "Ordinary key",
            &ordinary_key.lookup_prefix,
            &ordinary_key.password_hash,
            &[Scope::ApiKeysWrite.as_str().into()],
            None,
        )
        .await
        .expect("ordinary API key row");

    let authenticator = CombinedAuthenticator::new(
        PostgresAuthenticator::new(store.clone()),
        Some(LocalSessionAuthenticator::new(store.clone())),
        None,
        None,
    );
    let application = router(AppState::new_with_resources(
        Arc::new(store.clone()) as Arc<dyn Repository>,
        Arc::new(authenticator),
        [7; 32],
        Arc::new(MemoryObjectStore::default()),
    ));
    let meta = application
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/meta")
                .body(Body::empty())
                .expect("meta request"),
        )
        .await
        .expect("meta response");
    assert_eq!(meta.status(), StatusCode::OK);
    assert_eq!(response_json(meta).await["platform"]["accounts"], true);

    let created = application
        .clone()
        .oneshot(request(
            "PUT",
            "/v1/platform/accounts/customer-one",
            &platform_a.plaintext,
            Some(r#"{"name":"Customer One","metadata":{"plan":"pro"}}"#),
        ))
        .await
        .expect("create response");
    assert_eq!(created.status(), StatusCode::CREATED);
    let created = response_json(created).await;
    let customer_workspace: WorkspaceId = created["id"]
        .as_str()
        .expect("customer workspace")
        .parse()
        .expect("typed workspace");
    let customer_live: EnvironmentId = created["environments"]["live"]["id"]
        .as_str()
        .expect("live environment")
        .parse()
        .expect("typed environment");

    let cross_platform = application
        .clone()
        .oneshot(request(
            "GET",
            "/v1/platform/accounts/customer-one",
            &platform_b.plaintext,
            None,
        ))
        .await
        .expect("cross-platform response");
    assert_eq!(cross_platform.status(), StatusCode::NOT_FOUND);
    let platform_a_accounts = application
        .clone()
        .oneshot(request(
            "GET",
            "/v1/platform/accounts",
            &platform_a.plaintext,
            None,
        ))
        .await
        .expect("platform A list response");
    assert_eq!(platform_a_accounts.status(), StatusCode::OK);
    let platform_a_accounts = response_json(platform_a_accounts).await;
    assert_eq!(platform_a_accounts.as_array().map(Vec::len), Some(1));
    assert_eq!(platform_a_accounts[0]["external_id"], "customer-one");
    let platform_b_accounts = application
        .clone()
        .oneshot(request(
            "GET",
            "/v1/platform/accounts",
            &platform_b.plaintext,
            None,
        ))
        .await
        .expect("platform B list response");
    assert_eq!(platform_b_accounts.status(), StatusCode::OK);
    assert_eq!(
        response_json(platform_b_accounts)
            .await
            .as_array()
            .map(Vec::len),
        Some(0)
    );
    let ordinary_denied = application
        .clone()
        .oneshot(request(
            "GET",
            "/v1/platform/accounts",
            &ordinary_key.plaintext,
            None,
        ))
        .await
        .expect("ordinary key response");
    assert_eq!(ordinary_denied.status(), StatusCode::UNAUTHORIZED);
    let selected_tenant_denied = application
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/platform/accounts")
                .header("authorization", format!("Bearer {}", platform_a.plaintext))
                .header("x-piqae-workspace-id", owner_a.to_string())
                .header("x-piqae-environment-id", owner_a_environment.to_string())
                .body(Body::empty())
                .expect("tenant-selected manager request"),
        )
        .await
        .expect("tenant-selected manager response");
    assert_eq!(selected_tenant_denied.status(), StatusCode::UNAUTHORIZED);
    let human_owner = application
        .clone()
        .oneshot(request(
            "GET",
            "/v1/platform/accounts/customer-one",
            &owner_session.plaintext,
            None,
        ))
        .await
        .expect("human owner response");
    assert_eq!(human_owner.status(), StatusCode::OK);

    let first = application.clone().oneshot(request(
        "PUT",
        "/v1/platform/accounts/concurrent",
        &platform_a.plaintext,
        Some(r#"{"name":"Concurrent account","metadata":{"request":"one"}}"#),
    ));
    let second = application.clone().oneshot(request(
        "PUT",
        "/v1/platform/accounts/concurrent",
        &platform_a.plaintext,
        Some(r#"{"name":"Concurrent account","metadata":{"request":"two"}}"#),
    ));
    let (first, second) = tokio::join!(first, second);
    let first = first.expect("first concurrent response");
    let second = second.expect("second concurrent response");
    assert_eq!(
        [first.status(), second.status()]
            .into_iter()
            .filter(|status| *status == StatusCode::CREATED)
            .count(),
        1
    );
    assert_eq!(
        [first.status(), second.status()]
            .into_iter()
            .filter(|status| *status == StatusCode::OK)
            .count(),
        1
    );
    let concurrent_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM workspaces
         WHERE platform_service_account_id = $1 AND platform_external_id = 'concurrent'",
    )
    .bind(platform_a.id.to_string())
    .fetch_one(&pool)
    .await
    .expect("concurrent account count");
    assert_eq!(concurrent_count, 1);
    let concurrent_environment_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM environments environment
         JOIN workspaces workspace ON workspace.id = environment.workspace_id
         WHERE workspace.platform_service_account_id = $1
           AND workspace.platform_external_id = 'concurrent'
           AND environment.kind IN ('test', 'live')",
    )
    .bind(platform_a.id.to_string())
    .fetch_one(&pool)
    .await
    .expect("concurrent environment count");
    assert_eq!(concurrent_environment_count, 2);
    let exact_grants: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM platform_workspace_grants grant_row
         JOIN workspaces workspace ON workspace.id = grant_row.workspace_id
         WHERE workspace.platform_service_account_id = $1
           AND workspace.platform_external_id = 'concurrent'
           AND grant_row.service_account_id = $1
           AND grant_row.revoked_at IS NULL
           AND cardinality(grant_row.scopes) = 12
           AND grant_row.scopes @> ARRAY[
               'api_keys_read', 'api_keys_write', 'agents_read', 'agents_write',
               'printers_read', 'printers_write', 'jobs_read', 'jobs_write',
               'webhooks_read', 'webhooks_write', 'usage_read', 'audit_read'
           ]::text[]",
    )
    .bind(platform_a.id.to_string())
    .fetch_one(&pool)
    .await
    .expect("exact platform grants");
    assert_eq!(exact_grants, 2);

    let durable_job = insert_durable_job(&pool, &store, customer_workspace, customer_live).await;
    let tenant_request = |bearer: &str| {
        Request::builder()
            .uri("/v1/jobs")
            .header("authorization", format!("Bearer {bearer}"))
            .header("x-piqae-workspace-id", customer_workspace.to_string())
            .header("x-piqae-environment-id", customer_live.to_string())
            .body(Body::empty())
            .expect("tenant request")
    };
    let before_archive = application
        .clone()
        .oneshot(tenant_request(&platform_a.plaintext))
        .await
        .expect("pre-archive tenant response");
    assert_eq!(before_archive.status(), StatusCode::OK);
    let archive = application
        .clone()
        .oneshot(request(
            "DELETE",
            "/v1/platform/accounts/customer-one",
            &platform_a.plaintext,
            None,
        ))
        .await
        .expect("archive response");
    assert_eq!(archive.status(), StatusCode::NO_CONTENT);
    let repeated_archive = application
        .clone()
        .oneshot(request(
            "DELETE",
            "/v1/platform/accounts/customer-one",
            &platform_a.plaintext,
            None,
        ))
        .await
        .expect("repeated archive response");
    assert_eq!(repeated_archive.status(), StatusCode::NO_CONTENT);
    let after_archive = application
        .clone()
        .oneshot(tenant_request(&platform_a.plaintext))
        .await
        .expect("post-archive tenant response");
    assert_eq!(after_archive.status(), StatusCode::UNAUTHORIZED);
    let archived = application
        .clone()
        .oneshot(request(
            "GET",
            "/v1/platform/accounts/customer-one",
            &owner_session.plaintext,
            None,
        ))
        .await
        .expect("archived account response");
    assert_eq!(archived.status(), StatusCode::OK);
    assert_eq!(response_json(archived).await["status"], "cancelled");
    let durable_count: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs WHERE id = $1")
        .bind(durable_job.to_string())
        .fetch_one(&pool)
        .await
        .expect("durable job count");
    assert_eq!(durable_count, 1);
    let grants_remaining: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM platform_workspace_grants
         WHERE service_account_id = $1 AND workspace_id = $2 AND revoked_at IS NULL",
    )
    .bind(platform_a.id.to_string())
    .bind(customer_workspace.to_string())
    .fetch_one(&pool)
    .await
    .expect("active grants");
    assert_eq!(grants_remaining, 0);
    let secret_in_audit: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_events
         WHERE safe_metadata::text LIKE '%' || $1 || '%'",
    )
    .bind(&platform_a.plaintext)
    .fetch_one(&pool)
    .await
    .expect("audit secret scan");
    assert_eq!(secret_in_audit, 0);

    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .expect("drop disposable schema");
}
