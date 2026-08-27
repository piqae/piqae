#![allow(clippy::expect_used)]

//! `PostgreSQL` evidence for capability-aware scheduling. No executor or
//! physical printer is used: the test exercises only durable queue state,
//! leases, lifecycle events, tenant outbox rows, and route-fence absence.

use std::{collections::BTreeMap, env};

use chrono::{DateTime, Utc};
use piqae_domain::{
    AgentId, ContentKind, ContentSource, EnvironmentId, Job, JobFailureReason, JobId, JobOptions,
    JobState, PrinterId, WorkspaceId,
};
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

async fn seed_tenant(pool: &PgPool) -> (WorkspaceId, EnvironmentId, AgentId, PrinterId) {
    let workspace = WorkspaceId::new();
    let environment = EnvironmentId::new();
    let agent = AgentId::new();
    let printer = PrinterId::new();
    sqlx::query("INSERT INTO workspaces (id,name,slug) VALUES ($1,'Scheduling',$2)")
        .bind(workspace.to_string())
        .bind(format!("scheduling-{}", ulid::Ulid::new()).to_ascii_lowercase())
        .execute(pool)
        .await
        .expect("workspace");
    sqlx::query(
        "INSERT INTO environments (id,workspace_id,kind,name) VALUES ($1,$2,'live','Live')",
    )
    .bind(environment.to_string())
    .bind(workspace.to_string())
    .execute(pool)
    .await
    .expect("environment");
    sqlx::query(
        "INSERT INTO agents (
            id,workspace_id,environment_id,name,installation_id,public_key,
            os,architecture,version,protocol_version,state,last_seen_at
         ) VALUES ($1,$2,$3,'Virtual node',$4,$5,'test','test','0.1.0',1,'connected',now())",
    )
    .bind(agent.to_string())
    .bind(workspace.to_string())
    .bind(environment.to_string())
    .bind(format!("scheduling-installation-{}", ulid::Ulid::new()))
    .bind(vec![7_u8; 32])
    .execute(pool)
    .await
    .expect("agent");
    sqlx::query(
        "INSERT INTO printers (id,workspace_id,environment_id,agent_id,native_id,name)
         VALUES ($1,$2,$3,$4,$5,'Virtual printer')",
    )
    .bind(printer.to_string())
    .bind(workspace.to_string())
    .bind(environment.to_string())
    .bind(agent.to_string())
    .bind(format!("scheduling-printer-{}", ulid::Ulid::new()))
    .execute(pool)
    .await
    .expect("printer");
    (workspace, environment, agent, printer)
}

