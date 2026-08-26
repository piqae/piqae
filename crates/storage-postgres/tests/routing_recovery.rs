#![allow(clippy::expect_used, clippy::too_many_lines)]

use chrono::{Duration, Utc};
use piqae_domain::{
    AgentId, ContentKind, ContentSource, EnvironmentId, Job, JobId, JobOptions, JobState,
    PrinterCapabilities, PrinterId, WorkspaceId,
};
use piqae_storage_postgres::{
    DeliveryAttemptProof, DestinationRouteReassignment, PostgresStore, PrinterProfileSnapshot,
    StorageError, StoredTargetBinding,
    destination_topology::{
        DeliveryAttemptState, DestinationTopologyRepository, IdentityConfidence,
        NewDeliveryAttempt, StoredPhysicalDestination, StoredPrinterRoute, TenantScope,
    },
};
use sha2::{Digest, Sha256};
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
                state, capabilities, profiles, capabilities_revision, last_seen_at
             ) VALUES ($1,$2,$3,$4,$5,$5,'online',$6,$7,1,now())",
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

async fn create_direct_waiting_job(
    store: &PostgresStore,
    workspace_id: WorkspaceId,
    environment_id: EnvironmentId,
    printer_id: PrinterId,
    agent_id: AgentId,
) -> JobId {
    let now = Utc::now();
    let job = Job {
        id: JobId::new(),
        workspace_id,
        environment_id,
        printer_id,
        title: "Direct route recovery".into(),
        source: None,
        content_kind: ContentKind::Pdf,
        content: ContentSource::Base64 {
            data: "JVBERi0=".into(),
        },
        options: JobOptions::default(),
        metadata: BTreeMap::from([
            ("piqae.destination_id".into(), "destination_recovery".into()),
            ("piqae.route_id".into(), "route_primary".into()),
        ]),
        deliveries: 1,
        state: JobState::WaitingForAgent,
        created_at: now,
        expires_at: now + chrono::Duration::hours(1),
        delivery_uncertain_since: None,
    };
    store
        .create_job(&job, agent_id, None, b"direct-route-recovery")
        .await
        .expect("create direct waiting job");
    job.id
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
    let key_prefix = if suffix.starts_with("legacy-") {
        "spool"
    } else {
        "piqae"
    };
    let mut metadata = BTreeMap::from([
        (format!("{key_prefix}.target_id"), "tgt_recovery".into()),
        (format!("{key_prefix}.binding_id"), "tgb_primary".into()),
        (
            format!("{key_prefix}.profile_id"),
            "profile_shipping".into(),
        ),
        (format!("{key_prefix}.profile_revision"), "4".into()),
    ]);
    if !suffix.starts_with("legacy-") {
        metadata.insert("piqae.destination_id".into(), "destination_recovery".into());
        metadata.insert("piqae.route_id".into(), "route_primary".into());
    }
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
        metadata,
        deliveries: 1,
        state: JobState::WaitingForAgent,
        created_at: now,
        expires_at: now + chrono::Duration::hours(1),
        delivery_uncertain_since: None,
    };
    store
        .create_job(&job, agent_id, None, suffix.as_bytes())
        .await
        .expect("create waiting job");
    job.id
}

