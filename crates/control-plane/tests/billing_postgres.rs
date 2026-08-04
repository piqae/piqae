#![allow(
    clippy::expect_used,
    clippy::similar_names,
    clippy::struct_field_names,
    clippy::too_many_lines
)]

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::Utc;
use hmac::{Hmac, Mac};
use http_body_util::BodyExt;
use piqae_control_plane::{
    AppState, BillingCapabilities, DeploymentCapabilities,
    authentication::{StaticAuthenticator, TenantContext},
    repository::Repository,
    router,
};
use piqae_domain::{
    AgentId, EnvironmentId, EventId, JobEvent, JobId, JobState, PrinterId, WorkspaceId,
};
use piqae_object_store::MemoryObjectStore;
use piqae_storage_postgres::PostgresStore;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{env, sync::Arc};
use tower::ServiceExt;

type HmacSha256 = Hmac<Sha256>;

struct TenantFixture {
    workspace_id: WorkspaceId,
    test_environment_id: EnvironmentId,
    live_environment_id: EnvironmentId,
    live_agent_id: AgentId,
    live_printer_id: PrinterId,
}

async fn schema_pool(database_url: &str, schema: &str) -> PgPool {
    let schema = schema.to_owned();
    PgPoolOptions::new()
        .max_connections(12)
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

async fn insert_tenant(pool: &PgPool, suffix: &str) -> TenantFixture {
    let workspace_id = WorkspaceId::new();
    let test_environment_id = EnvironmentId::new();
    let live_environment_id = EnvironmentId::new();
    let live_agent_id = AgentId::new();
    let live_printer_id = PrinterId::new();
    sqlx::query("INSERT INTO workspaces (id,name,slug) VALUES ($1,$2,$3)")
        .bind(workspace_id.to_string())
        .bind(format!("Billing {suffix}"))
        .bind(format!("billing-{suffix}-{}", ulid::Ulid::new()).to_ascii_lowercase())
        .execute(pool)
        .await
        .expect("workspace");
    for (environment_id, kind, name) in [
        (test_environment_id, "test", "Test"),
        (live_environment_id, "live", "Live"),
    ] {
        sqlx::query("INSERT INTO environments (id,workspace_id,kind,name) VALUES ($1,$2,$3,$4)")
            .bind(environment_id.to_string())
            .bind(workspace_id.to_string())
            .bind(kind)
            .bind(name)
            .execute(pool)
            .await
            .expect("environment");
    }
    sqlx::query(
        "INSERT INTO agents (
            id,workspace_id,environment_id,name,installation_id,public_key,
            os,architecture,version,protocol_version,state,last_seen_at
         ) VALUES ($1,$2,$3,'Billing node',$4,$5,'test','test','0.1.0',1,'connected',now())",
    )
    .bind(live_agent_id.to_string())
    .bind(workspace_id.to_string())
    .bind(live_environment_id.to_string())
    .bind(format!("billing-installation-{suffix}"))
    .bind(vec![7_u8; 32])
    .execute(pool)
    .await
    .expect("agent");
    sqlx::query(
        "INSERT INTO printers (
            id,workspace_id,environment_id,agent_id,native_id,name
         ) VALUES ($1,$2,$3,$4,$5,'Billing printer')",
    )
    .bind(live_printer_id.to_string())
    .bind(workspace_id.to_string())
    .bind(live_environment_id.to_string())
    .bind(live_agent_id.to_string())
    .bind(format!("billing-printer-{suffix}"))
    .execute(pool)
    .await
    .expect("printer");
    TenantFixture {
        workspace_id,
        test_environment_id,
        live_environment_id,
        live_agent_id,
        live_printer_id,
    }
}

