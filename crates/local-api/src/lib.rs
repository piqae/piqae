//! Authenticated loopback operational API for local integrations and shells.
//!
//! Printing remains owned by the agent loop. HTTP handlers exchange bounded
//! commands with that loop and never access `SQLite` or device credentials.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post, put},
};
use piqae_local_ipc::{
    BrokerAuthorizationDecision, ConfirmLoadedMedia, NativeProfileCapturePayload,
    SessionAuthenticator,
};
pub use piqae_node_runtime::command::{
    CommandFailure as ControlFailure, ConfirmLoadedMediaRequest, DeleteProfileQuery,
    ExposureUpdate, HostLifecycleRequest, LocalConnectorDetail, LocalContent, LocalCreateJob,
    LocalHistoryJob, LocalJobAccepted, LocalJobHistory, NodeIdentityUpdate, NodeIdentityUpdated,
    ProfileCaptureBeginRequest, ProfileCreate, ProfileUpdate, RuntimeCommand as ControlRequest,
    TestPageRequest, ValidateProfileRequest,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};
use thiserror::Error;
use tokio::sync::{mpsc, oneshot};
use tower_http::limit::RequestBodyLimitLayer;

#[derive(Debug, Deserialize)]
struct ReprintRequest {
    idempotency_key: String,
    #[serde(default)]
    confirmed: bool,
}

#[derive(Debug, Serialize)]
struct DashboardSessionCreated {
    url: String,
    expires_in_seconds: u64,
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
    browser_sessions: Arc<tokio::sync::Mutex<BrowserSessions>>,
    bound_address: Arc<tokio::sync::RwLock<Option<SocketAddr>>>,
}

#[derive(Debug, Default)]
struct BrowserSessions {
    handoffs: BTreeMap<String, Instant>,
    sessions: BTreeMap<String, Instant>,
}

impl std::fmt::Debug for LocalApiState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LocalApiState")
            .field("authenticator", &"<redacted>")
            .field("control", &self.control)
            .field("browser_sessions", &"<redacted>")
            .field("bound_address", &"<loopback>")
            .finish()
    }
}

impl LocalApiState {
    #[must_use]
    pub fn new(challenge: &str, control: mpsc::Sender<ControlRequest>) -> Self {
        Self {
            authenticator: Arc::new(SessionAuthenticator::from_challenge(challenge)),
            control,
            browser_sessions: Arc::new(tokio::sync::Mutex::new(BrowserSessions::default())),
            bound_address: Arc::new(tokio::sync::RwLock::new(None)),
        }
    }
}

pub fn router(state: LocalApiState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/local/status", get(status))
        .route("/v1/local/node/identity", put(update_node_identity))
        .route("/v1/local/lifecycle", post(apply_host_lifecycle))
        .route(
            "/v1/local/broker/authorization-requests",
            get(pending_broker_authorizations),
        )
        .route(
            "/v1/local/broker/authorization-requests/{authorization_id}/decision",
            post(decide_broker_authorization),
        )
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
        .route(
            "/v1/local/dashboard-sessions",
            post(create_dashboard_session),
        )
        .route(
            "/v1/local/dashboard-sessions/{view}",
            post(create_dashboard_session_for_view),
        )
        .route("/local/node", get(open_dashboard))
        .route("/local/node/identity", put(dashboard_update_node_identity))
        .route("/local/history", get(open_dashboard))
        .route("/local/connections", get(open_dashboard))
        // Keep the pre-split URL alive so bookmarks and an older macOS shell
        // can renew their narrowly scoped dashboard cookie during upgrades.
        .route("/local/queue", get(open_dashboard))
        .route("/local/dashboard/data", get(dashboard_data))
        .route(
            "/local/history/jobs/{job_id}/reprint",
            post(dashboard_reprint),
        )
        .route("/v1/local/printers/{printer_id}/test-page", post(test_page))
        .route("/v1/local/pause", post(pause))
        .route("/v1/local/resume", post(resume))
        .route("/v1/local/connectors/reload", post(reload_connectors))
        .route(
            "/v1/local/connectors/{connector_id}",
            axum::routing::delete(revoke_connector),
        )
        .route("/v1/jobs", post(submit_job))
        // Leave enough envelope room for the 50 MiB content limit after
        // Base64 expansion. URI submissions remain the low-memory path.
        .layer(RequestBodyLimitLayer::new(72 * 1024 * 1024))
        .with_state(state)
}

