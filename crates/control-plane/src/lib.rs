//! Authoritative Spool HTTP control plane.

pub mod api;
pub mod authentication;
pub mod compatibility;
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
        let (events, _) = broadcast::channel(1_024);
        Self {
            repository,
            authenticator,
            events,
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
        .route("/v1/jobs", post(api::create_job).get(api::list_jobs))
        .route("/v1/jobs/{job_id}", get(api::get_job))
        .route("/v1/jobs/{job_id}/events", get(api::list_job_events))
        .route("/v1/jobs/{job_id}/cancel", post(api::cancel_job))
        .route("/v1/events/stream", get(api::stream_events))
        .route("/v1/agent/sync", post(api::agent_sync))
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
        .route("/computers", get(compatibility::empty_list))
        .route("/printers", get(compatibility::empty_list))
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
    use http_body_util::BodyExt;
    use spool_domain::{AgentId, EnvironmentId, PrinterId, WorkspaceId};
    use tower::ServiceExt;

    struct TestApplication {
        router: Router,
        repository: MemoryRepository,
        printer_id: PrinterId,
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
        repository
            .add_printer(
                tenant.workspace_id,
                tenant.environment_id,
                printer_id,
                AgentId::new(),
            )
            .await;
        authenticator.insert("spl_test_integration", tenant).await;
        TestApplication {
            router: router(AppState::new(
                Arc::new(repository.clone()),
                Arc::new(authenticator),
            )),
            repository,
            printer_id,
            tenant,
        }
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
}