async fn seed_free_quota(pool: &PgPool, tenant: &TenantFixture, suffix: &str) {
    sqlx::query(
        "INSERT INTO jobs (
            id,workspace_id,environment_id,printer_id,agent_id,payload,state,
            state_sequence,per_printer_sequence,expires_at,created_at,updated_at
         )
         SELECT
            'quota_' || $5 || '_' || value::text,$1,$2,$3,$4,'{}'::jsonb,
            'accepted_by_spooler',1,value,now() + interval '1 day',now(),now()
         FROM generate_series(2,101) value",
    )
    .bind(tenant.workspace_id.to_string())
    .bind(tenant.live_environment_id.to_string())
    .bind(tenant.live_printer_id.to_string())
    .bind(tenant.live_agent_id.to_string())
    .bind(suffix)
    .execute(pool)
    .await
    .expect("quota jobs");
    sqlx::query(
        "INSERT INTO usage_ledger (
            id,workspace_id,environment_id,job_id,kind,units,occurred_at
         )
         SELECT
            'usage_' || $3 || '_' || value::text,$1,$2,
            'quota_' || $3 || '_' || value::text,
            'print_job_accepted',1,now()
         FROM generate_series(2,101) value",
    )
    .bind(tenant.workspace_id.to_string())
    .bind(tenant.live_environment_id.to_string())
    .bind(suffix)
    .execute(pool)
    .await
    .expect("quota usage");
}

fn job_body(printer_id: PrinterId, title: &str) -> String {
    serde_json::json!({
        "printer_id": printer_id,
        "title": title,
        "content_type": "pdf",
        "content": {"type": "base64", "data": "JVBERi0xLjQK"}
    })
    .to_string()
}

fn bearer_request(method: &str, path: &str, bearer: &str, body: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {bearer}"));
    if body.is_some() {
        builder = builder.header("content-type", "application/json");
    }
    builder
        .body(body.map_or_else(Body::empty, |body| Body::from(body.to_owned())))
        .expect("request")
}

async fn json(response: axum::response::Response) -> serde_json::Value {
    serde_json::from_slice(
        &response
            .into_body()
            .collect()
            .await
            .expect("response body")
            .to_bytes(),
    )
    .expect("JSON response")
}

fn stripe_request(secret: &str, payload: &serde_json::Value) -> Request<Body> {
    let body = payload.to_string();
    let timestamp = Utc::now().timestamp();
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC key");
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b".");
    mac.update(body.as_bytes());
    let signature = hex::encode(mac.finalize().into_bytes());
    Request::builder()
        .method("POST")
        .uri("/v1/integrations/stripe/webhook")
        .header("content-type", "application/json")
        .header("stripe-signature", format!("t={timestamp},v1={signature}"))
        .body(Body::from(body))
        .expect("Stripe request")
}

