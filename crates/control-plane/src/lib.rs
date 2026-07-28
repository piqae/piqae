//! Authoritative Spool HTTP control plane.

pub mod api;
pub mod authentication;
pub mod compatibility;
pub mod device_auth;
pub mod error;
pub mod repository;

use authentication::{Authenticator, TenantContext};
use axum::{
    Router,
    extract::DefaultBodyLimit,
    routing::{get, post},
};
use repository::Repository;
use serde::Serialize;
use spool_object_store::{MemoryObjectStore, ObjectStore};
use spool_webhooks::WebhookSecretBox;
use std::{fmt, sync::Arc};
use tokio::sync::broadcast;
use tower_http::{compression::CompressionLayer, trace::TraceLayer};
use ulid::Ulid;

#[derive(Clone, Debug)]
pub struct PublishedEvent {
    pub id: String,
    pub tenant: TenantContext,
    pub event_type: String,
    pub data: serde_json::Value,
}

#[derive(Clone)]
pub struct AppState {
    pub repository: Arc<dyn Repository>,
    pub authenticator: Arc<dyn Authenticator>,
    pub events: broadcast::Sender<PublishedEvent>,
    pub webhook_secrets: Arc<WebhookSecretBox>,
    pub object_store: Arc<dyn ObjectStore>,
}

impl fmt::Debug for AppState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AppState")
            .field("event_receivers", &self.events.receiver_count())
            .finish_non_exhaustive()
    }
}

impl AppState {
    #[must_use]
    pub fn new(repository: Arc<dyn Repository>, authenticator: Arc<dyn Authenticator>) -> Self {
        Self::new_with_resources(
            repository,
            authenticator,
            [0; 32],
            Arc::new(MemoryObjectStore::default()),
        )
    }

    #[must_use]
    pub fn new_with_webhook_key(
        repository: Arc<dyn Repository>,
        authenticator: Arc<dyn Authenticator>,
        webhook_key: [u8; 32],
    ) -> Self {
        Self::new_with_resources(
            repository,
            authenticator,
            webhook_key,
            Arc::new(MemoryObjectStore::default()),
        )
    }

    #[must_use]
    pub fn new_with_resources(
        repository: Arc<dyn Repository>,
        authenticator: Arc<dyn Authenticator>,
        webhook_key: [u8; 32],
        object_store: Arc<dyn ObjectStore>,
    ) -> Self {
        let (events, _) = broadcast::channel(1_024);
        Self {
            repository,
            authenticator,
            events,
            webhook_secrets: Arc::new(WebhookSecretBox::new(webhook_key)),
            object_store,
        }
    }

