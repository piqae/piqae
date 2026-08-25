#![allow(
    clippy::expect_used,
    clippy::too_many_lines,
    reason = "the PostgreSQL evidence test keeps one disposable-schema lifecycle auditable"
)]

use chrono::{Duration, Utc};
use piqae_domain::{EnvironmentId, WorkspaceId};
use piqae_storage_postgres::{
    PostgresStore, StorageError,
    destination_topology::{
        DeliveryAttemptState, DestinationTopologyRepository, IdentityConfidence, IdentityDecision,
        IdentityDecisionKind, IdentityEvidence, NewDeliveryAttempt, ProjectionAcknowledgement,
        RouteObservation, SchedulingAuthority, SiteCoordinatorMembership,
        StoredPhysicalDestination, StoredPrinterRoute, TenantScope,
    },
};
use sqlx::{PgPool, migrate::Migrator, postgres::PgPoolOptions};
use std::{borrow::Cow, env};

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
        .bind(format!("job_{suffix}"))
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
    create_tenant_fixture(&store, second, "second", "ptr_shared").await;
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
                    "ptr_shared",
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
        Err(StorageError::Database(_))
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
        observed_at: Utc::now(),
        fresh_until: Utc::now() + Duration::seconds(20),
    };
    store
        .record_route_observation(first, &observation)
        .await
        .expect("privacy-safe observation");
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
                connector_id: "ncon_first".into(),
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
                reservation_id: "reservation_1",
                job_id: "job_first",
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
                    job_id: "job_first",
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
                    job_id: "job_first",
                    destination_id: "destination_shared",
                    route_id: "route_first",
                    lease_until: Utc::now() + Duration::minutes(1),
                },
            )
            .await,
        Err(StorageError::ConcurrentStateChange)
    ));

    sqlx::query("INSERT INTO jobs (id,workspace_id,environment_id,printer_id,agent_id,payload,state,per_printer_sequence,expires_at,destination_id,route_id) VALUES ('job_expired',$1,$2,'ptr_backup','agt_first_backup','{}'::jsonb,'registered',2,now()+interval '1 hour','destination_shared','route_first_backup')")
        .bind(first.workspace_id.to_string()).bind(first.environment_id.to_string()).execute(store.pool()).await.expect("expired lease job");
    let expired = store
        .begin_delivery_attempt(
            first,
            NewDeliveryAttempt {
                attempt_id: "attempt_expired_1",
                reservation_id: "reservation_expired_1",
                job_id: "job_expired",
                destination_id: "destination_shared",
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
                destination_id: "destination_shared",
                route_id: "route_first_backup",
                lease_until: Utc::now() + Duration::minutes(1),
            },
        )
        .await
        .expect("atomically supersede expired unaccepted attempt");
    assert_eq!(recovered.attempt.generation, 2);
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
                    destination_id: "destination_shared",
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

    for (job_id, sequence) in [("job_race_a", 2_i64), ("job_race_b", 3_i64)] {
        sqlx::query("INSERT INTO jobs (id,workspace_id,environment_id,printer_id,agent_id,payload,state,per_printer_sequence,expires_at,destination_id,route_id) VALUES ($1,$2,$3,'ptr_shared','agt_first','{}'::jsonb,'registered',$4,now()+interval '1 hour','destination_shared','route_first')")
            .bind(job_id).bind(first.workspace_id.to_string()).bind(first.environment_id.to_string()).bind(sequence).execute(store.pool()).await.expect("scheduler race job");
    }
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
                route_id: "route_first",
                lease_until: Utc::now() + Duration::minutes(1)
            }
        )
    );
    assert_eq!(usize::from(race_a.is_ok()) + usize::from(race_b.is_ok()), 1);
    assert!(matches!(
        race_a.as_ref().err().or_else(|| race_b.as_ref().err()),
        Some(StorageError::ConcurrentStateChange)
    ));

    let decision_count: i64 = sqlx::query_scalar("SELECT count(*) FROM destination_identity_decisions WHERE workspace_id=$1 AND environment_id=$2").bind(first.workspace_id.to_string()).bind(first.environment_id.to_string()).fetch_one(&pool).await.expect("decision count");
    assert_eq!(decision_count, 2);
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
async fn migration_42_upgrades_41_and_backfills_without_inferring_route_merges() {
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
                .filter(|migration| migration.version < 42)
                .cloned()
                .collect(),
        ),
        ignore_missing: false,
        locking: true,
        no_tx: false,
    };
    previous.run(&pool).await.expect("apply schema version 41");
    let store = PostgresStore::from_pool(pool.clone());
    let scope = TenantScope {
        workspace_id: WorkspaceId::new(),
        environment_id: EnvironmentId::new(),
    };
    create_tenant_fixture(&store, scope, "upgrade", "ptr_upgrade").await;
    store
        .migrate()
        .await
        .expect("upgrade application from 41 to 42");
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
    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .expect("drop exact disposable schema");
}
