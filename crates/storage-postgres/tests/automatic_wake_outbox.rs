#![allow(clippy::expect_used)]

use piqae_domain::{AgentId, EnvironmentId, JobId, PrinterId, WorkspaceId};
use piqae_storage_postgres::PostgresStore;
use piqae_storage_postgres::destination_topology::{DestinationTopologyRepository, TenantScope};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{env, time::Duration};

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
#[allow(
    clippy::too_many_lines,
    reason = "one disposable-schema lifecycle proves persistence, retry identity, repair, and cleanup together"
)]
async fn postgres_wake_outbox_is_idempotent_content_free_and_at_least_once() {
    let Some(database_url) = env::var("PIQAE_TEST_DATABASE_URL").ok() else {
        eprintln!("skipped: set PIQAE_TEST_DATABASE_URL for automatic wake evidence");
        return;
    };
    let schema = format!("piqae_auto_wake_{}", ulid::Ulid::new()).to_ascii_lowercase();
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .expect("connect test PostgreSQL");
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .expect("create exact disposable schema");
    let pool = schema_pool(&database_url, &schema).await;
    let store = PostgresStore::from_pool(pool.clone());
    store.migrate().await.expect("migrate empty database");

    let workspace_id = WorkspaceId::new();
    let environment_id = EnvironmentId::new();
    store
        .ensure_bootstrap_tenant(workspace_id, environment_id)
        .await
        .expect("bootstrap tenant");
    let assigned_agent = AgentId::new();
    let standby_agent = AgentId::new();
    let paused_agent = AgentId::new();
    let assigned_printer = PrinterId::new();
    let standby_printer = PrinterId::new();
    let paused_printer = PrinterId::new();
    for (agent_id, installation) in [
        (assigned_agent, "automatic-wake-primary"),
        (standby_agent, "automatic-wake-standby"),
        (paused_agent, "automatic-wake-paused"),
    ] {
        sqlx::query("INSERT INTO agents (id,workspace_id,environment_id,name,installation_id,os,architecture,version,protocol_version) VALUES ($1,$2,$3,'Node',$4,'linux','x86_64','test',1)")
            .bind(agent_id.to_string()).bind(workspace_id.to_string()).bind(environment_id.to_string()).bind(installation)
            .execute(&pool).await.expect("insert agent");
    }
    for (printer_id, agent_id, native_id) in [
        (assigned_printer, assigned_agent, "wake-primary"),
        (standby_printer, standby_agent, "wake-standby"),
        (paused_printer, paused_agent, "wake-paused"),
    ] {
        sqlx::query("INSERT INTO printers (id,workspace_id,environment_id,agent_id,native_id,name,state) VALUES ($1,$2,$3,$4,$5,'Printer','online')")
            .bind(printer_id.to_string()).bind(workspace_id.to_string()).bind(environment_id.to_string()).bind(agent_id.to_string()).bind(native_id)
            .execute(&pool).await.expect("insert printer");
    }
    let destination_id = format!("pdst_{}", ulid::Ulid::new());
    sqlx::query("INSERT INTO physical_destinations (workspace_id,environment_id,id,name,state) VALUES ($1,$2,$3,'Wake destination','available')")
        .bind(workspace_id.to_string()).bind(environment_id.to_string()).bind(&destination_id)
        .execute(&pool).await.expect("insert destination");
    for (index, (printer_id, agent_id, native_id, role, priority)) in [
        (
            assigned_printer,
            assigned_agent,
            "wake-primary",
            "primary",
            0_i32,
        ),
        (
            standby_printer,
            standby_agent,
            "wake-standby",
            "standby",
            100_i32,
        ),
        (
            paused_printer,
            paused_agent,
            "wake-paused",
            "standby",
            200_i32,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let route_state = if agent_id == paused_agent {
            "paused"
        } else {
            "available"
        };
        sqlx::query("INSERT INTO printer_routes (workspace_id,environment_id,id,destination_id,printer_id,agent_id,native_queue_id,state,role,priority,enabled) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,true)")
            .bind(workspace_id.to_string()).bind(environment_id.to_string()).bind(format!("rte_wake_{index}")).bind(&destination_id)
            .bind(printer_id.to_string()).bind(agent_id.to_string()).bind(native_id).bind(route_state).bind(role).bind(priority)
            .execute(&pool).await.expect("insert route");
    }

    let first_job = JobId::new();
    sqlx::query("INSERT INTO jobs (id,workspace_id,environment_id,printer_id,agent_id,payload,state,state_sequence,per_printer_sequence,expires_at,destination_id,route_id) VALUES ($1,$2,$3,$4,$5,'{}','waiting_for_agent',2,1,now()+interval '1 hour',$6,'rte_wake_0')")
        .bind(first_job.to_string()).bind(workspace_id.to_string()).bind(environment_id.to_string()).bind(assigned_printer.to_string()).bind(assigned_agent.to_string()).bind(&destination_id)
        .execute(&pool).await.expect("insert waiting job");
    assert_eq!(
        store
            .ensure_waiting_job_wake_hints(workspace_id, environment_id, first_job)
            .await
            .expect("ensure wake hints"),
        2
    );
    assert_eq!(
        store
            .ensure_waiting_job_wake_hints(workspace_id, environment_id, first_job)
            .await
            .expect("idempotent ensure"),
        2
    );
    let hint_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM node_wake_hints WHERE workspace_id=$1 AND environment_id=$2",
    )
    .bind(workspace_id.to_string())
    .bind(environment_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("count hints");
    assert_eq!(hint_count, 2);
    let paused_hint_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM node_wake_hints
         WHERE workspace_id=$1 AND environment_id=$2 AND agent_id=$3",
    )
    .bind(workspace_id.to_string())
    .bind(environment_id.to_string())
    .bind(paused_agent.to_string())
    .fetch_one(&pool)
    .await
    .expect("count excluded paused-route hints");
    assert_eq!(paused_hint_count, 0, "paused routes must not be woken");
    let payloads = sqlx::query_scalar::<_, serde_json::Value>(
        "SELECT payload FROM routing_outbox WHERE aggregate_type='node_wake_hint' ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .expect("read wake outbox payloads");
    assert_eq!(payloads.len(), 2);
    for payload in &payloads {
        let object = payload.as_object().expect("content-free payload object");
        assert_eq!(
            object.get("reason"),
            Some(&serde_json::json!("job_available"))
        );
        assert_eq!(
            object.get("delivery_channel"),
            Some(&serde_json::json!("external_push"))
        );
        assert!(!object.contains_key("job_id"));
        assert!(!object.contains_key("title"));
        assert!(!object.contains_key("content"));
    }

    let first_claim = store
        .claim_wake_hint_dispatches(10)
        .await
        .expect("claim wake dispatches");
    assert_eq!(first_claim.len(), 2);
    let first_ids = first_claim
        .iter()
        .map(|item| item.hint.id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    for item in &first_claim {
        store
            .retry_wake_hint_dispatch(&item.outbox_id, Duration::ZERO)
            .await
            .expect("make crashed dispatch immediately retryable");
    }
    let retry_claim = store
        .claim_wake_hint_dispatches(10)
        .await
        .expect("reclaim wake dispatches");
    assert_eq!(
        retry_claim
            .iter()
            .map(|item| item.hint.id.clone())
            .collect::<std::collections::BTreeSet<_>>(),
        first_ids,
        "at-least-once delivery must preserve the opaque hint IDs"
    );
    assert!(retry_claim.iter().all(|item| item.attempt == 2));
    for item in &retry_claim {
        store
            .complete_wake_hint_dispatch(&item.outbox_id)
            .await
            .expect("complete dispatch");
    }

    let repair_job = JobId::new();
    sqlx::query("INSERT INTO jobs (id,workspace_id,environment_id,printer_id,agent_id,payload,state,state_sequence,per_printer_sequence,expires_at,destination_id,route_id) VALUES ($1,$2,$3,$4,$5,'{}','waiting_for_agent',2,2,now()+interval '1 hour',$6,'rte_wake_0')")
        .bind(repair_job.to_string()).bind(workspace_id.to_string()).bind(environment_id.to_string()).bind(assigned_printer.to_string()).bind(assigned_agent.to_string()).bind(&destination_id)
        .execute(&pool).await.expect("insert N-1 waiting job");
    assert_eq!(
        store
            .repair_waiting_job_wake_hints(10)
            .await
            .expect("repair N-1 waiting transition"),
        2
    );

    let scope = TenantScope {
        workspace_id,
        environment_id,
    };
    let observed = store
        .observe_pending_node_wake_hints(scope, &standby_agent.to_string(), chrono::Utc::now(), 10)
        .await
        .expect("observe standby wake before dispatch");
    assert!(!observed.is_empty());

    store
        .revoke_agent(workspace_id, environment_id, assigned_agent)
        .await
        .expect("revoke assigned agent");
    let no_candidate_destination = format!("pdst_{}", ulid::Ulid::new());
    sqlx::query("INSERT INTO physical_destinations (workspace_id,environment_id,id,name,state) VALUES ($1,$2,$3,'No wake candidate','unavailable')")
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .bind(&no_candidate_destination)
        .execute(&pool)
        .await
        .expect("insert destination without active routes");
    let no_candidate_job = JobId::new();
    sqlx::query("INSERT INTO jobs (id,workspace_id,environment_id,printer_id,agent_id,payload,state,state_sequence,per_printer_sequence,expires_at,destination_id,route_id) VALUES ($1,$2,$3,$4,$5,'{}','waiting_for_agent',2,3,now()+interval '1 hour',$6,NULL)")
        .bind(no_candidate_job.to_string())
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .bind(assigned_printer.to_string())
        .bind(assigned_agent.to_string())
        .bind(&no_candidate_destination)
        .execute(&pool)
        .await
        .expect("insert waiting job with revoked assignment");
    assert_eq!(
        store
            .ensure_waiting_job_wake_hints(workspace_id, environment_id, no_candidate_job,)
            .await
            .expect("reconcile zero-candidate job"),
        0,
        "a revoked assignment must not receive a wake without an active destination route"
    );
    let reconciled_candidates: i32 = sqlx::query_scalar(
        "SELECT candidate_count FROM job_wake_reconciliations
         WHERE workspace_id=$1 AND environment_id=$2 AND job_id=$3",
    )
    .bind(workspace_id.to_string())
    .bind(environment_id.to_string())
    .bind(no_candidate_job.to_string())
    .fetch_one(&pool)
    .await
    .expect("read zero-candidate reconciliation");
    assert_eq!(reconciled_candidates, 0);
    assert_eq!(
        store
            .repair_waiting_job_wake_hints(10)
            .await
            .expect("repeat repair after zero-candidate reconciliation"),
        0,
        "durable zero-candidate reconciliation must prevent a repair hot-loop"
    );
    assert_eq!(
        store
            .claim_wake_hint_dispatches(10)
            .await
            .expect("claim repaired dispatches")
            .len(),
        0,
        "observed and cancelled wake hints must never dispatch"
    );
    let unprocessed_terminal_outbox: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM routing_outbox outbox
         JOIN node_wake_hints hint
           ON hint.workspace_id=outbox.workspace_id
          AND hint.environment_id=outbox.environment_id
          AND hint.id=outbox.aggregate_id
         WHERE outbox.aggregate_type='node_wake_hint'
           AND hint.status<>'pending' AND outbox.processed_at IS NULL",
    )
    .fetch_one(&pool)
    .await
    .expect("terminal wake outbox count");
    assert_eq!(unprocessed_terminal_outbox, 0);

    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .expect("drop exact disposable schema");
}