    pub fn publish(&self, tenant: TenantContext, event_type: &str, data: &impl Serialize) {
        if let Ok(data) = serde_json::to_value(data) {
            let _ = self.events.send(PublishedEvent {
                id: format!("evt_{}", Ulid::new()),
                tenant,
                event_type: event_type.into(),
                data,
            });
        }
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1/health", get(api::health))
        .route("/v1/ready", get(api::ready))
        .route("/v1/agents", get(api::list_agents))
        .route("/v1/printers", get(api::list_printers))
        .route("/v1/agent-enrolments", post(api::create_agent_enrolment))
        .route("/v1/agents/enrol", post(api::enrol_agent))
        .route("/v1/uploads", post(api::create_upload))
        .route(
            "/v1/uploads/{upload_id}/content",
            axum::routing::put(api::upload_content),
        )
        .route(
            "/v1/uploads/{upload_id}/complete",
            post(api::complete_upload),
        )
        .route(
            "/v1/webhooks",
            get(api::list_webhooks).post(api::create_webhook),
        )
        .route(
            "/v1/webhooks/{webhook_id}",
            axum::routing::delete(api::delete_webhook),
        )
        .route(
            "/v1/webhooks/{webhook_id}/deliveries",
            get(api::list_webhook_deliveries),
        )
        .route(
            "/v1/webhook-deliveries/{delivery_id}/replay",
            post(api::replay_webhook_delivery),
        )
        .route("/v1/jobs", post(api::create_job).get(api::list_jobs))
        .route("/v1/jobs/{job_id}", get(api::get_job))
        .route("/v1/jobs/{job_id}/events", get(api::list_job_events))
        .route("/v1/jobs/{job_id}/cancel", post(api::cancel_job))
        .route("/v1/events/stream", get(api::stream_events))
        .route("/v1/agent/sync", post(api::agent_sync))
        .route(
            "/v1/agent/jobs/{job_id}/accept",
            post(api::accept_agent_job),
        )
        .route(
            "/v1/agent/jobs/{job_id}/lease",
            post(api::renew_agent_lease),
        )
        .route(
            "/v1/agent/jobs/{job_id}/release",
            post(api::release_agent_lease),
        )
        .route(
            "/v1/agent/jobs/{job_id}/content",
            get(api::get_agent_content),
        )
        .route("/whoami", get(compatibility::whoami))
        .route("/ping", get(compatibility::ping))
        .route("/noop", get(compatibility::noop))
        .route(
            "/printjobs",
            post(compatibility::create_print_job).get(compatibility::list_print_jobs),
        )
        .route(
            "/printjobs/states",
            get(compatibility::get_print_job_states),
        )
        .route("/printjobs/{set}", get(compatibility::get_print_jobs))
        .route(
            "/printjobs/{set}/states",
            get(compatibility::get_print_job_states),
        )
        .route("/computers", get(compatibility::list_computers))
        .route("/printers", get(compatibility::list_printers))
        .layer(DefaultBodyLimit::max(52_428_800))
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;
    use crate::{
        authentication::{StaticAuthenticator, TenantContext},
        repository::{MemoryRepository, Repository},
    };
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use base64::{Engine, engine::general_purpose::STANDARD_NO_PAD};
    use chrono::Utc;
    use ed25519_dalek::{Signer, SigningKey};
    use http_body_util::BodyExt;
    use rand::rngs::OsRng;
    use sha2::{Digest, Sha256};
    use spool_domain::{AgentId, EnvironmentId, PrinterId, WorkspaceId};
    use spool_protocol::agent::{
        AgentAcceptJobRequest, AgentHealth, AgentSyncRequest, AgentSyncResponse, QueueSnapshot,
    };
    use tower::ServiceExt;

    struct TestApplication {
        router: Router,
        repository: MemoryRepository,
        printer_id: PrinterId,
        agent_id: AgentId,
        signing_key: SigningKey,
        tenant: TenantContext,
    }

    async fn application() -> TestApplication {
        let repository = MemoryRepository::default();
        let authenticator = StaticAuthenticator::default();
        let tenant = TenantContext {
            workspace_id: WorkspaceId::new(),
            environment_id: EnvironmentId::new(),
        };
        let printer_id = PrinterId::new();
        let agent_id = AgentId::new();
        let signing_key = SigningKey::generate(&mut OsRng);
        repository
            .add_printer(
                tenant.workspace_id,
                tenant.environment_id,
                printer_id,
                agent_id,
            )
            .await;
        repository
            .set_agent_public_key(agent_id, signing_key.verifying_key().to_bytes().to_vec())
            .await;
        authenticator.insert("spl_test_integration", tenant).await;
        TestApplication {
            router: router(AppState::new(
                Arc::new(repository.clone()),
                Arc::new(authenticator),
            )),
            repository,
            printer_id,
            agent_id,
            signing_key,
            tenant,
        }
    }

    fn signed_request(
        application: &TestApplication,
        method: &str,
        path: &str,
        body: Vec<u8>,
    ) -> Request<Body> {
        let timestamp = Utc::now().timestamp_millis();
        let nonce = uuid::Uuid::new_v4();
        let digest = format!("{:x}", Sha256::digest(&body));
        let canonical = format!("{method}\n{path}\n{timestamp}\n{nonce}\n{digest}");
        let signature = application.signing_key.sign(canonical.as_bytes());
        Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json")
            .header("x-spool-agent-id", application.agent_id.to_string())
            .header("x-spool-timestamp", timestamp.to_string())
            .header("x-spool-nonce", nonce.to_string())
            .header("x-spool-body-sha256", digest)
            .header(
                "x-spool-signature",
                STANDARD_NO_PAD.encode(signature.to_bytes()),
            )
            .body(Body::from(body))
            .expect("valid signed request")
    }

    #[tokio::test]
    async fn native_create_is_durable_and_redacts_content() {
        let application = application().await;
        let request = Request::builder()
            .method("POST")
            .uri("/v1/jobs")
            .header("authorization", "Bearer spl_test_integration")
            .header("content-type", "application/json")
            .header("idempotency-key", "order-481")
            .body(Body::from(format!(
                r#"{{"printer_id":"{}","title":"Order 481","content_type":"pdf","content":{{"type":"base64","data":"c2VjcmV0"}}}}"#,
                application.printer_id
            )))
            .expect("valid test request");
        let response = application
            .router
            .clone()
            .oneshot(request)
            .await
            .expect("router response");
        assert_eq!(response.status(), StatusCode::CREATED);
        let body = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).expect("JSON");
        assert_eq!(json["state"], "registered");
        assert!(json.get("content").is_none());
        let stored = application
            .repository
            .list_jobs(
                application.tenant.workspace_id,
                application.tenant.environment_id,
                10,
            )
            .await
            .expect("stored jobs");
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].state, spool_domain::JobState::WaitingForAgent);
    }

    #[tokio::test]
    async fn native_api_requires_bearer_authentication() {
        let application = application().await;
        let response = application
            .router
            .oneshot(
                Request::builder()
                    .uri("/v1/jobs")
                    .body(Body::empty())
                    .expect("valid request"),
            )
            .await
            .expect("router response");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn compatibility_create_returns_numeric_identifier() {
        let application = application().await;
        let compatibility_printer_id = application
            .repository
            .compatibility_id(
                application.tenant.workspace_id,
                application.tenant.environment_id,
                "printer",
                &application.printer_id.to_string(),
            )
            .await
            .expect("compatibility printer ID");
        let credentials = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            "spl_test_integration:",
        );
        let response = application
            .router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/printjobs")
                    .header("authorization", format!("Basic {credentials}"))
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        r#"{{"printerId":{compatibility_printer_id},"title":"Label","contentType":"pdf_base64","content":"JVBERi0="}}"#
                    )))
                    .expect("valid request"),
            )
            .await
            .expect("router response");
        assert_eq!(response.status(), StatusCode::CREATED);
        let body = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        let id: i64 = serde_json::from_slice(&body).expect("numeric ID");
        assert!(id > 0);
    }

    #[tokio::test]
    async fn signed_agent_claim_and_durable_accept_flow() {
        let application = application().await;
        let create = Request::builder()
            .method("POST")
            .uri("/v1/jobs")
            .header("authorization", "Bearer spl_test_integration")
            .header("content-type", "application/json")
            .body(Body::from(format!(
                r#"{{"printer_id":"{}","title":"Agent lease","content_type":"pdf","content":{{"type":"base64","data":"cHJpbnQ="}}}}"#,
                application.printer_id
            )))
            .expect("valid request");
        let created = application
            .router
            .clone()
            .oneshot(create)
            .await
            .expect("create response");
        assert_eq!(created.status(), StatusCode::CREATED);

        let now = Utc::now();
        let sync = AgentSyncRequest {
            agent_id: application.agent_id,
            protocol_version: 1,
            agent_version: "test".into(),
            printer_revision: 0,
            acknowledged_command_cursor: None,
            event_cursor: None,
            queue: QueueSnapshot {
                queued_jobs: 0,
                active_jobs: 0,
                content_bytes: 0,
                accepts_jobs: true,
            },
            health: AgentHealth {
                started_at: now,
                observed_at: now,
                sqlite_integrity_ok: true,
                executor_crashes: 0,
                last_error_code: None,
            },
            printers: None,
            events: Vec::new(),
        };
        let sync_body = serde_json::to_vec(&sync).expect("sync JSON");
        let sync_response = application
            .router
            .clone()
            .oneshot(signed_request(
                &application,
                "POST",
                "/v1/agent/sync",
                sync_body,
            ))
            .await
            .expect("sync response");
        assert_eq!(sync_response.status(), StatusCode::OK);
        let sync_body = sync_response
            .into_body()
            .collect()
            .await
            .expect("sync body")
            .to_bytes();
        let sync: AgentSyncResponse =
            serde_json::from_slice(&sync_body).expect("sync response JSON");
        assert_eq!(sync.candidate_jobs.len(), 1);
        let offer = &sync.candidate_jobs[0];
        let accept = AgentAcceptJobRequest {
            lease_id: offer.lease_id,
            lease_token: offer.lease_token.clone(),
            content_sha256: match &offer.content {
                spool_protocol::agent::ContentDescriptor::InlineBase64 {
                    sha256: Some(sha256),
                    ..
                } => sha256.clone(),
                _ => panic!("expected inline content"),
            },
            local_sequence: 1,
        };
        let path = format!("/v1/agent/jobs/{}/accept", offer.job.id);
        let response = application
            .router
            .clone()
            .oneshot(signed_request(
                &application,
                "POST",
                &path,
                serde_json::to_vec(&accept).expect("accept JSON"),
            ))
            .await
            .expect("accept response");
        assert_eq!(response.status(), StatusCode::OK);
        let stored = application
            .repository
            .get_job(
                application.tenant.workspace_id,
                application.tenant.environment_id,
                offer.job.id,
            )
            .await
            .expect("accepted job");
        assert_eq!(stored.state, spool_domain::JobState::AgentAccepted);
    }
}