fn job(
    workspace: WorkspaceId,
    environment: EnvironmentId,
    printer: PrinterId,
    title: &str,
    created_at: DateTime<Utc>,
) -> Job {
    Job {
        id: JobId::new(),
        workspace_id: workspace,
        environment_id: environment,
        printer_id: printer,
        title: title.into(),
        source: None,
        content_kind: ContentKind::Raw,
        content: ContentSource::Base64 {
            data: "G0BmaXh0dXJl".into(),
        },
        options: JobOptions::default(),
        metadata: BTreeMap::new(),
        deliveries: 1,
        state: JobState::WaitingForAgent,
        created_at,
        expires_at: Utc::now() + chrono::Duration::hours(1),
        delivery_uncertain_since: None,
    }
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn capability_block_and_recovery_are_atomic_fair_and_tenant_scoped() {
    let Some(database_url) = env::var("PIQAE_TEST_DATABASE_URL").ok() else {
        eprintln!("skipped: set PIQAE_TEST_DATABASE_URL for capability scheduling evidence");
        return;
    };
    let schema = format!("piqae_capability_schedule_{}", ulid::Ulid::new()).to_ascii_lowercase();
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
    let (workspace, environment, agent, printer) = seed_tenant(&pool).await;
    let other_workspace = WorkspaceId::new();
    let oldest = job(
        workspace,
        environment,
        printer,
        "old incompatible",
        Utc::now() - chrono::Duration::seconds(2),
    );
    let later = job(
        workspace,
        environment,
        printer,
        "later compatible",
        Utc::now() - chrono::Duration::seconds(1),
    );
    store
        .create_job(&oldest, agent, None, b"old incompatible")
        .await
        .expect("oldest job");
    store
        .create_job(&later, agent, None, b"later compatible")
        .await
        .expect("later job");

    let leases = store
        .claim_jobs(workspace, environment, agent, "virtual-sync", 16)
        .await
        .expect("ordered leases");
    assert_eq!(leases.len(), 2);
    assert_eq!(leases[0].job.id, oldest.id);
    assert_eq!(leases[1].job.id, later.id);
    store
        .block_agent_lease_for_node_update(
            workspace,
            environment,
            agent,
            oldest.id,
            leases[0].lease_id,
            &leases[0].lease_token,
        )
        .await
        .expect("block oldest lease");
    assert!(
        store
            .block_agent_lease_for_node_update(
                workspace,
                environment,
                agent,
                oldest.id,
                leases[0].lease_id,
                &leases[0].lease_token,
            )
            .await
            .is_err(),
        "a replayed lease must not append another event"
    );
    let events = store
        .list_job_events(workspace, environment, oldest.id)
        .await
        .expect("job events");
    assert_eq!(
        events
            .iter()
            .filter(|event| event.reason == Some(JobFailureReason::NodeUpdateRequired))
            .count(),
        1
    );
    assert_eq!(
        events.last().and_then(|event| event.agent_id),
        None,
        "automatic recovery must be limited to server-authored capability blocks"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM webhook_events
             WHERE workspace_id=$1 AND environment_id=$2
               AND event_type='job.updated' AND payload->>'id'=$3 AND payload->>'state'='blocked'",
        )
        .bind(workspace.to_string())
        .bind(environment.to_string())
        .bind(oldest.id.as_ulid().to_string())
        .fetch_one(&pool)
        .await
        .expect("blocked outbox count"),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM route_reservations
             WHERE workspace_id=$1 AND environment_id=$2 AND job_id=$3",
        )
        .bind(workspace.to_string())
        .bind(environment.to_string())
        .bind(oldest.id.to_string())
        .fetch_one(&pool)
        .await
        .expect("route leak count"),
        0
    );
    assert_eq!(
        store
            .list_node_update_required_jobs(workspace, environment, agent, 1)
            .await
            .expect("blocked list")
            .iter()
            .map(|job| job.id)
            .collect::<Vec<_>>(),
        vec![oldest.id]
    );
    assert!(
        store
            .list_node_update_required_jobs(other_workspace, environment, agent, 16)
            .await
            .expect("cross-tenant blocked list")
            .is_empty()
    );
    assert!(
        store
            .recover_node_update_required_job(workspace, environment, agent, oldest.id)
            .await
            .expect("recover blocked job")
            .is_some()
    );
    assert!(
        store
            .recover_node_update_required_job(workspace, environment, agent, oldest.id)
            .await
            .expect("idempotent recovery")
            .is_none()
    );
    assert_eq!(
        store
            .get_job(workspace, environment, oldest.id)
            .await
            .expect("recovered job")
            .state,
        JobState::WaitingForAgent
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM webhook_events
             WHERE workspace_id=$1 AND environment_id=$2
               AND event_type='job.updated' AND payload->>'id'=$3",
        )
        .bind(workspace.to_string())
        .bind(environment.to_string())
        .bind(oldest.id.as_ulid().to_string())
        .fetch_one(&pool)
        .await
        .expect("transition outbox count"),
        2
    );
    // The later compatible lease remains valid while the oldest job is
    // blocked, proving the head-of-line transition does not consume it.
    store
        .validate_agent_lease(
            workspace,
            environment,
            agent,
            later.id,
            leases[1].lease_id,
            &leases[1].lease_token,
        )
        .await
        .expect("later lease remains dispatchable");

    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .expect("drop schema");
}