#[tokio::test]
async fn cloud_billing_is_tenant_scoped_idempotent_and_stripe_projected() {
    let Some(database_url) = env::var("PIQAE_TEST_DATABASE_URL").ok() else {
        eprintln!("skipped: set PIQAE_TEST_DATABASE_URL to run PostgreSQL billing evidence");
        return;
    };
    let schema = format!("piqae_billing_{}", ulid::Ulid::new()).to_ascii_lowercase();
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
    let tenant_a = insert_tenant(&pool, "a").await;
    let tenant_b = insert_tenant(&pool, "b").await;
    sqlx::query(
        "UPDATE workspaces SET
            workos_organization_id = 'org_workos_a',
            identity_provider = 'workos',
            identity_organization_id = 'org_workos_a'
         WHERE id = $1",
    )
    .bind(tenant_a.workspace_id.to_string())
    .execute(&pool)
    .await
    .expect("WorkOS mapping");

    let authenticator = StaticAuthenticator::default();
    authenticator
        .insert(
            "tenant-a-live",
            TenantContext::unrestricted(tenant_a.workspace_id, tenant_a.live_environment_id),
        )
        .await;
    authenticator
        .insert(
            "tenant-a-test",
            TenantContext::unrestricted(tenant_a.workspace_id, tenant_a.test_environment_id),
        )
        .await;
    authenticator
        .insert(
            "tenant-b-live",
            TenantContext::unrestricted(tenant_b.workspace_id, tenant_b.live_environment_id),
        )
        .await;
    let capabilities = DeploymentCapabilities {
        deployment: "cloud".into(),
        billing: BillingCapabilities { enabled: true },
        ..DeploymentCapabilities::default()
    };
    let stripe_secret = "whsec_test_billing_contract";
    let application = router(
        AppState::new_with_resources(
            Arc::new(store.clone()) as Arc<dyn Repository>,
            Arc::new(authenticator),
            [5; 32],
            Arc::new(MemoryObjectStore::default()),
        )
        .with_capabilities(capabilities)
        .with_stripe_webhook_secret(stripe_secret),
    );

    let first_body = job_body(tenant_a.live_printer_id, "idempotent fixture");
    let first = application
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/jobs")
                .header("authorization", "Bearer tenant-a-live")
                .header("content-type", "application/json")
                .header("idempotency-key", "billing-idempotency-one")
                .body(Body::from(first_body.clone()))
                .expect("first request"),
        )
        .await
        .expect("first response");
    assert_eq!(first.status(), StatusCode::CREATED);
    seed_free_quota(&pool, &tenant_a, "a").await;

    let retry = application
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/jobs")
                .header("authorization", "Bearer tenant-a-live")
                .header("content-type", "application/json")
                .header("idempotency-key", "billing-idempotency-one")
                .body(Body::from(first_body))
                .expect("retry request"),
        )
        .await
        .expect("retry response");
    assert_eq!(retry.status(), StatusCode::OK);
    let upload_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM uploads WHERE workspace_id = $1")
            .bind(tenant_a.workspace_id.to_string())
            .fetch_one(&pool)
            .await
            .expect("upload count");
    assert_eq!(upload_count, 1);

    let test_agent_id = AgentId::new();
    let test_printer_id = PrinterId::new();
    sqlx::query(
        "INSERT INTO agents (
            id,workspace_id,environment_id,name,installation_id,public_key,
            os,architecture,version,protocol_version,state,last_seen_at
         ) VALUES ($1,$2,$3,'Test node',$4,$5,'test','test','0.1.0',1,'connected',now())",
    )
    .bind(test_agent_id.to_string())
    .bind(tenant_a.workspace_id.to_string())
    .bind(tenant_a.test_environment_id.to_string())
    .bind("billing-test-installation-a")
    .bind(vec![8_u8; 32])
    .execute(&pool)
    .await
    .expect("test node");
    sqlx::query(
        "INSERT INTO printers (
            id,workspace_id,environment_id,agent_id,native_id,name
         ) VALUES ($1,$2,$3,$4,'billing-test-printer-a','Test printer')",
    )
    .bind(test_printer_id.to_string())
    .bind(tenant_a.workspace_id.to_string())
    .bind(tenant_a.test_environment_id.to_string())
    .bind(test_agent_id.to_string())
    .execute(&pool)
    .await
    .expect("test printer");
    let free_test_job = application
        .clone()
        .oneshot(bearer_request(
            "POST",
            "/v1/jobs",
            "tenant-a-test",
            Some(&job_body(test_printer_id, "test environment is unmetered")),
        ))
        .await
        .expect("test job response");
    assert_eq!(free_test_job.status(), StatusCode::CREATED);

    let blocked = application
        .clone()
        .oneshot(bearer_request(
            "POST",
            "/v1/jobs",
            "tenant-a-live",
            Some(&job_body(tenant_a.live_printer_id, "quota blocked")),
        ))
        .await
        .expect("quota response");
    assert_eq!(blocked.status(), StatusCode::PAYMENT_REQUIRED);
    assert_eq!(json(blocked).await["error"]["code"], "quota_exceeded");

    let compatibility_id = store
        .compatibility_id(
            tenant_a.workspace_id,
            tenant_a.live_environment_id,
            "printer",
            &tenant_a.live_printer_id.to_string(),
        )
        .await
        .expect("compatibility printer id");
    let basic = STANDARD.encode("tenant-a-live:");
    let compatibility = application
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/printjobs")
                .header("authorization", format!("Basic {basic}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "printerId": compatibility_id,
                        "title": "compatibility blocked",
                        "contentType": "pdf_base64",
                        "content": "JVBERi0xLjQK"
                    })
                    .to_string(),
                ))
                .expect("compatibility request"),
        )
        .await
        .expect("compatibility response");
    assert_eq!(compatibility.status(), StatusCode::PAYMENT_REQUIRED);
    assert_eq!(json(compatibility).await["code"], "quota_exceeded");

    let other_tenant = application
        .clone()
        .oneshot(bearer_request(
            "POST",
            "/v1/jobs",
            "tenant-b-live",
            Some(&job_body(tenant_b.live_printer_id, "tenant B allowed")),
        ))
        .await
        .expect("tenant B response");
    assert_eq!(other_tenant.status(), StatusCode::CREATED);

    let usage_a = application
        .clone()
        .oneshot(bearer_request("GET", "/v1/usage", "tenant-a-live", None))
        .await
        .expect("usage A");
    assert_eq!(usage_a.status(), StatusCode::OK);
    let usage_a = json(usage_a).await;
    assert_eq!(usage_a["reported_complete_live_jobs"], 100);
    assert!(usage_a.get("accepted_live_jobs").is_none());
    let usage_b = application
        .clone()
        .oneshot(bearer_request("GET", "/v1/usage", "tenant-b-live", None))
        .await
        .expect("usage B");
    let usage_b = json(usage_b).await;
    assert_eq!(usage_b["reported_complete_live_jobs"], 0);
    assert!(usage_b.get("accepted_live_jobs").is_none());

    let created = Utc::now().timestamp();
    let subscription = serde_json::json!({
        "id": "evt_subscription_new",
        "type": "customer.subscription.updated",
        "created": created,
        "data": {"object": {
            "id": "sub_pro",
            "customer": "cus_pro",
            "status": "active",
            "current_period_start": created,
            "current_period_end": created + 31_536_000,
            "cancel_at_period_end": false,
            "metadata": {
                "workspace_id": "org_workos_a",
                "plan": "pro",
                "interval": "annual"
            }
        }}
    });
    let projected = application
        .clone()
        .oneshot(stripe_request(stripe_secret, &subscription))
        .await
        .expect("Stripe projection");
    assert_eq!(projected.status(), StatusCode::OK);
    assert_eq!(json(projected).await["duplicate"], false);
    let duplicate = application
        .clone()
        .oneshot(stripe_request(stripe_secret, &subscription))
        .await
        .expect("Stripe duplicate");
    assert_eq!(json(duplicate).await["duplicate"], true);

    let older = serde_json::json!({
        "id": "evt_subscription_old",
        "type": "customer.subscription.updated",
        "created": created - 60,
        "data": {"object": {
            "id": "sub_pro",
            "customer": "cus_pro",
            "status": "active",
            "metadata": {"workspace_id": "org_workos_a", "plan": "free"}
        }}
    });
    let older_response = application
        .clone()
        .oneshot(stripe_request(stripe_secret, &older))
        .await
        .expect("older Stripe event");
    assert_eq!(older_response.status(), StatusCode::OK);
    let plan: String =
        sqlx::query_scalar("SELECT plan FROM workspace_entitlements WHERE workspace_id = $1")
            .bind(tenant_a.workspace_id.to_string())
            .fetch_one(&pool)
            .await
            .expect("projected plan");
    assert_eq!(plan, "pro");

    let pro_job = application
        .clone()
        .oneshot(bearer_request(
            "POST",
            "/v1/jobs",
            "tenant-a-live",
            Some(&job_body(tenant_a.live_printer_id, "Pro overage allowed")),
        ))
        .await
        .expect("Pro job");
    assert_eq!(pro_job.status(), StatusCode::CREATED);
    let pro_job_id: JobId = json(pro_job).await["id"]
        .as_str()
        .expect("created job id")
        .parse()
        .expect("job id");
    for state in [
        JobState::AgentDownloading,
        JobState::AgentAccepted,
        JobState::QueuedLocal,
        JobState::Preparing,
        JobState::SpoolIntent,
        JobState::AcceptedBySpooler,
    ] {
        let current_sequence: i64 =
            sqlx::query_scalar("SELECT state_sequence FROM jobs WHERE id = $1")
                .bind(pro_job_id.to_string())
                .fetch_one(&pool)
                .await
                .expect("current job sequence");
        let current_state: String = sqlx::query_scalar("SELECT state FROM jobs WHERE id = $1")
            .bind(pro_job_id.to_string())
            .fetch_one(&pool)
            .await
            .expect("current job state");
        store
            .append_event(
                tenant_a.workspace_id,
                tenant_a.live_environment_id,
                &JobEvent {
                    id: EventId::new(),
                    job_id: pro_job_id,
                    sequence: u64::try_from(current_sequence + 1).expect("small sequence"),
                    state,
                    reason: None,
                    message: None,
                    agent_id: Some(tenant_a.live_agent_id),
                    native_job_id: (state == JobState::AcceptedBySpooler)
                        .then(|| "native-billing-once".to_owned()),
                    occurred_at: Utc::now(),
                },
            )
            .await
            .unwrap_or_else(|error| {
                panic!("job lifecycle transition {current_state} -> {state:?}: {error}")
            });
    }
    let usage_before_reported_completion: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM usage_ledger
         WHERE workspace_id = $1 AND environment_id = $2
           AND job_id = $3
           AND kind IN ('print_job_accepted', 'print_job_reported_complete')",
    )
    .bind(tenant_a.workspace_id.to_string())
    .bind(tenant_a.live_environment_id.to_string())
    .bind(pro_job_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("pre-completion usage entries");
    assert_eq!(usage_before_reported_completion, 0);

    store
        .apply_agent_event(
            tenant_a.workspace_id,
            tenant_a.live_environment_id,
            tenant_a.live_agent_id,
            &JobEvent {
                id: EventId::new(),
                job_id: pro_job_id,
                sequence: 0,
                state: JobState::CompletedReported,
                reason: None,
                message: None,
                agent_id: Some(tenant_a.live_agent_id),
                native_job_id: Some("native-billing-once".to_owned()),
                occurred_at: Utc::now(),
            },
        )
        .await
        .expect("reported-complete transition")
        .expect("new agent event");
    let reported_complete_entries: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM usage_ledger
         WHERE workspace_id = $1 AND environment_id = $2
           AND job_id = $3 AND kind = 'print_job_reported_complete'",
    )
    .bind(tenant_a.workspace_id.to_string())
    .bind(tenant_a.live_environment_id.to_string())
    .bind(pro_job_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("reported-complete usage entries");
    assert_eq!(reported_complete_entries, 1);
    let summary = application
        .clone()
        .oneshot(bearer_request(
            "GET",
            "/v1/billing/summary",
            "tenant-a-live",
            None,
        ))
        .await
        .expect("billing summary");
    let summary = json(summary).await;
    assert_eq!(summary["plan"], "pro");
    assert_eq!(summary["billing_interval"], "annual");
    assert_eq!(summary["entitlement"]["included_live_jobs"], 300_000);
    assert_eq!(summary["entitlement"]["node_limit"], 25);
    assert_eq!(summary["entitlement"]["overage_price_cents"], 25);

    let failed_invoice = serde_json::json!({
        "id": "evt_invoice_failed",
        "type": "invoice.payment_failed",
        "created": created + 1,
        "data": {"object": {
            "id": "in_failed",
            "customer": "cus_pro",
            "subscription": "sub_pro"
        }}
    });
    let failed = application
        .clone()
        .oneshot(stripe_request(stripe_secret, &failed_invoice))
        .await
        .expect("failed invoice projection");
    assert_eq!(failed.status(), StatusCode::OK);
    let grace_summary = application
        .clone()
        .oneshot(bearer_request(
            "GET",
            "/v1/billing/summary",
            "tenant-a-live",
            None,
        ))
        .await
        .expect("past-due grace summary");
    assert_eq!(json(grace_summary).await["accept_new_cloud_jobs"], true);
    sqlx::query(
        "UPDATE billing_subscriptions
         SET grace_ends_at = now() - interval '1 second'
         WHERE workspace_id = $1",
    )
    .bind(tenant_a.workspace_id.to_string())
    .execute(&pool)
    .await
    .expect("expire grace");
    let expired_grace = application
        .clone()
        .oneshot(bearer_request(
            "POST",
            "/v1/jobs",
            "tenant-a-live",
            Some(&job_body(tenant_a.live_printer_id, "past due after grace")),
        ))
        .await
        .expect("expired grace response");
    assert_eq!(expired_grace.status(), StatusCode::PAYMENT_REQUIRED);
    assert_eq!(
        json(expired_grace).await["error"]["code"],
        "billing_blocked"
    );

    store
        .create_enrolment(
            "enrolment-over-limit",
            tenant_b.workspace_id,
            tenant_b.live_environment_id,
            "over-limit-secret",
            Utc::now() + chrono::Duration::minutes(10),
        )
        .await
        .expect("over-limit enrollment");
    let over_limit = store
        .enrol_agent_with_billing(
            "over-limit-secret",
            &[10_u8; 32],
            "Second node",
            "second-host",
            "test",
            "test",
            "0.1.0",
            1,
            true,
        )
        .await;
    assert!(matches!(
        over_limit,
        Err(piqae_storage_postgres::StorageError::NodeQuotaExceeded)
    ));

    let replay_key = [11_u8; 32];
    let replay_host = "existing-host";
    let existing_installation = format!("{replay_host}:{:x}", Sha256::digest(replay_key));
    sqlx::query("UPDATE agents SET installation_id = $2 WHERE id = $1")
        .bind(tenant_b.live_agent_id.to_string())
        .bind(existing_installation)
        .execute(&pool)
        .await
        .expect("existing installation");
    store
        .create_enrolment(
            "enrolment-existing",
            tenant_b.workspace_id,
            tenant_b.live_environment_id,
            "existing-secret",
            Utc::now() + chrono::Duration::minutes(10),
        )
        .await
        .expect("existing enrollment");
    let replay = store
        .enrol_agent_with_billing(
            "existing-secret",
            &replay_key,
            "Existing node",
            replay_host,
            "test",
            "test",
            "0.1.0",
            1,
            true,
        )
        .await
        .expect("existing installation remains idempotent");
    assert_eq!(replay.agent_id, tenant_b.live_agent_id);
    let changed_key_replay = store
        .enrol_agent_with_billing(
            "existing-secret",
            &[12_u8; 32],
            "Attacker rename",
            replay_host,
            "test",
            "test",
            "9.9.9",
            1,
            true,
        )
        .await;
    assert!(
        matches!(
            changed_key_replay,
            Err(piqae_storage_postgres::StorageError::NotFound)
        ),
        "a consumed capability must not rotate an existing public key"
    );
    sqlx::query(
        "UPDATE enrolment_tokens SET expires_at = now() - interval '11 minutes'
         WHERE id = 'enrolment-existing'",
    )
    .execute(&pool)
    .await
    .expect("expire replay recovery window");
    let stale_replay = store
        .enrol_agent_with_billing(
            "existing-secret",
            &replay_key,
            "Existing node",
            replay_host,
            "test",
            "test",
            "0.1.0",
            1,
            true,
        )
        .await;
    assert!(
        matches!(
            stale_replay,
            Err(piqae_storage_postgres::StorageError::NotFound)
        ),
        "recovery retries must expire"
    );

    sqlx::query(
        "INSERT INTO device_authorizations (
            id,device_code_hash,user_code_hash,user_code_display,device_public_key,
            installation_id,proposed_name,hostname,platform,architecture,
            installation_mode,agent_version,protocol_version,state,expires_at,
            workspace_id,environment_id,approved_by,approved_at
         ) VALUES (
            'device-over-limit','device-over-limit-hash','device-user-hash','ABCD-EFGH',
            $1,'new-browser-install','Browser node','browser-host','test','test',
            'user','0.1.0',1,'approved',now() + interval '10 minutes',
            $2,$3,'billing-test',now()
         )",
    )
    .bind(vec![12_u8; 32])
    .bind(tenant_b.workspace_id.to_string())
    .bind(tenant_b.live_environment_id.to_string())
    .execute(&pool)
    .await
    .expect("browser authorization");
    let browser_over_limit = store
        .exchange_device_authorization_with_billing("device-over-limit-hash", true)
        .await;
    assert!(matches!(
        browser_over_limit,
        Err(piqae_storage_postgres::StorageError::NodeQuotaExceeded)
    ));

    sqlx::query(
        "INSERT INTO device_authorizations (
            id,device_code_hash,user_code_hash,user_code_display,device_public_key,
            installation_id,proposed_name,hostname,platform,architecture,
            installation_mode,agent_version,protocol_version,state,expires_at,
            workspace_id,environment_id,approved_by,approved_at
         ) VALUES (
            'device-existing','device-existing-hash','device-existing-user','IJKL-MNOP',
            $1,$2,'Browser existing','browser-existing','test','test',
            'user','0.1.0',1,'approved',now() + interval '10 minutes',
            $3,$4,'billing-test',now()
         )",
    )
    .bind(vec![13_u8; 32])
    .bind(format!("{}:{:x}", replay_host, Sha256::digest(replay_key)))
    .bind(tenant_b.workspace_id.to_string())
    .bind(tenant_b.live_environment_id.to_string())
    .execute(&pool)
    .await
    .expect("existing browser authorization");
    let browser_replay = store
        .exchange_device_authorization_with_billing("device-existing-hash", true)
        .await
        .expect("browser existing installation remains idempotent");
    assert_eq!(browser_replay.agent_id, tenant_b.live_agent_id);

    sqlx::query(
        "UPDATE billing_subscriptions
         SET billing_interval = 'monthly',
             current_period_start = now() - interval '1 hour',
             current_period_end = now() + interval '30 minutes'
         WHERE workspace_id = $1",
    )
    .bind(tenant_a.workspace_id.to_string())
    .execute(&pool)
    .await
    .expect("closed export period");
    sqlx::query(
        "UPDATE workspace_entitlements SET included_jobs = 1
         WHERE workspace_id = $1",
    )
    .bind(tenant_a.workspace_id.to_string())
    .execute(&pool)
    .await
    .expect("export test allowance");
    let renewal = serde_json::json!({
        "id": "evt_subscription_renewal",
        "type": "customer.subscription.updated",
        "created": created + 120,
        "data": {"object": {
            "id": "sub_pro",
            "customer": "cus_pro",
            "status": "active",
            "current_period_start": created + 1_800,
            "current_period_end": created + 2_593_800,
            "cancel_at_period_end": false,
            "metadata": {
                "workspace_id": "org_workos_a",
                "plan": "pro",
                "interval": "monthly"
            }
        }}
    });
    let renewal_response = application
        .clone()
        .oneshot(stripe_request(stripe_secret, &renewal))
        .await
        .expect("renewal projection");
    assert_eq!(renewal_response.status(), StatusCode::OK);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM usage_exports WHERE workspace_id = $1",)
            .bind(tenant_a.workspace_id.to_string())
            .fetch_one(&pool)
            .await
            .expect("renewal usage snapshot"),
        1,
        "renewal must snapshot the old period before replacing it"
    );
    assert_eq!(
        store
            .prepare_due_usage_exports(Utc::now() + chrono::Duration::days(31))
            .await
            .expect("scan due usage periods"),
        0
    );
    sqlx::query(
        "INSERT INTO usage_exports (
            id,workspace_id,period_start,period_end,units,
            stripe_event_identifier,state,created_at
         ) VALUES (
            'export-without-customer',$1,
            now() - interval '2 months',now() - interval '1 month',1,
            'export-without-customer','pending',now() - interval '1 day'
         )",
    )
    .bind(tenant_b.workspace_id.to_string())
    .execute(&pool)
    .await
    .expect("unbillable export fixture");
    let export = store
        .claim_usage_export(Utc::now())
        .await
        .expect("claim usage export")
        .expect("prepared usage export");
    assert_eq!(export.workspace_id, tenant_a.workspace_id);
    assert_eq!(export.stripe_customer_id, "cus_pro");
    assert_eq!(export.overage_blocks, 1);
    store
        .complete_usage_export(&export.id, &export.claim_token)
        .await
        .expect("complete usage export");
    assert!(
        store
            .claim_usage_export(Utc::now())
            .await
            .expect("empty export queue")
            .is_none()
    );

    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .expect("drop schema");
}