#[tokio::test]
async fn postgres_reroute_is_atomic_and_fenced_by_lease_and_acceptance() {
    let Some(database_url) = env::var("PIQAE_TEST_DATABASE_URL").ok() else {
        eprintln!("skipped: set PIQAE_TEST_DATABASE_URL to run PostgreSQL routing evidence");
        return;
    };
    let schema = format!("piqae_routing_test_{}", ulid::Ulid::new()).to_ascii_lowercase();
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
    let scope = TenantScope {
        workspace_id,
        environment_id,
    };
    first
        .upsert_destination(
            scope,
            &StoredPhysicalDestination {
                id: "destination_recovery".into(),
                name: "Recovery printer".into(),
                identity_confidence: IdentityConfidence::Verified,
                state: "available".into(),
                scheduling_authority_id: None,
                identity_revision: 1,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
        )
        .await
        .expect("physical destination");
    for (id, printer, agent, native_queue, role, priority) in [
        (
            "route_primary",
            primary_printer,
            primary_agent,
            "primary",
            "primary",
            0,
        ),
        (
            "route_standby",
            standby_printer,
            standby_binding.agent_id,
            "standby",
            "standby",
            100,
        ),
    ] {
        first
            .upsert_route(
                scope,
                &StoredPrinterRoute {
                    id: id.into(),
                    destination_id: "destination_recovery".into(),
                    printer_id: printer.to_string(),
                    agent_id: agent.to_string(),
                    native_queue_id: native_queue.into(),
                    local_route_key: Some(format!("rte_local_{native_queue}")),
                    state: "available".into(),
                    role: role.into(),
                    priority,
                    enabled: true,
                    capability_revision: 1,
                    profile_revision: 4,
                    last_seen_at: Some(Utc::now()),
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                },
            )
            .await
            .expect("route fixture");
    }
    sqlx::query("UPDATE target_bindings SET destination_id='destination_recovery',route_id=CASE id WHEN 'tgb_primary' THEN 'route_primary' ELSE 'route_standby' END WHERE workspace_id=$1 AND environment_id=$2")
        .bind(workspace_id.to_string()).bind(environment_id.to_string()).execute(&first_pool).await.expect("route-aware target bindings");

    let atomic_job = create_waiting_job(
        &first,
        workspace_id,
        environment_id,
        primary_printer,
        primary_agent,
        "atomic-accept",
    )
    .await;
    let atomic_lease = first
        .claim_jobs(
            workspace_id,
            environment_id,
            primary_agent,
            "atomic-accept-test",
            1,
        )
        .await
        .expect("claim topology job")
        .pop()
        .expect("topology job lease");
    let atomic_job_text = atomic_job.to_string();
    let atomic_attempt = first
        .begin_delivery_attempt(
            scope,
            NewDeliveryAttempt {
                attempt_id: "attempt_atomic_accept",
                reservation_id: "reservation_atomic_accept",
                job_id: &atomic_job_text,
                destination_id: "destination_recovery",
                route_id: "route_primary",
                lease_until: Utc::now() + Duration::minutes(1),
            },
        )
        .await
        .expect("reserve topology delivery");
    let content_sha256 = format!("{:x}", Sha256::digest(b"%PDF-"));
    let renewed_until = first
        .renew_agent_lease_with_delivery_attempt(
            workspace_id,
            environment_id,
            primary_agent,
            atomic_job,
            atomic_lease.lease_id,
            &atomic_lease.lease_token,
            DeliveryAttemptProof {
                reservation_id: "reservation_atomic_accept",
                generation: atomic_attempt.attempt.generation,
                fencing_token: &atomic_attempt.fencing_token,
            },
        )
        .await
        .expect("job lease and destination reservation renew atomically");
    assert!(renewed_until > Utc::now());
    assert!(matches!(
        first
            .accept_agent_job(
                workspace_id,
                environment_id,
                primary_agent,
                atomic_job,
                atomic_lease.lease_id,
                &atomic_lease.lease_token,
                Some(&content_sha256),
                1,
            )
            .await,
        Err(StorageError::InvalidData(_))
    ));
    first
        .accept_agent_job_with_delivery_attempt(
            workspace_id,
            environment_id,
            primary_agent,
            atomic_job,
            atomic_lease.lease_id,
            &atomic_lease.lease_token,
            Some(&content_sha256),
            1,
            DeliveryAttemptProof {
                reservation_id: "reservation_atomic_accept",
                generation: atomic_attempt.attempt.generation,
                fencing_token: &atomic_attempt.fencing_token,
            },
        )
        .await
        .expect("job acceptance and topology attempt commit atomically");
    assert_eq!(
        first
            .get_latest_delivery_attempt(scope, &atomic_job_text)
            .await
            .expect("atomic accepted attempt")
            .state,
        DeliveryAttemptState::QueuedLocal
    );
    first
        .transition_delivery_attempt(
            scope,
            "attempt_atomic_accept",
            atomic_attempt.attempt.generation,
            &atomic_attempt.fencing_token,
            DeliveryAttemptState::Failed,
        )
        .await
        .expect("close atomic acceptance fixture");

    let physical_job = create_waiting_job(
        &first,
        workspace_id,
        environment_id,
        primary_printer,
        primary_agent,
        "physical-target",
    )
    .await;
    let moved = first
        .reroute_job_to_destination_route_before_acceptance(
            workspace_id,
            environment_id,
            DestinationRouteReassignment {
                job_id: physical_job,
                target_id: Some("tgt_recovery"),
                route_id: "route_standby",
                profile_id: Some("profile_shipping"),
                profile_revision: Some(4),
                expected_capability_revision: None,
                resolved_ticket_digest: None,
                reason: "standby_recovery",
            },
        )
        .await
        .expect("authoritative physical route reassignment")
        .expect("pre-acceptance job moved");
    assert_eq!(moved.printer_id, standby_printer);
    let destination_after_move: String = sqlx::query_scalar(
        "SELECT destination_id FROM jobs WHERE workspace_id=$1 AND environment_id=$2 AND id=$3",
    )
    .bind(workspace_id.to_string())
    .bind(environment_id.to_string())
    .bind(physical_job.to_string())
    .fetch_one(&first_pool)
    .await
    .expect("fixed logical destination");
    assert_eq!(destination_after_move, "destination_recovery");

    let ticket_digest = format!("{:x}", Sha256::digest(b"standby-exact-ticket"));
    sqlx::query("INSERT INTO resolved_print_tickets (workspace_id,environment_id,digest,printer_id,capability_revision,display_ticket,expires_at) VALUES ($1,$2,$3,$4,1,'{}'::jsonb,now()+interval '1 hour')")
        .bind(workspace_id.to_string()).bind(environment_id.to_string()).bind(&ticket_digest).bind(standby_printer.to_string()).execute(&first_pool).await.expect("candidate-specific immutable ticket");
    let direct_job = create_direct_waiting_job(
        &first,
        workspace_id,
        environment_id,
        primary_printer,
        primary_agent,
    )
    .await;
    let direct_moved = first
        .reroute_job_to_destination_route_before_acceptance(
            workspace_id,
            environment_id,
            DestinationRouteReassignment {
                job_id: direct_job,
                target_id: None,
                route_id: "route_standby",
                profile_id: None,
                profile_revision: None,
                expected_capability_revision: Some(1),
                resolved_ticket_digest: Some(&ticket_digest),
                reason: "node_recovered",
            },
        )
        .await
        .expect("direct route reassignment")
        .expect("direct pre-acceptance job moved");
    assert_eq!(direct_moved.printer_id, standby_printer);

    let concurrent_job = create_waiting_job(
        &first,
        workspace_id,
        environment_id,
        primary_printer,
        primary_agent,
        "legacy-concurrent",
    )
    .await;
    let reroutable = first
        .list_reroutable_target_jobs(workspace_id, environment_id, 100)
        .await
        .expect("list reroutable legacy job");
    assert!(reroutable.iter().any(|job| job.id == concurrent_job));
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
    let rerouted = first
        .get_job(workspace_id, environment_id, concurrent_job)
        .await
        .expect("read rerouted legacy job");
    assert_eq!(
        rerouted.metadata.get("piqae.target_id").map(String::as_str),
        Some("tgt_recovery")
    );
    assert!(!rerouted.metadata.contains_key("spool.target_id"));

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
        "legacy-accepted",
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
    assert!(
        second
            .reroute_job_to_destination_route_before_acceptance(
                workspace_id,
                environment_id,
                DestinationRouteReassignment {
                    job_id: accepted_job,
                    target_id: Some("tgt_recovery"),
                    route_id: "route_standby",
                    profile_id: Some("profile_shipping"),
                    profile_revision: Some(4),
                    expected_capability_revision: None,
                    resolved_ticket_digest: None,
                    reason: "standby_recovery",
                },
            )
            .await
            .expect("accepted physical-route reroute fence")
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

    let legacy_pair_job = create_waiting_job(
        &first,
        workspace_id,
        environment_id,
        primary_printer,
        primary_agent,
        "legacy-topology-pair",
    )
    .await;
    sqlx::query(
        "UPDATE target_bindings SET destination_id=NULL,route_id='route_standby'
         WHERE workspace_id=$1 AND environment_id=$2 AND id='tgb_standby'",
    )
    .bind(workspace_id.to_string())
    .bind(environment_id.to_string())
    .execute(&first_pool)
    .await
    .expect("one-sided binding fixture");
    assert!(matches!(
        first
            .reroute_job_before_acceptance(
                workspace_id,
                environment_id,
                legacy_pair_job,
                "tgt_recovery",
                &standby_binding,
                "standby_recovery",
            )
            .await,
        Err(StorageError::InvalidData(_))
    ));
    let unchanged_topology: (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT destination_id,route_id FROM jobs
         WHERE workspace_id=$1 AND environment_id=$2 AND id=$3",
    )
    .bind(workspace_id.to_string())
    .bind(environment_id.to_string())
    .bind(legacy_pair_job.to_string())
    .fetch_one(&first_pool)
    .await
    .expect("one-sided binding did not partially update job");
    assert_eq!(unchanged_topology, (None, None));

    sqlx::query(
        "UPDATE target_bindings SET destination_id='destination_recovery',route_id='route_standby'
         WHERE workspace_id=$1 AND environment_id=$2 AND id='tgb_standby'",
    )
    .bind(workspace_id.to_string())
    .bind(environment_id.to_string())
    .execute(&first_pool)
    .await
    .expect("restore complete binding topology");
    first
        .reroute_job_before_acceptance(
            workspace_id,
            environment_id,
            legacy_pair_job,
            "tgt_recovery",
            &standby_binding,
            "standby_recovery",
        )
        .await
        .expect("complete binding topology")
        .expect("legacy job rerouted");
    let updated_topology: (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT destination_id,route_id FROM jobs
         WHERE workspace_id=$1 AND environment_id=$2 AND id=$3",
    )
    .bind(workspace_id.to_string())
    .bind(environment_id.to_string())
    .bind(legacy_pair_job.to_string())
    .fetch_one(&first_pool)
    .await
    .expect("complete destination/route update");
    assert_eq!(
        updated_topology,
        (
            Some("destination_recovery".into()),
            Some("route_standby".into())
        )
    );

    first_pool.close().await;
    second_pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .expect("drop disposable schema");
}
