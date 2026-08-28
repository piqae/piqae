#![allow(clippy::expect_used)]

//! The dashboard has always offered family subscriptions — a "Jobs" checkbox
//! that submits `job.*`. Matching was exact, so every endpoint created that way
//! received nothing at all: a silent no-op, which is worse than an error
//! because the operator believes they are covered.

use std::env;

use piqae_domain::{EnvironmentId, WorkspaceId};
use piqae_storage_postgres::PostgresStore;
use sqlx::{PgPool, postgres::PgPoolOptions};

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

async fn endpoint(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    environment_id: EnvironmentId,
    label: &str,
    events: &[&str],
) -> String {
    let id = format!("whe_{}", ulid::Ulid::new());
    sqlx::query(
        "INSERT INTO webhook_endpoints (
            id, workspace_id, environment_id, url, secret_ciphertext, subscribed_events
         ) VALUES ($1,$2,$3,$4,$5,$6)",
    )
    .bind(&id)
    .bind(workspace_id.to_string())
    .bind(environment_id.to_string())
    .bind(format!("https://example.invalid/{label}"))
    .bind(vec![1_u8; 16])
    .bind(events.iter().map(|e| (*e).to_owned()).collect::<Vec<_>>())
    .execute(pool)
    .await
    .expect("endpoint");
    id
}

async fn delivered_to(pool: &PgPool, endpoint_id: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM webhook_deliveries WHERE endpoint_id = $1")
        .bind(endpoint_id)
        .fetch_one(pool)
        .await
        .expect("count deliveries")
}

#[tokio::test]
async fn a_family_subscription_receives_its_events_without_swallowing_others() {
    let Some(database_url) = env::var("PIQAE_TEST_DATABASE_URL").ok() else {
        eprintln!("skipped: set PIQAE_TEST_DATABASE_URL to run webhook subscription evidence");
        return;
    };
    let schema = format!("piqae_whsub_{}", ulid::Ulid::new()).to_ascii_lowercase();
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .expect("connect PostgreSQL");
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .expect("create schema");

    let pool = schema_pool(&database_url, &schema).await;
    let store = PostgresStore::from_pool(pool.clone());
    store.migrate().await.expect("migrations");

    let workspace_id = WorkspaceId::new();
    let environment_id = EnvironmentId::new();
    sqlx::query("INSERT INTO workspaces (id,name,slug) VALUES ($1,$2,$3)")
        .bind(workspace_id.to_string())
        .bind("Webhooks")
        .bind(format!("webhooks-{}", ulid::Ulid::new()).to_ascii_lowercase())
        .execute(&pool)
        .await
        .expect("workspace");
    sqlx::query(
        "INSERT INTO environments (id,workspace_id,kind,name) VALUES ($1,$2,'live','Live')",
    )
    .bind(environment_id.to_string())
    .bind(workspace_id.to_string())
    .execute(&pool)
    .await
    .expect("environment");

    // Exactly what the dashboard's "Jobs" checkbox submits.
    let family = endpoint(&pool, workspace_id, environment_id, "family", &["job.*"]).await;
    let exact = endpoint(
        &pool,
        workspace_id,
        environment_id,
        "exact",
        &["job.delivery_uncertain"],
    )
    .await;
    let unrelated = endpoint(
        &pool,
        workspace_id,
        environment_id,
        "unrelated",
        &["printer.*"],
    )
    .await;

    store
        .enqueue_webhook_event(
            workspace_id,
            environment_id,
            "job.delivery_uncertain",
            &serde_json::json!({"job_id": "job_test"}),
        )
        .await
        .expect("enqueue");
    let claimed = store
        .claim_webhook_deliveries(25)
        .await
        .expect("materialize subscribed deliveries");
    assert_eq!(claimed.len(), 2, "fan-out remains bounded and exact");

    assert_eq!(
        delivered_to(&pool, &family).await,
        1,
        "a job.* subscription must receive a job event"
    );
    assert_eq!(
        delivered_to(&pool, &exact).await,
        1,
        "an exact subscription must keep working"
    );
    assert_eq!(
        delivered_to(&pool, &unrelated).await,
        0,
        "printer.* must not receive a job event"
    );

    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .expect("drop schema");
}
