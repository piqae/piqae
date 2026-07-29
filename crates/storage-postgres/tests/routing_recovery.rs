#![allow(clippy::expect_used, clippy::too_many_lines)]

use chrono::Utc;
use sha2::{Digest, Sha256};
use spool_domain::{
    AgentId, ContentKind, ContentSource, EnvironmentId, Job, JobId, JobOptions, JobState,
    PrinterCapabilities, PrinterId, WorkspaceId,
};
use spool_storage_postgres::{PostgresStore, PrinterProfileSnapshot, StoredTargetBinding};
use sqlx::{PgPool, Row, postgres::PgPoolOptions};
use std::{collections::BTreeMap, env};

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

async fn insert_fixture(
    pool: &PgPool,
) -> (
    WorkspaceId,
    EnvironmentId,
    AgentId,
    AgentId,
    PrinterId,
    PrinterId,
    StoredTargetBinding,
) {
    let workspace_id = WorkspaceId::new();
    let environment_id = EnvironmentId::new();
    let primary_agent = AgentId::new();
    let standby_agent = AgentId::new();
    let primary_printer = PrinterId::new();
    let standby_printer = PrinterId::new();
    sqlx::query(
        "INSERT INTO workspaces (id, name, slug)
         VALUES ($1, 'Routing test', $2)",
    )
    .bind(workspace_id.to_string())
    .bind(format!("routing-{}", ulid::Ulid::new()).to_ascii_lowercase())
    .execute(pool)
    .await
    .expect("workspace fixture");
    sqlx::query(
        "INSERT INTO environments (id, workspace_id, kind, name)
         VALUES ($1, $2, 'test', 'Test')",
    )
    .bind(environment_id.to_string())
    .bind(workspace_id.to_string())
    .execute(pool)
    .await
    .expect("environment fixture");
    for (agent, installation) in [
        (primary_agent, "routing-primary"),
        (standby_agent, "routing-standby"),
    ] {
        sqlx::query(
            "INSERT INTO agents (
                id, workspace_id, environment_id, name, installation_id,
                os, architecture, version, protocol_version, state, last_seen_at
             ) VALUES ($1,$2,$3,$4,$5,'test','test','0.1.0',1,'connected',now())",
        )
        .bind(agent.to_string())
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .bind(installation)
        .bind(installation)
        .execute(pool)
        .await
        .expect("agent fixture");
    }
    let profiles = serde_json::to_value(vec![PrinterProfileSnapshot {
        profile_id: "profile_shipping".into(),
        revision: 4,
        name: "Shipping".into(),
        is_default: true,
        options: JobOptions::default(),
        status: Some("ready".into()),
        native_kind: None,
        native_digest: Some("sha256:routing-test".into()),
        driver_fingerprint: None,
        summary: None,
        stock_id: None,
        safe_overrides: Vec::new(),
        last_validated_at: None,
        last_test_job_id: None,
        published: true,
    }])
    .expect("profile JSON");
    for (printer, agent, native_id) in [
        (primary_printer, primary_agent, "primary"),
        (standby_printer, standby_agent, "standby"),
    ] {
        sqlx::query(
            "INSERT INTO printers (
                id, workspace_id, environment_id, agent_id, native_id, name,
                state, capabilities, profiles, last_seen_at
             ) VALUES ($1,$2,$3,$4,$5,$5,'online',$6,$7,now())",
        )
        .bind(printer.to_string())
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .bind(agent.to_string())
        .bind(native_id)
        .bind(serde_json::to_value(PrinterCapabilities::default()).expect("capabilities JSON"))
        .bind(&profiles)
        .execute(pool)
        .await
        .expect("printer fixture");
    }
    sqlx::query(
        "INSERT INTO targets (
            id, workspace_id, environment_id, name, enabled, routing_policy
         ) VALUES ('tgt_recovery',$1,$2,'Recovery target',true,'primary_then_standby')",
    )
    .bind(workspace_id.to_string())
    .bind(environment_id.to_string())
    .execute(pool)
    .await
    .expect("target fixture");
    for (id, printer, agent, role) in [
        ("tgb_primary", primary_printer, primary_agent, "primary"),
        ("tgb_standby", standby_printer, standby_agent, "standby"),
    ] {
        sqlx::query(
            "INSERT INTO target_bindings (
                id, workspace_id, environment_id, target_id, printer_id,
                agent_id, profile_id, profile_revision, role, enabled
             ) VALUES ($1,$2,$3,'tgt_recovery',$4,$5,'profile_shipping',4,$6,true)",
        )
        .bind(id)
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .bind(printer.to_string())
        .bind(agent.to_string())
        .bind(role)
        .execute(pool)
        .await
        .expect("binding fixture");
    }
    (
        workspace_id,
        environment_id,
        primary_agent,
        standby_agent,
        primary_printer,
        standby_printer,
        StoredTargetBinding {
            id: "tgb_standby".into(),
            target_id: "tgt_recovery".into(),
            printer_id: standby_printer,
            agent_id: standby_agent,
            profile_id: "profile_shipping".into(),
            profile_revision: 4,
            role: "standby".into(),
            enabled: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        },
    )
}

