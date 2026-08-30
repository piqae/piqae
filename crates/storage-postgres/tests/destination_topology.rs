#![allow(
    clippy::expect_used,
    clippy::too_many_lines,
    reason = "the PostgreSQL evidence test keeps one disposable-schema lifecycle auditable"
)]

use chrono::{DateTime, Duration, Utc};
use piqae_domain::{EnvironmentId, WorkspaceId};
use piqae_storage_postgres::{
    PostgresStore, StorageError,
    destination_topology::{
        DeliveryAttemptState, DestinationTopologyRepository, IdentityConfidence, IdentityDecision,
        IdentityDecisionKind, IdentityEvidence, NewDeliveryAttempt, NodeRuntimeObservation,
        NodeWakeHint, ProjectionAcknowledgement, RouteObservation, SchedulingAuthority,
        SiteCoordinatorMembership, StoredPhysicalDestination, StoredPrinterRoute, TenantScope,
    },
};
use sqlx::{PgPool, migrate::Migrator, postgres::PgPoolOptions};
use std::{borrow::Cow, env};

const FIRST_JOB_ID: &str = "job_01J00000000000000000000000";
const FIRST_RESERVATION_ID: &str = "00000000-0000-0000-0000-000000000001";

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

async fn create_tenant_fixture(
    store: &PostgresStore,
    scope: TenantScope,
    suffix: &str,
    printer_id: &str,
) {
    store
        .ensure_bootstrap_tenant(scope.workspace_id, scope.environment_id)
        .await
        .expect("bootstrap tenant");
    let agent_id = format!("agt_{suffix}");
    sqlx::query("INSERT INTO agents (id,workspace_id,environment_id,name,installation_id,public_key,os,architecture,version,protocol_version) VALUES ($1,$2,$3,'Node',$4,$5,'linux','x86_64','test',1)")
        .bind(&agent_id)
        .bind(scope.workspace_id.to_string())
        .bind(scope.environment_id.to_string())
        .bind(format!("installation-{suffix}"))
        .bind(vec![u8::try_from(suffix.len()).unwrap_or(1); 32])
        .execute(store.pool()).await.expect("agent fixture");
    sqlx::query(
        "INSERT INTO node_installations (id,installation_key,public_key) VALUES ($1,$2,$3)",
    )
    .bind(format!("ninst_{suffix}"))
    .bind(format!("installation-key-{suffix}"))
    .bind(vec![u8::try_from(suffix.len()).unwrap_or(1); 32])
    .execute(store.pool())
    .await
    .expect("installation fixture");
    sqlx::query("INSERT INTO node_connectors (id,installation_id,workspace_id,environment_id,agent_id) VALUES ($1,$2,$3,$4,$5)")
        .bind(format!("ncon_{suffix}"))
        .bind(format!("ninst_{suffix}"))
        .bind(scope.workspace_id.to_string())
        .bind(scope.environment_id.to_string())
        .bind(&agent_id)
        .execute(store.pool()).await.expect("connector fixture");
    sqlx::query("INSERT INTO printers (id,workspace_id,environment_id,agent_id,native_id,name,state,capabilities_revision) VALUES ($1,$2,$3,$4,$5,'Shared printer','online',1)")
        .bind(printer_id)
        .bind(scope.workspace_id.to_string())
        .bind(scope.environment_id.to_string())
        .bind(&agent_id)
        .bind(format!("native-{suffix}"))
        .execute(store.pool()).await.expect("printer fixture");
    sqlx::query("INSERT INTO jobs (id,workspace_id,environment_id,printer_id,agent_id,payload,state,per_printer_sequence,expires_at) VALUES ($1,$2,$3,$4,$5,'{}'::jsonb,'registered',1,now()+interval '1 hour')")
        .bind(if suffix == "first" {
            FIRST_JOB_ID.to_owned()
        } else {
            format!("job_{suffix}")
        })
        .bind(scope.workspace_id.to_string())
        .bind(scope.environment_id.to_string())
        .bind(printer_id)
        .bind(&agent_id)
        .execute(store.pool()).await.expect("job fixture");
}

