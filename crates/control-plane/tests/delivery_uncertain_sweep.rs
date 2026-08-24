#![allow(clippy::expect_used)]

//! Uncertain delivery is the one print outcome that cannot be proved either
//! way. Entering the state is unremarkable and often transient; staying in it
//! is the fault a human has to resolve. These tests pin the two properties the
//! sweep exists for: it ignores jobs inside the threshold, and it surfaces a
//! stuck job exactly once rather than on every pass.

use std::{env, time::Duration};

use piqae_domain::{AgentId, EnvironmentId, PrinterId, WorkspaceId};
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

/// Inserts a job already in `delivery_uncertain`, anchored `age_seconds` ago.
async fn insert_uncertain_job(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    environment_id: EnvironmentId,
    printer_id: PrinterId,
    agent_id: AgentId,
    age_seconds: i32,
) -> String {
    let job_id = piqae_domain::JobId::new().to_string();
    sqlx::query(
        "INSERT INTO jobs (
            id, workspace_id, environment_id, printer_id, agent_id, state,
            state_sequence, per_printer_sequence, payload, expires_at,
            created_at, updated_at, delivery_uncertain_since
         ) VALUES ($1,$2,$3,$4,$5,'delivery_uncertain',1,
                   (SELECT COALESCE(MAX(per_printer_sequence),0)+1
                      FROM jobs WHERE printer_id = $4),
                   '{}'::jsonb, now() + interval '1 day', now(), now(),
                   now() - make_interval(secs => $6::double precision))",
    )
    .bind(&job_id)
    .bind(workspace_id.to_string())
    .bind(environment_id.to_string())
    .bind(printer_id.to_string())
    .bind(agent_id.to_string())
    .bind(f64::from(age_seconds))
    .execute(pool)
    .await
    .expect("insert uncertain job");
    job_id
}

/// Creates the workspace, environment, node and printer a job row needs.
async fn seed_tenant(pool: &PgPool) -> (WorkspaceId, EnvironmentId, PrinterId, AgentId) {
    let workspace_id = WorkspaceId::new();
    let environment_id = EnvironmentId::new();
    let agent_id = AgentId::new();
    let printer_id = PrinterId::new();
    sqlx::query("INSERT INTO workspaces (id,name,slug) VALUES ($1,$2,$3)")
        .bind(workspace_id.to_string())
        .bind("Uncertain")
        .bind(format!("uncertain-{}", ulid::Ulid::new()).to_ascii_lowercase())
        .execute(pool)
        .await
        .expect("workspace");
    sqlx::query(
        "INSERT INTO environments (id,workspace_id,kind,name) VALUES ($1,$2,'live','Live')",
    )
    .bind(environment_id.to_string())
    .bind(workspace_id.to_string())
    .execute(pool)
    .await
    .expect("environment");
    sqlx::query(
        "INSERT INTO agents (
            id,workspace_id,environment_id,name,installation_id,public_key,
            os,architecture,version,protocol_version,state,last_seen_at
         ) VALUES ($1,$2,$3,'Uncertain node',$4,$5,'test','test','0.1.0',1,'connected',now())",
    )
    .bind(agent_id.to_string())
    .bind(workspace_id.to_string())
    .bind(environment_id.to_string())
    .bind(format!("uncertain-installation-{}", ulid::Ulid::new()))
    .bind(vec![7_u8; 32])
    .execute(pool)
    .await
    .expect("agent");
    sqlx::query(
        "INSERT INTO printers (
            id,workspace_id,environment_id,agent_id,native_id,name
         ) VALUES ($1,$2,$3,$4,$5,'Uncertain printer')",
    )
    .bind(printer_id.to_string())
    .bind(workspace_id.to_string())
    .bind(environment_id.to_string())
    .bind(agent_id.to_string())
    .bind(format!("uncertain-printer-{}", ulid::Ulid::new()))
    .execute(pool)
    .await
    .expect("printer");
    (workspace_id, environment_id, printer_id, agent_id)
}

#[tokio::test]
async fn the_sweep_surfaces_a_stuck_job_once_and_leaves_recent_ones_alone() {
    let Some(database_url) = env::var("PIQAE_TEST_DATABASE_URL").ok() else {
        eprintln!("skipped: set PIQAE_TEST_DATABASE_URL to run the uncertain delivery sweep");
        return;
    };
    let schema = format!("piqae_uncertain_{}", ulid::Ulid::new()).to_ascii_lowercase();
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

    let (workspace_id, environment_id, printer_id, agent_id) = seed_tenant(&pool).await;

    let stuck = insert_uncertain_job(
        &pool,
        workspace_id,
        environment_id,
        printer_id,
        agent_id,
        3_600,
    )
    .await;
    let recent = insert_uncertain_job(
        &pool,
        workspace_id,
        environment_id,
        printer_id,
        agent_id,
        30,
    )
    .await;

    let threshold = Duration::from_secs(900);

    // A job inside the threshold is normal and must not be surfaced.
    let first = store
        .claim_stuck_uncertain_jobs(threshold, 50)
        .await
        .expect("first sweep");
    let ids: Vec<String> = first.iter().map(|job| job.job_id.to_string()).collect();
    assert!(ids.contains(&stuck), "the stuck job must be surfaced");
    assert!(
        !ids.contains(&recent),
        "a job still inside the threshold must not be surfaced"
    );

    // The whole point of the alert fence: a second pass must stay silent, or a
    // periodic sweep re-reports the same job forever and gets ignored.
    let second = store
        .claim_stuck_uncertain_jobs(threshold, 50)
        .await
        .expect("second sweep");
    assert!(
        second.is_empty(),
        "an already-surfaced job must not be reported again, got {second:?}"
    );

    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .expect("drop schema");
}