async fn create_waiting_job(
    store: &PostgresStore,
    workspace_id: WorkspaceId,
    environment_id: EnvironmentId,
    printer_id: PrinterId,
    agent_id: AgentId,
    suffix: &str,
) -> JobId {
    let now = Utc::now();
    let job = Job {
        id: JobId::new(),
        workspace_id,
        environment_id,
        printer_id,
        title: format!("Routing {suffix}"),
        source: None,
        content_kind: ContentKind::Pdf,
        content: ContentSource::Base64 {
            data: "JVBERi0=".into(),
        },
        options: JobOptions::default(),
        metadata: BTreeMap::from([
            ("spool.target_id".into(), "tgt_recovery".into()),
            ("spool.binding_id".into(), "tgb_primary".into()),
            ("spool.profile_id".into(), "profile_shipping".into()),
            ("spool.profile_revision".into(), "4".into()),
        ]),
        deliveries: 1,
        state: JobState::WaitingForAgent,
        created_at: now,
        expires_at: now + chrono::Duration::hours(1),
    };
    store
        .create_job(&job, agent_id, None, suffix.as_bytes())
        .await
        .expect("create waiting job");
    job.id
}

#[tokio::test]
async fn postgres_reroute_is_atomic_and_fenced_by_lease_and_acceptance() {
    let Some(database_url) = env::var("SPOOL_TEST_DATABASE_URL").ok() else {
        eprintln!("skipped: set SPOOL_TEST_DATABASE_URL to run PostgreSQL routing evidence");
        return;
    };
    let schema = format!("spool_routing_test_{}", ulid::Ulid::new()).to_ascii_lowercase();
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .expect("connect PostgreSQL test database");
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .expect("create disposable schema");
    let first_pool = schema_pool(&database_url, &schema).await;
    let second_pool = schema_pool(&database_url, &schema).await;
    let first = PostgresStore::from_pool(first_pool.clone());
    let second = PostgresStore::from_pool(second_pool.clone());
    first.migrate().await.expect("apply migrations");
    let (
        workspace_id,
        environment_id,
        primary_agent,
        _standby_agent,
        primary_printer,
        standby_printer,
        standby_binding,
    ) = insert_fixture(&first_pool).await;

    let concurrent_job = create_waiting_job(
        &first,
        workspace_id,
        environment_id,
        primary_printer,
        primary_agent,
        "concurrent",
    )
    .await;
    let first_attempt = first.reroute_job_before_acceptance(
        workspace_id,
        environment_id,
        concurrent_job,
        "tgt_recovery",
        &standby_binding,
        "standby_recovery",
    );
    let second_attempt = second.reroute_job_before_acceptance(
        workspace_id,
        environment_id,
        concurrent_job,
        "tgt_recovery",
        &standby_binding,
        "standby_recovery",
    );
    let (first_result, second_result) = tokio::join!(first_attempt, second_attempt);
    assert_eq!(
        usize::from(first_result.expect("first reroute").is_some())
            + usize::from(second_result.expect("second reroute").is_some()),
        1
    );
    let route_row = sqlx::query(
        "SELECT count(*) AS attempts, min(to_printer_id) AS printer_id
         FROM job_routing_attempts WHERE job_id = $1",
    )
    .bind(concurrent_job.to_string())
    .fetch_one(&first_pool)
    .await
    .expect("routing attempt evidence");
    assert_eq!(route_row.get::<i64, _>("attempts"), 1);
    assert_eq!(
        route_row.get::<String, _>("printer_id"),
        standby_printer.to_string()
    );

    let leased_job = create_waiting_job(
        &first,
        workspace_id,
        environment_id,
        primary_printer,
        primary_agent,
        "leased",
    )
    .await;
    let leased = first
        .claim_jobs(
            workspace_id,
            environment_id,
            primary_agent,
            "routing-test",
            1,
        )
        .await
        .expect("lease job");
    assert_eq!(leased.len(), 1);
    assert_eq!(leased[0].job.id, leased_job);
    assert!(
        second
            .reroute_job_before_acceptance(
                workspace_id,
                environment_id,
                leased_job,
                "tgt_recovery",
                &standby_binding,
                "standby_recovery",
            )
            .await
            .expect("leased reroute fence")
            .is_none()
    );

    let accepted_job = create_waiting_job(
        &first,
        workspace_id,
        environment_id,
        primary_printer,
        primary_agent,
        "accepted",
    )
    .await;
    let accepted_lease = first
        .claim_jobs(
            workspace_id,
            environment_id,
            primary_agent,
            "routing-test",
            1,
        )
        .await
        .expect("lease accepted job")
        .pop()
        .expect("accepted job lease");
    assert_eq!(accepted_lease.job.id, accepted_job);
    let content_sha256 = format!("{:x}", Sha256::digest(b"%PDF-"));
    first
        .accept_agent_job(
            workspace_id,
            environment_id,
            primary_agent,
            accepted_job,
            accepted_lease.lease_id,
            &accepted_lease.lease_token,
            Some(&content_sha256),
            1,
        )
        .await
        .expect("accept job durably");
    assert!(
        second
            .reroute_job_before_acceptance(
                workspace_id,
                environment_id,
                accepted_job,
                "tgt_recovery",
                &standby_binding,
                "standby_recovery",
            )
            .await
            .expect("accepted reroute fence")
            .is_none()
    );
    let fenced_attempts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM job_routing_attempts WHERE job_id = ANY($1::text[])",
    )
    .bind(vec![leased_job.to_string(), accepted_job.to_string()])
    .fetch_one(&first_pool)
    .await
    .expect("fenced routing evidence");
    assert_eq!(fenced_attempts, 0);

    first_pool.close().await;
    second_pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .expect("drop disposable schema");
}
