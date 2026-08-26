#![allow(clippy::expect_used, clippy::too_many_lines)]

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use base64::{Engine as _, engine::general_purpose::STANDARD_NO_PAD};
use chrono::Utc;
use ed25519_dalek::{Signer as _, SigningKey};
use piqae_control_plane::{
    AppState, authentication::PostgresAuthenticator, repository::Repository, router,
};
use piqae_domain::{AgentId, EnvironmentId, JobId, PrinterId, WorkspaceId};
use piqae_storage_postgres::PostgresStore;
use sha2::{Digest as _, Sha256};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{env, sync::Arc};
use tower::ServiceExt as _;

async fn schema_pool(database_url: &str, schema: &str) -> PgPool {
    let schema = schema.to_owned();
    PgPoolOptions::new()
        .max_connections(4)
        .after_connect(move |connection, _| {
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

fn signed_request(
    agent_id: AgentId,
    signing_key: &SigningKey,
    method: &str,
    path: &str,
    body: Vec<u8>,
) -> Request<Body> {
    let timestamp = Utc::now().timestamp_millis();
    let nonce = uuid::Uuid::new_v4();
    let digest = format!("{:x}", Sha256::digest(&body));
    let canonical = format!("{method}\n{path}\n{timestamp}\n{nonce}\n{digest}");
    let signature = signing_key.sign(canonical.as_bytes());
    Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json")
        .header("x-piqae-agent-id", agent_id.to_string())
        .header("x-piqae-timestamp", timestamp.to_string())
        .header("x-piqae-nonce", nonce.to_string())
        .header("x-piqae-body-sha256", digest)
        .header(
            "x-piqae-signature",
            STANDARD_NO_PAD.encode(signature.to_bytes()),
        )
        .body(Body::from(body))
        .expect("valid signed request")
}

#[tokio::test]
async fn signed_revoke_preserves_accepted_work_and_denies_ordinary_agent_calls() {
    let Some(database_url) = env::var("PIQAE_TEST_DATABASE_URL").ok() else {
        eprintln!("skipped: set PIQAE_TEST_DATABASE_URL for connector revoke HTTP evidence");
        return;
    };
    let schema = format!("piqae_connector_revoke_{}", ulid::Ulid::new()).to_ascii_lowercase();
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

    let workspace_id = WorkspaceId::new();
    let environment_id = EnvironmentId::new();
    let agent_id = AgentId::new();
    let printer_id = PrinterId::new();
    let job_id = JobId::new();
    let connector_id = format!("ncon_{}", ulid::Ulid::new());
    let installation_id = format!("ninst_{}", ulid::Ulid::new());
    let signing_key = SigningKey::from_bytes(&[37; 32]);
    let public_key = signing_key.verifying_key().to_bytes().to_vec();
    sqlx::query("INSERT INTO workspaces (id,name,slug) VALUES ($1,'Revoke fixture',$2)")
        .bind(workspace_id.to_string())
        .bind(format!("revoke-{}", ulid::Ulid::new()).to_ascii_lowercase())
        .execute(&pool)
        .await
        .expect("workspace fixture");
    sqlx::query(
        "INSERT INTO environments (id,workspace_id,kind,name) VALUES ($1,$2,'live','Live')",
    )
    .bind(environment_id.to_string())
    .bind(workspace_id.to_string())
    .execute(&pool)
    .await
    .expect("environment fixture");
    sqlx::query(
        "INSERT INTO node_installations (id,installation_key,public_key)
         VALUES ($1,$2,$3)",
    )
    .bind(&installation_id)
    .bind(format!("test:{}", ulid::Ulid::new()))
    .bind(&public_key)
    .execute(&pool)
    .await
    .expect("installation fixture");
    sqlx::query(
        "INSERT INTO agents
         (id,workspace_id,environment_id,name,installation_id,public_key,os,architecture,version,protocol_version)
         VALUES ($1,$2,$3,'Embedded node',$4,$5,'ios','arm64','test',1)",
    )
    .bind(agent_id.to_string())
    .bind(workspace_id.to_string())
    .bind(environment_id.to_string())
    .bind(format!("embedded:{}", ulid::Ulid::new()))
    .bind(&public_key)
    .execute(&pool)
    .await
    .expect("agent fixture");
    sqlx::query(
        "INSERT INTO node_connectors
         (id,installation_id,workspace_id,environment_id,agent_id)
         VALUES ($1,$2,$3,$4,$5)",
    )
    .bind(&connector_id)
    .bind(&installation_id)
    .bind(workspace_id.to_string())
    .bind(environment_id.to_string())
    .bind(agent_id.to_string())
    .execute(&pool)
    .await
    .expect("connector fixture");
    sqlx::query(
        "INSERT INTO printers
         (id,workspace_id,environment_id,agent_id,native_id,name,state,capabilities_revision)
         VALUES ($1,$2,$3,$4,'native-revoke','Revoke printer','online',1)",
    )
    .bind(printer_id.to_string())
    .bind(workspace_id.to_string())
    .bind(environment_id.to_string())
    .bind(agent_id.to_string())
    .execute(&pool)
    .await
    .expect("printer fixture");
    sqlx::query(
        "INSERT INTO jobs
         (id,workspace_id,environment_id,printer_id,agent_id,payload,state,per_printer_sequence,expires_at)
         VALUES ($1,$2,$3,$4,$5,'{}'::jsonb,'agent_accepted',1,now()+interval '1 hour')",
    )
    .bind(job_id.to_string())
    .bind(workspace_id.to_string())
    .bind(environment_id.to_string())
    .bind(printer_id.to_string())
    .bind(agent_id.to_string())
    .execute(&pool)
    .await
    .expect("accepted job fixture");

    let application = router(AppState::new_for_tests(
        Arc::new(store.clone()) as Arc<dyn Repository>,
        Arc::new(PostgresAuthenticator::new(store.clone())),
    ));
    let revoke_path = format!("/v1/agent/connectors/{connector_id}/revoke");
    let revoked = application
        .clone()
        .oneshot(signed_request(
            agent_id,
            &signing_key,
            "POST",
            &revoke_path,
            b"{}".to_vec(),
        ))
        .await
        .expect("revoke response");
    assert_eq!(revoked.status(), StatusCode::OK);

    let retained_state: String = sqlx::query_scalar("SELECT state FROM jobs WHERE id=$1")
        .bind(job_id.to_string())
        .fetch_one(&pool)
        .await
        .expect("accepted work remains durable");
    assert_eq!(retained_state, "agent_accepted");
    let denied = application
        .oneshot(signed_request(
            agent_id,
            &signing_key,
            "POST",
            "/v1/agent/sync",
            b"{}".to_vec(),
        ))
        .await
        .expect("post-revoke sync response");
    assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);

    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .expect("drop disposable schema");
}