fn destination(id: &str, authority_id: &str) -> StoredPhysicalDestination {
    StoredPhysicalDestination {
        id: id.to_owned(),
        name: "Warehouse printer".into(),
        identity_confidence: IdentityConfidence::High,
        state: "available".into(),
        scheduling_authority_id: Some(authority_id.to_owned()),
        identity_revision: 1,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

fn route(
    id: &str,
    destination_id: &str,
    suffix: &str,
    printer_id: &str,
    role: &str,
) -> StoredPrinterRoute {
    StoredPrinterRoute {
        id: id.to_owned(),
        destination_id: destination_id.to_owned(),
        printer_id: printer_id.to_owned(),
        agent_id: format!("agt_{suffix}"),
        native_queue_id: format!("native-{suffix}"),
        local_route_key: Some(format!("rte_local_{suffix}")),
        state: "available".into(),
        role: role.into(),
        priority: if role == "primary" { 0 } else { 100 },
        enabled: true,
        capability_revision: 1,
        profile_revision: 1,
        last_seen_at: Some(Utc::now()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

#[tokio::test]
async fn postgres_topology_is_tenant_isolated_and_fences_delivery() {
    let Some(database_url) = env::var("PIQAE_TEST_DATABASE_URL").ok() else {
        eprintln!("skipped: set PIQAE_TEST_DATABASE_URL for PostgreSQL topology evidence");
        return;
    };
    let schema = format!("piqae_destination_topology_{}", ulid::Ulid::new()).to_ascii_lowercase();
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

    let first = TenantScope {
        workspace_id: WorkspaceId::new(),
        environment_id: EnvironmentId::new(),
    };
    let second = TenantScope {
        workspace_id: WorkspaceId::new(),
        environment_id: EnvironmentId::new(),
    };
    create_tenant_fixture(&store, first, "first", "ptr_shared").await;
    create_tenant_fixture(&store, second, "second", "ptr_second").await;
    let runtime_observed_at = Utc::now();
    for sequence in 1..=80 {
        store
            .record_node_runtime_observation(
                first,
                &NodeRuntimeObservation {
                    id: format!("nro_first_{sequence}"),
                    agent_id: "agt_first".into(),
                    sequence,
                    host_mode: "machine_service".into(),
                    availability_class: "continuous_while_awake".into(),
                    lifecycle_state: "available".into(),
                    accepts_cloud_jobs: true,
                    execution_budget_ms: None,
                    wake_mechanisms: vec!["local_broker".into()],
                    observed_at: runtime_observed_at,
                    fresh_until: runtime_observed_at + Duration::minutes(1),
                },
            )
            .await
            .expect("record bounded runtime history");
    }
    let retained_runtime: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM node_runtime_observations WHERE workspace_id=$1 AND environment_id=$2 AND agent_id='agt_first'",
    )
    .bind(first.workspace_id.to_string())
    .bind(first.environment_id.to_string())
    .fetch_one(store.pool())
    .await
    .expect("count bounded runtime history");
    assert_eq!(retained_runtime, 64);
    let stale_runtime = NodeRuntimeObservation {
        id: "nro_first_replay".into(),
        agent_id: "agt_first".into(),
        sequence: 1,
        host_mode: "machine_service".into(),
        availability_class: "continuous_while_awake".into(),
        lifecycle_state: "available".into(),
        accepts_cloud_jobs: true,
        execution_budget_ms: None,
        wake_mechanisms: vec!["local_broker".into()],
        observed_at: runtime_observed_at,
        fresh_until: runtime_observed_at + Duration::minutes(1),
    };
    assert!(matches!(
        store
            .record_node_runtime_observation(first, &stale_runtime)
            .await,
        Err(StorageError::ConcurrentStateChange)
    ));
    for (scope, suffix) in [(first, "first"), (second, "second")] {
        sqlx::query("INSERT INTO node_wake_hints (workspace_id,environment_id,id,agent_id,idempotency_key,reason,delivery_channel,status,requested_at,expires_at) VALUES ($1,$2,$3,$4,$5,'job_available','connected_session','pending',now()-interval '9 days',now()-interval '9 days'+interval '5 minutes')")
            .bind(scope.workspace_id.to_string())
            .bind(scope.environment_id.to_string())
            .bind(format!("wkh_old_{suffix}"))
            .bind(format!("agt_{suffix}"))
            .bind(format!("old-wake-key-{suffix}"))
            .execute(store.pool())
            .await
            .expect("insert expired wake fixture");
    }
    let current_hint = NodeWakeHint {
        id: "wkh_current_first".into(),
        agent_id: "agt_first".into(),
        reason: "inventory_refresh".into(),
        delivery_channel: "connected_session".into(),
        status: "pending".into(),
        requested_at: runtime_observed_at,
        expires_at: runtime_observed_at + Duration::minutes(5),
        observed_at: None,
    };
    store
        .create_node_wake_hint(first, &current_hint, "current-wake-key-first")
        .await
        .expect("create wake hint and prune old tenant-local history");
    let first_old_wakes: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM node_wake_hints WHERE workspace_id=$1 AND environment_id=$2 AND id='wkh_old_first'",
    )
    .bind(first.workspace_id.to_string())
    .bind(first.environment_id.to_string())
    .fetch_one(store.pool())
    .await
    .expect("count pruned first-tenant wake history");
    let second_old_wakes: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM node_wake_hints WHERE workspace_id=$1 AND environment_id=$2 AND id='wkh_old_second'",
    )
    .bind(second.workspace_id.to_string())
    .bind(second.environment_id.to_string())
    .fetch_one(store.pool())
    .await
    .expect("count isolated second-tenant wake history");
    assert_eq!((first_old_wakes, second_old_wakes), (0, 1));
    sqlx::query("INSERT INTO agents (id,workspace_id,environment_id,name,installation_id,public_key,os,architecture,version,protocol_version) VALUES ('agt_first_backup',$1,$2,'Backup node','installation-first-backup',$3,'linux','x86_64','test',1)")
        .bind(first.workspace_id.to_string()).bind(first.environment_id.to_string()).bind(vec![9_u8;32]).execute(store.pool()).await.expect("backup route agent");
    sqlx::query("INSERT INTO printers (id,workspace_id,environment_id,agent_id,native_id,name,state,capabilities_revision) VALUES ('ptr_backup',$1,$2,'agt_first_backup','native-first-backup','Shared printer backup route','online',1)")
        .bind(first.workspace_id.to_string()).bind(first.environment_id.to_string()).execute(store.pool()).await.expect("backup route printer");

    for (scope, suffix) in [(first, "first"), (second, "second")] {
        let authority = SchedulingAuthority {
            id: "authority_main".into(),
            kind: "hosted_control_plane".into(),
            authority_key: "cloud:production".into(),
            display_name: "Piqae Cloud".into(),
            active: true,
        };
        store
            .upsert_scheduling_authority(scope, &authority)
            .await
            .expect("authority");
        store
            .upsert_destination(scope, &destination("destination_shared", "authority_main"))
            .await
            .expect("tenant destination");
        store
            .upsert_route(
                scope,
                &route(
                    &format!("route_{suffix}"),
                    "destination_shared",
                    suffix,
                    if suffix == "first" {
                        "ptr_shared"
                    } else {
                        "ptr_second"
                    },
                    "primary",
                ),
            )
            .await
            .expect("tenant route");
    }
    store
        .upsert_route(
            first,
            &route(
                "route_first_backup",
                "destination_shared",
                "first_backup",
                "ptr_backup",
                "standby",
            ),
        )
        .await
        .expect("second node route to same destination");
    assert_eq!(
        store
            .list_routes(first, "destination_shared")
            .await
            .expect("multiple routes")
            .len(),
        2
    );
    assert_eq!(
        store
            .get_destination(first, "destination_shared")
            .await
            .expect("first destination")
            .name,
        "Warehouse printer"
    );
    let initial_created_at = store
        .get_destination(first, "destination_shared")
        .await
        .expect("destination timestamps")
        .created_at;
    assert_eq!(
        store
            .recompute_destination_attention(first, "destination_shared")
            .await
            .expect("recomputed destination includes its creation time")
            .created_at,
        initial_created_at
    );
    assert_eq!(
        store
            .list_destinations(second)
            .await
            .expect("second list")
            .len(),
        1
    );
    assert!(matches!(
        store.latest_route_observation(second, "route_first").await,
        Err(StorageError::NotFound)
    ));

    let cross_tenant_route = route(
        "route_probe",
        "destination_shared",
        "first",
        "ptr_shared",
        "standby",
    );
    assert!(matches!(
        store.upsert_route(second, &cross_tenant_route).await,
        Err(StorageError::NotFound)
    ));

    let evidence = IdentityEvidence {
        id: "evidence_conflict".into(),
        destination_id: "destination_shared".into(),
        route_id: "route_first".into(),
        kind: "device_serial".into(),
        value_digest: format!("hmac-sha256:{}", "a".repeat(64)),
        strength: "strong".into(),
        conflicts: true,
        observed_at: Utc::now(),
        expires_at: None,
        metadata: serde_json::json!({"source":"node"}),
    };
    store
        .record_identity_evidence(first, &evidence)
        .await
        .expect("record conflict evidence");
    let mut evidence_retry = evidence.clone();
    evidence_retry.id = "evidence_conflict_retry".into();
    evidence_retry.observed_at = Utc::now() + Duration::seconds(1);
    store
        .record_identity_evidence(first, &evidence_retry)
        .await
        .expect("repeat projection updates the existing pseudonymous evidence");
    assert_eq!(
        store
            .list_identity_evidence(first, "destination_shared")
            .await
            .expect("deduplicated evidence")
            .len(),
        1
    );
    let mut raw_evidence = evidence.clone();
    raw_evidence.id = "evidence_raw_rejected".into();
    raw_evidence.value_digest = "a".repeat(64);
    assert!(matches!(
        store.record_identity_evidence(first, &raw_evidence).await,
        Err(StorageError::Database(_))
    ));
    let mut leaking_metadata = evidence.clone();
    leaking_metadata.id = "evidence_metadata_rejected".into();
    leaking_metadata.value_digest = format!("hmac-sha256:{}", "b".repeat(64));
    leaking_metadata.metadata = serde_json::json!({"serial":"must-never-be-stored"});
    assert!(matches!(
        store
            .record_identity_evidence(first, &leaking_metadata)
            .await,
        Err(StorageError::Database(_))
    ));
    assert_eq!(
        store
            .get_destination(first, "destination_shared")
            .await
            .expect("conflicted destination")
            .identity_confidence,
        IdentityConfidence::Conflict
    );
    assert_eq!(
        store
            .get_destination(second, "destination_shared")
            .await
            .expect("other tenant destination")
            .identity_confidence,
        IdentityConfidence::High
    );

    let mut source_destination = destination("destination_source", "authority_main");
    source_destination.name = "Previously separate queue".into();
    store
        .upsert_destination(first, &source_destination)
        .await
        .expect("source destination");
    store
        .upsert_route(
            first,
            &route(
                "route_first_backup",
                "destination_source",
                "first_backup",
                "ptr_backup",
                "primary",
            ),
        )
        .await
        .expect("move backup route to source before merge");
    let original = IdentityDecision {
        id: "decision_confirm".into(),
        kind: IdentityDecisionKind::Merge,
        destination_id: "destination_shared".into(),
        related_destination_ids: vec!["destination_source".into()],
        route_ids: vec!["route_first_backup".into()],
        evidence_ids: vec!["evidence_conflict".into()],
        actor_kind: "operator".into(),
        actor_id: Some("user_redacted".into()),
        reason: "operator inspected durable device evidence".into(),
        reverses_decision_id: None,
        request_id: Some("request_confirm".into()),
        created_at: Utc::now(),
    };
    store
        .record_identity_decision(first, &original)
        .await
        .expect("merge applies topology and audit");
    assert_eq!(
        store
            .get_route(first, "route_first_backup")
            .await
            .expect("merged route")
            .destination_id,
        "destination_shared"
    );
    let source_state:String=sqlx::query_scalar("SELECT state FROM physical_destinations WHERE workspace_id=$1 AND environment_id=$2 AND id='destination_source'").bind(first.workspace_id.to_string()).bind(first.environment_id.to_string()).fetch_one(store.pool()).await.expect("retired empty source");
    assert_eq!(source_state, "retired");
    let reversal = IdentityDecision {
        id: "decision_reverse".into(),
        kind: IdentityDecisionKind::Reverse,
        destination_id: "destination_shared".into(),
        related_destination_ids: vec![],
        route_ids: vec![],
        evidence_ids: vec![],
        actor_kind: "operator".into(),
        actor_id: Some("user_redacted".into()),
        reason: "new evidence invalidated the prior confirmation".into(),
        reverses_decision_id: Some("decision_confirm".into()),
        request_id: Some("request_reverse".into()),
        created_at: Utc::now(),
    };
    store
        .reverse_identity_decision(first, &reversal)
        .await
        .expect("reversal restores topology");
    let mut duplicate_reversal = reversal.clone();
    duplicate_reversal.id = "decision_reverse_again".into();
    duplicate_reversal.request_id = Some("request_reverse_again".into());
    assert!(matches!(
        store
            .reverse_identity_decision(first, &duplicate_reversal)
            .await,
        Err(StorageError::ConcurrentStateChange)
    ));
    assert_eq!(
        store
            .get_route(first, "route_first_backup")
            .await
            .expect("restored split route")
            .destination_id,
        "destination_source"
    );
    let restored_state:String=sqlx::query_scalar("SELECT state FROM physical_destinations WHERE workspace_id=$1 AND environment_id=$2 AND id='destination_source'").bind(first.workspace_id.to_string()).bind(first.environment_id.to_string()).fetch_one(store.pool()).await.expect("restored source");
    assert_eq!(restored_state, "available");

    let observed_at = DateTime::parse_from_rfc3339("2026-08-26T01:02:03.123456789Z")
        .expect("valid nanosecond timestamp")
        .with_timezone(&Utc);
    let observation = RouteObservation {
        id: "observation_1".into(),
        route_id: "route_first".into(),
        sequence: 1,
        printer_state: "processing".into(),
        accepting_jobs: Some(true),
        state_reasons: vec!["moving-to-paused".into()],
        total_jobs: 4,
        connector_jobs: 1,
        other_piqae_or_external_jobs: 3,
        unknown_jobs: 1,
        active_jobs: 1,
        held_jobs: 0,
        estimated_busy_seconds: None,
        privacy_level: "counts_only".into(),
        stock_state: serde_json::json!({"reported":false}),
        observed_at,
        fresh_until: observed_at + Duration::seconds(20),
    };
    store
        .record_route_observation(first, &observation)
        .await
        .expect("privacy-safe observation");
    store
        .record_route_observation(first, &observation)
        .await
        .expect("observation retry is idempotent");
    assert_eq!(
        store
            .list_route_observations(first, "route_first", 10)
            .await
            .expect("one observation after retry")
            .len(),
        1
    );
    let mut concurrent_a = observation.clone();
    concurrent_a.id = "observation_2".into();
    let mut concurrent_b = observation.clone();
    concurrent_b.id = "observation_3".into();
    let observer_a = store.clone();
    let observer_b = store.clone();
    let (recorded_a, recorded_b) = tokio::join!(
        observer_a.record_route_observation(first, &concurrent_a),
        observer_b.record_route_observation(first, &concurrent_b)
    );
    recorded_a.expect("first concurrent retry");
    recorded_b.expect("second concurrent retry");
    let mut next_observation = observation.clone();
    next_observation.id = "observation_4".into();
    next_observation.sequence = 2;
    next_observation.observed_at = Utc::now();
    next_observation.fresh_until = Utc::now() + Duration::seconds(20);
    store
        .record_route_observation(first, &next_observation)
        .await
        .expect("next durable node sequence");
    let mut stale_conflict = observation.clone();
    stale_conflict.id = "observation_stale".into();
    stale_conflict.printer_state = "idle".into();
    assert!(matches!(
        store.record_route_observation(first, &stale_conflict).await,
        Err(StorageError::IdempotencyConflict)
    ));
    assert_eq!(
        store
            .list_route_observations(first, "route_first", 10)
            .await
            .expect("atomic observation sequence")
            .iter()
            .map(|item| item.sequence)
            .collect::<Vec<_>>(),
        vec![2, 1]
    );
    assert_eq!(
        store
            .latest_route_observation(first, "route_first")
            .await
            .expect("latest observation")
            .other_piqae_or_external_jobs,
        3
    );
    assert!(matches!(
        store.latest_route_observation(second, "route_first").await,
        Err(StorageError::NotFound)
    ));

    store
        .acknowledge_projection(
            first,
            &ProjectionAcknowledgement {
                agent_id: "agt_first".into(),
                route_id: "route_first".into(),
                inventory_revision: 10,
                capability_revision: 1,
                status: "acknowledged".into(),
                error_code: None,
                observed_at: Utc::now(),
                acknowledged_at: Some(Utc::now()),
            },
        )
        .await
        .expect("projection acknowledgement");
    assert_eq!(
        store
            .get_projection_acknowledgement(first, "agt_first", "route_first")
            .await
            .expect("agent-scoped projection acknowledgement")
            .inventory_revision,
        10
    );
    assert!(matches!(
        store
            .get_projection_acknowledgement(second, "agt_first", "route_first")
            .await,
        Err(StorageError::NotFound)
    ));
    let batched_route_ids = vec!["route_first".into(), "route_first_backup".into()];
    let batched_observations = store
        .latest_route_observations(first, &batched_route_ids)
        .await
        .expect("latest observations load in one tenant-scoped batch");
    assert_eq!(batched_observations.len(), 1);
    assert_eq!(batched_observations[0].route_id, "route_first");
    assert_eq!(batched_observations[0].sequence, 2);
    let batched_acknowledgements = store
        .projection_acknowledgements_for_routes(first, &batched_route_ids)
        .await
        .expect("projection acknowledgements load in one tenant-scoped batch");
    assert_eq!(batched_acknowledgements.len(), 1);
    assert_eq!(batched_acknowledgements[0].inventory_revision, 10);
    assert!(
        store
            .latest_route_observations(second, &batched_route_ids)
            .await
            .expect("other tenant batch remains isolated")
            .is_empty()
    );
    assert!(
        store
            .projection_acknowledgements_for_routes(second, &batched_route_ids)
            .await
            .expect("other tenant projection batch remains isolated")
            .is_empty()
    );
    store
        .upsert_site_membership(
            first,
            &SiteCoordinatorMembership {
                authority_id: "authority_main".into(),
                agent_id: "agt_first".into(),
                site_id: "site_warehouse".into(),
                state: "active".into(),
                last_seen_at: Some(Utc::now()),
            },
        )
        .await
        .expect("site membership");

    let started = store
        .begin_delivery_attempt(
            first,
            NewDeliveryAttempt {
                attempt_id: "attempt_1",
                reservation_id: FIRST_RESERVATION_ID,
                job_id: FIRST_JOB_ID,
                destination_id: "destination_shared",
                route_id: "route_first",
                lease_until: Utc::now() + Duration::minutes(1),
            },
        )
        .await
        .expect("begin fenced attempt");
    assert_eq!(started.attempt.generation, 1);
    assert!(matches!(
        store
            .begin_delivery_attempt(
                first,
                NewDeliveryAttempt {
                    attempt_id: "attempt_concurrent",
                    reservation_id: "reservation_concurrent",
                    job_id: FIRST_JOB_ID,
                    destination_id: "destination_shared",
                    route_id: "route_first",
                    lease_until: Utc::now() + Duration::minutes(1)
                }
            )
            .await,
        Err(StorageError::ConcurrentStateChange)
    ));
    assert!(matches!(
        store
            .transition_delivery_attempt(
                first,
                "attempt_1",
                1,
                "stale-token",
                DeliveryAttemptState::AcceptedByNode
            )
            .await,
        Err(StorageError::ConcurrentStateChange)
    ));
    store
        .transition_delivery_attempt(
            first,
            "attempt_1",
            1,
            &started.fencing_token,
            DeliveryAttemptState::AcceptedByNode,
        )
        .await
        .expect("node accepts");
    store
        .transition_delivery_attempt(
            first,
            "attempt_1",
            1,
            &started.fencing_token,
            DeliveryAttemptState::QueuedLocal,
        )
        .await
        .expect("queued locally");
    store
        .transition_delivery_attempt(
            first,
            "attempt_1",
            1,
            &started.fencing_token,
            DeliveryAttemptState::HandingToSpooler,
        )
        .await
        .expect("handoff starts");
    store
        .transition_delivery_attempt(
            first,
            "attempt_1",
            1,
            &started.fencing_token,
            DeliveryAttemptState::DeliveryUncertain,
        )
        .await
        .expect("ambiguous handoff stops failover");
    assert!(matches!(
        store
            .begin_delivery_attempt(
                first,
                NewDeliveryAttempt {
                    attempt_id: "attempt_unsafe_retry",
                    reservation_id: "reservation_unsafe_retry",
                    job_id: FIRST_JOB_ID,
                    destination_id: "destination_shared",
                    route_id: "route_first",
                    lease_until: Utc::now() + Duration::minutes(1),
                },
            )
            .await,
        Err(StorageError::ConcurrentStateChange)
    ));
    assert!(
        store
            .has_unresolved_destination_uncertainty(first, "destination_shared")
            .await
            .expect("unresolved uncertainty")
    );
    let pending_resolution = store
        .enqueue_delivery_uncertainty_resolution(
            first,
            FIRST_JOB_ID,
            "reprint_authorized",
            Some("operator verified the physical output"),
            "operator_redacted",
            "resolve_job_first",
        )
        .await
        .expect("enqueue durable uncertainty resolution command");
    assert_eq!(
        pending_resolution.command,
        serde_json::json!({
            "type": "resolve_ambiguous_handoff",
            "job_id": "01J00000000000000000000000",
            "local_route_key": "rte_local_first",
            "reservation_id": FIRST_RESERVATION_ID,
            "generation": 1,
            "resolution": "confirm_accepted"
        })
    );
    assert!(
        store
            .finalize_delivery_uncertainty_resolution(first, "resolve_job_first")
            .await
            .expect("pending resolution lookup")
            .is_none()
    );
    sqlx::query("UPDATE agent_commands SET delivered_at=now(),acknowledged_at=now() WHERE workspace_id=$1 AND environment_id=$2 AND cursor=$3")
        .bind(first.workspace_id.to_string())
        .bind(first.environment_id.to_string())
        .bind(i64::try_from(pending_resolution.agent_command_cursor).expect("command cursor"))
        .execute(store.pool()).await.expect("acknowledge exact agent command");
    let resolution = store
        .finalize_delivery_uncertainty_resolution(first, "resolve_job_first")
        .await
        .expect("finalize acknowledged uncertainty resolution")
        .expect("acknowledged resolution");
    assert_eq!(resolution.attempt_id, "attempt_1");
    let recoverable = store
        .finalize_acknowledged_uncertainty_resolutions(first, "agt_first", 100)
        .await
        .expect("finalized reprint remains recoverable");
    assert_eq!(recoverable.as_slice(), [resolution.clone()]);
    sqlx::query("INSERT INTO jobs (id,workspace_id,environment_id,printer_id,agent_id,payload,state,per_printer_sequence,expires_at,destination_id,route_id) VALUES ('job_reprint_registered',$1,$2,'ptr_shared','agt_first',jsonb_build_object('metadata',jsonb_build_object('piqae.uncertainty_resolution_id',$3::text)),'registered',20,now()+interval '1 hour','destination_shared','route_first')")
        .bind(first.workspace_id.to_string()).bind(first.environment_id.to_string()).bind(&resolution.id).execute(store.pool()).await.expect("crash-left registered replacement");
    assert_eq!(
        store
            .finalize_acknowledged_uncertainty_resolutions(first, "agt_first", 100)
            .await
            .expect("registered replacement remains recoverable")
            .as_slice(),
        [resolution.clone()]
    );
    sqlx::query("UPDATE jobs SET state='completed_reported' WHERE workspace_id=$1 AND environment_id=$2 AND id='job_reprint_registered'")
        .bind(first.workspace_id.to_string()).bind(first.environment_id.to_string()).execute(store.pool()).await.expect("repair registered replacement");
    assert!(
        store
            .finalize_acknowledged_uncertainty_resolutions(first, "agt_first", 100)
            .await
            .expect("durable replacement closes recovery intent")
            .is_empty()
    );
    assert!(
        !store
            .has_unresolved_destination_uncertainty(first, "destination_shared")
            .await
            .expect("uncertainty cleared")
    );

    sqlx::query("INSERT INTO jobs (id,workspace_id,environment_id,printer_id,agent_id,payload,state,per_printer_sequence,expires_at,destination_id,route_id,created_at) VALUES ('job_stale_owner',$1,$2,'ptr_backup','agt_first_backup','{}'::jsonb,'waiting_for_agent',1,now()+interval '1 hour','destination_source','route_first_backup',now()-interval '2 minutes'),('job_after_stale',$1,$2,'ptr_backup','agt_first_backup','{}'::jsonb,'waiting_for_agent',2,now()+interval '1 hour','destination_source','route_first_backup',now()-interval '1 minute')")
        .bind(first.workspace_id.to_string()).bind(first.environment_id.to_string()).execute(store.pool()).await.expect("stale destination handoff jobs");
    store
        .begin_delivery_attempt(
            first,
            NewDeliveryAttempt {
                attempt_id: "attempt_stale_owner",
                reservation_id: "reservation_stale_owner",
                job_id: "job_stale_owner",
                destination_id: "destination_source",
                route_id: "route_first_backup",
                lease_until: Utc::now() - Duration::seconds(1),
            },
        )
        .await
        .expect("expired other-job handoff");
    let after_stale = store
        .begin_delivery_attempt(
            first,
            NewDeliveryAttempt {
                attempt_id: "attempt_after_stale",
                reservation_id: "reservation_after_stale",
                job_id: "job_after_stale",
                destination_id: "destination_source",
                route_id: "route_first_backup",
                lease_until: Utc::now() + Duration::minutes(1),
            },
        )
        .await
        .expect("expired other-job reservation is retired");
    store
        .transition_delivery_attempt(
            first,
            "attempt_after_stale",
            1,
            &after_stale.fencing_token,
            DeliveryAttemptState::Failed,
        )
        .await
        .expect("close test-only replacement handoff");

    sqlx::query("INSERT INTO jobs (id,workspace_id,environment_id,printer_id,agent_id,payload,state,per_printer_sequence,expires_at,destination_id,route_id) VALUES ('job_expired',$1,$2,'ptr_backup','agt_first_backup','{}'::jsonb,'registered',3,now()+interval '1 hour','destination_source','route_first_backup')")
        .bind(first.workspace_id.to_string()).bind(first.environment_id.to_string()).execute(store.pool()).await.expect("expired lease job");
    let expired = store
        .begin_delivery_attempt(
            first,
            NewDeliveryAttempt {
                attempt_id: "attempt_expired_1",
                reservation_id: "reservation_expired_1",
                job_id: "job_expired",
                destination_id: "destination_source",
                route_id: "route_first_backup",
                lease_until: Utc::now() - Duration::seconds(1),
            },
        )
        .await
        .expect("expired pre-acceptance attempt");
    let recovered = store
        .begin_delivery_attempt(
            first,
            NewDeliveryAttempt {
                attempt_id: "attempt_expired_2",
                reservation_id: "reservation_expired_2",
                job_id: "job_expired",
                destination_id: "destination_source",
                route_id: "route_first_backup",
                lease_until: Utc::now() + Duration::minutes(1),
            },
        )
        .await
        .expect("atomically supersede expired unaccepted attempt");
    assert_eq!(recovered.attempt.generation, 2);
    assert!(matches!(
        store
            .renew_delivery_attempt(
                first,
                "reservation_expired_2",
                2,
                "stale-token",
                Utc::now() + Duration::minutes(2),
            )
            .await,
        Err(StorageError::ConcurrentStateChange)
    ));
    let renewed = store
        .renew_delivery_attempt(
            first,
            "reservation_expired_2",
            2,
            &recovered.fencing_token,
            Utc::now() + Duration::minutes(2),
        )
        .await
        .expect("fenced attempt and reservation renew atomically");
    assert_eq!(renewed.attempt.lease_until, renewed.reservation.lease_until);
    assert!(matches!(
        store
            .transition_delivery_attempt(
                first,
                "attempt_expired_1",
                1,
                &expired.fencing_token,
                DeliveryAttemptState::AcceptedByNode
            )
            .await,
        Err(StorageError::ConcurrentStateChange)
    ));
    store
        .transition_delivery_attempt(
            first,
            "attempt_expired_2",
            2,
            &recovered.fencing_token,
            DeliveryAttemptState::AcceptedByNode,
        )
        .await
        .expect("accepted generation owns route");
    sqlx::query("UPDATE delivery_attempts SET lease_until=now()-interval '1 second' WHERE workspace_id=$1 AND environment_id=$2 AND id='attempt_expired_2'")
        .bind(first.workspace_id.to_string()).bind(first.environment_id.to_string()).execute(store.pool()).await.expect("simulate disconnect after acceptance");
    assert!(matches!(
        store
            .begin_delivery_attempt(
                first,
                NewDeliveryAttempt {
                    attempt_id: "attempt_expired_3",
                    reservation_id: "reservation_expired_3",
                    job_id: "job_expired",
                    destination_id: "destination_source",
                    route_id: "route_first_backup",
                    lease_until: Utc::now() + Duration::minutes(1)
                }
            )
            .await,
        Err(StorageError::ConcurrentStateChange)
    ));
    assert!(matches!(
        store
            .transition_delivery_attempt(
                second,
                "attempt_expired_2",
                2,
                &recovered.fencing_token,
                DeliveryAttemptState::AcceptedByNode
            )
            .await,
        Err(StorageError::NotFound)
    ));

    let busy_merge = IdentityDecision {
        id: "decision_busy_merge".into(),
        kind: IdentityDecisionKind::Merge,
        destination_id: "destination_shared".into(),
        related_destination_ids: vec!["destination_source".into()],
        route_ids: vec!["route_first_backup".into()],
        evidence_ids: vec![],
        actor_kind: "operator".into(),
        actor_id: Some("operator_redacted".into()),
        reason: "combine redundant routes after verified evidence".into(),
        reverses_decision_id: None,
        request_id: Some("request_busy_merge".into()),
        created_at: Utc::now(),
    };
    assert!(matches!(
        store.record_identity_decision(first, &busy_merge).await,
        Err(StorageError::ConcurrentStateChange)
    ));
    store
        .transition_delivery_attempt(
            first,
            "attempt_expired_2",
            2,
            &recovered.fencing_token,
            DeliveryAttemptState::QueuedLocal,
        )
        .await
        .expect("queue recovered job");
    store
        .transition_delivery_attempt(
            first,
            "attempt_expired_2",
            2,
            &recovered.fencing_token,
            DeliveryAttemptState::HandingToSpooler,
        )
        .await
        .expect("handoff recovered job");
    let overlapping = store
        .transition_delivery_attempt(
            first,
            "attempt_expired_2",
            2,
            &recovered.fencing_token,
            DeliveryAttemptState::AcceptedBySpooler,
        )
        .await
        .expect("release only the handoff reservation");
    assert!(overlapping.final_at.is_none());
    assert!(matches!(
        store.record_identity_decision(first, &busy_merge).await,
        Err(StorageError::ConcurrentStateChange)
    ));
    store
        .transition_delivery_attempt(
            first,
            "attempt_expired_2",
            2,
            &recovered.fencing_token,
            DeliveryAttemptState::PrintingReported,
        )
        .await
        .expect("printing remains tracked after reservation release");
    store
        .transition_delivery_attempt(
            first,
            "attempt_expired_2",
            2,
            &recovered.fencing_token,
            DeliveryAttemptState::CompletedReported,
        )
        .await
        .expect("finish before topology mutation");
    store
        .record_identity_decision(first, &busy_merge)
        .await
        .expect("topology mutation succeeds once handoff reservation releases");

    for (job_id, sequence, printer_id, agent_id, route_id) in [
        (
            "job_race_a",
            2_i64,
            "ptr_shared",
            "agt_first",
            "route_first",
        ),
        (
            "job_race_b",
            4_i64,
            "ptr_backup",
            "agt_first_backup",
            "route_first_backup",
        ),
    ] {
        sqlx::query("INSERT INTO jobs (id,workspace_id,environment_id,printer_id,agent_id,payload,state,per_printer_sequence,expires_at,destination_id,route_id,created_at) VALUES ($1,$2,$3,$4,$5,'{}'::jsonb,'waiting_for_agent',$6,now()+interval '1 hour','destination_shared',$7,now()+($8::bigint * interval '1 second'))")
            .bind(job_id).bind(first.workspace_id.to_string()).bind(first.environment_id.to_string()).bind(printer_id).bind(agent_id).bind(sequence).bind(route_id).bind(sequence).execute(store.pool()).await.expect("scheduler race job");
    }
    sqlx::query("UPDATE jobs SET lease_until=now()+interval '1 minute',lease_owner='older-scheduler' WHERE workspace_id=$1 AND environment_id=$2 AND id='job_race_a'")
        .bind(first.workspace_id.to_string()).bind(first.environment_id.to_string()).execute(store.pool()).await.expect("active older claim");
    sqlx::query("INSERT INTO jobs (id,workspace_id,environment_id,printer_id,agent_id,payload,state,per_printer_sequence,expires_at,destination_id,route_id,created_at) VALUES ('job_nonready_older',$1,$2,'ptr_shared','agt_first','{}'::jsonb,'registered',4,now()+interval '1 hour','destination_shared','route_first',now()-interval '1 hour')")
        .bind(first.workspace_id.to_string()).bind(first.environment_id.to_string()).execute(store.pool()).await.expect("non-ready older job");
    let scheduler_a = store.clone();
    let scheduler_b = store.clone();
    let (race_a, race_b) = tokio::join!(
        scheduler_a.begin_delivery_attempt(
            first,
            NewDeliveryAttempt {
                attempt_id: "attempt_race_a",
                reservation_id: "reservation_race_a",
                job_id: "job_race_a",
                destination_id: "destination_shared",
                route_id: "route_first",
                lease_until: Utc::now() + Duration::minutes(1)
            }
        ),
        scheduler_b.begin_delivery_attempt(
            first,
            NewDeliveryAttempt {
                attempt_id: "attempt_race_b",
                reservation_id: "reservation_race_b",
                job_id: "job_race_b",
                destination_id: "destination_shared",
                route_id: "route_first_backup",
                lease_until: Utc::now() + Duration::minutes(1)
            }
        )
    );
    let winner = race_a.expect("oldest eligible job wins destination order");
    assert!(matches!(race_b, Err(StorageError::ConcurrentStateChange)));
    let (loser_job, loser_route) = ("job_race_b", "route_first_backup");
    for next in [
        DeliveryAttemptState::AcceptedByNode,
        DeliveryAttemptState::QueuedLocal,
        DeliveryAttemptState::HandingToSpooler,
        DeliveryAttemptState::AcceptedBySpooler,
    ] {
        store
            .transition_delivery_attempt(
                first,
                &winner.attempt.id,
                winner.attempt.generation,
                &winner.fencing_token,
                next,
            )
            .await
            .expect("advance winning handoff");
    }
    let after_handoff = store
        .begin_delivery_attempt(
            first,
            NewDeliveryAttempt {
                attempt_id: "attempt_after_handoff",
                reservation_id: "reservation_after_handoff",
                job_id: loser_job,
                destination_id: "destination_shared",
                route_id: loser_route,
                lease_until: Utc::now() + Duration::minutes(1),
            },
        )
        .await
        .expect("next job may hand off while the prior spooler job remains active");
    assert_eq!(
        after_handoff.attempt.state,
        DeliveryAttemptState::RouteLeased
    );
    let winner_attempts = store
        .list_delivery_attempts(first, &winner.attempt.job_id)
        .await
        .expect("winner still tracked");
    assert_eq!(
        winner_attempts.last().expect("winner attempt").state,
        DeliveryAttemptState::AcceptedBySpooler
    );
    assert!(
        winner_attempts
            .last()
            .expect("winner attempt")
            .final_at
            .is_none()
    );
    store
        .transition_post_spooler_attempt(
            first,
            &winner.attempt.job_id,
            "agt_first",
            "route_first",
            DeliveryAttemptState::PrintingReported,
        )
        .await
        .expect("native printing event");
    let definitively_failed = store
        .transition_post_spooler_attempt(
            first,
            &winner.attempt.job_id,
            "agt_first",
            "route_first",
            DeliveryAttemptState::Failed,
        )
        .await
        .expect("definitive post-spooler native failure becomes final");
    assert!(definitively_failed.final_at.is_some());
    assert_eq!(
        store
            .get_latest_delivery_attempt(first, &winner.attempt.job_id)
            .await
            .expect("latest attempt by job")
            .state,
        DeliveryAttemptState::Failed
    );

    let decision_count: i64 = sqlx::query_scalar("SELECT count(*) FROM destination_identity_decisions WHERE workspace_id=$1 AND environment_id=$2").bind(first.workspace_id.to_string()).bind(first.environment_id.to_string()).fetch_one(&pool).await.expect("decision count");
    assert_eq!(decision_count, 3);
    let raw_token_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM delivery_attempts WHERE fencing_token_hash=$1")
            .bind(&started.fencing_token)
            .fetch_one(&pool)
            .await
            .expect("token leak check");
    assert_eq!(raw_token_count, 0);

    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .expect("drop exact disposable schema");
}

#[tokio::test]
async fn revoking_node_retires_only_safe_routes_and_preserves_accepted_evidence() {
    let Some(database_url) = env::var("PIQAE_TEST_DATABASE_URL").ok() else {
        eprintln!("skipped: set PIQAE_TEST_DATABASE_URL for node revocation evidence");
        return;
    };
    let schema = format!("piqae_node_revoke_{}", ulid::Ulid::new()).to_ascii_lowercase();
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
    let scope = TenantScope {
        workspace_id: WorkspaceId::new(),
        environment_id: EnvironmentId::new(),
    };
    let agent_suffix = "01J00000000000000000000000";
    let agent_id = format!("agt_{agent_suffix}");
    let safe_job_id = format!("job_{agent_suffix}");
    create_tenant_fixture(&store, scope, agent_suffix, "ptr_revoke").await;
    sqlx::query("INSERT INTO printers (id,workspace_id,environment_id,agent_id,native_id,name,state,capabilities_revision) VALUES ('ptr_revoke_accepted',$1,$2,$3,'native-revoke-accepted','Accepted route printer','online',1)")
        .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(&agent_id).execute(store.pool()).await.expect("accepted printer fixture");
    sqlx::query("INSERT INTO jobs (id,workspace_id,environment_id,printer_id,agent_id,payload,state,per_printer_sequence,expires_at) VALUES ('job_revoke_accepted',$1,$2,'ptr_revoke_accepted',$3,'{}'::jsonb,'waiting_for_agent',1,now()+interval '1 hour')")
        .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(&agent_id).execute(store.pool()).await.expect("accepted job fixture");
    sqlx::query("UPDATE jobs SET state='waiting_for_agent' WHERE workspace_id=$1 AND environment_id=$2 AND id=$3")
        .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(&safe_job_id).execute(store.pool()).await.expect("safe job ready fixture");
    store
        .upsert_scheduling_authority(
            scope,
            &SchedulingAuthority {
                id: "authority_revoke".into(),
                kind: "hosted_control_plane".into(),
                authority_key: "cloud:revoke-test".into(),
                display_name: "Revocation fixture".into(),
                active: true,
            },
        )
        .await
        .expect("revocation authority fixture");
    let backup_agent_id = "agt_01J00000000000000000000001";
    sqlx::query("INSERT INTO agents (id,workspace_id,environment_id,name,installation_id,public_key,os,architecture,version,protocol_version) VALUES ($1,$2,$3,'Backup node','installation-revoke-backup',$4,'linux','x86_64','test',1)")
        .bind(backup_agent_id).bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(vec![7_u8;32]).execute(store.pool()).await.expect("backup agent fixture");
    sqlx::query("INSERT INTO printers (id,workspace_id,environment_id,agent_id,native_id,name,state,capabilities_revision) VALUES ('ptr_revoke_backup',$1,$2,$3,'native-revoke-backup','Backup route printer','online',1)")
        .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(backup_agent_id).execute(store.pool()).await.expect("backup printer fixture");
    for (destination_id, route_id, printer_id, native_queue_id) in [
        (
            "pdst_revoke_safe",
            "rte_revoke_safe",
            "ptr_revoke",
            "native-revoke",
        ),
        (
            "pdst_revoke_accepted",
            "rte_revoke_accepted",
            "ptr_revoke_accepted",
            "native-revoke-accepted",
        ),
    ] {
        store
            .upsert_destination(scope, &destination(destination_id, "authority_revoke"))
            .await
            .expect("destination fixture");
        let mut route = route(
            route_id,
            destination_id,
            agent_suffix,
            printer_id,
            "primary",
        );
        route.native_queue_id = native_queue_id.into();
        route.local_route_key = Some(format!("rte_local_{route_id}"));
        store
            .upsert_route(scope, &route)
            .await
            .expect("route fixture");
    }
    sqlx::query(
        "UPDATE physical_destinations SET state='attention'
         WHERE workspace_id=$1 AND environment_id=$2 AND id='pdst_revoke_accepted'",
    )
    .bind(scope.workspace_id.to_string())
    .bind(scope.environment_id.to_string())
    .execute(store.pool())
    .await
    .expect("attention destination fixture");
    let unrelated_destination = destination("pdst_revoke_unrelated", "authority_revoke");
    store
        .upsert_destination(scope, &unrelated_destination)
        .await
        .expect("unrelated orphan destination fixture");
    let backup_route = StoredPrinterRoute {
        id: "rte_revoke_backup".into(),
        destination_id: "pdst_revoke_safe".into(),
        printer_id: "ptr_revoke_backup".into(),
        agent_id: backup_agent_id.into(),
        native_queue_id: "native-revoke-backup".into(),
        local_route_key: Some("rte_local_revoke_backup".into()),
        state: "available".into(),
        role: "standby".into(),
        priority: 100,
        enabled: true,
        capability_revision: 1,
        profile_revision: 1,
        last_seen_at: Some(Utc::now()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    store
        .upsert_route(scope, &backup_route)
        .await
        .expect("backup route fixture");
    let safe = store
        .begin_delivery_attempt(
            scope,
            NewDeliveryAttempt {
                attempt_id: "attempt_revoke_safe",
                reservation_id: "reservation_revoke_safe",
                job_id: &safe_job_id,
                destination_id: "pdst_revoke_safe",
                route_id: "rte_revoke_safe",
                lease_until: Utc::now() + Duration::minutes(1),
            },
        )
        .await
        .expect("pre-acceptance route lease");
    assert_eq!(safe.attempt.state, DeliveryAttemptState::RouteLeased);
    let accepted = store
        .begin_delivery_attempt(
            scope,
            NewDeliveryAttempt {
                attempt_id: "attempt_revoke_accepted",
                reservation_id: "reservation_revoke_accepted",
                job_id: "job_revoke_accepted",
                destination_id: "pdst_revoke_accepted",
                route_id: "rte_revoke_accepted",
                lease_until: Utc::now() + Duration::minutes(1),
            },
        )
        .await
        .expect("accepted route lease");
    store
        .transition_delivery_attempt(
            scope,
            &accepted.attempt.id,
            accepted.attempt.generation,
            &accepted.fencing_token,
            DeliveryAttemptState::AcceptedByNode,
        )
        .await
        .expect("node acceptance evidence");
    sqlx::query("UPDATE jobs SET state='agent_accepted' WHERE workspace_id=$1 AND environment_id=$2 AND id='job_revoke_accepted'")
        .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).execute(store.pool()).await.expect("accepted job state fixture");
    // Exercise the nanosecond-to-microsecond round trip used by PostgreSQL.
    // Deriving these endpoints independently must not make the inserted hint
    // conflict with the row returned by its own INSERT.
    let wake_requested_at = DateTime::parse_from_rfc3339("2026-08-30T01:02:03.123456789Z")
        .expect("wake request timestamp")
        .with_timezone(&Utc);
    let wake_expires_at = DateTime::parse_from_rfc3339("2026-08-30T01:07:03.123456700Z")
        .expect("wake expiry timestamp")
        .with_timezone(&Utc);
    store
        .create_node_wake_hint(
            scope,
            &NodeWakeHint {
                id: "wkh_revoke".into(),
                agent_id: agent_id.clone(),
                reason: "job_available".into(),
                delivery_channel: "connected_session".into(),
                status: "pending".into(),
                requested_at: wake_requested_at,
                expires_at: wake_expires_at,
                observed_at: None,
            },
            "wake-revoke-node",
        )
        .await
        .expect("pending wake hint");

    store
        .revoke_agent(
            scope.workspace_id,
            scope.environment_id,
            agent_id.parse().expect("fixture agent id"),
        )
        .await
        .expect("revoke node atomically");

    let routes: Vec<(bool, String, String, bool)> = sqlx::query_as(
        "SELECT enabled,state,role,retired_at IS NOT NULL
         FROM printer_routes
         WHERE workspace_id=$1 AND environment_id=$2
         ORDER BY id",
    )
    .bind(scope.workspace_id.to_string())
    .bind(scope.environment_id.to_string())
    .fetch_all(store.pool())
    .await
    .expect("retained route audit");
    assert_eq!(routes.len(), 3);
    assert_eq!(
        routes
            .iter()
            .filter(|(enabled, state, role, retired)| {
                !enabled && state == "retired" && role == "disabled" && *retired
            })
            .count(),
        2
    );
    assert!(routes.contains(&(true, "available".into(), "standby".into(), false)));
    let safe_attempt = store
        .get_latest_delivery_attempt(scope, &safe_job_id)
        .await
        .expect("safe attempt retained");
    assert_eq!(safe_attempt.state, DeliveryAttemptState::Superseded);
    let accepted_attempt = store
        .get_latest_delivery_attempt(scope, "job_revoke_accepted")
        .await
        .expect("accepted attempt retained");
    assert_eq!(accepted_attempt.state, DeliveryAttemptState::AcceptedByNode);
    let job_states: Vec<(String, String)> = sqlx::query_as(
        "SELECT id,state FROM jobs WHERE workspace_id=$1 AND environment_id=$2 ORDER BY id",
    )
    .bind(scope.workspace_id.to_string())
    .bind(scope.environment_id.to_string())
    .fetch_all(store.pool())
    .await
    .expect("job states after revocation");
    assert!(job_states.contains(&(safe_job_id.clone(), "waiting_for_agent".into())));
    assert!(job_states.contains(&("job_revoke_accepted".into(), "agent_accepted".into())));
    let wake_status: String = sqlx::query_scalar(
        "SELECT status FROM node_wake_hints WHERE workspace_id=$1 AND environment_id=$2 AND id='wkh_revoke'",
    )
    .bind(scope.workspace_id.to_string())
    .bind(scope.environment_id.to_string())
    .fetch_one(store.pool())
    .await
    .expect("cancelled wake hint");
    assert_eq!(wake_status, "cancelled");
    let destination_states: Vec<(String, String)> = sqlx::query_as(
        "SELECT id,state FROM physical_destinations WHERE workspace_id=$1 AND environment_id=$2 ORDER BY id",
    )
    .bind(scope.workspace_id.to_string())
    .bind(scope.environment_id.to_string())
    .fetch_all(store.pool())
    .await
    .expect("destination states after revocation");
    assert_eq!(
        destination_states,
        vec![
            ("pdst_revoke_accepted".into(), "attention".into()),
            ("pdst_revoke_safe".into(), "available".into()),
            ("pdst_revoke_unrelated".into(), "available".into()),
        ]
    );
    let stale_route = route(
        "rte_revoke_stale",
        "pdst_revoke_unrelated",
        agent_suffix,
        "ptr_revoke",
        "primary",
    );
    assert!(matches!(
        store.upsert_route(scope, &stale_route).await,
        Err(StorageError::NotFound)
    ));
    assert!(matches!(
        store
            .begin_delivery_attempt(
                scope,
                NewDeliveryAttempt {
                    attempt_id: "attempt_revoke_after",
                    reservation_id: "reservation_revoke_after",
                    job_id: &safe_job_id,
                    destination_id: "pdst_revoke_safe",
                    route_id: "rte_revoke_safe",
                    lease_until: Utc::now() + Duration::minutes(1),
                },
            )
            .await,
        Err(StorageError::NotFound)
    ));

    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .expect("drop exact disposable schema");
}

#[tokio::test]
async fn revocation_serializes_with_route_projection_and_retires_the_winner() {
    let Some(database_url) = env::var("PIQAE_TEST_DATABASE_URL").ok() else {
        eprintln!("skipped: set PIQAE_TEST_DATABASE_URL for revocation concurrency evidence");
        return;
    };
    let schema = format!("piqae_revoke_race_{}", ulid::Ulid::new()).to_ascii_lowercase();
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
    let scope = TenantScope {
        workspace_id: WorkspaceId::new(),
        environment_id: EnvironmentId::new(),
    };
    let suffix = "01J00000000000000000000010";
    let agent_id = format!("agt_{suffix}");
    create_tenant_fixture(&store, scope, suffix, "ptr_revoke_race").await;
    let mut projected_destination = destination("pdst_revoke_race", "unused");
    projected_destination.scheduling_authority_id = None;
    store
        .upsert_destination(scope, &projected_destination)
        .await
        .expect("race destination fixture");

    let mut projection = pool.begin().await.expect("begin projection transaction");
    sqlx::query(
        "SELECT id FROM agents
         WHERE workspace_id=$1 AND environment_id=$2 AND id=$3 AND revoked_at IS NULL
         FOR KEY SHARE",
    )
    .bind(scope.workspace_id.to_string())
    .bind(scope.environment_id.to_string())
    .bind(&agent_id)
    .fetch_one(&mut *projection)
    .await
    .expect("hold projection's active-agent fence");

    let revoke_store = store.clone();
    let revoked_agent_id = agent_id.clone();
    let mut revoke = tokio::spawn(async move {
        revoke_store
            .revoke_agent(
                scope.workspace_id,
                scope.environment_id,
                revoked_agent_id.parse().expect("fixture agent id"),
            )
            .await
    });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), &mut revoke)
            .await
            .is_err(),
        "revocation must wait for the projection holding the active-agent fence"
    );
    sqlx::query(
        "INSERT INTO printer_routes
         (workspace_id,environment_id,id,destination_id,printer_id,agent_id,native_queue_id,
          local_route_key,state,role,priority,enabled,capability_revision,profile_revision)
         VALUES ($1,$2,'rte_revoke_race',$3,'ptr_revoke_race',$4,'native-race',
                 'local-race','available','primary',0,true,1,1)",
    )
    .bind(scope.workspace_id.to_string())
    .bind(scope.environment_id.to_string())
    .bind(&projected_destination.id)
    .bind(&agent_id)
    .execute(&mut *projection)
    .await
    .expect("commit projection before revocation");
    projection.commit().await.expect("release projection fence");
    tokio::time::timeout(std::time::Duration::from_secs(5), revoke)
        .await
        .expect("revocation completed after projection")
        .expect("revocation task")
        .expect("revocation result");

    let retired: (bool, String, bool) = sqlx::query_as(
        "SELECT enabled,state,retired_at IS NOT NULL FROM printer_routes
         WHERE workspace_id=$1 AND environment_id=$2 AND id='rte_revoke_race'",
    )
    .bind(scope.workspace_id.to_string())
    .bind(scope.environment_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("projected route retained as audit");
    assert_eq!(retired, (false, "retired".into(), true));

    let reserve_suffix = "01J00000000000000000000011";
    let reserve_agent = format!("agt_{reserve_suffix}");
    let reserve_job = format!("job_{reserve_suffix}");
    create_tenant_fixture(&store, scope, reserve_suffix, "ptr_revoke_reserve").await;
    sqlx::query(
        "UPDATE jobs SET state='waiting_for_agent'
         WHERE workspace_id=$1 AND environment_id=$2 AND id=$3",
    )
    .bind(scope.workspace_id.to_string())
    .bind(scope.environment_id.to_string())
    .bind(&reserve_job)
    .execute(&pool)
    .await
    .expect("reserve race job fixture");
    let mut reserve_destination = destination("pdst_revoke_reserve", "unused");
    reserve_destination.scheduling_authority_id = None;
    store
        .upsert_destination(scope, &reserve_destination)
        .await
        .expect("reserve race destination");
    store
        .upsert_route(
            scope,
            &route(
                "rte_revoke_reserve",
                &reserve_destination.id,
                reserve_suffix,
                "ptr_revoke_reserve",
                "primary",
            ),
        )
        .await
        .expect("reserve race route");
    let mut blocked_job = pool.begin().await.expect("begin blocked job transaction");
    sqlx::query(
        "SELECT id FROM jobs WHERE workspace_id=$1 AND environment_id=$2 AND id=$3 FOR UPDATE",
    )
    .bind(scope.workspace_id.to_string())
    .bind(scope.environment_id.to_string())
    .bind(&reserve_job)
    .fetch_one(&mut *blocked_job)
    .await
    .expect("hold job row while reservation locks route");
    let reservation_store = store.clone();
    let reservation_job = reserve_job.clone();
    let reservation_destination = reserve_destination.id.clone();
    let reservation = tokio::spawn(async move {
        reservation_store
            .begin_delivery_attempt(
                scope,
                NewDeliveryAttempt {
                    attempt_id: "attempt_revoke_reserve_race",
                    reservation_id: "reservation_revoke_reserve_race",
                    job_id: &reservation_job,
                    destination_id: &reservation_destination,
                    route_id: "rte_revoke_reserve",
                    lease_until: Utc::now() + Duration::minutes(1),
                },
            )
            .await
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let revoke_store = store.clone();
    let revoke_reserve = tokio::spawn(async move {
        revoke_store
            .revoke_agent(
                scope.workspace_id,
                scope.environment_id,
                reserve_agent.parse().expect("reserve fixture agent id"),
            )
            .await
    });
    blocked_job.commit().await.expect("release blocked job");
    let reservation_result = tokio::time::timeout(std::time::Duration::from_secs(5), reservation)
        .await
        .expect("reservation/revocation cannot deadlock")
        .expect("reservation task");
    assert!(
        reservation_result.is_ok() || matches!(reservation_result, Err(StorageError::NotFound)),
        "the reservation may win or be fenced, but cannot survive revocation"
    );
    tokio::time::timeout(std::time::Duration::from_secs(5), revoke_reserve)
        .await
        .expect("revocation/reservation cannot deadlock")
        .expect("reserve revocation task")
        .expect("reserve revocation result");
    let active_attempts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM delivery_attempts
         WHERE workspace_id=$1 AND environment_id=$2
           AND route_id='rte_revoke_reserve' AND state='route_leased' AND final_at IS NULL",
    )
    .bind(scope.workspace_id.to_string())
    .bind(scope.environment_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("active reserve-race attempts");
    assert_eq!(active_attempts, 0);

    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .expect("drop exact disposable schema");
}

#[tokio::test]
async fn migration_40_upgrades_39_and_backfills_without_inferring_route_merges() {
    let Some(database_url) = env::var("PIQAE_TEST_DATABASE_URL").ok() else {
        eprintln!("skipped: set PIQAE_TEST_DATABASE_URL for PostgreSQL migration evidence");
        return;
    };
    let schema = format!("piqae_destination_upgrade_{}", ulid::Ulid::new()).to_ascii_lowercase();
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
    let all = sqlx::migrate!("../../migrations/postgres");
    let previous = Migrator {
        migrations: Cow::Owned(
            all.iter()
                .filter(|migration| migration.version < 40)
                .cloned()
                .collect(),
        ),
        ignore_missing: false,
        locking: true,
        no_tx: false,
    };
    previous.run(&pool).await.expect("apply schema version 39");
    let store = PostgresStore::from_pool(pool.clone());
    let scope = TenantScope {
        workspace_id: WorkspaceId::new(),
        environment_id: EnvironmentId::new(),
    };
    create_tenant_fixture(&store, scope, "upgrade", "ptr_upgrade").await;
    store
        .migrate()
        .await
        .expect("upgrade application from 39 to 40");
    sqlx::query("INSERT INTO jobs (id,workspace_id,environment_id,printer_id,agent_id,payload,state,per_printer_sequence,expires_at) VALUES ('job_upgrade_legacy_writer',$1,$2,'ptr_upgrade','agt_upgrade','{}'::jsonb,'registered',2,now()+interval '1 hour')")
        .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).execute(&pool).await.expect("N-1 writer remains compatible while destination columns are nullable");
    let legacy_destination: Option<String> = sqlx::query_scalar("SELECT destination_id FROM jobs WHERE workspace_id=$1 AND environment_id=$2 AND id='job_upgrade_legacy_writer'")
        .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).fetch_one(&pool).await.expect("legacy row remains intentionally unprojected");
    assert!(legacy_destination.is_none());
    sqlx::query("INSERT INTO targets (id,workspace_id,environment_id,name) VALUES ('target_upgrade_legacy',$1,$2,'Legacy writer target')")
        .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).execute(&pool).await.expect("legacy target fixture");
    sqlx::query("INSERT INTO target_bindings (id,workspace_id,environment_id,target_id,printer_id,agent_id,profile_id,profile_revision,role) VALUES ('binding_upgrade_legacy',$1,$2,'target_upgrade_legacy','ptr_upgrade','agt_upgrade','profile_legacy',1,'primary')")
        .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).execute(&pool).await.expect("N-1 target writer remains compatible while route columns are nullable");
    let legacy_binding_destination: Option<String> = sqlx::query_scalar("SELECT destination_id FROM target_bindings WHERE workspace_id=$1 AND environment_id=$2 AND id='binding_upgrade_legacy'")
        .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).fetch_one(&pool).await.expect("legacy binding remains intentionally unprojected");
    assert!(legacy_binding_destination.is_none());
    let destination_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM physical_destinations WHERE workspace_id=$1 AND environment_id=$2",
    )
    .bind(scope.workspace_id.to_string())
    .bind(scope.environment_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("backfilled destination");
    let route_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM printer_routes WHERE workspace_id=$1 AND environment_id=$2",
    )
    .bind(scope.workspace_id.to_string())
    .bind(scope.environment_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("backfilled route");
    let confidence:String=sqlx::query_scalar("SELECT identity_confidence FROM physical_destinations WHERE workspace_id=$1 AND environment_id=$2").bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).fetch_one(&pool).await.expect("conservative identity confidence");
    assert_eq!(
        (destination_count, route_count, confidence.as_str()),
        (1, 1, "unknown")
    );
    let backfilled_route = store
        .list_all_routes(scope)
        .await
        .expect("backfilled route is readable")
        .pop()
        .expect("one backfilled route");
    assert!(backfilled_route.id.starts_with("rte_"));
    assert!(backfilled_route.local_route_key.is_none());
    let stable_route_id = backfilled_route.id.clone();
    let mut upgraded_node_route = backfilled_route.clone();
    upgraded_node_route.id = format!("rte_{}", "f".repeat(32));
    upgraded_node_route.local_route_key = Some(format!("rte_{}", "e".repeat(32)));
    upgraded_node_route.updated_at = Utc::now();
    store
        .upsert_route(scope, &upgraded_node_route)
        .await
        .expect("new node snapshot attaches its local route key to the backfilled row");
    let resolved = store
        .get_route_by_local_key(
            scope,
            "agt_upgrade",
            upgraded_node_route
                .local_route_key
                .as_deref()
                .expect("local route key"),
        )
        .await
        .expect("resolve node-local route key");
    assert_eq!(resolved.id, stable_route_id);
    let started = store
        .begin_delivery_attempt(
            scope,
            NewDeliveryAttempt {
                attempt_id: "attempt_upgrade",
                reservation_id: "reservation_upgrade",
                job_id: "job_upgrade",
                destination_id: &resolved.destination_id,
                route_id: &resolved.id,
                lease_until: Utc::now() + Duration::minutes(1),
            },
        )
        .await
        .expect("server route ID reserves the upgraded node route without rewriting its PK");
    assert_eq!(started.attempt.route_id, stable_route_id);
    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .expect("drop exact disposable schema");
}
