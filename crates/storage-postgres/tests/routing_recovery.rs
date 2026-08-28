#![allow(clippy::expect_used, clippy::too_many_lines)]

use chrono::{Duration, Utc};
use piqae_domain::{
    AgentId, ContentKind, ContentSource, EnvironmentId, Job, JobId, JobOptions, JobState,
    PrinterCapabilities, PrinterId, WorkspaceId,
};
use piqae_storage_postgres::{
    DeliveryAttemptProof, DestinationRouteReassignment, PostgresStore, PrinterProfileSnapshot,
    StorageError, StoredLoadedMedia, StoredTargetBinding,
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
            destination_id: Some("destination_recovery".into()),
            route_id: Some("route_standby".into()),
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

async fn target_design_revision(
    store: &PostgresStore,
    workspace_id: WorkspaceId,
    environment_id: EnvironmentId,
) -> String {
    let target = store
        .get_target(workspace_id, environment_id, "tgt_recovery")
        .await
        .expect("target for design revision");
    let stock = match target.stock_id.as_deref() {
        Some(stock_id) => Some(
            store
                .get_stock(workspace_id, environment_id, stock_id)
                .await
                .expect("stock for design revision"),
        ),
        None => None,
    };
    let mut bindings = store
        .list_target_bindings(workspace_id, environment_id, "tgt_recovery")
        .await
        .expect("bindings for design revision")
        .into_iter()
        .map(|binding| {
            serde_json::json!({
                "id": binding.id,
                "target_id": binding.target_id,
                "printer_id": binding.printer_id,
                "agent_id": binding.agent_id,
                "profile_id": binding.profile_id,
                "profile_revision": binding.profile_revision,
                "destination_id": binding.destination_id,
                "route_id": binding.route_id,
                "role": binding.role,
                "enabled": binding.enabled,
            })
        })
        .collect::<Vec<_>>();
    bindings.sort_by(|left, right| left["id"].as_str().cmp(&right["id"].as_str()));
    let projection = serde_json::json!({
        "target": {
            "id": target.id,
            "stock_id": target.stock_id,
            "enabled": target.enabled,
            "routing_policy": target.routing_policy,
        },
        "stock": stock.map(|stock| serde_json::json!({
            "id": stock.id,
            "revision": stock.revision,
            "attributes": stock.attributes,
            "archived": stock.archived,
        })),
        "bindings": bindings,
    });
    format!(
        "spec_{:x}",
        Sha256::digest(serde_json::to_vec(&projection).expect("design projection JSON"))
    )
}

async fn create_printpacket_waiting_job(
    store: &PostgresStore,
    workspace_id: WorkspaceId,
    environment_id: EnvironmentId,
    printer_id: PrinterId,
    agent_id: AgentId,
    specification_revision: &str,
    suffix: &str,
) -> JobId {
    let now = Utc::now();
    let job = Job {
        id: JobId::new(),
        workspace_id,
        environment_id,
        printer_id,
        title: format!("PrintPacket reroute {suffix}"),
        source: Some("piqae.documents".into()),
        content_kind: ContentKind::Pdf,
        content: ContentSource::Base64 {
            data: "JVBERi0=".into(),
        },
        options: JobOptions::default(),
        metadata: BTreeMap::from([
            ("piqae.target_id".into(), "tgt_recovery".into()),
            ("piqae.binding_id".into(), "tgb_primary".into()),
            ("piqae.profile_id".into(), "profile_shipping".into()),
            ("piqae.profile_revision".into(), "4".into()),
            ("piqae.stock_id".into(), "stk_routing".into()),
            ("piqae.stock_revision".into(), "1".into()),
            (
                "piqae.design_specification_revision".into(),
                specification_revision.into(),
            ),
            (
                "piqae.document.media".into(),
                r#"{"kind":"paged","size":"a4","orientation":"portrait","margins":{"top_mm":0,"right_mm":0,"bottom_mm":0,"left_mm":0}}"#.into(),
            ),
            ("piqae.destination_id".into(), "destination_recovery".into()),
            ("piqae.route_id".into(), "route_primary".into()),
            ("piqae.route_agent_id".into(), agent_id.to_string()),
        ]),
        deliveries: 1,
        state: JobState::WaitingForAgent,
        created_at: now,
        expires_at: now + Duration::hours(1),
        delivery_uncertain_since: None,
    };
    store
        .create_job(&job, agent_id, None, suffix.as_bytes())
        .await
        .expect("create PrintPacket waiting job");
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
                binding_id: Some("tgb_standby"),
                destination_id: "destination_recovery",
                route_id: "route_standby",
                printer_id: &standby_printer.to_string(),
                agent_id: &standby_binding.agent_id.to_string(),
                profile_id: Some("profile_shipping"),
                profile_revision: Some(4),
                stock_revision: None,
                expected_design_specification_revision: None,
                loaded_media_snapshot: None,
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
                binding_id: None,
                destination_id: "destination_recovery",
                route_id: "route_standby",
                printer_id: &standby_printer.to_string(),
                agent_id: &standby_binding.agent_id.to_string(),
                profile_id: None,
                profile_revision: None,
                stock_revision: None,
                expected_design_specification_revision: None,
                loaded_media_snapshot: None,
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
                    binding_id: Some("tgb_standby"),
                    destination_id: "destination_recovery",
                    route_id: "route_standby",
                    printer_id: &standby_printer.to_string(),
                    agent_id: &standby_binding.agent_id.to_string(),
                    profile_id: Some("profile_shipping"),
                    profile_revision: Some(4),
                    stock_revision: None,
                    expected_design_specification_revision: None,
                    loaded_media_snapshot: None,
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

    // Exercise the full PrintPacket target fence across a genuinely distinct
    // standby destination and then read it through a second store, modelling a
    // control-plane restart.
    let cross_destination_agent = AgentId::new();
    let cross_destination_printer = PrinterId::new();
    sqlx::query(
        "INSERT INTO agents
         (id,workspace_id,environment_id,name,installation_id,os,architecture,version,
          protocol_version,state,last_seen_at)
         VALUES ($1,$2,$3,'cross-standby','cross-standby','test','test','0.1.0',1,'connected',now())",
    )
    .bind(cross_destination_agent.to_string())
    .bind(workspace_id.to_string())
    .bind(environment_id.to_string())
    .execute(&first_pool)
    .await
    .expect("cross-destination agent");
    sqlx::query(
        "INSERT INTO printers
         (id,workspace_id,environment_id,agent_id,native_id,name,state,capabilities,profiles,
          capabilities_revision,last_seen_at)
         SELECT $1,workspace_id,environment_id,$2,'cross-standby','cross-standby','online',
                capabilities,profiles,1,now()
         FROM printers WHERE workspace_id=$3 AND environment_id=$4 AND id=$5",
    )
    .bind(cross_destination_printer.to_string())
    .bind(cross_destination_agent.to_string())
    .bind(workspace_id.to_string())
    .bind(environment_id.to_string())
    .bind(standby_printer.to_string())
    .execute(&first_pool)
    .await
    .expect("cross-destination printer");
    first
        .upsert_destination(
            scope,
            &StoredPhysicalDestination {
                id: "destination_target_standby".into(),
                name: "Target standby printer".into(),
                identity_confidence: IdentityConfidence::Verified,
                state: "available".into(),
                scheduling_authority_id: None,
                identity_revision: 1,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
        )
        .await
        .expect("distinct standby destination");
    first
        .upsert_route(
            scope,
            &StoredPrinterRoute {
                id: "route_target_standby".into(),
                destination_id: "destination_target_standby".into(),
                printer_id: cross_destination_printer.to_string(),
                agent_id: cross_destination_agent.to_string(),
                native_queue_id: "cross-standby".into(),
                local_route_key: Some("rte_local_cross_standby".into()),
                state: "available".into(),
                role: "standby".into(),
                priority: 0,
                enabled: true,
                capability_revision: 1,
                profile_revision: 4,
                last_seen_at: Some(Utc::now()),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
        )
        .await
        .expect("cross-destination route");
    sqlx::query(
        "UPDATE target_bindings SET printer_id=$1,agent_id=$2,
                destination_id='destination_target_standby',route_id='route_target_standby'
         WHERE workspace_id=$3 AND environment_id=$4 AND id='tgb_standby'",
    )
    .bind(cross_destination_printer.to_string())
    .bind(cross_destination_agent.to_string())
    .bind(workspace_id.to_string())
    .bind(environment_id.to_string())
    .execute(&first_pool)
    .await
    .expect("cross-destination target binding");
    sqlx::query(
        "INSERT INTO stocks
         (id,workspace_id,environment_id,revision,name,attributes,archived)
         VALUES ('stk_routing',$1,$2,1,'Routing A4',$3,false)",
    )
    .bind(workspace_id.to_string())
    .bind(environment_id.to_string())
    .bind(serde_json::json!({"kind":"sheet","width_mm":210.0,"height_mm":297.0}))
    .execute(&first_pool)
    .await
    .expect("PrintPacket stock fixture");
    sqlx::query(
        "UPDATE targets SET stock_id='stk_routing' WHERE workspace_id=$1 AND environment_id=$2 AND id='tgt_recovery'",
    )
    .bind(workspace_id.to_string())
    .bind(environment_id.to_string())
    .execute(&first_pool)
    .await
    .expect("target stock fixture");
    let media_profiles = serde_json::to_value(vec![PrinterProfileSnapshot {
        profile_id: "profile_shipping".into(),
        revision: 4,
        name: "Shipping".into(),
        is_default: true,
        options: JobOptions::default(),
        status: Some("ready".into()),
        native_kind: None,
        native_digest: Some("sha256:routing-test".into()),
        driver_fingerprint: None,
        summary: Some(serde_json::json!({"dimensions_mm":[210.0,297.0],"source":"main"})),
        stock_id: Some("stk_routing".into()),
        safe_overrides: Vec::new(),
        last_validated_at: None,
        last_test_job_id: None,
        published: true,
    }])
    .expect("media profiles JSON");
    sqlx::query(
        "UPDATE printers SET profiles=$1 WHERE workspace_id=$2 AND environment_id=$3 AND id = ANY($4::text[])",
    )
    .bind(&media_profiles)
    .bind(workspace_id.to_string())
    .bind(environment_id.to_string())
    .bind(vec![primary_printer.to_string(), cross_destination_printer.to_string()])
    .execute(&first_pool)
    .await
    .expect("stock-bound profiles");
    let observed_at = Utc::now();
    first
        .upsert_loaded_media(
            workspace_id,
            environment_id,
            &StoredLoadedMedia {
                printer_id: cross_destination_printer,
                source: "main".into(),
                stock_id: Some("stk_routing".into()),
                stock_revision: Some(1),
                confidence: "operator_confirmed".into(),
                calibration_state: "current".into(),
                remaining_amount: None,
                observed_at,
                updated_at: observed_at,
            },
        )
        .await
        .expect("standby loaded-media evidence");
    let observed_at = first
        .list_loaded_media(workspace_id, environment_id, cross_destination_printer)
        .await
        .expect("read authoritative loaded-media timestamp")
        .into_iter()
        .find(|observation| observation.source == "main")
        .expect("authoritative main loaded-media observation")
        .observed_at;
    let specification_revision = target_design_revision(&first, workspace_id, environment_id).await;
    let loaded_snapshot = serde_json::json!({
        "source":"main",
        "confidence":"operator_confirmed",
        "observed_at": observed_at,
        "fresh_until": observed_at + Duration::minutes(15),
        "stock":{"id":"stk_routing","revision":1}
    })
    .to_string();
    let printpacket_job = create_printpacket_waiting_job(
        &first,
        workspace_id,
        environment_id,
        primary_printer,
        primary_agent,
        &specification_revision,
        "printpacket-cross-destination",
    )
    .await;
    first
        .reroute_job_to_destination_route_before_acceptance(
            workspace_id,
            environment_id,
            DestinationRouteReassignment {
                job_id: printpacket_job,
                target_id: Some("tgt_recovery"),
                binding_id: Some("tgb_standby"),
                destination_id: "destination_target_standby",
                route_id: "route_target_standby",
                printer_id: &cross_destination_printer.to_string(),
                agent_id: &cross_destination_agent.to_string(),
                profile_id: Some("profile_shipping"),
                profile_revision: Some(4),
                stock_revision: Some(1),
                expected_design_specification_revision: Some(&specification_revision),
                loaded_media_snapshot: Some(&loaded_snapshot),
                expected_capability_revision: None,
                resolved_ticket_digest: None,
                reason: "standby_recovery",
            },
        )
        .await
        .expect("PrintPacket cross-destination transaction")
        .expect("PrintPacket target rerouted");
    let restarted_job = second
        .get_job(workspace_id, environment_id, printpacket_job)
        .await
        .expect("restart-visible PrintPacket reroute");
    assert_eq!(restarted_job.printer_id, cross_destination_printer);
    assert_eq!(
        restarted_job
            .metadata
            .get("piqae.destination_id")
            .map(String::as_str),
        Some("destination_target_standby")
    );
    assert_eq!(
        restarted_job
            .metadata
            .get("piqae.route_id")
            .map(String::as_str),
        Some("route_target_standby")
    );
    assert_eq!(
        restarted_job
            .metadata
            .get("piqae.binding_id")
            .map(String::as_str),
        Some("tgb_standby")
    );
    assert_eq!(
        restarted_job.metadata.get("piqae.loaded_media_snapshot"),
        Some(&loaded_snapshot)
    );
    let attempt_projection: (Option<String>, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT to_binding_id,destination_id,to_route_id FROM job_routing_attempts
         WHERE workspace_id=$1 AND environment_id=$2 AND job_id=$3
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(workspace_id.to_string())
    .bind(environment_id.to_string())
    .bind(printpacket_job.to_string())
    .fetch_one(&second_pool)
    .await
    .expect("restart-visible target routing attempt");
    assert_eq!(
        attempt_projection,
        (
            Some("tgb_standby".into()),
            Some("destination_target_standby".into()),
            Some("route_target_standby".into())
        )
    );
    let outbox_payload: serde_json::Value = sqlx::query_scalar(
        "SELECT payload FROM routing_outbox
         WHERE workspace_id=$1 AND environment_id=$2 AND aggregate_id=$3
           AND event_type='job.routing_attempted'
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(workspace_id.to_string())
    .bind(environment_id.to_string())
    .bind(printpacket_job.to_string())
    .fetch_one(&second_pool)
    .await
    .expect("restart-visible target routing outbox");
    assert_eq!(
        outbox_payload["to_destination_id"],
        "destination_target_standby"
    );
    assert_eq!(outbox_payload["to_binding_id"], "tgb_standby");

    let stale_evidence_job = create_printpacket_waiting_job(
        &first,
        workspace_id,
        environment_id,
        primary_printer,
        primary_agent,
        &specification_revision,
        "printpacket-stale-evidence",
    )
    .await;
    first
        .upsert_loaded_media(
            workspace_id,
            environment_id,
            &StoredLoadedMedia {
                printer_id: cross_destination_printer,
                source: "main".into(),
                stock_id: None,
                stock_revision: None,
                confidence: "operator_confirmed".into(),
                calibration_state: "current".into(),
                remaining_amount: None,
                observed_at: Utc::now(),
                updated_at: Utc::now(),
            },
        )
        .await
        .expect("loaded stock mutates before reroute lock");
    assert!(matches!(
        first
            .reroute_job_to_destination_route_before_acceptance(
                workspace_id,
                environment_id,
                DestinationRouteReassignment {
                    job_id: stale_evidence_job,
                    target_id: Some("tgt_recovery"),
                    binding_id: Some("tgb_standby"),
                    destination_id: "destination_target_standby",
                    route_id: "route_target_standby",
                    printer_id: &cross_destination_printer.to_string(),
                    agent_id: &cross_destination_agent.to_string(),
                    profile_id: Some("profile_shipping"),
                    profile_revision: Some(4),
                    stock_revision: Some(1),
                    expected_design_specification_revision: Some(&specification_revision),
                    loaded_media_snapshot: Some(&loaded_snapshot),
                    expected_capability_revision: None,
                    resolved_ticket_digest: None,
                    reason: "standby_recovery",
                },
            )
            .await,
        Err(StorageError::ConcurrentStateChange)
    ));
    assert_eq!(
        second
            .get_job(workspace_id, environment_id, stale_evidence_job)
            .await
            .expect("stale evidence job remains durable")
            .printer_id,
        primary_printer
    );
    first
        .upsert_loaded_media(
            workspace_id,
            environment_id,
            &StoredLoadedMedia {
                printer_id: cross_destination_printer,
                source: "main".into(),
                stock_id: Some("stk_routing".into()),
                stock_revision: Some(1),
                confidence: "operator_confirmed".into(),
                calibration_state: "current".into(),
                remaining_amount: None,
                observed_at,
                updated_at: Utc::now(),
            },
        )
        .await
        .expect("restore locked loaded-media evidence");
    let stock_drift_job = create_printpacket_waiting_job(
        &first,
        workspace_id,
        environment_id,
        primary_printer,
        primary_agent,
        &specification_revision,
        "printpacket-stock-drift",
    )
    .await;
    sqlx::query(
        "UPDATE stocks SET revision=2 WHERE workspace_id=$1 AND environment_id=$2 AND id='stk_routing'",
    )
    .bind(workspace_id.to_string())
    .bind(environment_id.to_string())
    .execute(&first_pool)
    .await
    .expect("stock revision mutates before reroute lock");
    assert!(matches!(
        first
            .reroute_job_to_destination_route_before_acceptance(
                workspace_id,
                environment_id,
                DestinationRouteReassignment {
                    job_id: stock_drift_job,
                    target_id: Some("tgt_recovery"),
                    binding_id: Some("tgb_standby"),
                    destination_id: "destination_target_standby",
                    route_id: "route_target_standby",
                    printer_id: &cross_destination_printer.to_string(),
                    agent_id: &cross_destination_agent.to_string(),
                    profile_id: Some("profile_shipping"),
                    profile_revision: Some(4),
                    stock_revision: Some(1),
                    expected_design_specification_revision: Some(&specification_revision),
                    loaded_media_snapshot: Some(&loaded_snapshot),
                    expected_capability_revision: None,
                    resolved_ticket_digest: None,
                    reason: "standby_recovery",
                },
            )
            .await,
        Err(StorageError::ConcurrentStateChange)
    ));

    first_pool.close().await;
    second_pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .expect("drop disposable schema");
}
