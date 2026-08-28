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
use piqae_storage_postgres::{
    PostgresStore, PreHandoffTransitionOutcome,
    destination_topology::{DestinationTopologyRepository, NewDeliveryAttempt, TenantScope},
};
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

async fn seed_topology(
    pool: &PgPool,
    workspace: WorkspaceId,
    environment: EnvironmentId,
    agent: AgentId,
    printer: PrinterId,
) {
    sqlx::query(
        "INSERT INTO physical_destinations (
             workspace_id,environment_id,id,name,state
         ) VALUES ($1,$2,'pdst_capability','Virtual destination','available')",
    )
    .bind(workspace.to_string())
    .bind(environment.to_string())
    .execute(pool)
    .await
    .expect("physical destination");
    sqlx::query(
        "INSERT INTO printer_routes (
             workspace_id,environment_id,id,destination_id,printer_id,agent_id,
             native_queue_id,state,role
         ) VALUES ($1,$2,'rte_capability','pdst_capability',$3,$4,
                   'virtual-capability','available','primary')",
    )
    .bind(workspace.to_string())
    .bind(environment.to_string())
    .bind(printer.to_string())
    .bind(agent.to_string())
    .execute(pool)
    .await
    .expect("printer route");
}

async fn seed_active_attempt(
    pool: &PgPool,
    workspace: WorkspaceId,
    environment: EnvironmentId,
    job_id: JobId,
    suffix: &str,
    state: &str,
) {
    sqlx::query(
        "UPDATE jobs SET state='failed_retryable',destination_id='pdst_capability',
                         route_id='rte_capability',updated_at=now()
         WHERE workspace_id=$1 AND environment_id=$2 AND id=$3",
    )
    .bind(workspace.to_string())
    .bind(environment.to_string())
    .bind(job_id.to_string())
    .execute(pool)
    .await
    .expect("failed-retryable fixture state");
    let attempt_id = format!("attempt_{suffix}");
    let reservation_id = format!("reservation_{suffix}");
    let fence = "f".repeat(64);
    sqlx::query(
        "INSERT INTO delivery_attempts (
             workspace_id,environment_id,id,job_id,destination_id,route_id,
             generation,fencing_token_hash,state,lease_until
         ) VALUES ($1,$2,$3,$4,'pdst_capability','rte_capability',1,$5,$6,
                   now()+interval '1 hour')",
    )
    .bind(workspace.to_string())
    .bind(environment.to_string())
    .bind(&attempt_id)
    .bind(job_id.to_string())
    .bind(&fence)
    .bind(state)
    .execute(pool)
    .await
    .expect("active attempt fixture");
    sqlx::query(
        "INSERT INTO route_reservations (
             workspace_id,environment_id,id,route_id,destination_id,job_id,
             attempt_id,generation,fencing_token_hash,state,lease_until
         ) VALUES ($1,$2,$3,'rte_capability','pdst_capability',$4,$5,1,$6,
                   'active',now()+interval '1 hour')",
    )
    .bind(workspace.to_string())
    .bind(environment.to_string())
    .bind(reservation_id)
    .bind(job_id.to_string())
    .bind(attempt_id)
    .bind(fence)
    .execute(pool)
    .await
    .expect("active reservation fixture");
}

