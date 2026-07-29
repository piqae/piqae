//! Authenticated loopback operational API for local integrations and shells.
//!
//! Printing remains owned by the agent loop. HTTP handlers exchange bounded
//! commands with that loop and never access `SQLite` or device credentials.

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Serialize;
use spool_local_ipc::{LocalPrinter, LocalStatus, SessionAuthenticator};
use std::{net::SocketAddr, sync::Arc};
use thiserror::Error;
use tokio::sync::{mpsc, oneshot};
use tower_http::limit::RequestBodyLimitLayer;

#[derive(Debug)]
pub enum ControlRequest {
    Status {
        respond_to: oneshot::Sender<LocalStatus>,
    },
    Printers {
        respond_to: oneshot::Sender<Vec<LocalPrinter>>,
    },
    Pause {
        respond_to: oneshot::Sender<Result<(), ControlFailure>>,
    },
    Resume {
        respond_to: oneshot::Sender<Result<(), ControlFailure>>,
    },
    SubmitJob {
        request: Box<LocalCreateJob>,
        respond_to: oneshot::Sender<Result<LocalJobAccepted, ControlFailure>>,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct ControlFailure {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct LocalCreateJob {
    pub printer_id: String,
    pub printer_native_id: String,
    pub title: String,
    pub content_kind: spool_domain::ContentKind,
    pub content: LocalContent,
    #[serde(default)]
    pub options: spool_domain::JobOptions,
    pub expires_unix_ms: Option<i64>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LocalContent {
    Base64 { data: String },
    Uri { uri: String },
}

#[derive(Debug, Clone, Serialize)]
pub struct LocalJobAccepted {
    pub job_id: String,
    pub state: String,
}

#[derive(Debug, Error)]
pub enum LocalApiError {
    #[error("local API must bind to a loopback address, got {0}")]
    NonLoopback(SocketAddr),
    #[error("local API server failed: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Clone)]
pub struct LocalApiState {
    authenticator: Arc<SessionAuthenticator>,
    control: mpsc::Sender<ControlRequest>,
}

impl std::fmt::Debug for LocalApiState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LocalApiState")
            .field("authenticator", &"<redacted>")
            .field("control", &self.control)
            .finish()
    }
}

impl LocalApiState {
    #[must_use]
    pub fn new(challenge: &str, control: mpsc::Sender<ControlRequest>) -> Self {
        Self {
            authenticator: Arc::new(SessionAuthenticator::from_challenge(challenge)),
            control,
        }
    }
}

pub fn router(state: LocalApiState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/local/status", get(status))
        .route("/v1/local/printers", get(printers))
        .route("/v1/local/pause", post(pause))
        .route("/v1/local/resume", post(resume))
        .route("/v1/jobs", post(submit_job))
        // Leave enough envelope room for the 50 MiB content limit after
        // Base64 expansion. URI submissions remain the low-memory path.
        .layer(RequestBodyLimitLayer::new(72 * 1024 * 1024))
        .with_state(state)
}

/// Serves the operational API on a loopback-only socket.
///
/// # Errors
///
/// Returns an error for non-loopback addresses or when binding or serving the
/// socket fails.
pub async fn serve(address: SocketAddr, state: LocalApiState) -> Result<(), LocalApiError> {
    if !address.ip().is_loopback() {
        return Err(LocalApiError::NonLoopback(address));
    }
    let listener = tokio::net::TcpListener::bind(address).await?;
    axum::serve(listener, router(state)).await?;
    Ok(())
}

async fn health() -> impl IntoResponse {
    StatusCode::NO_CONTENT
}

async fn status(State(state): State<LocalApiState>, headers: HeaderMap) -> Response {
    if !authenticate(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let (send, receive) = oneshot::channel();
    if state
        .control
        .send(ControlRequest::Status { respond_to: send })
        .await
        .is_err()
    {
        return unavailable();
    }
    receive
        .await
        .map_or_else(|_| unavailable(), |status| Json(status).into_response())
}

async fn printers(State(state): State<LocalApiState>, headers: HeaderMap) -> Response {
    if !authenticate(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let (send, receive) = oneshot::channel();
    if state
        .control
        .send(ControlRequest::Printers { respond_to: send })
        .await
        .is_err()
    {
        return unavailable();
    }
    receive
        .await
        .map_or_else(|_| unavailable(), |printers| Json(printers).into_response())
}

async fn pause(State(state): State<LocalApiState>, headers: HeaderMap) -> Response {
    control_action(state, headers, |respond_to| ControlRequest::Pause {
        respond_to,
    })
    .await
}

async fn resume(State(state): State<LocalApiState>, headers: HeaderMap) -> Response {
    control_action(state, headers, |respond_to| ControlRequest::Resume {
        respond_to,
    })
    .await
}

async fn submit_job(
    State(state): State<LocalApiState>,
    headers: HeaderMap,
    Json(request): Json<LocalCreateJob>,
) -> Response {
    if !authenticate(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let (send, receive) = oneshot::channel();
    if state
        .control
        .send(ControlRequest::SubmitJob {
            request: Box::new(request),
            respond_to: send,
        })
        .await
        .is_err()
    {
        return unavailable();
    }
    match receive.await {
        Ok(Ok(accepted)) => (StatusCode::ACCEPTED, Json(accepted)).into_response(),
        Ok(Err(failure)) => (StatusCode::UNPROCESSABLE_ENTITY, Json(failure)).into_response(),
        Err(_) => unavailable(),
    }
}

async fn control_action(
    state: LocalApiState,
    headers: HeaderMap,
    operation: impl FnOnce(oneshot::Sender<Result<(), ControlFailure>>) -> ControlRequest,
) -> Response {
    if !authenticate(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let (send, receive) = oneshot::channel();
    if state.control.send(operation(send)).await.is_err() {
        return unavailable();
    }
    match receive.await {
        Ok(Ok(())) => StatusCode::NO_CONTENT.into_response(),
        Ok(Err(failure)) => (StatusCode::CONFLICT, Json(failure)).into_response(),
        Err(_) => unavailable(),
    }
}

fn authenticate(state: &LocalApiState, headers: &HeaderMap) -> bool {
    let Some(value) = headers.get("authorization") else {
        return false;
    };
    let Ok(value) = value.to_str() else {
        return false;
    };
    let Some(candidate) = value.strip_prefix("Bearer ") else {
        return false;
    };
    state.authenticator.authenticate(candidate)
}

fn unavailable() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(ControlFailure {
            code: "agent_control_unavailable".into(),
            message: "the agent control loop is unavailable".into(),
        }),
    )
        .into_response()
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    fn test_state() -> (LocalApiState, mpsc::Receiver<ControlRequest>) {
        let (send, receive) = mpsc::channel(4);
        (LocalApiState::new("secret", send), receive)
    }

    #[tokio::test]
    async fn rejects_missing_or_wrong_bearer_token() {
        let (state, _receive) = test_state();
        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri("/v1/local/status")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn authenticated_status_uses_control_channel() {
        let (state, mut receive) = test_state();
        let responder = tokio::spawn(async move {
            if let Some(ControlRequest::Status { respond_to }) = receive.recv().await {
                let _ = respond_to.send(LocalStatus {
                    agent_id: Some("agt_test".into()),
                    workspace_name: Some("Test".into()),
                    version: "0.1.0".into(),
                    connection: spool_local_ipc::ConnectionState::Connected,
                    queued_jobs: 0,
                    active_jobs: 0,
                    printer_warnings: 0,
                    paused: false,
                });
            }
        });
        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri("/v1/local/status")
                    .header("authorization", "Bearer secret")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        responder.await.expect("responder");
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn refuses_non_loopback_bind_before_opening_socket() {
        let (state, _receive) = test_state();
        let address = "0.0.0.0:39100".parse().expect("address");
        assert!(matches!(
            serve(address, state).await,
            Err(LocalApiError::NonLoopback(_))
        ));
    }
}
