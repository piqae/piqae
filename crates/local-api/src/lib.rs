//! Authenticated loopback operational API for local integrations and shells.
//!
//! Printing remains owned by the agent loop. HTTP handlers exchange bounded
//! commands with that loop and never access `SQLite` or device credentials.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use piqae_local_ipc::{
    ConfirmLoadedMedia, LocalPrinter, LocalPrinterProfile, LocalPrinterQueue, LocalStatus,
    NativeProfileCapturePayload, ProfileCaptureAuthorized, ProfileValidationResult,
    SessionAuthenticator,
};
use serde::{Deserialize, Serialize};
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
    SetPrinterExposure {
        printer_id: String,
        exposed: bool,
        respond_to: oneshot::Sender<Result<LocalPrinter, ControlFailure>>,
    },
    Profiles {
        printer_id: String,
        respond_to: oneshot::Sender<Result<Vec<LocalPrinterProfile>, ControlFailure>>,
    },
    CreateProfile {
        printer_id: String,
        request: ProfileCreate,
        respond_to: oneshot::Sender<Result<LocalPrinterProfile, ControlFailure>>,
    },
    UpdateProfile {
        printer_id: String,
        profile_id: String,
        request: ProfileUpdate,
        respond_to: oneshot::Sender<Result<LocalPrinterProfile, ControlFailure>>,
    },
    DeleteProfile {
        printer_id: String,
        profile_id: String,
        expected_revision: u64,
        respond_to: oneshot::Sender<Result<(), ControlFailure>>,
    },
    BeginProfileCapture {
        printer_id: String,
        request: ProfileCaptureBeginRequest,
        respond_to: oneshot::Sender<Result<ProfileCaptureAuthorized, ControlFailure>>,
    },
    CommitProfileCapture {
        session_id: String,
        capture_token: String,
        capture: Box<NativeProfileCapturePayload>,
        respond_to: oneshot::Sender<Result<LocalPrinterProfile, ControlFailure>>,
    },
    CancelProfileCapture {
        session_id: String,
        capture_token: String,
        respond_to: oneshot::Sender<Result<(), ControlFailure>>,
    },
    ValidateProfile {
        profile_id: String,
        revision: u64,
        respond_to: oneshot::Sender<Result<ProfileValidationResult, ControlFailure>>,
    },
    ConfirmLoadedMedia {
        request: ConfirmLoadedMedia,
        respond_to: oneshot::Sender<Result<(), ControlFailure>>,
    },
    PrinterQueue {
        printer_id: String,
        respond_to: oneshot::Sender<Result<LocalPrinterQueue, ControlFailure>>,
    },
    TestPage {
        printer_id: String,
        profile_id: String,
        confirmed: bool,
        respond_to: oneshot::Sender<Result<LocalJobAccepted, ControlFailure>>,
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
    #[serde(default)]
    pub printer_native_id: Option<String>,
    pub title: String,
    pub content_kind: piqae_domain::ContentKind,
    pub content: LocalContent,
    #[serde(default)]
    pub options: piqae_domain::JobOptions,
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

#[derive(Debug, Clone, Deserialize)]
pub struct ExposureUpdate {
    pub exposed: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileCreate {
    pub name: String,
    #[serde(default)]
    pub is_default: bool,
    #[serde(default)]
    pub options: piqae_domain::JobOptions,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileUpdate {
    pub expected_revision: u64,
    pub name: String,
    #[serde(default)]
    pub is_default: bool,
    #[serde(default)]
    pub options: piqae_domain::JobOptions,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeleteProfileQuery {
    pub expected_revision: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TestPageRequest {
    pub profile_id: String,
    /// A local driver test may address an installed printer before it is
    /// exposed to cloud/API jobs, but only after an explicit user action.
    #[serde(default)]
    pub confirmed: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileCaptureBeginRequest {
    pub operation: piqae_domain::ProfileCaptureOperation,
    pub profile_id: Option<String>,
    pub expected_revision: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidateProfileRequest {
    pub revision: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfirmLoadedMediaRequest {
    pub stock_id: Option<String>,
    pub confidence: piqae_domain::LoadedMediaConfidence,
    pub confirmed_by: Option<String>,
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
        .route(
            "/v1/local/printers/{printer_id}/exposure",
            put(set_printer_exposure),
        )
        .route(
            "/v1/local/printers/{printer_id}/profiles",
            get(profiles).post(create_profile),
        )
        .route(
            "/v1/local/printers/{printer_id}/profiles/{profile_id}",
            put(update_profile).delete(delete_profile),
        )
        .route(
            "/v1/local/printers/{printer_id}/profile-capture-sessions",
            post(begin_profile_capture),
        )
        .route(
            "/v1/local/profile-capture-sessions/{session_id}/complete",
            post(commit_profile_capture),
        )
        .route(
            "/v1/local/profile-capture-sessions/{session_id}",
            axum::routing::delete(cancel_profile_capture),
        )
        .route(
            "/v1/local/profiles/{profile_id}/validate",
            post(validate_profile),
        )
        .route(
            "/v1/local/devices/{device_id}/loaded-media/{source}",
            put(confirm_loaded_media),
        )
        .route("/v1/local/printers/{printer_id}/queue", get(printer_queue))
        .route("/v1/local/printers/{printer_id}/test-page", post(test_page))
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

async fn set_printer_exposure(
    State(state): State<LocalApiState>,
    Path(printer_id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<ExposureUpdate>,
) -> Response {
    request_response(
        state,
        headers,
        |respond_to| ControlRequest::SetPrinterExposure {
            printer_id,
            exposed: request.exposed,
            respond_to,
        },
        StatusCode::OK,
    )
    .await
}

async fn profiles(
    State(state): State<LocalApiState>,
    Path(printer_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    request_response(
        state,
        headers,
        |respond_to| ControlRequest::Profiles {
            printer_id,
            respond_to,
        },
        StatusCode::OK,
    )
    .await
}

async fn create_profile(
    State(state): State<LocalApiState>,
    Path(printer_id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<ProfileCreate>,
) -> Response {
    request_response(
        state,
        headers,
        |respond_to| ControlRequest::CreateProfile {
            printer_id,
            request,
            respond_to,
        },
        StatusCode::CREATED,
    )
    .await
}

async fn update_profile(
    State(state): State<LocalApiState>,
    Path((printer_id, profile_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(request): Json<ProfileUpdate>,
) -> Response {
    request_response(
        state,
        headers,
        |respond_to| ControlRequest::UpdateProfile {
            printer_id,
            profile_id,
            request,
            respond_to,
        },
        StatusCode::OK,
    )
    .await
}

async fn delete_profile(
    State(state): State<LocalApiState>,
    Path((printer_id, profile_id)): Path<(String, String)>,
    Query(query): Query<DeleteProfileQuery>,
    headers: HeaderMap,
) -> Response {
    control_action(state, headers, |respond_to| ControlRequest::DeleteProfile {
        printer_id,
        profile_id,
        expected_revision: query.expected_revision,
        respond_to,
    })
    .await
}

async fn begin_profile_capture(
    State(state): State<LocalApiState>,
    Path(printer_id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<ProfileCaptureBeginRequest>,
) -> Response {
    request_response(
        state,
        headers,
        |respond_to| ControlRequest::BeginProfileCapture {
            printer_id,
            request,
            respond_to,
        },
        StatusCode::CREATED,
    )
    .await
}

async fn commit_profile_capture(
    State(state): State<LocalApiState>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
    Json(capture): Json<NativeProfileCapturePayload>,
) -> Response {
    if !authenticate(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Some(capture_token) = capture_token(&headers) else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(ControlFailure {
                code: "capture_token_required".into(),
                message: "X-Piqae-Capture-Token is required".into(),
            }),
        )
            .into_response();
    };
    request_response(
        state,
        headers,
        |respond_to| ControlRequest::CommitProfileCapture {
            session_id,
            capture_token,
            capture: Box::new(capture),
            respond_to,
        },
        StatusCode::CREATED,
    )
    .await
}

async fn cancel_profile_capture(
    State(state): State<LocalApiState>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !authenticate(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Some(capture_token) = capture_token(&headers) else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(ControlFailure {
                code: "capture_token_required".into(),
                message: "X-Piqae-Capture-Token is required".into(),
            }),
        )
            .into_response();
    };
    control_action(state, headers, |respond_to| {
        ControlRequest::CancelProfileCapture {
            session_id,
            capture_token,
            respond_to,
        }
    })
    .await
}

async fn validate_profile(
    State(state): State<LocalApiState>,
    Path(profile_id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<ValidateProfileRequest>,
) -> Response {
    request_response(
        state,
        headers,
        |respond_to| ControlRequest::ValidateProfile {
            profile_id,
            revision: request.revision,
            respond_to,
        },
        StatusCode::OK,
    )
    .await
}

async fn confirm_loaded_media(
    State(state): State<LocalApiState>,
    Path((device_id, source)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<ConfirmLoadedMediaRequest>,
) -> Response {
    let request = ConfirmLoadedMedia {
        device_id,
        source,
        stock_id: body.stock_id,
        confidence: body.confidence,
        confirmed_by: body.confirmed_by,
    };
    control_action(state, headers, |respond_to| {
        ControlRequest::ConfirmLoadedMedia {
            request,
            respond_to,
        }
    })
    .await
}

async fn printer_queue(
    State(state): State<LocalApiState>,
    Path(printer_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    request_response(
        state,
        headers,
        |respond_to| ControlRequest::PrinterQueue {
            printer_id,
            respond_to,
        },
        StatusCode::OK,
    )
    .await
}

async fn test_page(
    State(state): State<LocalApiState>,
    Path(printer_id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<TestPageRequest>,
) -> Response {
    request_response(
        state,
        headers,
        |respond_to| ControlRequest::TestPage {
            printer_id,
            profile_id: request.profile_id,
            confirmed: request.confirmed,
            respond_to,
        },
        StatusCode::ACCEPTED,
    )
    .await
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
        Ok(Err(failure)) => (failure_status(&failure.code), Json(failure)).into_response(),
        Err(_) => unavailable(),
    }
}

async fn request_response<T: Serialize>(
    state: LocalApiState,
    headers: HeaderMap,
    operation: impl FnOnce(oneshot::Sender<Result<T, ControlFailure>>) -> ControlRequest,
    success: StatusCode,
) -> Response {
    if !authenticate(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let (send, receive) = oneshot::channel();
    if state.control.send(operation(send)).await.is_err() {
        return unavailable();
    }
    match receive.await {
        Ok(Ok(value)) => (success, Json(value)).into_response(),
        Ok(Err(failure)) => (failure_status(&failure.code), Json(failure)).into_response(),
        Err(_) => unavailable(),
    }
}

fn failure_status(code: &str) -> StatusCode {
    match code {
        "printer_not_found" | "profile_not_found" | "profile_capture_not_found" => {
            StatusCode::NOT_FOUND
        }
        "profile_revision_conflict" => StatusCode::CONFLICT,
        "profile_capture_token_invalid" => StatusCode::UNAUTHORIZED,
        "profile_capture_timed_out"
        | "profile_capture_cancelled"
        | "profile_capture_not_authorized" => StatusCode::GONE,
        _ => StatusCode::UNPROCESSABLE_ENTITY,
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

fn capture_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-piqae-capture-token")
        .or_else(|| headers.get("x-spool-capture-token"))
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty() && value.len() <= 512)
        .map(str::to_owned)
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

    fn profile(profile_id: &str, name: String) -> LocalPrinterProfile {
        LocalPrinterProfile {
            profile_id: profile_id.into(),
            revision: 1,
            name,
            is_default: true,
            options: piqae_domain::JobOptions::default(),
            status: piqae_domain::ProfileStatus::NeedsTest,
            native_kind: Some(piqae_domain::NativeProfileKind::PortableOptions),
            native_digest: None,
            driver_fingerprint: piqae_domain::DriverFingerprint::default(),
            summary: piqae_domain::ProfileSummary::default(),
            stock_id: None,
            dependencies: Vec::new(),
            safe_overrides: Vec::new(),
            last_validated_unix_ms: None,
            last_test_job_id: None,
            published: false,
            uses_current_printer_defaults: false,
        }
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
                    connection: piqae_local_ipc::ConnectionState::Connected,
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
    async fn authenticated_profile_create_dispatches_bounded_control_request() {
        let (state, mut receive) = test_state();
        let responder = tokio::spawn(async move {
            if let Some(ControlRequest::CreateProfile {
                printer_id,
                request,
                respond_to,
            }) = receive.recv().await
            {
                assert_eq!(printer_id, "ptr_test");
                assert_eq!(request.name, "A4 Colour");
                let _ = respond_to.send(Ok(LocalPrinterProfile {
                    options: request.options,
                    ..profile("prf_test", request.name)
                }));
            }
        });
        let response = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/local/printers/ptr_test/profiles")
                    .header("authorization", "Bearer secret")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"name":"A4 Colour","is_default":true,"options":{}}"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        responder.await.expect("responder");
        assert_eq!(response.status(), StatusCode::CREATED);
    }

    #[tokio::test]
    async fn local_test_page_dispatches_explicit_confirmation() {
        let (state, mut receive) = test_state();
        let responder = tokio::spawn(async move {
            if let Some(ControlRequest::TestPage {
                printer_id,
                profile_id,
                confirmed,
                respond_to,
            }) = receive.recv().await
            {
                assert_eq!(printer_id, "ptr_test");
                assert_eq!(profile_id, "prf_defaults");
                assert!(confirmed);
                let _ = respond_to.send(Ok(LocalJobAccepted {
                    job_id: "job_test".into(),
                    state: "queued_local".into(),
                }));
            }
        });
        let response = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/local/printers/ptr_test/test-page")
                    .header("authorization", "Bearer secret")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"profile_id":"prf_defaults","confirmed":true}"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        responder.await.expect("responder");
        assert_eq!(response.status(), StatusCode::ACCEPTED);
    }

    #[tokio::test]
    async fn native_capture_commit_requires_both_session_and_capture_tokens() {
        let (state, mut receive) = test_state();
        let responder = tokio::spawn(async move {
            if let Some(ControlRequest::CommitProfileCapture {
                session_id,
                capture_token,
                capture,
                respond_to,
            }) = receive.recv().await
            {
                assert_eq!(session_id, "pcs_test");
                assert_eq!(capture_token, "one-time-token");
                assert_eq!(capture.name, "A4 colour");
                assert_eq!(
                    capture.native_kind,
                    piqae_domain::NativeProfileKind::MacosPrintcore
                );
                let _ = respond_to.send(Ok(profile("prf_native", capture.name.clone())));
            }
        });
        let response = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/local/profile-capture-sessions/pcs_test/complete")
                    .header("authorization", "Bearer secret")
                    .header("x-piqae-capture-token", "one-time-token")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{
                          "name":"A4 colour",
                          "native_kind":"macos_printcore",
                          "native_schema_version":1,
                          "native_digest":"sha256:test",
                          "native_blob_base64":"b3BhcXVl",
                          "driver_fingerprint":{},
                          "summary":{}
                        }"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::CREATED);
        responder.await.expect("responder");
    }

    #[tokio::test]
    async fn capture_token_presence_is_hidden_until_session_authentication() {
        let (state, mut receive) = test_state();
        let response = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/local/profile-capture-sessions/pcs_test/complete")
                    .header("x-piqae-capture-token", "one-time-token")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{
                          "name":"A4",
                          "native_kind":"macos_printcore",
                          "native_schema_version":1,
                          "native_digest":"sha256:test",
                          "native_blob_base64":"b3BhcXVl",
                          "driver_fingerprint":{},
                          "summary":{}
                        }"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(receive.try_recv().is_err());
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