async fn cleanup_active_attempt(
    pool: &PgPool,
    workspace: WorkspaceId,
    environment: EnvironmentId,
    job_id: JobId,
) {
    sqlx::query(
        "UPDATE delivery_attempts SET state='superseded',final_at=now(),updated_at=now()
         WHERE workspace_id=$1 AND environment_id=$2 AND job_id=$3 AND final_at IS NULL",
    )
    .bind(workspace.to_string())
    .bind(environment.to_string())
    .bind(job_id.to_string())
    .execute(pool)
    .await
    .expect("finalize exact test attempt");
    sqlx::query(
        "UPDATE route_reservations
         SET state='superseded',released_at=now(),updated_at=now()
         WHERE workspace_id=$1 AND environment_id=$2 AND job_id=$3 AND state='active'",
    )
    .bind(workspace.to_string())
    .bind(environment.to_string())
    .bind(job_id.to_string())
    .execute(pool)
    .await
    .expect("release exact test reservation");
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

#[tokio::test]
async fn capability_recovery_progress_survives_restart_past_sixteen_jobs() {
    let Some(database_url) = env::var("PIQAE_TEST_DATABASE_URL").ok() else {
        eprintln!("skipped: set PIQAE_TEST_DATABASE_URL for capability scheduling evidence");
        return;
    };
    let schema = format!("piqae_capability_page_{}", ulid::Ulid::new()).to_ascii_lowercase();
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
    let base = Utc::now() - chrono::Duration::minutes(1);
    let mut jobs = Vec::new();
    for index in 0..17 {
        let job = job(
            workspace,
            environment,
            printer,
            &format!("paged {index}"),
            base + chrono::Duration::milliseconds(i64::from(index)),
        );
        store
            .create_job(&job, agent, None, job.title.as_bytes())
            .await
            .expect("paged job");
        jobs.push(job);
    }
    let leases = store
        .claim_jobs(workspace, environment, agent, "virtual-page", 100)
        .await
        .expect("claim paged jobs");
    assert_eq!(leases.len(), 17);
    for lease in leases {
        assert!(matches!(
            store
                .block_agent_lease_for_node_update(
                    workspace,
                    environment,
                    agent,
                    lease.job.id,
                    lease.lease_id,
                    &lease.lease_token,
                )
                .await
                .expect("block paged job"),
            PreHandoffTransitionOutcome::Transitioned(_)
        ));
    }
    let first_page = store
        .list_node_update_required_jobs(workspace, environment, agent, 16)
        .await
        .expect("first recovery page");
    assert_eq!(first_page.len(), 16);
    assert_eq!(first_page[0].id, jobs[0].id);
    drop(store);
    pool.close().await;

    let restarted_pool = schema_pool(&database_url, &schema).await;
    let restarted = PostgresStore::from_pool(restarted_pool.clone());
    let second_page = restarted
        .list_node_update_required_jobs(workspace, environment, agent, 16)
        .await
        .expect("recovery page after restart");
    assert_eq!(
        second_page.iter().map(|job| job.id).collect::<Vec<_>>(),
        vec![jobs[16].id]
    );

    restarted_pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .expect("drop schema");
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn prehandoff_transitions_finalize_only_unaccepted_route_leases() {
    let Some(database_url) = env::var("PIQAE_TEST_DATABASE_URL").ok() else {
        eprintln!("skipped: set PIQAE_TEST_DATABASE_URL for capability scheduling evidence");
        return;
    };
    let schema = format!("piqae_capability_attempt_{}", ulid::Ulid::new()).to_ascii_lowercase();
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
    seed_topology(&pool, workspace, environment, agent, printer).await;
    let scope = TenantScope {
        workspace_id: workspace,
        environment_id: environment,
    };

    let blocked_job = job(
        workspace,
        environment,
        printer,
        "block route lease",
        Utc::now() - chrono::Duration::seconds(3),
    );
    store
        .create_job(&blocked_job, agent, None, b"block route lease")
        .await
        .expect("block job");
    seed_active_attempt(
        &pool,
        workspace,
        environment,
        blocked_job.id,
        "block",
        "route_leased",
    )
    .await;
    let lease = store
        .claim_jobs(workspace, environment, agent, "block-attempt", 1)
        .await
        .expect("claim block job")
        .pop()
        .expect("block lease");
    assert!(matches!(
        store
            .block_agent_lease_for_node_update(
                workspace,
                environment,
                agent,
                blocked_job.id,
                lease.lease_id,
                &lease.lease_token,
            )
            .await
            .expect("block route-leased job"),
        PreHandoffTransitionOutcome::Transitioned(_)
    ));
    let projection: (String, bool, String, bool) = sqlx::query_as(
        "SELECT attempt.state,attempt.final_at IS NOT NULL,
                reservation.state,reservation.released_at IS NOT NULL
         FROM delivery_attempts AS attempt
         JOIN route_reservations AS reservation
           ON reservation.workspace_id=attempt.workspace_id
          AND reservation.environment_id=attempt.environment_id
          AND reservation.attempt_id=attempt.id
         WHERE attempt.job_id=$1",
    )
    .bind(blocked_job.id.to_string())
    .fetch_one(&pool)
    .await
    .expect("blocked delivery projection");
    assert_eq!(
        projection,
        ("superseded".into(), true, "superseded".into(), true)
    );
    let _ = store
        .list_node_update_required_jobs(workspace, environment, agent, 1)
        .await
        .expect("schedule recovery check");
    assert!(
        store
            .recover_node_update_required_job(workspace, environment, agent, blocked_job.id)
            .await
            .expect("recover blocked route lease")
            .is_some()
    );
    let blocked_job_id = blocked_job.id.to_string();
    store
        .begin_delivery_attempt(
            scope,
            NewDeliveryAttempt {
                attempt_id: "attempt_block_recovered",
                reservation_id: "reservation_block_recovered",
                job_id: &blocked_job_id,
                destination_id: "pdst_capability",
                route_id: "rte_capability",
                lease_until: Utc::now() + chrono::Duration::minutes(1),
            },
        )
        .await
        .expect("recovered job can acquire a fresh route fence");
    cleanup_active_attempt(&pool, workspace, environment, blocked_job.id).await;
    sqlx::query(
        "UPDATE jobs SET state='failed_terminal',final_at=now(),updated_at=now()
         WHERE workspace_id=$1 AND environment_id=$2 AND id=$3",
    )
    .bind(workspace.to_string())
    .bind(environment.to_string())
    .bind(blocked_job.id.to_string())
    .execute(&pool)
    .await
    .expect("retire recovered fixture before later scheduling assertions");

    let failed_job = job(
        workspace,
        environment,
        printer,
        "fail route lease",
        Utc::now() - chrono::Duration::seconds(2),
    );
    store
        .create_job(&failed_job, agent, None, b"fail route lease")
        .await
        .expect("failure job");
    seed_active_attempt(
        &pool,
        workspace,
        environment,
        failed_job.id,
        "failure",
        "route_leased",
    )
    .await;
    let lease = store
        .claim_jobs(workspace, environment, agent, "fail-attempt", 1)
        .await
        .expect("claim failure job")
        .pop()
        .expect("failure lease");
    let first_fence = store.fail_agent_lease_before_handoff(
        workspace,
        environment,
        agent,
        failed_job.id,
        lease.lease_id,
        &lease.lease_token,
        JobFailureReason::StockNotLoaded,
        "fresh loaded-stock observation expired",
    );
    let second_fence = store.fail_agent_lease_before_handoff(
        workspace,
        environment,
        agent,
        failed_job.id,
        lease.lease_id,
        &lease.lease_token,
        JobFailureReason::StockNotLoaded,
        "fresh loaded-stock observation expired",
    );
    let (first_fence, second_fence) = tokio::join!(first_fence, second_fence);
    assert_eq!(
        [first_fence.as_ref(), second_fence.as_ref()]
            .into_iter()
            .filter(|outcome| {
                matches!(outcome, Ok(PreHandoffTransitionOutcome::Transitioned(_)))
            })
            .count(),
        1,
        "concurrent media fences must produce exactly one durable transition"
    );
    assert_eq!(
        [first_fence.as_ref(), second_fence.as_ref()]
            .into_iter()
            .filter(Result::is_err)
            .count(),
        1,
        "the stale lease loses the execution-boundary race"
    );
    let projection: (String, bool, String, bool) = sqlx::query_as(
        "SELECT attempt.state,attempt.final_at IS NOT NULL,
                reservation.state,reservation.released_at IS NOT NULL
         FROM delivery_attempts AS attempt
         JOIN route_reservations AS reservation
           ON reservation.attempt_id=attempt.id
          AND reservation.workspace_id=attempt.workspace_id
          AND reservation.environment_id=attempt.environment_id
         WHERE attempt.job_id=$1",
    )
    .bind(failed_job.id.to_string())
    .fetch_one(&pool)
    .await
    .expect("failed delivery projection");
    assert_eq!(
        projection,
        ("superseded".into(), true, "superseded".into(), true)
    );
    let failure_events = store
        .list_job_events(workspace, environment, failed_job.id)
        .await
        .expect("failure events");
    assert_eq!(
        failure_events
            .iter()
            .filter(|event| event.reason == Some(JobFailureReason::StockNotLoaded))
            .count(),
        1
    );
    assert_eq!(failure_events.last().and_then(|event| event.agent_id), None);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM webhook_events
             WHERE workspace_id=$1 AND environment_id=$2
               AND event_type='job.updated' AND payload->>'id'=$3
               AND payload->>'state'='failed_terminal'",
        )
        .bind(workspace.to_string())
        .bind(environment.to_string())
        .bind(failed_job.id.as_ulid().to_string())
        .fetch_one(&pool)
        .await
        .expect("media fence outbox count"),
        1
    );

    for (index, attempt_state) in ["accepted_by_node", "queued_local", "handing_to_spooler"]
        .into_iter()
        .enumerate()
    {
        let unsafe_job = job(
            workspace,
            environment,
            printer,
            &format!("unsafe {attempt_state}"),
            Utc::now() + chrono::Duration::milliseconds(i64::try_from(index).unwrap_or(0)),
        );
        store
            .create_job(&unsafe_job, agent, None, unsafe_job.title.as_bytes())
            .await
            .expect("unsafe job");
        seed_active_attempt(
            &pool,
            workspace,
            environment,
            unsafe_job.id,
            &format!("unsafe_{index}"),
            attempt_state,
        )
        .await;
        let lease = store
            .claim_jobs(workspace, environment, agent, "unsafe-attempt", 1)
            .await
            .expect("claim unsafe job")
            .pop()
            .expect("unsafe lease");
        assert!(matches!(
            store
                .fail_agent_lease_before_handoff(
                    workspace,
                    environment,
                    agent,
                    unsafe_job.id,
                    lease.lease_id,
                    &lease.lease_token,
                    JobFailureReason::TargetConfigurationChanged,
                    "target changed after local acceptance",
                )
                .await
                .expect("classify local responsibility"),
            PreHandoffTransitionOutcome::UnsafeLocalResponsibility
        ));
        assert_eq!(
            store
                .get_job(workspace, environment, unsafe_job.id)
                .await
                .expect("unsafe job unchanged")
                .state,
            JobState::FailedRetryable
        );
        let attempt: (String, bool) =
            sqlx::query_as("SELECT state,final_at IS NULL FROM delivery_attempts WHERE job_id=$1")
                .bind(unsafe_job.id.to_string())
                .fetch_one(&pool)
                .await
                .expect("unsafe attempt unchanged");
        assert_eq!(attempt, (attempt_state.into(), true));
        store
            .release_agent_lease(
                workspace,
                environment,
                agent,
                unsafe_job.id,
                lease.lease_id,
                &lease.lease_token,
            )
            .await
            .expect("release unsafe lease");
        cleanup_active_attempt(&pool, workspace, environment, unsafe_job.id).await;
        sqlx::query(
            "UPDATE jobs SET state='failed_terminal',final_at=now(),updated_at=now()
             WHERE workspace_id=$1 AND environment_id=$2 AND id=$3",
        )
        .bind(workspace.to_string())
        .bind(environment.to_string())
        .bind(unsafe_job.id.to_string())
        .execute(&pool)
        .await
        .expect("retire exact unsafe fixture");
    }

    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .expect("drop schema");
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn expiry_is_bounded_restart_safe_and_preserves_local_responsibility() {
    let Some(database_url) = env::var("PIQAE_TEST_DATABASE_URL").ok() else {
        eprintln!("skipped: set PIQAE_TEST_DATABASE_URL for expiry evidence");
        return;
    };
    let schema = format!("piqae_safe_expiry_{}", ulid::Ulid::new()).to_ascii_lowercase();
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
    seed_topology(&pool, workspace, environment, agent, printer).await;
    let base = Utc::now() - chrono::Duration::minutes(2);

    let mut safe = job(workspace, environment, printer, "safe expiry", base);
    safe.expires_at = base + chrono::Duration::seconds(1);
    store
        .create_job(&safe, agent, None, b"safe expiry")
        .await
        .expect("safe job");
    seed_active_attempt(
        &pool,
        workspace,
        environment,
        safe.id,
        "safe_expiry",
        "route_leased",
    )
    .await;

    let mut unsafe_job = job(
        workspace,
        environment,
        printer,
        "unsafe expiry",
        base + chrono::Duration::seconds(2),
    );
    unsafe_job.expires_at = base + chrono::Duration::seconds(3);
    store
        .create_job(&unsafe_job, agent, None, b"unsafe expiry")
        .await
        .expect("unsafe job");
    let mut blocked = job(
        workspace,
        environment,
        printer,
        "blocked expiry",
        base + chrono::Duration::seconds(4),
    );
    blocked.expires_at = Utc::now() + chrono::Duration::minutes(5);
    store
        .create_job(&blocked, agent, None, b"blocked expiry")
        .await
        .expect("blocked job");
    let lease = store
        .claim_jobs(workspace, environment, agent, "expiry-block", 1)
        .await
        .expect("blocked lease")
        .pop()
        .expect("blocked lease exists");
    assert_eq!(lease.job.id, blocked.id);
    assert!(matches!(
        store
            .block_agent_lease_for_node_update(
                workspace,
                environment,
                agent,
                blocked.id,
                lease.lease_id,
                &lease.lease_token,
            )
            .await
            .expect("capability block"),
        PreHandoffTransitionOutcome::Transitioned(_)
    ));
    let blocked_expiry = base + chrono::Duration::seconds(5);
    sqlx::query(
        "UPDATE jobs SET expires_at=$4,
             payload=jsonb_set(payload,'{expires_at}',to_jsonb($5::text))
         WHERE workspace_id=$1 AND environment_id=$2 AND id=$3",
    )
    .bind(workspace.to_string())
    .bind(environment.to_string())
    .bind(blocked.id.to_string())
    .bind(blocked_expiry)
    .bind(blocked_expiry.to_rfc3339())
    .execute(&pool)
    .await
    .expect("age capability block");

    let first = store
        .expire_jobs_before_handoff(1)
        .await
        .expect("first bounded expiry");
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].transition.job.id, safe.id);
    assert!(matches!(
        store
            .begin_delivery_attempt(
                TenantScope {
                    workspace_id: workspace,
                    environment_id: environment,
                },
                NewDeliveryAttempt {
                    attempt_id: "attempt_stale_after_expiry",
                    reservation_id: "reservation_stale_after_expiry",
                    job_id: &safe.id.to_string(),
                    destination_id: "pdst_capability",
                    route_id: "rte_capability",
                    lease_until: Utc::now() + chrono::Duration::minutes(1),
                },
            )
            .await,
        Err(piqae_storage_postgres::StorageError::ConcurrentStateChange)
    ));
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM delivery_attempts
             WHERE workspace_id=$1 AND environment_id=$2 AND job_id=$3",
        )
        .bind(workspace.to_string())
        .bind(environment.to_string())
        .bind(safe.id.to_string())
        .fetch_one(&pool)
        .await
        .expect("stale scheduler attempt count"),
        1,
        "expiry's superseded attempt must not be followed by a stale route lease"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM route_reservations
             WHERE workspace_id=$1 AND environment_id=$2 AND job_id=$3
               AND state='active'",
        )
        .bind(workspace.to_string())
        .bind(environment.to_string())
        .bind(safe.id.to_string())
        .fetch_one(&pool)
        .await
        .expect("stale scheduler reservation count"),
        0
    );
    seed_active_attempt(
        &pool,
        workspace,
        environment,
        unsafe_job.id,
        "unsafe_expiry",
        "accepted_by_node",
    )
    .await;
    assert_eq!(
        store
            .get_job(workspace, environment, unsafe_job.id)
            .await
            .expect("unsafe job unchanged")
            .state,
        JobState::FailedRetryable
    );
    let unsafe_attempt: (String, bool) =
        sqlx::query_as("SELECT state,final_at IS NULL FROM delivery_attempts WHERE job_id=$1")
            .bind(unsafe_job.id.to_string())
            .fetch_one(&pool)
            .await
            .expect("unsafe attempt remains fenced");
    assert_eq!(unsafe_attempt, ("accepted_by_node".into(), true));
    drop(store);
    pool.close().await;

    let restarted_pool = schema_pool(&database_url, &schema).await;
    let restarted = PostgresStore::from_pool(restarted_pool.clone());
    let second = restarted
        .expire_jobs_before_handoff(1)
        .await
        .expect("expiry after restart");
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].transition.job.id, blocked.id);
    assert!(
        restarted
            .expire_jobs_before_handoff(100)
            .await
            .expect("idempotent expiry")
            .is_empty()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM job_events
             WHERE workspace_id=$1 AND environment_id=$2
               AND state='expired' AND job_id=ANY($3)",
        )
        .bind(workspace.to_string())
        .bind(environment.to_string())
        .bind(vec![safe.id.to_string(), blocked.id.to_string()])
        .fetch_one(&restarted_pool)
        .await
        .expect("expired lifecycle events"),
        2
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM webhook_events
             WHERE workspace_id=$1 AND environment_id=$2
               AND event_type='job.updated' AND payload->>'state'='expired'",
        )
        .bind(workspace.to_string())
        .bind(environment.to_string())
        .fetch_one(&restarted_pool)
        .await
        .expect("expiry outbox events"),
        2
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM node_capability_recovery_checks
             WHERE workspace_id=$1 AND environment_id=$2 AND job_id=$3",
        )
        .bind(workspace.to_string())
        .bind(environment.to_string())
        .bind(blocked.id.to_string())
        .fetch_one(&restarted_pool)
        .await
        .expect("capability recovery cleanup"),
        0
    );
    let safe_projection: (String, bool, String, bool) = sqlx::query_as(
        "SELECT attempt.state,attempt.final_at IS NOT NULL,
                reservation.state,reservation.released_at IS NOT NULL
         FROM delivery_attempts AS attempt
         JOIN route_reservations AS reservation
           ON reservation.attempt_id=attempt.id
          AND reservation.workspace_id=attempt.workspace_id
          AND reservation.environment_id=attempt.environment_id
         WHERE attempt.job_id=$1",
    )
    .bind(safe.id.to_string())
    .fetch_one(&restarted_pool)
    .await
    .expect("safe expiry route cleanup");
    assert_eq!(
        safe_projection,
        ("superseded".into(), true, "superseded".into(), true)
    );

    restarted_pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .expect("drop schema");
}