const HANDOFF_LIFETIME: Duration = Duration::from_secs(30);
const BROWSER_SESSION_LIFETIME: Duration = Duration::from_secs(15 * 60);

async fn create_dashboard_session(
    State(state): State<LocalApiState>,
    headers: HeaderMap,
) -> Response {
    if !authenticate(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let token = uuid::Uuid::new_v4().simple().to_string();
    {
        let mut sessions = state.browser_sessions.lock().await;
        prune_browser_sessions(&mut sessions);
        sessions
            .handoffs
            .insert(token.clone(), Instant::now() + HANDOFF_LIFETIME);
    }
    let header_authority = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .and_then(loopback_authority);
    let authority = if let Some(authority) = header_authority {
        authority
    } else {
        state.bound_address.read().await.map_or_else(
            || "127.0.0.1:39100".to_owned(),
            |address| address.to_string(),
        )
    };
    let destination = match headers
        .get("x-piqae-dashboard-view")
        .and_then(|value| value.to_str().ok())
    {
        Some("connections") => "connections",
        Some("node") => "node",
        _ => "history",
    };
    Json(DashboardSessionCreated {
        url: format!("http://{authority}/local/{destination}?handoff={token}"),
        expires_in_seconds: HANDOFF_LIFETIME.as_secs(),
    })
    .into_response()
}

async fn create_dashboard_session_for_view(
    State(state): State<LocalApiState>,
    Path(view): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !matches!(view.as_str(), "history" | "connections" | "node") {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Ok(value) = HeaderValue::from_str(&view) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let mut headers = headers;
    headers.insert("x-piqae-dashboard-view", value);
    create_dashboard_session(State(state), headers).await
}

#[derive(Debug, Deserialize)]
struct HandoffQuery {
    handoff: Option<String>,
    offset: Option<usize>,
}

async fn open_dashboard(
    State(state): State<LocalApiState>,
    Query(query): Query<HandoffQuery>,
    uri: axum::http::Uri,
    headers: HeaderMap,
) -> Response {
    if dashboard_authenticated(&state, &headers).await {
        if uri.path() == "/local/queue" {
            let mut response = Redirect::to("/local/history").into_response();
            if let Some(session) = cookie_value(&headers, "piqae_local_session") {
                set_dashboard_cookie(&mut response, &session);
            }
            return response;
        }
        return dashboard_html().into_response();
    }
    let Some(handoff) = query.handoff.filter(|value| value.len() <= 64) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let mut sessions = state.browser_sessions.lock().await;
    prune_browser_sessions(&mut sessions);
    if sessions.handoffs.remove(&handoff).is_none() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let session = uuid::Uuid::new_v4().simple().to_string();
    sessions
        .sessions
        .insert(session.clone(), Instant::now() + BROWSER_SESSION_LIFETIME);
    drop(sessions);
    let path = match uri.path() {
        "/local/connections" => "/local/connections",
        "/local/node" => "/local/node",
        _ => "/local/history",
    };
    let mut response = Redirect::to(path).into_response();
    set_dashboard_cookie(&mut response, &session);
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn set_dashboard_cookie(response: &mut Response, session: &str) {
    if let Ok(cookie) = HeaderValue::from_str(&format!(
        "piqae_local_session={session}; HttpOnly; SameSite=Strict; Path=/local; Max-Age={}",
        BROWSER_SESSION_LIFETIME.as_secs()
    )) {
        response.headers_mut().insert(header::SET_COOKIE, cookie);
    }
}

async fn dashboard_data(
    State(state): State<LocalApiState>,
    Query(query): Query<HandoffQuery>,
    headers: HeaderMap,
) -> Response {
    if !dashboard_authenticated(&state, &headers).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let offset = query.offset.unwrap_or(0).min(1_000_000);
    let (history_send, history_receive) = oneshot::channel();
    if state
        .control
        .send(ControlRequest::JobHistory {
            offset,
            limit: 100,
            respond_to: history_send,
        })
        .await
        .is_err()
    {
        return unavailable();
    }
    let (connector_send, connector_receive) = oneshot::channel();
    if state
        .control
        .send(ControlRequest::ConnectorDetails {
            respond_to: connector_send,
        })
        .await
        .is_err()
    {
        return unavailable();
    }
    let (status_send, status_receive) = oneshot::channel();
    if state
        .control
        .send(ControlRequest::Status {
            respond_to: status_send,
        })
        .await
        .is_err()
    {
        return unavailable();
    }
    match (
        history_receive.await,
        connector_receive.await,
        status_receive.await,
    ) {
        (Ok(Ok(history)), Ok(Ok(connectors)), Ok(status)) => Json(
            serde_json::json!({"history": history, "connectors": connectors, "status": status}),
        )
        .into_response(),
        (Ok(Err(failure)), _, _) | (_, Ok(Err(failure)), _) => {
            (failure_status(&failure.code), Json(failure)).into_response()
        }
        _ => unavailable(),
    }
}

async fn dashboard_update_node_identity(
    State(state): State<LocalApiState>,
    headers: HeaderMap,
    Json(request): Json<NodeIdentityUpdate>,
) -> Response {
    if !dashboard_authenticated(&state, &headers).await
        || headers.get("x-piqae-local-action").is_none()
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let (send, receive) = oneshot::channel();
    if state
        .control
        .send(ControlRequest::UpdateNodeIdentity {
            request,
            respond_to: send,
        })
        .await
        .is_err()
    {
        return unavailable();
    }
    match receive.await {
        Ok(Ok(updated)) => Json(updated).into_response(),
        Ok(Err(failure)) => (failure_status(&failure.code), Json(failure)).into_response(),
        Err(_) => unavailable(),
    }
}

async fn dashboard_reprint(
    State(state): State<LocalApiState>,
    Path(job_id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<ReprintRequest>,
) -> Response {
    if !dashboard_authenticated(&state, &headers).await
        || headers.get("x-piqae-local-action").is_none()
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if job_id.len() > 128
        || request.idempotency_key.is_empty()
        || request.idempotency_key.len() > 128
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let (send, receive) = oneshot::channel();
    if state
        .control
        .send(ControlRequest::ReprintJob {
            job_id,
            idempotency_key: request.idempotency_key,
            confirmed: request.confirmed,
            respond_to: send,
        })
        .await
        .is_err()
    {
        return unavailable();
    }
    match receive.await {
        Ok(Ok(accepted)) => (StatusCode::ACCEPTED, Json(accepted)).into_response(),
        Ok(Err(failure)) => (failure_status(&failure.code), Json(failure)).into_response(),
        Err(_) => unavailable(),
    }
}

async fn dashboard_authenticated(state: &LocalApiState, headers: &HeaderMap) -> bool {
    let Some(session) = cookie_value(headers, "piqae_local_session") else {
        return false;
    };
    let mut sessions = state.browser_sessions.lock().await;
    prune_browser_sessions(&mut sessions);
    sessions
        .sessions
        .get(&session)
        .is_some_and(|expiry| *expiry > Instant::now())
}

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .map(str::trim)
        .find_map(|part| {
            let (key, value) = part.split_once('=')?;
            (key == name && !value.is_empty() && value.len() <= 64).then(|| value.to_owned())
        })
}

fn prune_browser_sessions(sessions: &mut BrowserSessions) {
    let now = Instant::now();
    sessions.handoffs.retain(|_, expiry| *expiry > now);
    sessions.sessions.retain(|_, expiry| *expiry > now);
}

fn loopback_authority(value: &str) -> Option<String> {
    if let Ok(address) = value.parse::<SocketAddr>() {
        return address.ip().is_loopback().then(|| address.to_string());
    }
    let port = value.strip_prefix("localhost:")?.parse::<u16>().ok()?;
    Some(format!("localhost:{port}"))
}

fn dashboard_html() -> Response {
    let mut response = Html(include_str!("dashboard.html")).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; connect-src 'self'; img-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'",
        ),
    );
    response.headers_mut().insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    response
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
    *state.bound_address.write().await = Some(listener.local_addr()?);
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

async fn update_node_identity(
    State(state): State<LocalApiState>,
    headers: HeaderMap,
    Json(request): Json<NodeIdentityUpdate>,
) -> Response {
    request_response(
        state,
        headers,
        |respond_to| ControlRequest::UpdateNodeIdentity {
            request,
            respond_to,
        },
        StatusCode::OK,
    )
    .await
}

async fn apply_host_lifecycle(
    State(state): State<LocalApiState>,
    headers: HeaderMap,
    Json(request): Json<HostLifecycleRequest>,
) -> Response {
    if !authenticate(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let (send, receive) = oneshot::channel();
    if state
        .control
        .send(ControlRequest::ApplyHostLifecycle {
            event: request.event,
            respond_to: send,
        })
        .await
        .is_err()
    {
        return unavailable();
    }
    receive
        .await
        .map_or_else(|_| unavailable(), |snapshot| Json(snapshot).into_response())
}

async fn pending_broker_authorizations(
    State(state): State<LocalApiState>,
    headers: HeaderMap,
) -> Response {
    if !authenticate(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let (send, receive) = oneshot::channel();
    if state
        .control
        .send(ControlRequest::PendingBrokerAuthorizations { respond_to: send })
        .await
        .is_err()
    {
        return unavailable();
    }
    receive
        .await
        .map_or_else(|_| unavailable(), |pending| Json(pending).into_response())
}

async fn decide_broker_authorization(
    State(state): State<LocalApiState>,
    Path(authorization_id): Path<uuid::Uuid>,
    headers: HeaderMap,
    Json(decision): Json<BrokerAuthorizationDecision>,
) -> Response {
    control_action(state, headers, |respond_to| {
        ControlRequest::DecideBrokerAuthorization {
            authorization_id,
            decision,
            respond_to,
        }
    })
    .await
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
                current_revision: None,
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
                current_revision: None,
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

async fn reload_connectors(State(state): State<LocalApiState>, headers: HeaderMap) -> Response {
    control_action(state, headers, |respond_to| {
        ControlRequest::ReloadConnectors { respond_to }
    })
    .await
}

async fn revoke_connector(
    State(state): State<LocalApiState>,
    Path(connector_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let dashboard_request = dashboard_authenticated(&state, &headers).await
        && headers
            .get("x-piqae-local-action")
            .is_some_and(|value| value == "disconnect");
    if !authenticate(&state, &headers) && !dashboard_request {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if connector_id.len() > 128
        || !connector_id.starts_with("ncon_")
        || !connector_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let (send, receive) = oneshot::channel();
    if state
        .control
        .send(ControlRequest::RevokeConnector {
            connector_id,
            respond_to: send,
        })
        .await
        .is_err()
    {
        return unavailable();
    }
    match receive.await {
        Ok(Ok(())) => StatusCode::NO_CONTENT.into_response(),
        Ok(Err(failure)) => (failure_status(&failure.code), Json(failure)).into_response(),
        Err(_) => unavailable(),
    }
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
        "printer_not_found"
        | "profile_not_found"
        | "profile_capture_not_found"
        | "broker_authorization_not_found" => StatusCode::NOT_FOUND,
        "profile_revision_conflict" | "node_identity_revision_conflict" => StatusCode::CONFLICT,
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
            current_revision: None,
        }),
    )
        .into_response()
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use piqae_local_ipc::{
        BrokerApplicationIdentity, BrokerCapability, LocalPrinterProfile, LocalStatus,
        PendingBrokerAuthorization,
    };
    use piqae_node_runtime::{
        LifecycleEvent, LifecycleSnapshot, NetworkAvailability, PowerAvailability,
    };
    use tower::ServiceExt;

    fn test_state() -> (LocalApiState, mpsc::Receiver<ControlRequest>) {
        let (send, receive) = mpsc::channel(4);
        (LocalApiState::new("secret", send), receive)
    }

    #[test]
    fn embedded_dashboard_deduplicates_jobs_across_offset_pages() {
        let html = include_str!("dashboard.html");
        assert!(html.contains("const seenJobs=new Set()"));
        assert!(html.contains("if(!seenJobs.has(job.job_id))"));
        assert!(html.contains("seenJobs.clear()"));
    }

    #[test]
    fn embedded_dashboard_wraps_identifiers_and_keeps_recovery_owner_scoped() {
        let html = include_str!("dashboard.html");
        assert!(html.contains("overflow-wrap:anywhere"));
        assert!(html.contains("Connected workspace"));
        assert!(html.contains("Service / platform"));
        assert!(html.contains("Reconnect with owner"));
        assert!(html.contains("Return to the workspace or app that created it"));
        assert!(html.contains("rel=\"noopener noreferrer\""));
        assert!(html.contains("prefers-color-scheme:dark"));
        assert!(html.contains("type=\"datetime-local\""));
        assert!(html.contains("aria-label=\"Search print history\""));
        assert!(
            html.contains(
                "Standalone node · user-managed · multiple isolated connections supported."
            )
        );
        assert!(html.contains("Piqae does not infer your account name or address."));
        assert!(html.contains("Updated node details are stored locally and will retry"));
        assert!(html.contains("Open the owner to reconcile the local and cloud values"));
        assert!(html.contains(
            "Multiple scheduling authorities; local handoffs serialized; automatic cross-server failover disabled."
        ));
    }

    #[test]
    fn connector_diagnostics_are_bounded_and_explain_empty_projection() {
        let detail = LocalConnectorDetail {
            connector_id: "ncon_test".into(),
            display_name: "Shopify store".into(),
            workspace_name: Some("Managed customer".into()),
            authorization_type: Some("platform_customer".into()),
            workspace_id: Some("wsp_test".into()),
            environment_id: Some("env_live".into()),
            requesting_service_account_id: Some("svc_shopify".into()),
            endpoint: "https://api.example.test".into(),
            connection: "unauthorized".into(),
            permission: "all_local_printers".into(),
            allowed_printer_ids: Vec::new(),
            selected_printer_count: 0,
            last_sync_error_code: Some("invalid_agent_signature".into()),
            local_printer_count: 3,
            eligible_printer_count: 3,
            inventory_revision: 1,
            inventory_refresh_pending: true,
            identity_sync_status: "conflict".into(),
            identity_server_revision: Some(4),
            identity_conflict_revision: Some(5),
            cross_authority_route_warning: true,
            manage_url: Some("https://shop.example/settings".into()),
        };
        let encoded = serde_json::to_value(detail).expect("serialize diagnostics");
        assert_eq!(encoded["local_printer_count"], 3);
        assert_eq!(encoded["eligible_printer_count"], 3);
        assert_eq!(encoded["last_sync_error_code"], "invalid_agent_signature");
        assert_eq!(encoded["workspace_id"], "wsp_test");
        assert_eq!(encoded["requesting_service_account_id"], "svc_shopify");
        assert_eq!(encoded["manage_url"], "https://shop.example/settings");
        assert_eq!(encoded["cross_authority_route_warning"], true);
        assert_eq!(encoded["identity_sync_status"], "conflict");
        assert_eq!(encoded["identity_server_revision"], 4);
        assert_eq!(encoded["identity_conflict_revision"], 5);
        assert!(encoded.get("device_key").is_none());
        assert!(encoded.get("token").is_none());
        assert!(encoded.get("identity_evidence").is_none());
        assert!(encoded.get("fencing_token").is_none());
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
                    node_identity: None,
                    node_identity_revision: None,
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
    async fn authenticated_identity_update_is_revision_checked_by_the_agent() {
        let (state, mut receive) = test_state();
        let responder = tokio::spawn(async move {
            if let Some(ControlRequest::UpdateNodeIdentity {
                request,
                respond_to,
            }) = receive.recv().await
            {
                assert_eq!(request.expected_revision, 4);
                assert_eq!(request.display_name, "Dispatch PC");
                assert_eq!(request.site.as_deref(), Some("Warehouse"));
                let _ = respond_to.send(Ok(NodeIdentityUpdated {
                    revision: 5,
                    identity: piqae_local_ipc::LocalNodeIdentity {
                        display_name: request.display_name,
                        site: request.site,
                        location: request.location,
                        labels: request.labels,
                    },
                }));
            }
        });
        let response = router(state)
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/v1/local/node/identity")
                    .header("authorization", "Bearer secret")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"expected_revision":4,"display_name":"Dispatch PC","site":"Warehouse","location":null,"labels":[]}"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        responder.await.expect("responder");
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 4096).await.expect("body");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(json["revision"], 5);
        assert_eq!(json["identity"]["display_name"], "Dispatch PC");
    }

    #[tokio::test]
    async fn authenticated_lifecycle_event_is_dispatched_to_the_durable_host() {
        let (state, mut receive) = test_state();
        let responder = tokio::spawn(async move {
            if let Some(ControlRequest::ApplyHostLifecycle { event, respond_to }) =
                receive.recv().await
            {
                assert_eq!(event, LifecycleEvent::Sleeping);
                let _ = respond_to.send(LifecycleSnapshot {
                    foreground: false,
                    power: PowerAvailability::Sleeping,
                    network: NetworkAvailability::Unknown,
                    accepting_cloud_leases: false,
                    shutdown_requested: false,
                    generation: 2,
                });
            }
        });
        let response = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/local/lifecycle")
                    .header("authorization", "Bearer secret")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"event":"sleeping"}"#))
                    .expect("request"),
            )
            .await
            .expect("response");
        responder.await.expect("responder");
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn broker_consent_listing_and_decision_require_local_auth_and_dispatch() {
        let (state, mut receive) = test_state();
        let authorization_id = uuid::Uuid::new_v4();
        let responder = tokio::spawn(async move {
            if let Some(ControlRequest::PendingBrokerAuthorizations { respond_to }) =
                receive.recv().await
            {
                let _ = respond_to.send(vec![PendingBrokerAuthorization {
                    authorization_id,
                    application: BrokerApplicationIdentity {
                        application_id: "com.example.pos".into(),
                        display_name: "Example POS".into(),
                        signing_identity_sha256: Some("a".repeat(64)),
                    },
                    requested_capabilities: vec![BrokerCapability::ObservePrinters],
                    requested_unix_ms: 1,
                    expires_unix_ms: 2,
                }]);
            }
            if let Some(ControlRequest::DecideBrokerAuthorization {
                authorization_id: received,
                decision,
                respond_to,
            }) = receive.recv().await
            {
                assert_eq!(received, authorization_id);
                assert!(decision.approved);
                assert_eq!(
                    decision.granted_capabilities,
                    vec![BrokerCapability::ObservePrinters]
                );
                let _ = respond_to.send(Ok(()));
            }
        });
        let pending = router(state.clone())
            .oneshot(
                Request::builder()
                    .uri("/v1/local/broker/authorization-requests")
                    .header("authorization", "Bearer secret")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(pending.status(), StatusCode::OK);
        let pending_body = to_bytes(pending.into_body(), 4096).await.expect("body");
        assert!(!String::from_utf8_lossy(&pending_body).contains("nonce"));

        let decided = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/v1/local/broker/authorization-requests/{authorization_id}/decision"
                    ))
                    .header("authorization", "Bearer secret")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"approved":true,"granted_capabilities":["observe_printers"]}"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        responder.await.expect("responder");
        assert_eq!(decided.status(), StatusCode::NO_CONTENT);
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
    async fn connector_controls_require_authentication_and_validate_ids() {
        let (state, mut receive) = test_state();
        let unauthorized = router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/local/connectors/reload")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
        let invalid = router(state)
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/v1/local/connectors/../escape")
                    .header("authorization", "Bearer secret")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert!(matches!(
            invalid.status(),
            StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND
        ));
        assert!(receive.try_recv().is_err());
    }

    #[tokio::test]
    async fn authenticated_connector_reload_dispatches_control_request() {
        let (state, mut receive) = test_state();
        let responder = tokio::spawn(async move {
            if let Some(ControlRequest::ReloadConnectors { respond_to }) = receive.recv().await {
                let _ = respond_to.send(Ok(()));
            }
        });
        let response = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/local/connectors/reload")
                    .header("authorization", "Bearer secret")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        responder.await.expect("responder");
    }

    #[tokio::test]
    async fn dashboard_session_can_disconnect_a_connector_with_explicit_action() {
        let (state, mut receive) = test_state();
        let session = "connector-browser-session".to_owned();
        state
            .browser_sessions
            .lock()
            .await
            .sessions
            .insert(session.clone(), Instant::now() + BROWSER_SESSION_LIFETIME);
        let responder = tokio::spawn(async move {
            if let Some(ControlRequest::RevokeConnector {
                connector_id,
                respond_to,
            }) = receive.recv().await
            {
                assert_eq!(connector_id, "ncon_stale");
                let _ = respond_to.send(Ok(()));
            }
        });
        let response = router(state)
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/v1/local/connectors/ncon_stale")
                    .header("cookie", format!("piqae_local_session={session}"))
                    .header("x-piqae-local-action", "disconnect")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        responder.await.expect("responder");
    }

    #[tokio::test]
    async fn dashboard_disconnect_requires_explicit_action() {
        let (state, mut receive) = test_state();
        let session = "connector-browser-session".to_owned();
        state
            .browser_sessions
            .lock()
            .await
            .sessions
            .insert(session.clone(), Instant::now() + BROWSER_SESSION_LIFETIME);
        let response = router(state)
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/v1/local/connectors/ncon_stale")
                    .header("cookie", format!("piqae_local_session={session}"))
                    .body(Body::empty())
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

    #[tokio::test]
    async fn dashboard_handoff_is_authenticated_one_time_and_url_token_is_removed() {
        let (state, _receive) = test_state();
        let created = router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/local/dashboard-sessions")
                    .header("authorization", "Bearer secret")
                    .header("host", "127.0.0.1:49100")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(created.status(), StatusCode::OK);
        let body = to_bytes(created.into_body(), 4096).await.expect("body");
        let value: serde_json::Value = serde_json::from_slice(&body).expect("json");
        let url = value["url"].as_str().expect("url");
        assert!(url.starts_with("http://127.0.0.1:49100/local/history?handoff="));
        let path = url.strip_prefix("http://127.0.0.1:49100").expect("path");

        let opened = router(state.clone())
            .oneshot(
                Request::builder()
                    .uri(path)
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(opened.status(), StatusCode::SEE_OTHER);
        assert_eq!(opened.headers()[header::LOCATION], "/local/history");
        let cookie = opened.headers()[header::SET_COOKIE]
            .to_str()
            .expect("cookie");
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Strict"));

        let replay = router(state)
            .oneshot(
                Request::builder()
                    .uri(path)
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(replay.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn dashboard_handoff_falls_back_to_the_actual_bound_address() {
        let (state, _receive) = test_state();
        *state.bound_address.write().await =
            Some("127.0.0.1:49277".parse().expect("loopback test address"));
        let created = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/local/dashboard-sessions")
                    .header("authorization", "Bearer secret")
                    .header("host", "remote.example:443")
                    .header("x-piqae-dashboard-view", "connections")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(created.status(), StatusCode::OK);
        let body = to_bytes(created.into_body(), 4096).await.expect("body");
        let value: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert!(
            value["url"]
                .as_str()
                .expect("url")
                .starts_with("http://127.0.0.1:49277/local/connections?handoff=")
        );
    }

    #[tokio::test]
    async fn legacy_queue_url_renews_cookie_scope_and_redirects_to_history() {
        let (state, _receive) = test_state();
        let session = "legacy-browser-session".to_owned();
        state
            .browser_sessions
            .lock()
            .await
            .sessions
            .insert(session.clone(), Instant::now() + BROWSER_SESSION_LIFETIME);

        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri("/local/queue")
                    .header("cookie", format!("piqae_local_session={session}"))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(response.headers()[header::LOCATION], "/local/history");
        let cookie = response.headers()[header::SET_COOKIE]
            .to_str()
            .expect("cookie");
        assert!(cookie.contains("Path=/local;"));
        assert!(cookie.contains("HttpOnly"));
    }

    #[tokio::test]
    async fn dashboard_reprint_requires_session_and_explicit_action_header() {
        let (state, mut receive) = test_state();
        let session = "browser-session".to_owned();
        state
            .browser_sessions
            .lock()
            .await
            .sessions
            .insert(session.clone(), Instant::now() + BROWSER_SESSION_LIFETIME);
        let without_action = router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/local/history/jobs/job_1/reprint")
                    .header("cookie", format!("piqae_local_session={session}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"idempotency_key":"once","confirmed":true}"#))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(without_action.status(), StatusCode::UNAUTHORIZED);
        assert!(receive.try_recv().is_err());

        let responder = tokio::spawn(async move {
            if let Some(ControlRequest::ReprintJob {
                idempotency_key,
                confirmed,
                respond_to,
                ..
            }) = receive.recv().await
            {
                assert_eq!(idempotency_key, "once");
                assert!(confirmed);
                let _ = respond_to.send(Ok(LocalJobAccepted {
                    job_id: "job_reprint".into(),
                    state: "queued_local".into(),
                }));
            }
        });
        let accepted = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/local/history/jobs/job_1/reprint")
                    .header("cookie", format!("piqae_local_session={session}"))
                    .header("x-piqae-local-action", "reprint")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"idempotency_key":"once","confirmed":true}"#))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(accepted.status(), StatusCode::ACCEPTED);
        responder.await.expect("responder");
    }
}
