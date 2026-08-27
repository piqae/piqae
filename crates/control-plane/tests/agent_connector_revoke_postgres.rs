#![allow(clippy::expect_used, clippy::too_many_lines)]

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use base64::{Engine as _, engine::general_purpose::STANDARD_NO_PAD};
use chrono::{Duration, Utc};
use ed25519_dalek::{Signer as _, SigningKey};
use http_body_util::BodyExt as _;
use piqae_agent_storage::{AcceptedJob, AgentStore, CloudRouteProof};
use piqae_control_plane::{
    AppState, authentication::PostgresAuthenticator, repository::Repository, router,
};
use piqae_domain::{
    AgentId, ContentKind, ContentSource, EnvironmentId, EventId, Job, JobEvent, JobId, JobOptions,
    JobState, PrinterId, WorkspaceId,
};
use piqae_protocol::agent::{
    AgentAcceptJobRequest, AgentAcceptanceReconciliationResponse, AgentReleaseLeaseRequest,
};
use piqae_storage_postgres::{
    DeliveryAttemptProof, PostgresStore, StorageError,
    acceptance_revocation_webhook_idempotency_key, agent_acceptance_webhook_idempotency_key,
    preaccept_cancellation_webhook_idempotency_key,
};
use sha2::{Digest as _, Sha256};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{collections::BTreeMap, env, sync::Arc};
use tower::ServiceExt as _;

async fn schema_pool(database_url: &str, schema: &str) -> PgPool {
    let schema = schema.to_owned();
    PgPoolOptions::new()
        .max_connections(4)
        .after_connect(move |connection, _| {
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

async fn single_schema_pool(database_url: &str, schema: &str) -> PgPool {
    let schema = schema.to_owned();
    PgPoolOptions::new()
        .max_connections(1)
        .after_connect(move |connection, _| {
            let statement = format!("SET search_path TO {schema}");
            Box::pin(async move {
                sqlx::query(&statement).execute(connection).await?;
                Ok(())
            })
        })
        .connect(database_url)
        .await
        .expect("connect serialized PostgreSQL test pool")
}

async fn wait_until_blocked(pool: &PgPool, pid: i32) {
    for _ in 0..100 {
        let blocked: bool = sqlx::query_scalar("SELECT cardinality(pg_blocking_pids($1)) > 0")
            .bind(pid)
            .fetch_one(pool)
            .await
            .expect("inspect blocked backend");
        if blocked {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("backend did not reach the connector-row lock");
}

fn signed_request(
    agent_id: AgentId,
    signing_key: &SigningKey,
    method: &str,
    path: &str,
    body: Vec<u8>,
) -> Request<Body> {
    let timestamp = Utc::now().timestamp_millis();
    let nonce = uuid::Uuid::new_v4();
    let digest = format!("{:x}", Sha256::digest(&body));
    let canonical = format!("{method}\n{path}\n{timestamp}\n{nonce}\n{digest}");
    let signature = signing_key.sign(canonical.as_bytes());
    Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json")
        .header("x-piqae-agent-id", agent_id.to_string())
        .header("x-piqae-timestamp", timestamp.to_string())
        .header("x-piqae-nonce", nonce.to_string())
        .header("x-piqae-body-sha256", digest)
        .header(
            "x-piqae-signature",
            STANDARD_NO_PAD.encode(signature.to_bytes()),
        )
        .body(Body::from(body))
        .expect("valid signed request")
}

struct Fixture {
    workspace_id: WorkspaceId,
    environment_id: EnvironmentId,
    agent_id: AgentId,
    printer_id: PrinterId,
    job_id: JobId,
    connector_id: String,
    signing_key: SigningKey,
    lease_id: uuid::Uuid,
    lease_token: String,
    content_sha256: String,
    proof: CloudRouteProof,
}

async fn insert_fixture(pool: &PgPool, store: &PostgresStore) -> Fixture {
    let workspace_id = WorkspaceId::new();
    let environment_id = EnvironmentId::new();
    let agent_id = AgentId::new();
    let printer_id = PrinterId::new();
    let job_id = JobId::new();
    let connector_id = format!("ncon_{}", ulid::Ulid::new());
    let installation_id = format!("ninst_{}", ulid::Ulid::new());
    let destination_id = format!("pdst_{}", ulid::Ulid::new());
    let route_id = format!("rte_{}", ulid::Ulid::new());
    let attempt_id = format!("datt_{}", ulid::Ulid::new());
    let reservation_id = uuid::Uuid::new_v4().to_string();
    let fencing_token = format!("fence-{}", ulid::Ulid::new());
    let fencing_hash = format!("{:x}", Sha256::digest(fencing_token.as_bytes()));
    let signing_key = SigningKey::from_bytes(&[37; 32]);
    let public_key = signing_key.verifying_key().to_bytes().to_vec();
    sqlx::query("INSERT INTO workspaces (id,name,slug) VALUES ($1,'Revoke fixture',$2)")
        .bind(workspace_id.to_string())
        .bind(format!("revoke-{}", ulid::Ulid::new()).to_ascii_lowercase())
        .execute(pool)
        .await
        .expect("workspace fixture");
    sqlx::query(
        "INSERT INTO environments (id,workspace_id,kind,name) VALUES ($1,$2,'live','Live')",
    )
    .bind(environment_id.to_string())
    .bind(workspace_id.to_string())
    .execute(pool)
    .await
    .expect("environment fixture");
    sqlx::query(
        "INSERT INTO node_installations (id,installation_key,public_key) VALUES ($1,$2,$3)",
    )
    .bind(&installation_id)
    .bind(format!("test:{}", ulid::Ulid::new()))
    .bind(&public_key)
    .execute(pool)
    .await
    .expect("installation fixture");
    sqlx::query("INSERT INTO agents (id,workspace_id,environment_id,name,installation_id,public_key,os,architecture,version,protocol_version) VALUES ($1,$2,$3,'Embedded node',$4,$5,'ios','arm64','test',1)")
        .bind(agent_id.to_string()).bind(workspace_id.to_string()).bind(environment_id.to_string())
        .bind(format!("embedded:{}", ulid::Ulid::new())).bind(&public_key)
        .execute(pool).await.expect("agent fixture");
    sqlx::query("INSERT INTO node_connectors (id,installation_id,workspace_id,environment_id,agent_id) VALUES ($1,$2,$3,$4,$5)")
        .bind(&connector_id).bind(&installation_id).bind(workspace_id.to_string())
        .bind(environment_id.to_string()).bind(agent_id.to_string())
        .execute(pool).await.expect("connector fixture");
    sqlx::query("INSERT INTO printers (id,workspace_id,environment_id,agent_id,native_id,name,state,capabilities_revision) VALUES ($1,$2,$3,$4,'native-revoke','Revoke printer','online',1)")
        .bind(printer_id.to_string()).bind(workspace_id.to_string()).bind(environment_id.to_string())
        .bind(agent_id.to_string()).execute(pool).await.expect("printer fixture");
    sqlx::query("INSERT INTO physical_destinations (workspace_id,environment_id,id,name,state) VALUES ($1,$2,$3,'Destination','available')")
        .bind(workspace_id.to_string()).bind(environment_id.to_string()).bind(&destination_id)
        .execute(pool).await.expect("destination fixture");
    sqlx::query("INSERT INTO printer_routes (workspace_id,environment_id,id,destination_id,printer_id,agent_id,native_queue_id,state,role,priority,enabled) VALUES ($1,$2,$3,$4,$5,$6,'native-revoke','available','primary',0,true)")
        .bind(workspace_id.to_string()).bind(environment_id.to_string()).bind(&route_id)
        .bind(&destination_id).bind(printer_id.to_string()).bind(agent_id.to_string())
        .execute(pool).await.expect("route fixture");
    let content = "cHJpbnQ=";
    let content_sha256 = format!("{:x}", Sha256::digest(b"print"));
    let now = Utc::now();
    let job = Job {
        id: job_id,
        workspace_id,
        environment_id,
        printer_id,
        title: "Crash boundary".into(),
        source: None,
        content_kind: ContentKind::Pdf,
        content: ContentSource::Base64 {
            data: content.into(),
        },
        options: JobOptions::default(),
        metadata: BTreeMap::from([
            ("piqae.destination_id".into(), destination_id.clone()),
            ("piqae.route_id".into(), route_id.clone()),
        ]),
        deliveries: 1,
        state: JobState::WaitingForAgent,
        created_at: now,
        expires_at: now + Duration::hours(1),
        delivery_uncertain_since: None,
    };
    store
        .create_job(&job, agent_id, None, b"accept-crash-boundary")
        .await
        .expect("waiting job fixture");
    sqlx::query("UPDATE jobs SET destination_id=$2,route_id=$3 WHERE id=$1")
        .bind(job_id.to_string())
        .bind(&destination_id)
        .bind(&route_id)
        .execute(pool)
        .await
        .expect("route job");
    sqlx::query("INSERT INTO delivery_attempts (workspace_id,environment_id,id,job_id,destination_id,route_id,generation,fencing_token_hash,state,lease_until) VALUES ($1,$2,$3,$4,$5,$6,7,$7,'route_leased',now()+interval '1 hour')")
        .bind(workspace_id.to_string()).bind(environment_id.to_string()).bind(&attempt_id)
        .bind(job_id.to_string()).bind(&destination_id).bind(&route_id).bind(&fencing_hash)
        .execute(pool).await.expect("attempt fixture");
    sqlx::query("INSERT INTO route_reservations (workspace_id,environment_id,id,route_id,destination_id,job_id,attempt_id,generation,fencing_token_hash,state,lease_until) VALUES ($1,$2,$3,$4,$5,$6,$7,7,$8,'active',now()+interval '1 hour')")
        .bind(workspace_id.to_string()).bind(environment_id.to_string()).bind(&reservation_id)
        .bind(&route_id).bind(&destination_id).bind(job_id.to_string()).bind(&attempt_id)
        .bind(&fencing_hash).execute(pool).await.expect("reservation fixture");
    let lease = store
        .claim_jobs(workspace_id, environment_id, agent_id, "fixture", 1)
        .await
        .expect("claim job")
        .pop()
        .expect("one lease");
    Fixture {
        workspace_id,
        environment_id,
        agent_id,
        printer_id,
        job_id,
        connector_id,
        signing_key,
        lease_id: lease.lease_id,
        lease_token: lease.lease_token,
        content_sha256,
        proof: CloudRouteProof {
            reservation_id,
            generation: 7,
            fencing_token,
        },
    }
}

#[tokio::test]
async fn preaccept_cancel_repair_commits_one_durable_event_across_retries() {
    let Some(database_url) = env::var("PIQAE_TEST_DATABASE_URL").ok() else {
        eprintln!("skipped: set PIQAE_TEST_DATABASE_URL for cancellation repair evidence");
        return;
    };
    let schema = format!("piqae_cancel_repair_{}", ulid::Ulid::new()).to_ascii_lowercase();
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .expect("connect PostgreSQL test database");
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .expect("create disposable schema");
    let pool = schema_pool(&database_url, &schema).await;
    let store = PostgresStore::from_pool(pool.clone());
    store.migrate().await.expect("apply migrations");

    let direct = insert_fixture(&pool, &store).await;
    let directly_cancelled = store
        .request_job_cancellation(
            direct.workspace_id,
            direct.environment_id,
            direct.job_id,
            &serde_json::to_value(piqae_protocol::agent::AgentCommand::CancelJob {
                job_id: direct.job_id,
            })
            .expect("cancel command JSON"),
        )
        .await
        .expect("direct pre-accept cancellation");
    assert_eq!(directly_cancelled.state, JobState::Cancelled);
    let direct_key = preaccept_cancellation_webhook_idempotency_key(
        direct.workspace_id,
        direct.environment_id,
        direct.job_id,
    );
    store
        .enqueue_webhook_event_idempotently(
            &direct_key,
            direct.workspace_id,
            direct.environment_id,
            "job.updated",
            &serde_json::to_value(&directly_cancelled).expect("cancelled job JSON"),
        )
        .await
        .expect("HTTP publication replay");
    let direct_event_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM webhook_events
         WHERE workspace_id=$1 AND environment_id=$2 AND idempotency_key=$3
           AND event_type='job.updated'",
    )
    .bind(direct.workspace_id.to_string())
    .bind(direct.environment_id.to_string())
    .bind(&direct_key)
    .fetch_one(&pool)
    .await
    .expect("direct cancellation event count");
    assert_eq!(direct_event_count, 1);

    let fixture = insert_fixture(&pool, &store).await;
    let sequence = store
        .list_job_events(fixture.workspace_id, fixture.environment_id, fixture.job_id)
        .await
        .expect("job events")
        .last()
        .map_or(1, |event| event.sequence.saturating_add(1));
    store
        .append_event(
            fixture.workspace_id,
            fixture.environment_id,
            &JobEvent {
                id: EventId::new(),
                job_id: fixture.job_id,
                sequence,
                state: JobState::CancelRequested,
                reason: None,
                message: Some("legacy cancellation request".into()),
                agent_id: None,
                native_job_id: None,
                occurred_at: Utc::now(),
            },
        )
        .await
        .expect("legacy cancel-requested state");
    store
        .enqueue_agent_command(
            fixture.workspace_id,
            fixture.environment_id,
            fixture.agent_id,
            &serde_json::to_value(piqae_protocol::agent::AgentCommand::CancelJob {
                job_id: fixture.job_id,
            })
            .expect("command JSON"),
        )
        .await
        .expect("legacy cancel command");

    for _ in 0..2 {
        assert!(
            store
                .retire_terminal_absent_local_cancellation(
                    fixture.workspace_id,
                    fixture.environment_id,
                    fixture.agent_id,
                    fixture.job_id,
                )
                .await
                .expect("idempotent repair")
        );
    }
    let idempotency_key = preaccept_cancellation_webhook_idempotency_key(
        fixture.workspace_id,
        fixture.environment_id,
        fixture.job_id,
    );
    let event_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM webhook_events
         WHERE workspace_id=$1 AND environment_id=$2 AND idempotency_key=$3
           AND event_type='job.updated'",
    )
    .bind(fixture.workspace_id.to_string())
    .bind(fixture.environment_id.to_string())
    .bind(idempotency_key)
    .fetch_one(&pool)
    .await
    .expect("repair event count");
    assert_eq!(event_count, 1);
    let command_acknowledged: bool = sqlx::query_scalar(
        "SELECT acknowledged_at IS NOT NULL FROM agent_commands
         WHERE workspace_id=$1 AND environment_id=$2 AND agent_id=$3
           AND ((command->>'type'='cancel_job' AND command->>'job_id'=$4)
                OR command->'cancel_job'->>'job_id'=$4)",
    )
    .bind(fixture.workspace_id.to_string())
    .bind(fixture.environment_id.to_string())
    .bind(fixture.agent_id.to_string())
    .bind(fixture.job_id.as_ulid().to_string())
    .fetch_one(&pool)
    .await
    .expect("command acknowledgement");
    assert!(command_acknowledged);

    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .expect("drop disposable schema");
}

#[tokio::test]
async fn accepted_http_crash_then_external_revoke_never_becomes_runnable() {
    let Some(database_url) = env::var("PIQAE_TEST_DATABASE_URL").ok() else {
        eprintln!("skipped: set PIQAE_TEST_DATABASE_URL for connector revoke HTTP evidence");
        return;
    };
    let schema = format!("piqae_connector_revoke_{}", ulid::Ulid::new()).to_ascii_lowercase();
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .expect("connect PostgreSQL test database");
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .expect("create disposable schema");
    let pool = schema_pool(&database_url, &schema).await;
    let store = PostgresStore::from_pool(pool.clone());
    store.migrate().await.expect("apply migrations");
    let fixture = insert_fixture(&pool, &store).await;
    let application = router(AppState::new_for_tests(
        Arc::new(store.clone()) as Arc<dyn Repository>,
        Arc::new(PostgresAuthenticator::new(store.clone())),
    ));
    let directory = tempfile::tempdir().expect("local queue directory");
    let database = directory.path().join("connector.sqlite");
    let mut local = AgentStore::open(&database).expect("open local queue");
    let accepted = local
        .prepare_cloud_job(
            &AcceptedJob {
                job_id: fixture.job_id.to_string(),
                submission_id: "remote-offer".into(),
                printer_id: fixture.printer_id.to_string(),
                printer_native_id: "native-revoke".into(),
                title: "Crash boundary".into(),
                content_sha256: fixture.content_sha256.clone(),
                content_path: directory
                    .path()
                    .join("content.pdf")
                    .to_string_lossy()
                    .into_owned(),
                content_kind: "pdf".into(),
                options_json: "{}".into(),
                expires_unix_ms: None,
                accepted_unix_ms: Utc::now().timestamp_millis(),
                cloud_managed: true,
            },
            &fixture.lease_id.to_string(),
            &fixture.lease_token,
            (Utc::now() + Duration::minutes(1)).timestamp_millis(),
            &fixture.proof,
        )
        .expect("durable local prepare");
    let request = AgentAcceptJobRequest {
        lease_id: fixture.lease_id,
        lease_token: fixture.lease_token.clone(),
        content_sha256: fixture.content_sha256.clone(),
        local_sequence: u64::try_from(accepted.printer_sequence).expect("sequence"),
        route_reservation_id: Some(
            fixture
                .proof
                .reservation_id
                .parse()
                .expect("reservation id"),
        ),
        route_generation: Some(fixture.proof.generation),
        route_fencing_token: Some(fixture.proof.fencing_token.clone()),
    };
    let accept_path = format!("/v1/agent/jobs/{}/accept", fixture.job_id);
    let body = serde_json::to_vec(&request).expect("accept body");
    let response = application
        .clone()
        .oneshot(signed_request(
            fixture.agent_id,
            &fixture.signing_key,
            "POST",
            &accept_path,
            body,
        ))
        .await
        .expect("accept response");
    assert_eq!(response.status(), StatusCode::OK);
    let acceptance_key = agent_acceptance_webhook_idempotency_key(
        fixture.workspace_id,
        fixture.environment_id,
        fixture.job_id,
    );
    let acceptance_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM webhook_events
         WHERE workspace_id=$1 AND environment_id=$2
           AND idempotency_key=$3 AND event_type='job.updated'",
    )
    .bind(fixture.workspace_id.to_string())
    .bind(fixture.environment_id.to_string())
    .bind(&acceptance_key)
    .fetch_one(&pool)
    .await
    .expect("acceptance transaction persisted tenant event");
    assert_eq!(acceptance_events, 1);
    // Simulate an N-1 replica accepting after migration without writing the
    // newly appended proof/generation columns. N reconciliation must validate
    // the live route under locks and upgrade the exact row atomically.
    sqlx::query(
        "UPDATE job_acceptances SET route_reservation_id=NULL,route_generation=NULL,
                route_fencing_token_hash=NULL,connector_generation=NULL WHERE job_id=$1",
    )
    .bind(fixture.job_id.to_string())
    .execute(&pool)
    .await
    .expect("simulate rolling N-1 acceptance");
    let reconcile_path = format!("/v1/agent/jobs/{}/acceptance/reconcile", fixture.job_id);
    let upgraded = application
        .clone()
        .oneshot(signed_request(
            fixture.agent_id,
            &fixture.signing_key,
            "POST",
            &reconcile_path,
            serde_json::to_vec(&request).expect("rolling reconcile body"),
        ))
        .await
        .expect("rolling reconcile response");
    assert_eq!(upgraded.status(), StatusCode::OK);
    let upgraded: AgentAcceptanceReconciliationResponse = serde_json::from_slice(
        &upgraded
            .into_body()
            .collect()
            .await
            .expect("rolling reconcile response body")
            .to_bytes(),
    )
    .expect("rolling reconcile JSON");
    assert!(upgraded.accepted && !upgraded.connector_revoked && !upgraded.fenced);
    let events_after_reconcile: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM webhook_events
         WHERE workspace_id=$1 AND environment_id=$2 AND idempotency_key=$3",
    )
    .bind(fixture.workspace_id.to_string())
    .bind(fixture.environment_id.to_string())
    .bind(&acceptance_key)
    .fetch_one(&pool)
    .await
    .expect("reconciliation replayed acceptance event idempotently");
    assert_eq!(events_after_reconcile, 1);
    let upgraded_generation: Option<i64> =
        sqlx::query_scalar("SELECT connector_generation FROM job_acceptances WHERE job_id=$1")
            .bind(fixture.job_id.to_string())
            .fetch_one(&pool)
            .await
            .expect("upgraded connector generation");
    assert_eq!(upgraded_generation, Some(1));
    drop(local); // exact crash: authority committed, local confirmation did not.

    let affected = store
        .revoke_node_connector(
            fixture.workspace_id,
            fixture.environment_id,
            fixture.agent_id,
            &fixture.connector_id,
        )
        .await
        .expect("external platform revoke");
    assert_eq!(affected.len(), 1);
    assert_eq!(affected[0].state, JobState::DeliveryUncertain);

    let response = application
        .clone()
        .oneshot(signed_request(
            fixture.agent_id,
            &fixture.signing_key,
            "POST",
            &reconcile_path,
            serde_json::to_vec(&request).expect("reconcile body"),
        ))
        .await
        .expect("reconcile response");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("reconcile body")
        .to_bytes();
    assert_eq!(
        status,
        StatusCode::OK,
        "reconcile failed: {}",
        String::from_utf8_lossy(&bytes)
    );
    let outcome: AgentAcceptanceReconciliationResponse =
        serde_json::from_slice(&bytes).expect("reconcile JSON");
    assert!(!outcome.accepted && outcome.connector_revoked && outcome.fenced);
    let denied = application
        .clone()
        .oneshot(signed_request(
            fixture.agent_id,
            &fixture.signing_key,
            "POST",
            "/v1/agent/sync",
            b"{}".to_vec(),
        ))
        .await
        .expect("post-revoke sync");
    assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
    sqlx::query(
        "UPDATE node_connectors SET revoked_at=NULL,updated_at=now()
         WHERE id=$1",
    )
    .bind(&fixture.connector_id)
    .execute(&pool)
    .await
    .expect("simulate exact connector re-enrolment");
    let reactivated_generation: i64 =
        sqlx::query_scalar("SELECT admission_generation FROM node_connectors WHERE id=$1")
            .bind(&fixture.connector_id)
            .fetch_one(&pool)
            .await
            .expect("reactivated connector generation");
    assert_eq!(reactivated_generation, 2);
    let reauthenticated = router(AppState::new_for_tests(
        Arc::new(store.clone()) as Arc<dyn Repository>,
        Arc::new(PostgresAuthenticator::new(store.clone())),
    ))
    .oneshot(signed_request(
        fixture.agent_id,
        &fixture.signing_key,
        "POST",
        &reconcile_path,
        serde_json::to_vec(&request).expect("re-enrolled reconcile body"),
    ))
    .await
    .expect("re-enrolled reconcile response");
    assert_eq!(reauthenticated.status(), StatusCode::OK);
    let bytes = reauthenticated
        .into_body()
        .collect()
        .await
        .expect("re-enrolled reconcile body")
        .to_bytes();
    let reauthenticated: AgentAcceptanceReconciliationResponse =
        serde_json::from_slice(&bytes).expect("re-enrolled reconcile JSON");
    assert!(!reauthenticated.accepted);
    assert!(!reauthenticated.connector_revoked);
    assert!(reauthenticated.fenced);
    let mut restarted = AgentStore::open(&database).expect("restart local queue");
    restarted
        .abandon_cloud_accept(&fixture.job_id.to_string(), Utc::now().timestamp_millis())
        .expect("fence local pending acceptance");
    drop(restarted);
    let restarted = AgentStore::open(&database).expect("second restart");
    assert!(
        restarted
            .pending_cloud_accepts()
            .expect("pending")
            .is_empty()
    );
    assert!(
        restarted
            .runnable_heads(Utc::now().timestamp_millis())
            .expect("runnable")
            .is_empty()
    );
    assert!(
        restarted
            .pending_cloud_events(0, 10)
            .expect("cloud events")
            .is_empty()
    );
    let state: String = sqlx::query_scalar("SELECT state FROM jobs WHERE id=$1")
        .bind(fixture.job_id.to_string())
        .fetch_one(&pool)
        .await
        .expect("job state");
    assert_eq!(state, "delivery_uncertain");
    let attempt: String = sqlx::query_scalar("SELECT state FROM delivery_attempts WHERE job_id=$1")
        .bind(fixture.job_id.to_string())
        .fetch_one(&pool)
        .await
        .expect("attempt state");
    assert_eq!(attempt, "delivery_uncertain");
    let reservation: String =
        sqlx::query_scalar("SELECT state FROM route_reservations WHERE job_id=$1")
            .bind(fixture.job_id.to_string())
            .fetch_one(&pool)
            .await
            .expect("reservation state");
    assert_eq!(reservation, "released");
    let destination: String = sqlx::query_scalar(
        "SELECT state FROM physical_destinations WHERE workspace_id=$1 AND environment_id=$2",
    )
    .bind(fixture.workspace_id.to_string())
    .bind(fixture.environment_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("destination state");
    assert_eq!(destination, "attention");
    // N-1 restart: local route proof is gone even though the authority
    // committed acceptance. A 409 release stays quarantined until connector
    // revocation force-fences the server state, without an outbound cancel.
    let legacy = insert_fixture(&pool, &store).await;
    let legacy_directory = tempfile::tempdir().expect("legacy queue directory");
    let legacy_database = legacy_directory.path().join("connector.sqlite");
    let mut legacy_local = AgentStore::open(&legacy_database).expect("open legacy queue");
    let legacy_job = legacy_local
        .prepare_cloud_job(
            &AcceptedJob {
                job_id: legacy.job_id.to_string(),
                submission_id: "legacy-offer".into(),
                printer_id: legacy.printer_id.to_string(),
                printer_native_id: "native-revoke".into(),
                title: "Legacy crash boundary".into(),
                content_sha256: legacy.content_sha256.clone(),
                content_path: legacy_directory
                    .path()
                    .join("content.pdf")
                    .to_string_lossy()
                    .into_owned(),
                content_kind: "pdf".into(),
                options_json: "{}".into(),
                expires_unix_ms: None,
                accepted_unix_ms: Utc::now().timestamp_millis(),
                cloud_managed: true,
            },
            &legacy.lease_id.to_string(),
            &legacy.lease_token,
            (Utc::now() + Duration::minutes(1)).timestamp_millis(),
            &legacy.proof,
        )
        .expect("durable legacy prepare");
    let legacy_request = AgentAcceptJobRequest {
        lease_id: legacy.lease_id,
        lease_token: legacy.lease_token.clone(),
        content_sha256: legacy.content_sha256.clone(),
        local_sequence: u64::try_from(legacy_job.printer_sequence).expect("legacy sequence"),
        route_reservation_id: Some(
            legacy
                .proof
                .reservation_id
                .parse()
                .expect("legacy reservation id"),
        ),
        route_generation: Some(legacy.proof.generation),
        route_fencing_token: Some(legacy.proof.fencing_token.clone()),
    };
    let legacy_accept_path = format!("/v1/agent/jobs/{}/accept", legacy.job_id);
    let legacy_router = router(AppState::new_for_tests(
        Arc::new(store.clone()) as Arc<dyn Repository>,
        Arc::new(PostgresAuthenticator::new(store.clone())),
    ));
    let accepted = legacy_router
        .clone()
        .oneshot(signed_request(
            legacy.agent_id,
            &legacy.signing_key,
            "POST",
            &legacy_accept_path,
            serde_json::to_vec(&legacy_request).expect("legacy accept body"),
        ))
        .await
        .expect("legacy accept response");
    assert_eq!(accepted.status(), StatusCode::OK);
    drop(legacy_local);
    let legacy_sqlite = rusqlite::Connection::open(&legacy_database).expect("open legacy sqlite");
    legacy_sqlite
        .execute(
            "UPDATE cloud_accept_intents SET route_reservation_id=NULL,
                    route_generation=NULL,route_fencing_token=NULL",
            [],
        )
        .expect("remove N-1 route proof");
    drop(legacy_sqlite);
    let mut legacy_restarted = AgentStore::open(&legacy_database).expect("restart N-1 queue");
    assert_eq!(
        legacy_restarted
            .quarantine_invalid_cloud_accepts(Utc::now().timestamp_millis())
            .expect("quarantine N-1 intent")
            .len(),
        1
    );
    assert!(
        legacy_restarted
            .pending_cloud_events(0, 10)
            .expect("events")
            .is_empty()
    );
    let release_path = format!("/v1/agent/jobs/{}/release", legacy.job_id);
    let release_response = legacy_router
        .oneshot(signed_request(
            legacy.agent_id,
            &legacy.signing_key,
            "POST",
            &release_path,
            serde_json::to_vec(&AgentReleaseLeaseRequest {
                lease_id: legacy.lease_id,
                lease_token: legacy.lease_token.clone(),
                reason: "legacy_route_proof_missing".into(),
            })
            .expect("release body"),
        ))
        .await
        .expect("release conflict response");
    assert_eq!(release_response.status(), StatusCode::CONFLICT);
    assert_eq!(
        legacy_restarted
            .pending_cloud_release_cleanups()
            .expect("cleanup")
            .len(),
        1
    );
    let affected = store
        .revoke_node_connector(
            legacy.workspace_id,
            legacy.environment_id,
            legacy.agent_id,
            &legacy.connector_id,
        )
        .await
        .expect("revoke N-1 connector");
    assert_eq!(affected.len(), 1);
    assert_eq!(affected[0].state, JobState::DeliveryUncertain);
    legacy_restarted
        .complete_cloud_release_cleanup(&legacy.job_id.to_string())
        .expect("complete cleanup after force fence");
    drop(legacy_restarted);
    let legacy_restarted = AgentStore::open(&legacy_database).expect("restart cleaned N-1 queue");
    assert!(
        legacy_restarted
            .pending_cloud_release_cleanups()
            .expect("cleanup")
            .is_empty()
    );
    assert!(
        legacy_restarted
            .runnable_heads(Utc::now().timestamp_millis())
            .expect("runnable")
            .is_empty()
    );
    let legacy_state: String = sqlx::query_scalar("SELECT state FROM jobs WHERE id=$1")
        .bind(legacy.job_id.to_string())
        .fetch_one(&pool)
        .await
        .expect("legacy job state");
    assert_eq!(legacy_state, "delivery_uncertain");

    let node_revoke = insert_fixture(&pool, &store).await;
    store
        .accept_agent_job_with_delivery_attempt(
            node_revoke.workspace_id,
            node_revoke.environment_id,
            node_revoke.agent_id,
            node_revoke.job_id,
            node_revoke.lease_id,
            &node_revoke.lease_token,
            Some(&node_revoke.content_sha256),
            1,
            DeliveryAttemptProof {
                reservation_id: &node_revoke.proof.reservation_id,
                generation: node_revoke.proof.generation,
                fencing_token: &node_revoke.proof.fencing_token,
            },
        )
        .await
        .expect("accept full-node revoke fixture");
    let node_affected = store
        .revoke_agent(
            node_revoke.workspace_id,
            node_revoke.environment_id,
            node_revoke.agent_id,
        )
        .await
        .expect("full-node revoke uses acceptance-aware connector sweep");
    assert_eq!(node_affected.len(), 1);
    assert_eq!(node_affected[0].state, JobState::DeliveryUncertain);
    let node_projection: (String, String, String) = sqlx::query_as(
        "SELECT attempt.state,reservation.state,destination.state
         FROM delivery_attempts AS attempt
         JOIN route_reservations AS reservation
           ON reservation.workspace_id=attempt.workspace_id
          AND reservation.environment_id=attempt.environment_id
          AND reservation.attempt_id=attempt.id
         JOIN physical_destinations AS destination
           ON destination.workspace_id=attempt.workspace_id
          AND destination.environment_id=attempt.environment_id
          AND destination.id=attempt.destination_id
         WHERE attempt.job_id=$1",
    )
    .bind(node_revoke.job_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("full-node revoke projections");
    assert_eq!(
        node_projection,
        (
            "delivery_uncertain".into(),
            "released".into(),
            "attention".into()
        )
    );

    let legacy_node_revoke = insert_fixture(&pool, &store).await;
    sqlx::query("DELETE FROM node_connectors WHERE id=$1")
        .bind(&legacy_node_revoke.connector_id)
        .execute(&pool)
        .await
        .expect("remove connector to model a legacy accepted node");
    store
        .accept_agent_job_with_delivery_attempt(
            legacy_node_revoke.workspace_id,
            legacy_node_revoke.environment_id,
            legacy_node_revoke.agent_id,
            legacy_node_revoke.job_id,
            legacy_node_revoke.lease_id,
            &legacy_node_revoke.lease_token,
            Some(&legacy_node_revoke.content_sha256),
            1,
            DeliveryAttemptProof {
                reservation_id: &legacy_node_revoke.proof.reservation_id,
                generation: legacy_node_revoke.proof.generation,
                fencing_token: &legacy_node_revoke.proof.fencing_token,
            },
        )
        .await
        .expect("accept legacy node without a connector row");
    let legacy_node_affected = store
        .revoke_agent(
            legacy_node_revoke.workspace_id,
            legacy_node_revoke.environment_id,
            legacy_node_revoke.agent_id,
        )
        .await
        .expect("full-node revoke sweeps acceptance evidence without a connector row");
    assert_eq!(legacy_node_affected.len(), 1);
    assert_eq!(legacy_node_affected[0].state, JobState::DeliveryUncertain);
    let legacy_node_projection: (String, String, String, String) = sqlx::query_as(
        "SELECT job.state,attempt.state,reservation.state,destination.state
         FROM jobs AS job
         JOIN delivery_attempts AS attempt
           ON attempt.workspace_id=job.workspace_id
          AND attempt.environment_id=job.environment_id
          AND attempt.job_id=job.id
         JOIN route_reservations AS reservation
           ON reservation.workspace_id=attempt.workspace_id
          AND reservation.environment_id=attempt.environment_id
          AND reservation.attempt_id=attempt.id
         JOIN physical_destinations AS destination
           ON destination.workspace_id=attempt.workspace_id
          AND destination.environment_id=attempt.environment_id
          AND destination.id=attempt.destination_id
         WHERE job.id=$1",
    )
    .bind(legacy_node_revoke.job_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("legacy full-node revoke projections");
    assert_eq!(
        legacy_node_projection,
        (
            "delivery_uncertain".into(),
            "delivery_uncertain".into(),
            "released".into(),
            "attention".into()
        )
    );

    let cancelled = insert_fixture(&pool, &store).await;
    store
        .accept_agent_job_with_delivery_attempt(
            cancelled.workspace_id,
            cancelled.environment_id,
            cancelled.agent_id,
            cancelled.job_id,
            cancelled.lease_id,
            &cancelled.lease_token,
            Some(&cancelled.content_sha256),
            1,
            DeliveryAttemptProof {
                reservation_id: &cancelled.proof.reservation_id,
                generation: cancelled.proof.generation,
                fencing_token: &cancelled.proof.fencing_token,
            },
        )
        .await
        .expect("accept cancellation compensation fixture");
    store
        .request_job_cancellation(
            cancelled.workspace_id,
            cancelled.environment_id,
            cancelled.job_id,
            &serde_json::json!({"type":"cancel_job","job_id":cancelled.job_id}),
        )
        .await
        .expect("request cancellation before connector compensation");
    assert!(
        store
            .abandon_agent_acceptance(
                cancelled.workspace_id,
                cancelled.environment_id,
                cancelled.agent_id,
                cancelled.job_id,
                cancelled.lease_id,
                &cancelled.lease_token,
                &cancelled.content_sha256,
                1,
                DeliveryAttemptProof {
                    reservation_id: &cancelled.proof.reservation_id,
                    generation: cancelled.proof.generation,
                    fencing_token: &cancelled.proof.fencing_token,
                },
            )
            .await
            .expect("compensate cancel-requested acceptance")
    );
    let cancellation_projection: (String, i64, i64) = sqlx::query_as(
        "SELECT job.state,
            count(*) FILTER (WHERE event.state='cancel_requested'),
            count(*) FILTER (WHERE event.state='cancelled')
         FROM jobs AS job
         JOIN job_events AS event
           ON event.workspace_id=job.workspace_id
          AND event.environment_id=job.environment_id
          AND event.job_id=job.id
         WHERE job.id=$1
         GROUP BY job.state",
    )
    .bind(cancelled.job_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("cancel-requested compensation projection");
    assert_eq!(cancellation_projection, ("cancelled".into(), 1, 1));

    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .expect("drop schema");
}

#[tokio::test]
async fn connector_revoke_sweeps_large_acceptance_backlog_with_bounded_publication() {
    let Some(database_url) = env::var("PIQAE_TEST_DATABASE_URL").ok() else {
        eprintln!("skipped: set PIQAE_TEST_DATABASE_URL for bounded revoke evidence");
        return;
    };
    let schema = format!("piqae_connector_large_{}", ulid::Ulid::new()).to_ascii_lowercase();
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .expect("connect PostgreSQL test database");
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .expect("create large revoke schema");
    let pool = schema_pool(&database_url, &schema).await;
    let store = PostgresStore::from_pool(pool.clone());
    store.migrate().await.expect("apply migrations");
    let fixture = insert_fixture(&pool, &store).await;
    let historical_event_id = store
        .enqueue_webhook_event(
            fixture.workspace_id,
            fixture.environment_id,
            "job.updated",
            &serde_json::json!({"historical": true}),
        )
        .await
        .expect("create event before endpoint");
    sqlx::query("UPDATE webhook_events SET occurred_at=now()-interval '1 minute' WHERE id=$1")
        .bind(&historical_event_id)
        .execute(&pool)
        .await
        .expect("make historical ordering explicit");
    for endpoint in 0..2 {
        store
            .create_webhook(
                &format!("wh_large_{endpoint}"),
                fixture.workspace_id,
                fixture.environment_id,
                &format!("https://example.test/hooks/{endpoint}"),
                &["job.*".into()],
                b"encrypted-test-secret",
            )
            .await
            .expect("create subscribed endpoint");
    }
    let now = Utc::now();
    for sequence in 0_u64..257 {
        let job_id = JobId::new();
        let job = Job {
            id: job_id,
            workspace_id: fixture.workspace_id,
            environment_id: fixture.environment_id,
            printer_id: fixture.printer_id,
            title: format!("Bounded revoke {sequence}"),
            source: None,
            content_kind: ContentKind::Pdf,
            content: ContentSource::Base64 {
                data: "cHJpbnQ=".into(),
            },
            options: JobOptions::default(),
            metadata: BTreeMap::new(),
            deliveries: 1,
            state: JobState::WaitingForAgent,
            created_at: now,
            expires_at: now + Duration::hours(1),
            delivery_uncertain_since: None,
        };
        store
            .create_job(
                &job,
                fixture.agent_id,
                None,
                format!("large-revoke-{sequence}").as_bytes(),
            )
            .await
            .expect("create large revoke job");
        sqlx::query(
            "INSERT INTO job_acceptances
             (job_id,workspace_id,environment_id,agent_id,lease_id,
              lease_token_hash,content_sha256,local_sequence)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
        )
        .bind(job_id.to_string())
        .bind(fixture.workspace_id.to_string())
        .bind(fixture.environment_id.to_string())
        .bind(fixture.agent_id.to_string())
        .bind(uuid::Uuid::new_v4())
        .bind(Sha256::digest(format!("lease-{sequence}").as_bytes()).to_vec())
        .bind(&fixture.content_sha256)
        .bind(i64::try_from(sequence).expect("bounded sequence"))
        .execute(&pool)
        .await
        .expect("insert large acceptance evidence");
    }
    let affected = store
        .revoke_node_connector(
            fixture.workspace_id,
            fixture.environment_id,
            fixture.agent_id,
            &fixture.connector_id,
        )
        .await
        .expect("large connector revoke commits");
    assert_eq!(affected.len(), 256, "live publication is memory bounded");
    let remaining: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM jobs AS job
         JOIN job_acceptances AS acceptance ON acceptance.job_id=job.id
          AND acceptance.workspace_id=job.workspace_id
          AND acceptance.environment_id=job.environment_id
         WHERE job.workspace_id=$1 AND job.environment_id=$2 AND job.agent_id=$3
           AND job.final_at IS NULL",
    )
    .bind(fixture.workspace_id.to_string())
    .bind(fixture.environment_id.to_string())
    .bind(fixture.agent_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("count unswept acceptances");
    assert_eq!(remaining, 0);
    let terminalized: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM jobs WHERE workspace_id=$1 AND environment_id=$2
           AND agent_id=$3 AND state='delivery_uncertain'",
    )
    .bind(fixture.workspace_id.to_string())
    .bind(fixture.environment_id.to_string())
    .bind(fixture.agent_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("count terminalized acceptances");
    assert_eq!(terminalized, 257);
    let durable_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM webhook_events
         WHERE workspace_id=$1 AND environment_id=$2
           AND idempotency_key IS NOT NULL
           AND event_type IN ('job.updated','job.delivery_uncertain')",
    )
    .bind(fixture.workspace_id.to_string())
    .bind(fixture.environment_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("count transactional tenant events");
    assert_eq!(durable_events, 514);
    let publication_state = AppState::new_for_tests(
        Arc::new(store.clone()) as Arc<dyn Repository>,
        Arc::new(PostgresAuthenticator::new(store.clone())),
    );
    for event_type in ["job.updated", "job.delivery_uncertain"] {
        let key = acceptance_revocation_webhook_idempotency_key(
            fixture.workspace_id,
            fixture.environment_id,
            affected[0].id,
            event_type,
        );
        publication_state
            .publish_idempotently(
                &key,
                piqae_control_plane::authentication::TenantContext::unrestricted(
                    fixture.workspace_id,
                    fixture.environment_id,
                ),
                event_type,
                &affected[0],
            )
            .await
            .expect("replay committed event into live publication");
    }
    let retry = store
        .revoke_node_connector(
            fixture.workspace_id,
            fixture.environment_id,
            fixture.agent_id,
            &fixture.connector_id,
        )
        .await
        .expect("idempotent revoke retry");
    assert!(retry.is_empty());
    let events_after_retry: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM webhook_events
         WHERE workspace_id=$1 AND environment_id=$2
           AND idempotency_key IS NOT NULL
           AND event_type IN ('job.updated','job.delivery_uncertain')",
    )
    .bind(fixture.workspace_id.to_string())
    .bind(fixture.environment_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("count events after retry");
    assert_eq!(events_after_retry, durable_events);

    let mut claimed = 0_usize;
    loop {
        let batch = store
            .claim_webhook_deliveries(100)
            .await
            .expect("materialize bounded webhook delivery batch");
        if batch.is_empty() {
            break;
        }
        assert!(batch.len() <= 100);
        claimed += batch.len();
    }
    assert_eq!(claimed, 1_028);
    let deliveries: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM webhook_deliveries AS delivery
         JOIN webhook_events AS event ON event.id=delivery.event_id
         WHERE event.workspace_id=$1 AND event.environment_id=$2",
    )
    .bind(fixture.workspace_id.to_string())
    .bind(fixture.environment_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("count asynchronously expanded deliveries");
    assert_eq!(deliveries, 1_028);
    let historical_deliveries: i64 =
        sqlx::query_scalar("SELECT count(*) FROM webhook_deliveries WHERE event_id=$1")
            .bind(historical_event_id)
            .fetch_one(&pool)
            .await
            .expect("count historical deliveries");
    assert_eq!(historical_deliveries, 0);
    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .expect("drop large revoke schema");
}

#[tokio::test]
async fn accept_and_revoke_connector_row_lock_has_two_stable_orderings() {
    let Some(database_url) = env::var("PIQAE_TEST_DATABASE_URL").ok() else {
        eprintln!("skipped: set PIQAE_TEST_DATABASE_URL for connector lock-order evidence");
        return;
    };
    let schema = format!("piqae_connector_race_{}", ulid::Ulid::new()).to_ascii_lowercase();
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .expect("connect PostgreSQL test database");
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .expect("create race schema");
    let pool = schema_pool(&database_url, &schema).await;
    let store = PostgresStore::from_pool(pool.clone());
    store.migrate().await.expect("apply migrations");

    for accept_first in [true, false] {
        let fixture = insert_fixture(&pool, &store).await;
        let accept_pool = single_schema_pool(&database_url, &schema).await;
        let revoke_pool = single_schema_pool(&database_url, &schema).await;
        let accept_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&accept_pool)
            .await
            .expect("accept backend pid");
        let revoke_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&revoke_pool)
            .await
            .expect("revoke backend pid");
        let mut blocker = pool.begin().await.expect("begin row blocker");
        sqlx::query("SELECT id FROM node_connectors WHERE id=$1 FOR UPDATE")
            .bind(&fixture.connector_id)
            .fetch_one(&mut *blocker)
            .await
            .expect("lock connector row");

        let spawn_accept = || {
            let accept_store = PostgresStore::from_pool(accept_pool.clone());
            let reservation_id = fixture.proof.reservation_id.clone();
            let fencing_token = fixture.proof.fencing_token.clone();
            let content_sha256 = fixture.content_sha256.clone();
            tokio::spawn(async move {
                accept_store
                    .accept_agent_job_with_delivery_attempt(
                        fixture.workspace_id,
                        fixture.environment_id,
                        fixture.agent_id,
                        fixture.job_id,
                        fixture.lease_id,
                        &fixture.lease_token,
                        Some(&content_sha256),
                        1,
                        DeliveryAttemptProof {
                            reservation_id: &reservation_id,
                            generation: fixture.proof.generation,
                            fencing_token: &fencing_token,
                        },
                    )
                    .await
            })
        };
        let spawn_revoke = || {
            let revoke_store = PostgresStore::from_pool(revoke_pool.clone());
            let connector_id = fixture.connector_id.clone();
            tokio::spawn(async move {
                revoke_store
                    .revoke_node_connector(
                        fixture.workspace_id,
                        fixture.environment_id,
                        fixture.agent_id,
                        &connector_id,
                    )
                    .await
            })
        };
        let (accept_task, revoke_task) = if accept_first {
            let accept = spawn_accept();
            wait_until_blocked(&pool, accept_pid).await;
            let revoke = spawn_revoke();
            wait_until_blocked(&pool, revoke_pid).await;
            (accept, revoke)
        } else {
            let revoke = spawn_revoke();
            wait_until_blocked(&pool, revoke_pid).await;
            let accept = spawn_accept();
            wait_until_blocked(&pool, accept_pid).await;
            (accept, revoke)
        };
        blocker.commit().await.expect("release connector row");
        let accept_result = accept_task.await.expect("accept task");
        let revoke_result = revoke_task.await.expect("revoke task");
        assert!(revoke_result.is_ok());
        let state: String = sqlx::query_scalar("SELECT state FROM jobs WHERE id=$1")
            .bind(fixture.job_id.to_string())
            .fetch_one(&pool)
            .await
            .expect("race job state");
        if accept_first {
            assert!(accept_result.is_ok());
            assert_eq!(state, "delivery_uncertain");
        } else {
            assert!(matches!(
                accept_result,
                Err(StorageError::NotFound | StorageError::ConcurrentStateChange)
            ));
            assert_ne!(state, "agent_accepted");
        }
        accept_pool.close().await;
        revoke_pool.close().await;
    }

    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .expect("drop race schema");
}

#[tokio::test]
async fn connectorless_accept_and_full_node_revoke_serialize_on_the_agent_row() {
    let Some(database_url) = env::var("PIQAE_TEST_DATABASE_URL").ok() else {
        eprintln!("skipped: set PIQAE_TEST_DATABASE_URL for node revoke race evidence");
        return;
    };
    let schema = format!("piqae_node_revoke_race_{}", ulid::Ulid::new()).to_ascii_lowercase();
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .expect("connect PostgreSQL test database");
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .expect("create node race schema");
    let pool = schema_pool(&database_url, &schema).await;
    let store = PostgresStore::from_pool(pool.clone());
    store.migrate().await.expect("apply migrations");

    for accept_first in [true, false] {
        let fixture = insert_fixture(&pool, &store).await;
        sqlx::query("DELETE FROM node_connectors WHERE id=$1")
            .bind(&fixture.connector_id)
            .execute(&pool)
            .await
            .expect("remove connector for legacy race");
        let accept_pool = single_schema_pool(&database_url, &schema).await;
        let revoke_pool = single_schema_pool(&database_url, &schema).await;
        let accept_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&accept_pool)
            .await
            .expect("accept backend pid");
        let revoke_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&revoke_pool)
            .await
            .expect("revoke backend pid");
        let mut blocker = pool.begin().await.expect("begin agent-row blocker");
        sqlx::query("SELECT id FROM agents WHERE id=$1 FOR UPDATE")
            .bind(fixture.agent_id.to_string())
            .fetch_one(&mut *blocker)
            .await
            .expect("lock agent row");

        let spawn_accept = || {
            let accept_store = PostgresStore::from_pool(accept_pool.clone());
            let reservation_id = fixture.proof.reservation_id.clone();
            let fencing_token = fixture.proof.fencing_token.clone();
            let content_sha256 = fixture.content_sha256.clone();
            tokio::spawn(async move {
                accept_store
                    .accept_agent_job_with_delivery_attempt(
                        fixture.workspace_id,
                        fixture.environment_id,
                        fixture.agent_id,
                        fixture.job_id,
                        fixture.lease_id,
                        &fixture.lease_token,
                        Some(&content_sha256),
                        1,
                        DeliveryAttemptProof {
                            reservation_id: &reservation_id,
                            generation: fixture.proof.generation,
                            fencing_token: &fencing_token,
                        },
                    )
                    .await
            })
        };
        let spawn_revoke = || {
            let revoke_store = PostgresStore::from_pool(revoke_pool.clone());
            tokio::spawn(async move {
                revoke_store
                    .revoke_agent(
                        fixture.workspace_id,
                        fixture.environment_id,
                        fixture.agent_id,
                    )
                    .await
            })
        };
        let (accept_task, revoke_task) = if accept_first {
            let accept = spawn_accept();
            wait_until_blocked(&pool, accept_pid).await;
            let revoke = spawn_revoke();
            wait_until_blocked(&pool, revoke_pid).await;
            (accept, revoke)
        } else {
            let revoke = spawn_revoke();
            wait_until_blocked(&pool, revoke_pid).await;
            let accept = spawn_accept();
            wait_until_blocked(&pool, accept_pid).await;
            (accept, revoke)
        };
        blocker.commit().await.expect("release agent row");
        let accept_result = accept_task.await.expect("accept task");
        let revoke_result = revoke_task.await.expect("revoke task");
        assert!(revoke_result.is_ok());
        let state: String = sqlx::query_scalar("SELECT state FROM jobs WHERE id=$1")
            .bind(fixture.job_id.to_string())
            .fetch_one(&pool)
            .await
            .expect("node race job state");
        if accept_first {
            assert!(accept_result.is_ok());
            assert_eq!(state, "delivery_uncertain");
        } else {
            assert!(matches!(accept_result, Err(StorageError::NotFound)));
            assert_ne!(state, "agent_accepted");
        }
        accept_pool.close().await;
        revoke_pool.close().await;
    }

    let old_writer = insert_fixture(&pool, &store).await;
    sqlx::query("DELETE FROM node_connectors WHERE id=$1")
        .bind(&old_writer.connector_id)
        .execute(&pool)
        .await
        .expect("remove connector for old writer");
    store
        .revoke_agent(
            old_writer.workspace_id,
            old_writer.environment_id,
            old_writer.agent_id,
        )
        .await
        .expect("revoke legacy node before N-1 insert");
    let old_insert = sqlx::query(
        "INSERT INTO job_acceptances (
            job_id,workspace_id,environment_id,agent_id,lease_id,
            lease_token_hash,content_sha256,local_sequence
         ) VALUES ($1,$2,$3,$4,$5,$6,$7,1)",
    )
    .bind(old_writer.job_id.to_string())
    .bind(old_writer.workspace_id.to_string())
    .bind(old_writer.environment_id.to_string())
    .bind(old_writer.agent_id.to_string())
    .bind(old_writer.lease_id)
    .bind(Sha256::digest(old_writer.lease_token.as_bytes()).to_vec())
    .bind(&old_writer.content_sha256)
    .execute(&pool)
    .await;
    assert!(
        old_insert.is_err(),
        "N-1 acceptance must reject a revoked node"
    );
    let acceptance_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM job_acceptances WHERE job_id=$1")
            .bind(old_writer.job_id.to_string())
            .fetch_one(&pool)
            .await
            .expect("count rejected old acceptance");
    assert_eq!(acceptance_count, 0);

    let connectorless_recovery = insert_fixture(&pool, &store).await;
    sqlx::query("DELETE FROM node_connectors WHERE id=$1")
        .bind(&connectorless_recovery.connector_id)
        .execute(&pool)
        .await
        .expect("remove connector for reconciliation fence");
    store
        .accept_agent_job_with_delivery_attempt(
            connectorless_recovery.workspace_id,
            connectorless_recovery.environment_id,
            connectorless_recovery.agent_id,
            connectorless_recovery.job_id,
            connectorless_recovery.lease_id,
            &connectorless_recovery.lease_token,
            Some(&connectorless_recovery.content_sha256),
            1,
            DeliveryAttemptProof {
                reservation_id: &connectorless_recovery.proof.reservation_id,
                generation: connectorless_recovery.proof.generation,
                fencing_token: &connectorless_recovery.proof.fencing_token,
            },
        )
        .await
        .expect("accept connectorless recovery fixture");
    let before_node_revoke = store
        .reconcile_agent_acceptance(
            connectorless_recovery.workspace_id,
            connectorless_recovery.environment_id,
            connectorless_recovery.agent_id,
            connectorless_recovery.job_id,
            connectorless_recovery.lease_id,
            &connectorless_recovery.lease_token,
            &connectorless_recovery.content_sha256,
            1,
            DeliveryAttemptProof {
                reservation_id: &connectorless_recovery.proof.reservation_id,
                generation: connectorless_recovery.proof.generation,
                fencing_token: &connectorless_recovery.proof.fencing_token,
            },
        )
        .await
        .expect("reconcile active connectorless acceptance");
    assert_eq!(before_node_revoke, (true, false, false));
    store
        .revoke_agent(
            connectorless_recovery.workspace_id,
            connectorless_recovery.environment_id,
            connectorless_recovery.agent_id,
        )
        .await
        .expect("revoke connectorless recovery node");
    let after_node_revoke = store
        .reconcile_agent_acceptance(
            connectorless_recovery.workspace_id,
            connectorless_recovery.environment_id,
            connectorless_recovery.agent_id,
            connectorless_recovery.job_id,
            connectorless_recovery.lease_id,
            &connectorless_recovery.lease_token,
            &connectorless_recovery.content_sha256,
            1,
            DeliveryAttemptProof {
                reservation_id: &connectorless_recovery.proof.reservation_id,
                generation: connectorless_recovery.proof.generation,
                fencing_token: &connectorless_recovery.proof.fencing_token,
            },
        )
        .await
        .expect("reconcile connectorless acceptance after full revoke");
    assert_eq!(after_node_revoke, (false, true, true));

    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .expect("drop node race schema");
}
