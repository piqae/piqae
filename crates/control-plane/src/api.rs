#![allow(
    clippy::missing_errors_doc,
    clippy::cognitive_complexity,
    clippy::too_many_lines,
    reason = "the sync and compatibility handlers keep one auditable transactional workflow"
)]

use crate::{
    AppState,
    authentication::TenantContext,
    device_auth::{authenticate_agent, authenticate_agent_for_revocation},
    error::AppError,
    repository::{CreateResult, RepositoryError},
    request_id,
};
use axum::{
    Json,
    body::{Body, Bytes},
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{
        IntoResponse, Response, Sse,
        sse::{Event, KeepAlive},
    },
};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE_NO_PAD},
};
use chrono::{Duration, Utc};
use ed25519_dalek::{Signature, Verifier as _, VerifyingKey};
use futures::StreamExt;
use p256::pkcs8::DecodePublicKey as _;
use piqae_auth::{Environment, Scope, generate_api_key};
use piqae_domain::{
    AgentId, ContentKind, ContentSource, EnvironmentId, Job, JobEvent, JobId, JobOptions, JobState,
    PrinterId, PrinterState, WorkspaceId,
};
use piqae_object_store::{ObjectByteStream, ObjectStoreError, digest_hex};
use piqae_protocol::agent::{
    AgentAcceptJobRequest, AgentAcceptJobResponse, AgentAcceptanceAbandonResponse,
    AgentAcceptanceReconciliationResponse, AgentCommand, AgentIdentityUpdateRequest,
    AgentIdentityUpdateResponse, AgentReleaseLeaseRequest, AgentRenewLeaseRequest,
    AgentRenewLeaseResponse, AgentSyncRequest, AgentSyncResponse, ConnectSessionPreview,
    ConnectSessionPreviewRequest, ContentDescriptor, EnrolRequest, EnrolResponse, JobOffer,
    NodeDisplayIdentity,
};
use piqae_storage_postgres::{
    StoredAgent, StoredApiKey, StoredNodeConnector, StoredPrinter, StoredTargetBinding,
    StoredUpload, StoredWebhook, StoredWebhookDelivery, SyncedPrinter,
    acceptance_revocation_webhook_idempotency_key, agent_acceptance_webhook_idempotency_key,
    preaccept_cancellation_webhook_idempotency_key,
};
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    convert::Infallible,
    str::FromStr,
};

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    status: &'static str,
    version: &'static str,
    service: &'static str,
    revision: String,
}

pub async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        service: "piqae-control-plane",
        revision: release_revision(),
    })
}

/// The commit the running build was produced from.
///
/// A deploy that reports only a package version cannot be checked against the
/// revision that was reviewed: the version is unchanged between most commits,
/// so a stale container answers a health probe indistinguishably from the
/// intended one. Reporting the commit lets a post-deploy gate assert the live
/// build is the reviewed build before it trusts any other signal.
///
/// Unset or malformed values become `unknown` rather than being echoed back,
/// so a gate can never be satisfied by an arbitrary string.
fn release_revision() -> String {
    ["PIQAE_RELEASE_SHA", "RAILWAY_GIT_COMMIT_SHA"]
        .into_iter()
        .filter_map(|name| std::env::var(name).ok())
        .map(|value| value.trim().to_ascii_lowercase())
        .find(|value| value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .unwrap_or_else(|| "unknown".into())
}

pub async fn meta(State(state): State<AppState>) -> Json<crate::DeploymentCapabilities> {
    Json(state.capabilities)
}

pub async fn ready(State(state): State<AppState>) -> Result<Json<HealthResponse>, AppError> {
    state.repository.ready().await?;
    state
        .object_store
        .exists("health/readiness-probe")
        .await
        .map_err(|_| AppError::service_unavailable("object_store_unavailable"))?;
    Ok(health().await)
}

pub async fn list_api_keys(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<StoredApiKey>>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::ApiKeysRead).await?;
    Ok(Json(
        state
            .repository
            .list_api_keys(tenant.workspace_id, tenant.environment_id)
            .await?,
    ))
}

#[derive(Debug, Deserialize)]
pub struct CreateApiKeyRequest {
    name: String,
    scopes: Vec<Scope>,
    expires_at: Option<chrono::DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct CreatedApiKeyResponse {
    #[serde(flatten)]
    key: StoredApiKey,
    secret: String,
}

pub async fn create_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateApiKeyRequest>,
) -> Result<Response, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::ApiKeysWrite).await?;
    let requested = request.scopes.iter().copied().collect::<BTreeSet<_>>();
    if request.name.trim().is_empty()
        || request.name.len() > 120
        || requested.is_empty()
        || requested.len() != request.scopes.len()
        || request
            .expires_at
            .is_some_and(|expiry| expiry <= Utc::now() || expiry > Utc::now() + Duration::days(365))
    {
        return Err(AppError::invalid(
            "invalid_api_key",
            "Name, scopes, or expiry are outside the supported limits.",
        ));
    }
    if requested.iter().any(|scope| !tenant.allows(*scope)) {
        return Err(AppError::forbidden());
    }
    let kind = state
        .repository
        .environment_kind(tenant.workspace_id, tenant.environment_id)
        .await?;
    let environment = match kind.as_str() {
        "test" => Environment::Test,
        "live" => Environment::Live,
        _ => return Err(AppError::service_unavailable("invalid_environment_kind")),
    };
    let generated = generate_api_key(environment)
        .map_err(|_| AppError::service_unavailable("api_key_generation_failed"))?;
    let scopes = requested
        .iter()
        .map(|scope| scope.as_str().to_owned())
        .collect::<Vec<_>>();
    let key = state
        .repository
        .create_api_key(
            tenant.workspace_id,
            tenant.environment_id,
            &generated.id.to_string(),
            request.name.trim(),
            &generated.lookup_prefix,
            &generated.password_hash,
            &scopes,
            request.expires_at,
        )
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(CreatedApiKeyResponse {
            key,
            secret: generated.plaintext,
        }),
    )
        .into_response())
}

pub async fn revoke_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key_id): Path<String>,
) -> Result<Json<StoredApiKey>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::ApiKeysWrite).await?;
    Ok(Json(
        state
            .repository
            .revoke_api_key(tenant.workspace_id, tenant.environment_id, &key_id)
            .await?,
    ))
}

pub async fn list_agents(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<StoredAgent>>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::AgentsRead).await?;
    Ok(Json(
        state
            .repository
            .list_agents(tenant.workspace_id, tenant.environment_id)
            .await?,
    ))
}

pub async fn get_node(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(node_id): Path<String>,
) -> Result<Json<StoredAgent>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::AgentsRead).await?;
    let node_id = node_id.parse().map_err(|_| AppError::not_found())?;
    Ok(Json(
        state
            .repository
            .get_agent(tenant.workspace_id, tenant.environment_id, node_id)
            .await?,
    ))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchNodeRequest {
    name: Option<String>,
    #[serde(default)]
    site: PatchOptionalString,
    #[serde(default)]
    location: PatchOptionalString,
    labels: Option<Vec<String>>,
    expected_revision: Option<u64>,
}

#[derive(Debug, Default)]
enum PatchOptionalString {
    #[default]
    Missing,
    Clear,
    Value(String),
}

impl<'de> Deserialize<'de> for PatchOptionalString {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(Option::<String>::deserialize(deserializer)?.map_or(Self::Clear, Self::Value))
    }
}

#[cfg(test)]
mod patch_node_tests {
    use super::*;

    #[test]
    fn optional_operator_details_distinguish_missing_clear_and_value()
    -> Result<(), serde_json::Error> {
        let missing: PatchNodeRequest =
            serde_json::from_value(serde_json::json!({"name": "Node"}))?;
        let clear: PatchNodeRequest = serde_json::from_value(serde_json::json!({"site": null}))?;
        let value: PatchNodeRequest =
            serde_json::from_value(serde_json::json!({"site": " Warehouse "}))?;

        assert!(matches!(missing.site, PatchOptionalString::Missing));
        assert!(matches!(clear.site, PatchOptionalString::Clear));
        assert!(
            matches!(value.site, PatchOptionalString::Value(ref site) if site == " Warehouse ")
        );
        Ok(())
    }
}

pub async fn patch_node(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(node_id): Path<String>,
    Json(request): Json<PatchNodeRequest>,
) -> Result<Response, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::AgentsWrite).await?;
    if request.name.is_none()
        && matches!(request.site, PatchOptionalString::Missing)
        && matches!(request.location, PatchOptionalString::Missing)
        && request.labels.is_none()
    {
        return Err(AppError::invalid(
            "invalid_node",
            "At least one node detail is required for this update.",
        ));
    }
    let node_id = node_id.parse().map_err(|_| AppError::not_found())?;
    let current = state
        .repository
        .get_agent(tenant.workspace_id, tenant.environment_id, node_id)
        .await?;
    let site = match request.site {
        PatchOptionalString::Missing => current.site,
        PatchOptionalString::Clear => None,
        PatchOptionalString::Value(site) => Some(site),
    };
    let location = match request.location {
        PatchOptionalString::Missing => current.location,
        PatchOptionalString::Clear => None,
        PatchOptionalString::Value(location) => Some(location),
    };
    let display_name = request.name.unwrap_or(current.name);
    let identity = validated_node_identity(
        &display_name,
        site,
        location,
        request.labels.unwrap_or(current.labels),
    )?;
    let expected_revision = request
        .expected_revision
        .or(Some(current.identity_revision));
    let node = match state
        .repository
        .update_agent_identity(
            tenant.workspace_id,
            tenant.environment_id,
            node_id,
            expected_revision,
            &identity,
        )
        .await
    {
        Ok(node) => node,
        Err(RepositoryError::NodeIdentityRevisionConflict(revision)) => {
            return Ok(node_identity_conflict(revision));
        }
        Err(error) => return Err(error.into()),
    };
    publish_node_identity_update(&state, tenant, &node).await?;
    Ok(Json(node).into_response())
}

fn validated_node_identity(
    display_name: &str,
    site: Option<String>,
    location: Option<String>,
    labels: Vec<String>,
) -> Result<NodeDisplayIdentity, AppError> {
    let display_name = display_name.trim().to_owned();
    let site = site.map(|value| value.trim().to_owned());
    let location = location.map(|value| value.trim().to_owned());
    let labels = labels
        .into_iter()
        .map(|label| label.trim().to_owned())
        .collect::<Vec<_>>();
    let fields_are_valid = !display_name.is_empty()
        && display_name.len() <= 120
        && !display_name.chars().any(char::is_control)
        && site.as_ref().is_none_or(|value| {
            !value.is_empty() && value.len() <= 120 && !value.chars().any(char::is_control)
        })
        && location.as_ref().is_none_or(|value| {
            !value.is_empty() && value.len() <= 120 && !value.chars().any(char::is_control)
        });
    let labels_are_valid = labels.len() <= 16
        && labels.iter().collect::<BTreeSet<_>>().len() == labels.len()
        && labels.iter().all(|label| {
            !label.is_empty()
                && label.len() <= 64
                && label.trim() == label
                && !label.chars().any(char::is_control)
        });
    if !fields_are_valid || !labels_are_valid {
        return Err(AppError::invalid(
            "invalid_node_identity",
            "The node display identity is outside the supported limits.",
        ));
    }
    Ok(NodeDisplayIdentity {
        display_name,
        site,
        location,
        labels,
    })
}

fn node_identity_conflict(current_revision: u64) -> Response {
    (
        StatusCode::CONFLICT,
        Json(serde_json::json!({
            "error": {
                "code": "node_identity_revision_conflict",
                "message": "The node identity changed; reconcile before saving.",
                "request_id": request_id::current(),
                "retryable": false,
                "details": { "current_revision": current_revision }
            }
        })),
    )
        .into_response()
}

async fn publish_acceptance_revocation(
    state: &AppState,
    tenant: TenantContext,
    job: &Job,
) -> Result<(), AppError> {
    for event_type in ["job.updated", "job.delivery_uncertain"] {
        let idempotency_key = acceptance_revocation_webhook_idempotency_key(
            tenant.workspace_id,
            tenant.environment_id,
            job.id,
            event_type,
        );
        state
            .publish_idempotently(&idempotency_key, tenant, event_type, job)
            .await?;
    }
    Ok(())
}

pub async fn delete_node(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(node_id): Path<AgentId>,
) -> Result<StatusCode, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::AgentsWrite).await?;
    let affected_jobs = state
        .repository
        .revoke_agent(tenant.workspace_id, tenant.environment_id, node_id)
        .await?;
    for job in affected_jobs {
        publish_acceptance_revocation(&state, tenant, &job).await?;
    }
    state
        .publish(
            tenant,
            "node.revoked",
            &serde_json::json!({"node_id": node_id}),
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn pause_node(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(node_id): Path<AgentId>,
) -> Result<StatusCode, AppError> {
    enqueue_node_command(
        &state,
        &headers,
        node_id,
        piqae_protocol::agent::AgentCommand::Pause,
    )
    .await
}

pub async fn resume_node(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(node_id): Path<AgentId>,
) -> Result<StatusCode, AppError> {
    enqueue_node_command(
        &state,
        &headers,
        node_id,
        piqae_protocol::agent::AgentCommand::Resume,
    )
    .await
}

#[derive(Debug, Serialize)]
pub struct DiagnosticsRequestResponse {
    request_id: String,
    state: &'static str,
}

pub async fn request_node_diagnostics(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(node_id): Path<AgentId>,
) -> Result<Response, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::AgentsWrite).await?;
    state
        .repository
        .get_agent(tenant.workspace_id, tenant.environment_id, node_id)
        .await?;
    let request_id = format!("diag_{}", ulid::Ulid::new());
    state
        .repository
        .create_node_diagnostic(
            tenant.workspace_id,
            tenant.environment_id,
            node_id,
            &request_id,
        )
        .await?;
    state
        .repository
        .enqueue_agent_command(
            tenant.workspace_id,
            tenant.environment_id,
            node_id,
            &piqae_protocol::agent::AgentCommand::CollectDiagnostics {
                request_id: request_id.clone(),
            },
        )
        .await?;
    state
        .publish(
            tenant,
            "node.diagnostics.requested",
            &serde_json::json!({"node_id": node_id, "request_id": request_id}),
        )
        .await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(DiagnosticsRequestResponse {
            request_id,
            state: "requested",
        }),
    )
        .into_response())
}

pub async fn list_node_diagnostics(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(node_id): Path<AgentId>,
) -> Result<Json<Vec<piqae_storage_postgres::StoredNodeDiagnostic>>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::AgentsRead).await?;
    Ok(Json(
        state
            .repository
            .list_node_diagnostics(tenant.workspace_id, tenant.environment_id, node_id)
            .await?,
    ))
}

pub async fn get_node_diagnostic(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((node_id, request_id)): Path<(AgentId, String)>,
) -> Result<Json<piqae_storage_postgres::StoredNodeDiagnostic>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::AgentsRead).await?;
    Ok(Json(
        state
            .repository
            .get_node_diagnostic(
                tenant.workspace_id,
                tenant.environment_id,
                node_id,
                &request_id,
            )
            .await?,
    ))
}

async fn enqueue_node_command(
    state: &AppState,
    headers: &HeaderMap,
    node_id: AgentId,
    command: piqae_protocol::agent::AgentCommand,
) -> Result<StatusCode, AppError> {
    let tenant = authenticate_native(state, headers, Scope::AgentsWrite).await?;
    state
        .repository
        .get_agent(tenant.workspace_id, tenant.environment_id, node_id)
        .await?;
    state
        .repository
        .enqueue_agent_command(
            tenant.workspace_id,
            tenant.environment_id,
            node_id,
            &command,
        )
        .await?;
    Ok(StatusCode::ACCEPTED)
}

pub async fn list_printers(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> Result<Json<Page<StoredPrinter>>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::PrintersRead).await?;
    let limit = query.limit.clamp(1, 500);
    let after = query
        .after
        .as_deref()
        .map(PrinterId::from_str)
        .transpose()
        .map_err(|_| AppError::invalid("invalid_cursor", "The pagination cursor is invalid."))?;
    let mut printers = state
        .repository
        .list_printers(tenant.workspace_id, tenant.environment_id, after, limit + 1)
        .await?;
    let has_more = printers.len() > usize::try_from(limit).unwrap_or(500);
    printers.truncate(usize::try_from(limit).unwrap_or(500));
    let next_cursor = has_more
        .then(|| printers.last().map(|printer| printer.id.to_string()))
        .flatten();
    Ok(Json(Page {
        data: printers,
        next_cursor,
        has_more,
    }))
}

pub async fn get_printer(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(printer_id): Path<String>,
) -> Result<Json<StoredPrinter>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::PrintersRead).await?;
    let printer_id = PrinterId::from_str(&printer_id)
        .map_err(|_| AppError::invalid("invalid_printer_id", "The printer ID is invalid."))?;
    Ok(Json(
        state
            .repository
            .get_printer(tenant.workspace_id, tenant.environment_id, printer_id)
            .await?,
    ))
}

pub async fn list_node_connectors(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(node_id): Path<String>,
) -> Result<Json<Vec<StoredNodeConnector>>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::AgentsRead).await?;
    let node_id = AgentId::from_str(&node_id)
        .map_err(|_| AppError::invalid("invalid_node_id", "The node ID is invalid."))?;
    state
        .repository
        .get_agent(tenant.workspace_id, tenant.environment_id, node_id)
        .await?;
    Ok(Json(
        state
            .repository
            .list_node_connectors(tenant.workspace_id, tenant.environment_id, node_id)
            .await?,
    ))
}

pub async fn revoke_node_connector(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((node_id, connector_id)): Path<(String, String)>,
) -> Result<StatusCode, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::AgentsWrite).await?;
    let node_id = AgentId::from_str(&node_id)
        .map_err(|_| AppError::invalid("invalid_node_id", "The node ID is invalid."))?;
    let affected_jobs = state
        .repository
        .revoke_node_connector(
            tenant.workspace_id,
            tenant.environment_id,
            node_id,
            &connector_id,
        )
        .await?;
    for job in affected_jobs {
        publish_acceptance_revocation(&state, tenant, &job).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Lets a connector revoke its own server grant with its final signed request.
///
/// The exact agent, tenant and connector tuple must match; after commit the
/// same credential is rejected by all agent-authenticated routes.
pub async fn revoke_agent_connector(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(connector_id): Path<String>,
    body: Bytes,
) -> Result<Json<serde_json::Value>, AppError> {
    let path = format!("/v1/agent/connectors/{connector_id}/revoke");
    let identity =
        authenticate_agent_for_revocation(&state, &headers, "POST", &path, &body).await?;
    let affected_jobs = state
        .repository
        .revoke_node_connector(
            identity.tenant.workspace_id,
            identity.tenant.environment_id,
            identity.agent_id,
            &connector_id,
        )
        .await?;
    for job in affected_jobs {
        publish_acceptance_revocation(&state, identity.tenant, &job).await?;
    }
    Ok(Json(serde_json::json!({"revoked": true})))
}

#[derive(Debug, Deserialize)]
pub struct CreateEnrolmentRequest {
    name: String,
    #[serde(default = "default_enrolment_expiry")]
    expires_in_seconds: i64,
}

const fn default_enrolment_expiry() -> i64 {
    600
}

#[derive(Debug, Serialize)]
pub struct EnrolmentResponse {
    id: String,
    token: String,
    expires_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateNodeConnectSessionRequest {
    name: Option<String>,
    return_url: Option<String>,
    #[serde(default = "default_enrolment_expiry")]
    expires_in_seconds: i64,
}

#[derive(Debug, Serialize)]
pub struct NodeConnectDownload {
    platform: &'static str,
    url: &'static str,
}

#[derive(Debug, Serialize)]
pub struct NodeConnectSessionResponse {
    id: String,
    state: &'static str,
    expires_at: chrono::DateTime<Utc>,
    node_id: Option<AgentId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    connect_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    native_connect_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    return_url: Option<String>,
    downloads: Vec<NodeConnectDownload>,
}

pub async fn create_node_connect_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateNodeConnectSessionRequest>,
) -> Result<Response, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::AgentsWrite).await?;
    if request
        .name
        .as_deref()
        .is_some_and(|name| name.trim().is_empty() || name.chars().count() > 120)
        || !(60..=900).contains(&request.expires_in_seconds)
    {
        return Err(AppError::invalid(
            "invalid_connect_session",
            "Optional name or expiry is outside the supported limits.",
        ));
    }
    let return_url = request
        .return_url
        .as_deref()
        .map(validate_return_url)
        .transpose()?;
    let control_plane_url = state.public_control_plane_url.as_ref();
    let mut secret = [0_u8; 24];
    OsRng.fill_bytes(&mut secret);
    let token = format!("piq_enr_{}", URL_SAFE_NO_PAD.encode(secret));
    let secret_hash = format!("{:x}", Sha256::digest(token.as_bytes()));
    let id = format!("enr_{}", ulid::Ulid::new());
    let expires_at = Utc::now() + Duration::seconds(request.expires_in_seconds);
    let requesting_service_account_id = tenant.platform_service_account_id.map(|id| id.to_string());
    state
        .repository
        .create_connect_enrolment(
            &id,
            tenant.workspace_id,
            tenant.environment_id,
            &secret_hash,
            expires_at,
            return_url.as_deref(),
            requesting_service_account_id.as_deref(),
        )
        .await?;
    let fragment = format!(
        "enrolment_token={}&control_plane_url={}",
        percent_encode(&token),
        percent_encode(control_plane_url)
    );
    let response = NodeConnectSessionResponse {
        id: id.clone(),
        state: "pending",
        expires_at,
        node_id: None,
        connect_url: Some(format!("https://app.piqae.com/connect#{fragment}")),
        native_connect_url: Some(format!("piqae://connect#{fragment}")),
        return_url,
        downloads: connect_downloads(),
    };
    state
        .publish(
            tenant,
            "node.connect_session.created",
            &serde_json::json!({
                "id": id,
                "expires_at": expires_at
            }),
        )
        .await?;
    Ok((StatusCode::CREATED, Json(response)).into_response())
}

pub async fn get_node_connect_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> Result<Json<NodeConnectSessionResponse>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::AgentsRead).await?;
    let (expires_at, node_id) = state
        .repository
        .enrolment_status(&session_id, tenant.workspace_id, tenant.environment_id)
        .await?;
    let state_name = if node_id.is_some() {
        "connected"
    } else if expires_at <= Utc::now() {
        "expired"
    } else {
        "pending"
    };
    Ok(Json(NodeConnectSessionResponse {
        id: session_id,
        state: state_name,
        expires_at,
        node_id,
        connect_url: None,
        native_connect_url: None,
        return_url: None,
        downloads: connect_downloads(),
    }))
}

pub async fn preview_node_connect_session(
    State(state): State<AppState>,
    Json(request): Json<ConnectSessionPreviewRequest>,
) -> Result<Json<ConnectSessionPreview>, AppError> {
    if request.token.len() > 128 || !request.token.starts_with("piq_enr_") {
        return Err(AppError::unauthorized());
    }
    let secret_hash = format!("{:x}", Sha256::digest(request.token.as_bytes()));
    let preview = state
        .repository
        .connect_session_preview(&secret_hash)
        .await
        .map_err(|_| AppError::unauthorized())?;
    let authorization_type = if preview.requesting_service_account_id.is_some() {
        "platform_customer"
    } else {
        "workspace"
    };
    Ok(Json(ConnectSessionPreview {
        workspace_id: preview.workspace_id.to_string(),
        workspace_name: preview.workspace_name,
        requesting_service_account_id: preview.requesting_service_account_id,
        requesting_service_name: preview.requesting_service_name,
        authorization_type: authorization_type.into(),
        environment_id: preview.environment_id.to_string(),
        requested_scopes: vec![
            "discover_printers".into(),
            "print".into(),
            "monitor_jobs".into(),
        ],
        printer_grant: "all_or_selected".into(),
        expires_at: preview.expires_at,
        return_url: preview.return_url,
    }))
}

fn validate_return_url(value: &str) -> Result<String, AppError> {
    let parsed = url::Url::parse(value).map_err(|_| {
        AppError::invalid(
            "invalid_return_url",
            "Return URL must be an absolute HTTPS URL.",
        )
    })?;
    let local_http = parsed.scheme() == "http"
        && matches!(parsed.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if parsed.scheme() != "https" && !local_http {
        return Err(AppError::invalid(
            "invalid_return_url",
            "Return URL must use HTTPS (localhost HTTP is allowed for development).",
        ));
    }
    if parsed.username() != "" || parsed.password().is_some() || parsed.fragment().is_some() {
        return Err(AppError::invalid(
            "invalid_return_url",
            "Return URL cannot contain credentials or a fragment.",
        ));
    }
    Ok(parsed.to_string())
}

pub fn validated_control_plane_url(value: &str) -> anyhow::Result<String> {
    let mut parsed = url::Url::parse(value)
        .map_err(|_| anyhow::anyhow!("control-plane URL must be absolute"))?;
    let local_http = parsed.scheme() == "http"
        && matches!(parsed.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if parsed.scheme() != "https" && !local_http {
        anyhow::bail!(
            "control-plane URL must use HTTPS (localhost HTTP is allowed for development)"
        );
    }
    if parsed.username() != ""
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        anyhow::bail!("control-plane URL cannot contain credentials, a query, or a fragment");
    }
    let normalized_path = parsed.path().trim_end_matches('/').to_owned();
    parsed.set_path(&normalized_path);
    Ok(parsed.to_string().trim_end_matches('/').to_owned())
}

fn percent_encode(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

fn connect_downloads() -> Vec<NodeConnectDownload> {
    vec![
        NodeConnectDownload {
            platform: "macos",
            url: "https://app.piqae.com/downloads?platform=macos",
        },
        NodeConnectDownload {
            platform: "windows",
            url: "https://app.piqae.com/downloads?platform=windows",
        },
        NodeConnectDownload {
            platform: "linux",
            url: "https://app.piqae.com/downloads?platform=linux",
        },
    ]
}

#[cfg(test)]
mod connect_session_tests {
    use super::{validate_return_url, validated_control_plane_url, verify_connector_proof};
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use ed25519_dalek::{Signer as _, SigningKey};

    #[test]
    fn return_urls_are_fail_closed() {
        assert!(validate_return_url("https://partner.example/printing/complete?job=42").is_ok());
        assert!(validate_return_url("http://localhost:5173/complete").is_ok());
        assert!(validate_return_url("http://partner.example/complete").is_err());
        assert!(validate_return_url("https://user:secret@partner.example/complete").is_err());
        assert!(validate_return_url("https://partner.example/complete#token").is_err());
        assert!(validate_return_url("/relative").is_err());
    }

    #[test]
    fn control_plane_origins_are_explicit_and_fail_closed() {
        assert!(matches!(
            validated_control_plane_url("https://print.example/api/"),
            Ok(value) if value == "https://print.example/api"
        ));
        assert!(validated_control_plane_url("http://127.0.0.1:8080").is_ok());
        assert!(validated_control_plane_url("http://print.example").is_err());
        assert!(validated_control_plane_url("https://user:secret@print.example").is_err());
        assert!(validated_control_plane_url("https://print.example?tenant=other").is_err());
    }

    #[test]
    fn installation_proof_rejects_every_tamper() {
        let key = SigningKey::from_bytes(&[7_u8; 32]);
        let message = b"piqae-connect-v1\ninvitation\ninstallation\nconnector\nprinter";
        let proof = URL_SAFE_NO_PAD.encode(key.sign(message).to_bytes());
        assert!(verify_connector_proof(key.verifying_key().as_bytes(), &proof, message).is_ok());
        assert!(
            verify_connector_proof(
                key.verifying_key().as_bytes(),
                &proof,
                b"piqae-connect-v1\ntampered"
            )
            .is_err()
        );
        let attacker = SigningKey::from_bytes(&[8_u8; 32]);
        let attacker_proof = URL_SAFE_NO_PAD.encode(attacker.sign(message).to_bytes());
        assert!(
            verify_connector_proof(key.verifying_key().as_bytes(), &attacker_proof, message)
                .is_err()
        );
    }
}

pub async fn create_agent_enrolment(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateEnrolmentRequest>,
) -> Result<Response, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::AgentsWrite).await?;
    if request.name.trim().is_empty()
        || request.name.len() > 120
        || !(60..=3_600).contains(&request.expires_in_seconds)
    {
        return Err(AppError::invalid(
            "invalid_enrolment",
            "Name and expiry are outside the supported limits.",
        ));
    }
    let mut secret = [0_u8; 24];
    OsRng.fill_bytes(&mut secret);
    let token = format!("piq_enr_{}", URL_SAFE_NO_PAD.encode(secret));
    let secret_hash = format!("{:x}", Sha256::digest(token.as_bytes()));
    let id = format!("enr_{}", ulid::Ulid::new());
    let expires_at = Utc::now() + Duration::seconds(request.expires_in_seconds);
    state
        .repository
        .create_enrolment(
            &id,
            tenant.workspace_id,
            tenant.environment_id,
            &secret_hash,
            expires_at,
        )
        .await?;
    state
        .publish(
            tenant,
            "agent_enrolment.created",
            &serde_json::json!({"id": id, "name": request.name, "expires_at": expires_at}),
        )
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(EnrolmentResponse {
            id,
            token,
            expires_at,
        }),
    )
        .into_response())
}

pub async fn enrol_agent(
    State(state): State<AppState>,
    Json(request): Json<EnrolRequest>,
) -> Result<Response, AppError> {
    if request.protocol_version != 1 {
        return Err(AppError::invalid(
            "unsupported_agent_protocol",
            "The agent protocol version is not supported.",
        ));
    }
    let public_key = decode_agent_public_key(&request.public_key)?;
    if public_key.len() != 32 {
        return Err(AppError::invalid(
            "invalid_agent_public_key",
            "Public key must contain exactly 32 bytes.",
        ));
    }
    let secret_hash = format!("{:x}", Sha256::digest(request.token.as_bytes()));
    validate_connector_printer_grant(&request)?;
    let installation_public_key = request
        .installation_public_key
        .as_deref()
        .map(decode_agent_public_key)
        .transpose()?;
    if installation_public_key
        .as_ref()
        .is_some_and(|key| key.len() != 32)
    {
        return Err(AppError::invalid(
            "invalid_installation_public_key",
            "Installation public key must contain exactly 32 bytes.",
        ));
    }
    let verified_installation_key =
        if let Some(installation_id) = request.installation_id.as_deref() {
            let existing_key = match state
                .repository
                .node_installation_public_key(installation_id)
                .await
            {
                Ok(key) => key,
                Err(RepositoryError::NotFound) => installation_public_key
                    .clone()
                    .ok_or_else(AppError::unauthorized)?,
                Err(_) => return Err(AppError::unauthorized()),
            };
            if installation_public_key
                .as_ref()
                .is_some_and(|candidate| candidate != &existing_key)
            {
                return Err(AppError::unauthorized());
            }
            let proof = request
                .installation_proof
                .as_deref()
                .ok_or_else(AppError::unauthorized)?;
            let message = connector_proof_for_request(&request, installation_id);
            verify_connector_proof(&existing_key, proof, &message)?;
            Some(existing_key)
        } else {
            None
        };
    let enrolled = if let Some(installation_id) = request.installation_id.as_deref() {
        state
            .repository
            .enrol_agent_connector_with_billing(
                &secret_hash,
                &public_key,
                &request.name,
                &request.hostname,
                &request.platform,
                &request.architecture,
                &request.agent_version,
                request.protocol_version,
                state.capabilities.billing.enabled,
                installation_id,
                verified_installation_key
                    .as_deref()
                    .ok_or_else(AppError::unauthorized)?,
                match request.printer_grant {
                    piqae_protocol::agent::PrinterGrant::SelectedPrinters => "selected_printers",
                    piqae_protocol::agent::PrinterGrant::AllLocalPrinters => "all_local_printers",
                },
                &request.allowed_printer_ids,
            )
            .await?
    } else {
        state
            .repository
            .enrol_agent_with_billing(
                &secret_hash,
                &public_key,
                &request.name,
                &request.hostname,
                &request.platform,
                &request.architecture,
                &request.agent_version,
                request.protocol_version,
                state.capabilities.billing.enabled,
            )
            .await?
    };
    Ok((
        StatusCode::CREATED,
        Json(EnrolResponse {
            agent_id: enrolled.agent_id,
            environment: enrolled.environment_id.to_string(),
            server_time: Utc::now(),
            sync_after_ms: 0,
            connector_id: enrolled.connector_id,
        }),
    )
        .into_response())
}

fn decode_agent_public_key(encoded: &str) -> Result<Vec<u8>, AppError> {
    URL_SAFE_NO_PAD
        .decode(encoded)
        .or_else(|_| STANDARD_NO_PAD.decode(encoded))
        .or_else(|_| STANDARD.decode(encoded))
        .map_err(|_| AppError::invalid("invalid_agent_public_key", "Public key is invalid."))
}

#[cfg(test)]
mod enrol_public_key_tests {
    use super::decode_agent_public_key;
    use base64::{
        Engine as _,
        engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE_NO_PAD},
    };

    #[test]
    fn accepts_canonical_and_legacy_ed25519_public_key_encodings() {
        let key = [0xfb_u8; 32];
        for encoded in [
            URL_SAFE_NO_PAD.encode(key),
            STANDARD_NO_PAD.encode(key),
            STANDARD.encode(key),
        ] {
            assert!(matches!(
                decode_agent_public_key(&encoded),
                Ok(decoded) if decoded == key
            ));
        }
        assert!(decode_agent_public_key("not a public key").is_err());
    }
}

fn validate_connector_printer_grant(request: &EnrolRequest) -> Result<(), AppError> {
    if request.installation_id.is_none() {
        return Ok(());
    }
    match request.printer_grant {
        piqae_protocol::agent::PrinterGrant::SelectedPrinters
            if request.allowed_printer_ids.is_empty() =>
        {
            Err(AppError::invalid(
                "printer_consent_required",
                "At least one locally approved printer is required for a connector.",
            ))
        }
        piqae_protocol::agent::PrinterGrant::AllLocalPrinters
            if !request.allowed_printer_ids.is_empty() =>
        {
            Err(AppError::invalid(
                "invalid_printer_grant",
                "All-printer access must not include selected printer identifiers.",
            ))
        }
        _ => Ok(()),
    }
}

fn connector_proof_for_request(request: &EnrolRequest, installation_id: &str) -> Vec<u8> {
    match request.printer_grant {
        piqae_protocol::agent::PrinterGrant::SelectedPrinters => {
            piqae_protocol::agent::connector_proof_message(
                &request.token,
                installation_id,
                &request.public_key,
                &request.allowed_printer_ids,
            )
        }
        piqae_protocol::agent::PrinterGrant::AllLocalPrinters => {
            piqae_protocol::agent::connector_grant_proof_message(
                &request.token,
                installation_id,
                &request.public_key,
                request.printer_grant,
                &request.allowed_printer_ids,
            )
        }
    }
}

fn verify_connector_proof(
    public_key: &[u8],
    encoded_signature: &str,
    message: &[u8],
) -> Result<(), AppError> {
    let verifying_key = VerifyingKey::from_bytes(
        &public_key
            .try_into()
            .map_err(|_| AppError::unauthorized())?,
    )
    .map_err(|_| AppError::unauthorized())?;
    let proof = URL_SAFE_NO_PAD
        .decode(encoded_signature)
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(encoded_signature))
        .map_err(|_| AppError::unauthorized())?;
    let signature = Signature::from_slice(&proof).map_err(|_| AppError::unauthorized())?;
    verifying_key
        .verify(message, &signature)
        .map_err(|_| AppError::unauthorized())
}

pub async fn list_webhooks(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<StoredWebhook>>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::WebhooksRead).await?;
    Ok(Json(
        state
            .repository
            .list_webhooks(tenant.workspace_id, tenant.environment_id)
            .await?,
    ))
}

#[derive(Debug, Deserialize)]
pub struct CreateWebhookRequest {
    url: String,
    events: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct CreatedWebhookResponse {
    #[serde(flatten)]
    webhook: StoredWebhook,
    secret: String,
}

pub async fn create_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateWebhookRequest>,
) -> Result<Response, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::WebhooksWrite).await?;
    validate_webhook_url(&request.url)?;
    if request.events.is_empty()
        || request.events.len() > 50
        || request.events.iter().any(|event| event.trim().is_empty())
    {
        return Err(AppError::invalid(
            "invalid_webhook_events",
            "At least one valid webhook event is required.",
        ));
    }
    let mut secret_bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut secret_bytes);
    let secret = format!("whsec_{}", URL_SAFE_NO_PAD.encode(secret_bytes));
    let ciphertext = state
        .webhook_secrets
        .encrypt(secret.as_bytes())
        .map_err(|_| AppError::service_unavailable("webhook_secret_encryption_failed"))?;
    let id = format!("whk_{}", ulid::Ulid::new());
    let webhook = state
        .repository
        .create_webhook(
            &id,
            tenant.workspace_id,
            tenant.environment_id,
            &request.url,
            &request.events,
            &ciphertext,
        )
        .await?;
    state.publish(tenant, "webhook.created", &webhook).await?;
    Ok((
        StatusCode::CREATED,
        Json(CreatedWebhookResponse { webhook, secret }),
    )
        .into_response())
}

pub async fn delete_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(webhook_id): Path<String>,
) -> Result<StatusCode, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::WebhooksWrite).await?;
    state
        .repository
        .delete_webhook(tenant.workspace_id, tenant.environment_id, &webhook_id)
        .await?;
    state
        .publish(
            tenant,
            "webhook.deleted",
            &serde_json::json!({"id": webhook_id}),
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_webhook_deliveries(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(webhook_id): Path<String>,
) -> Result<Json<Vec<StoredWebhookDelivery>>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::WebhooksRead).await?;
    Ok(Json(
        state
            .repository
            .list_webhook_deliveries(tenant.workspace_id, tenant.environment_id, &webhook_id)
            .await?,
    ))
}

pub async fn replay_webhook_delivery(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(delivery_id): Path<String>,
) -> Result<StatusCode, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::WebhooksWrite).await?;
    state
        .repository
        .replay_webhook_delivery(tenant.workspace_id, tenant.environment_id, &delivery_id)
        .await?;
    Ok(StatusCode::ACCEPTED)
}

fn validate_webhook_url(value: &str) -> Result<(), AppError> {
    let url = url::Url::parse(value)
        .map_err(|_| AppError::invalid("invalid_webhook_url", "Webhook URL is invalid."))?;
    if !matches!(url.scheme(), "http" | "https") || url.username() != "" || url.password().is_some()
    {
        return Err(AppError::invalid(
            "invalid_webhook_url",
            "Webhook URL must be HTTP(S) and cannot contain credentials.",
        ));
    }
    let host = url.host_str().unwrap_or_default();
    if host.eq_ignore_ascii_case("localhost")
        || host.ends_with(".localhost")
        || host.parse::<std::net::IpAddr>().is_ok_and(|address| {
            address.is_loopback()
                || address.is_unspecified()
                || match address {
                    std::net::IpAddr::V4(address) => {
                        address.is_private() || address.is_link_local()
                    }
                    std::net::IpAddr::V6(address) => {
                        address.is_unique_local() || address.is_unicast_link_local()
                    }
                }
        })
    {
        return Err(AppError::invalid(
            "webhook_target_blocked",
            "Webhook target is not permitted by the network policy.",
        ));
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct CreateUploadRequest {
    media_type: String,
    byte_length: i64,
    sha256: String,
}

#[derive(Debug, Serialize)]
pub struct UploadResponse {
    #[serde(flatten)]
    upload: StoredUpload,
    upload_url: String,
    upload_method: &'static str,
    upload_headers: BTreeMap<String, String>,
    requires_completion: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegisterContentEncryptionKeyRequest {
    key_id: String,
    algorithm: String,
    public_key_spki: String,
}

#[derive(Debug, Serialize)]
pub struct ContentEncryptionKeyResponse {
    key_id: String,
    algorithm: String,
    public_key_spki: String,
    node_id: AgentId,
    lifecycle_state: String,
    state_changed_at: chrono::DateTime<Utc>,
    created_at: chrono::DateTime<Utc>,
}

pub async fn update_agent_identity(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, AppError> {
    let path = "/v1/agent/identity";
    let authenticated = authenticate_agent(&state, &headers, "PUT", path, &body).await?;
    let request: AgentIdentityUpdateRequest = serde_json::from_slice(&body)?;
    let identity = validated_node_identity(
        &request.display_name,
        request.site,
        request.location,
        request.labels,
    )?;
    let node = match state
        .repository
        .update_agent_identity(
            authenticated.tenant.workspace_id,
            authenticated.tenant.environment_id,
            authenticated.agent_id,
            Some(request.expected_revision),
            &identity,
        )
        .await
    {
        Ok(node) => node,
        Err(RepositoryError::NodeIdentityRevisionConflict(revision)) => {
            return Ok(node_identity_conflict(revision));
        }
        Err(error) => return Err(error.into()),
    };
    publish_node_identity_update(&state, authenticated.tenant, &node).await?;
    Ok(Json(AgentIdentityUpdateResponse {
        revision: node.identity_revision,
        identity: NodeDisplayIdentity {
            display_name: node.name,
            site: node.site,
            location: node.location,
            labels: node.labels,
        },
    })
    .into_response())
}

async fn publish_node_identity_update(
    state: &AppState,
    tenant: TenantContext,
    node: &StoredAgent,
) -> Result<(), RepositoryError> {
    let key = format!(
        "node-identity:{}:{}:{}:{}",
        tenant.workspace_id, tenant.environment_id, node.id, node.identity_revision
    );
    state
        .publish_idempotently(&key, tenant, "node.updated", node)
        .await
}

pub async fn register_agent_content_encryption_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<ContentEncryptionKeyResponse>, AppError> {
    let identity = authenticate_agent(
        &state,
        &headers,
        "PUT",
        "/v1/agent/content-encryption-key",
        &body,
    )
    .await?;
    let request: RegisterContentEncryptionKeyRequest = serde_json::from_slice(&body)?;
    if request.algorithm != "ECDH-P256-HKDF-SHA256"
        || !(8..=255).contains(&request.key_id.len())
        || !valid_content_encryption_public_key(&request.public_key_spki)
        || !request
            .public_key_spki
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(AppError::invalid(
            "invalid_content_encryption_key",
            "The dedicated encryption key is invalid.",
        ));
    }
    let key = state
        .repository
        .rotate_content_encryption_key(
            identity.tenant.workspace_id,
            identity.tenant.environment_id,
            identity.agent_id,
            &request.key_id,
            &request.algorithm,
            &request.public_key_spki,
        )
        .await?;
    Ok(Json(ContentEncryptionKeyResponse {
        key_id: key.key_id,
        algorithm: key.algorithm,
        public_key_spki: key.public_key_spki,
        node_id: key.agent_id,
        lifecycle_state: key.lifecycle_state,
        state_changed_at: key.state_changed_at,
        created_at: key.created_at,
    }))
}

fn valid_content_encryption_public_key(encoded: &str) -> bool {
    URL_SAFE_NO_PAD
        .decode(encoded)
        .is_ok_and(|der| p256::PublicKey::from_public_key_der(&der).is_ok())
}

pub async fn revoke_agent_content_encryption_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key_id): Path<String>,
    body: Bytes,
) -> Result<StatusCode, AppError> {
    let path = format!("/v1/agent/content-encryption-key/{key_id}");
    let identity = authenticate_agent(&state, &headers, "DELETE", &path, &body).await?;
    state
        .repository
        .revoke_content_encryption_key(
            identity.tenant.workspace_id,
            identity.tenant.environment_id,
            identity.agent_id,
            &key_id,
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn printer_content_encryption_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(printer_id): Path<String>,
) -> Result<Json<ContentEncryptionKeyResponse>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::PrintersRead).await?;
    let printer_id = PrinterId::from_str(&printer_id)
        .map_err(|_| AppError::invalid("invalid_printer_id", "The printer ID is invalid."))?;
    let key = state
        .repository
        .content_encryption_key_for_printer(tenant.workspace_id, tenant.environment_id, printer_id)
        .await?;
    Ok(Json(ContentEncryptionKeyResponse {
        key_id: key.key_id,
        algorithm: key.algorithm,
        public_key_spki: key.public_key_spki,
        node_id: key.agent_id,
        lifecycle_state: key.lifecycle_state,
        state_changed_at: key.state_changed_at,
        created_at: key.created_at,
    }))
}

const MAX_UPLOAD_BYTES: i64 = 50 * 1024 * 1024;

pub async fn create_upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateUploadRequest>,
) -> Result<Response, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::JobsWrite).await?;
    if !matches!(
        request.media_type.as_str(),
        "application/pdf" | "application/octet-stream"
    ) || !(1..=MAX_UPLOAD_BYTES).contains(&request.byte_length)
        || request.sha256.len() != 64
        || !request.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(AppError::invalid(
            "invalid_upload",
            "Upload metadata is outside the supported limits.",
        ));
    }
    let id = format!("upl_{}", ulid::Ulid::new());
    let upload = StoredUpload {
        id: id.clone(),
        object_key: format!("{}/{}/{}", tenant.workspace_id, tenant.environment_id, id),
        media_type: request.media_type,
        expected_sha256: request.sha256.to_ascii_lowercase(),
        expected_bytes: request.byte_length,
        state: "pending".into(),
        expires_at: Utc::now() + Duration::hours(1),
    };
    state
        .repository
        .create_upload(&upload, tenant.workspace_id, tenant.environment_id)
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(UploadResponse {
            upload_url: format!("/v1/uploads/{id}/content"),
            upload_method: "PUT",
            upload_headers: BTreeMap::from([("content-type".into(), upload.media_type.clone())]),
            requires_completion: false,
            upload,
        }),
    )
        .into_response())
}

pub async fn upload_content(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(upload_id): Path<String>,
    body: Body,
) -> Result<Json<StoredUpload>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::JobsWrite).await?;
    let upload = state
        .repository
        .get_upload(tenant.workspace_id, tenant.environment_id, &upload_id)
        .await?;
    if upload.state != "pending" || upload.expires_at <= Utc::now() {
        return Err(AppError::invalid(
            "upload_not_writable",
            "Upload is expired or already complete.",
        ));
    }
    if headers
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<i64>().ok())
        .is_some_and(|length| length != upload.expected_bytes)
    {
        return Err(AppError::invalid(
            "upload_length_mismatch",
            "Content-Length does not match the declared upload byte length.",
        ));
    }
    let stream: ObjectByteStream = Box::pin(
        body.into_data_stream()
            .map(|result| result.map_err(|error| ObjectStoreError::Stream(error.to_string()))),
    );
    state
        .object_store
        .put_stream(
            &upload.object_key,
            stream,
            &upload.expected_sha256,
            u64::try_from(upload.expected_bytes)
                .map_err(|_| AppError::invalid("invalid_upload", "Upload length is invalid."))?,
        )
        .await
        .map_err(|error| match error {
            ObjectStoreError::DigestMismatch => {
                AppError::invalid("upload_digest_mismatch", "Upload digest does not match.")
            }
            ObjectStoreError::LengthMismatch => AppError::invalid(
                "upload_length_mismatch",
                "Upload byte length does not match.",
            ),
            _ => AppError::service_unavailable("object_store_unavailable"),
        })?;
    Ok(Json(
        state
            .repository
            .complete_upload(
                tenant.workspace_id,
                tenant.environment_id,
                &upload_id,
                &upload.expected_sha256,
                upload.expected_bytes,
            )
            .await?,
    ))
}

#[derive(Debug, Deserialize)]
pub struct CompleteUploadRequest {
    sha256: String,
    byte_length: i64,
}

pub async fn complete_upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(upload_id): Path<String>,
    Json(request): Json<CompleteUploadRequest>,
) -> Result<Json<StoredUpload>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::JobsWrite).await?;
    let upload = state
        .repository
        .get_upload(tenant.workspace_id, tenant.environment_id, &upload_id)
        .await?;
    if upload.state != "pending"
        || upload.expires_at <= Utc::now()
        || !request.sha256.eq_ignore_ascii_case(&upload.expected_sha256)
        || request.byte_length != upload.expected_bytes
    {
        return Err(AppError::invalid(
            "upload_not_completable",
            "Upload is expired, complete, or does not match its declared metadata.",
        ));
    }
    let verified = state
        .object_store
        .verify(
            &upload.object_key,
            &upload.expected_sha256,
            u64::try_from(upload.expected_bytes)
                .map_err(|_| AppError::invalid("invalid_upload", "Upload length is invalid."))?,
        )
        .await
        .map_err(|error| match error {
            ObjectStoreError::DigestMismatch | ObjectStoreError::LengthMismatch => {
                AppError::invalid(
                    "upload_verification_failed",
                    "Stored object does not match completion metadata.",
                )
            }
            _ => AppError::service_unavailable("object_store_unavailable"),
        })?;
    Ok(Json(
        state
            .repository
            .complete_upload(
                tenant.workspace_id,
                tenant.environment_id,
                &upload_id,
                &verified.sha256,
                i64::try_from(verified.bytes).map_err(|_| {
                    AppError::invalid("invalid_upload", "Upload length is invalid.")
                })?,
            )
            .await?,
    ))
}

pub async fn get_upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(upload_id): Path<String>,
) -> Result<Json<StoredUpload>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::JobsWrite).await?;
    Ok(Json(
        state
            .repository
            .get_upload(tenant.workspace_id, tenant.environment_id, &upload_id)
            .await?,
    ))
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CreateJobRequest {
    pub printer_id: Option<String>,
    pub target_id: Option<String>,
    pub title: String,
    pub source: Option<String>,
    pub content_type: ContentKind,
    /// Required language/output binding for printer-native bytes. Plain
    /// `raw` is not a language and is never routed without this descriptor.
    pub printer_native: Option<piqae_protocol::agent::PrinterNativeJobDescriptor>,
    pub content: ContentSource,
    #[serde(default)]
    pub options: JobOptions,
    #[serde(default = "default_deliveries")]
    pub deliveries: u16,
    #[serde(default = "default_expiry")]
    pub expire_after_seconds: i64,
    #[serde(default)]
    pub metadata: std::collections::BTreeMap<String, String>,
    pub resolved_ticket_digest: Option<String>,
}

const fn default_deliveries() -> u16 {
    1
}

const fn default_expiry() -> i64 {
    1_209_600
}

#[derive(Clone, Debug, Serialize)]
pub struct JobResponse {
    pub id: JobId,
    pub printer_id: PrinterId,
    pub title: String,
    pub source: Option<String>,
    pub content_type: ContentKind,
    pub metadata: std::collections::BTreeMap<String, String>,
    pub deliveries: u16,
    pub state: JobState,
    pub created_at: chrono::DateTime<Utc>,
    pub expires_at: chrono::DateTime<Utc>,
    /// Present only while delivery is unproven. Lets a caller show how long a
    /// handoff has gone unconfirmed rather than only that it has.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery_uncertain_since: Option<chrono::DateTime<Utc>>,
}

impl From<Job> for JobResponse {
    fn from(job: Job) -> Self {
        Self {
            id: job.id,
            printer_id: job.printer_id,
            title: job.title,
            source: job.source,
            content_type: job.content_kind,
            metadata: job.metadata,
            deliveries: job.deliveries,
            state: job.state,
            created_at: job.created_at,
            expires_at: job.expires_at,
            delivery_uncertain_since: job.delivery_uncertain_since,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    #[serde(default = "default_limit")]
    limit: i64,
    after: Option<String>,
    state: Option<JobState>,
    printer_id: Option<String>,
    target_id: Option<String>,
    metadata_key: Option<String>,
    metadata_value: Option<String>,
}

const fn default_limit() -> i64 {
    100
}

#[derive(Debug, Serialize)]
pub struct Page<T> {
    data: Vec<T>,
    next_cursor: Option<String>,
    has_more: bool,
}

pub async fn create_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateJobRequest>,
) -> Result<Response, AppError> {
    create_job_impl(state, headers, request, false).await
}

/// Creates a job from a trusted control-plane workflow that may attach
/// reserved `piqae.*` metadata. This is deliberately not an HTTP handler;
/// public callers always pass through [`create_job`] and cannot set these
/// routing keys.
pub(crate) async fn create_job_internal(
    state: AppState,
    headers: HeaderMap,
    request: CreateJobRequest,
) -> Result<Response, AppError> {
    create_job_impl(state, headers, request, true).await
}

async fn create_job_impl(
    state: AppState,
    headers: HeaderMap,
    request: CreateJobRequest,
    allow_reserved_metadata: bool,
) -> Result<Response, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::JobsWrite).await?;
    validate_create(&request, allow_reserved_metadata)?;
    let destination = resolve_job_destination(&state, tenant, &request).await?;
    let printer_native_metadata =
        validate_printer_native_job(&state, tenant, &request, &destination).await?;
    let resolved_ticket = validate_resolved_ticket(&state, tenant, &request, &destination).await?;
    validate_encrypted_job(&state, tenant, &request, &destination).await?;
    let request_bytes = serde_json::to_vec(&request)?;
    let now = Utc::now();
    let persisted =
        persist_job_content(&state, tenant, request.content_type, request.content).await?;
    let mut metadata = request.metadata;
    metadata.extend(destination.metadata);
    metadata.extend(printer_native_metadata);
    if let Some(ticket) = resolved_ticket {
        metadata.insert(
            "piqae.capability_revision".into(),
            ticket.capability_revision.to_string(),
        );
        metadata.insert("piqae.resolved_ticket_digest".into(), ticket.digest);
    }
    let job_expires_at = match &persisted.source {
        ContentSource::EncryptedUpload { manifest, .. } => {
            chrono::DateTime::parse_from_rfc3339(&manifest.binding.expires_at)
                .map(|value| value.with_timezone(&Utc))
                .map_err(|_| {
                    AppError::invalid(
                        "invalid_encrypted_job_binding",
                        "Encrypted expiry is invalid.",
                    )
                })?
        }
        _ => now + Duration::seconds(request.expire_after_seconds),
    };
    let job = Job {
        id: JobId::new(),
        workspace_id: tenant.workspace_id,
        environment_id: tenant.environment_id,
        printer_id: destination.printer_id,
        title: request.title,
        source: request.source,
        content_kind: request.content_type,
        content: persisted.source,
        options: request.options,
        metadata,
        deliveries: request.deliveries,
        state: JobState::Registered,
        created_at: now,
        expires_at: job_expires_at,
        // A new job is never uncertain; the transition stamps the anchor.
        delivery_uncertain_since: None,
    };
    let idempotency = headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok());
    if idempotency.is_some_and(|key| !(8..=255).contains(&key.len())) {
        return Err(AppError::invalid(
            "invalid_idempotency_key",
            "Idempotency-Key must be between 8 and 255 bytes.",
        ));
    }
    let created = state
        .repository
        .create_cloud_job(
            &job,
            destination.agent_id,
            idempotency,
            &request_bytes,
            state.capabilities.billing.enabled,
        )
        .await;
    if created.is_err() || matches!(&created, Ok(CreateResult::Existing(_))) {
        cleanup_owned_upload(&state, tenant, persisted.owned_upload.as_ref()).await;
    }
    match created? {
        CreateResult::Existing(existing) => {
            if let Err(error) = state
                .repository
                .ensure_waiting_job_wake_hints(
                    tenant.workspace_id,
                    tenant.environment_id,
                    existing.id,
                )
                .await
            {
                tracing::warn!(
                    error.type = "wake_hint_reconciliation",
                    job_id = %existing.id,
                    %error,
                    "durable job wake hint will be repaired by the worker"
                );
            }
            Ok((StatusCode::OK, Json(JobResponse::from(existing))).into_response())
        }
        CreateResult::Created(created) => {
            let queued = state
                .repository
                .transition_job(
                    tenant.workspace_id,
                    tenant.environment_id,
                    created.id,
                    JobState::WaitingForAgent,
                    None,
                    Some("Waiting for the target agent".into()),
                    None,
                    None,
                )
                .await?;
            if let Err(error) = state
                .repository
                .ensure_waiting_job_wake_hints(
                    tenant.workspace_id,
                    tenant.environment_id,
                    queued.id,
                )
                .await
            {
                tracing::warn!(
                    error.type = "wake_hint_reconciliation",
                    job_id = %queued.id,
                    %error,
                    "durable job wake hint will be repaired by the worker"
                );
            }
            state.publish(tenant, "job.updated", &queued).await?;
            Ok((StatusCode::CREATED, Json(JobResponse::from(queued))).into_response())
        }
    }
}

struct ResolvedJobDestination {
    printer_id: PrinterId,
    agent_id: AgentId,
    metadata: BTreeMap<String, String>,
    binding: Option<StoredTargetBinding>,
}

async fn resolve_job_destination(
    state: &AppState,
    tenant: TenantContext,
    request: &CreateJobRequest,
) -> Result<ResolvedJobDestination, AppError> {
    match (
        request
            .printer_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty()),
        request
            .target_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty()),
    ) {
        (Some(printer_id), None) => {
            let printer_id = PrinterId::from_str(printer_id).map_err(|_| {
                AppError::invalid("invalid_printer_id", "The printer ID is invalid.")
            })?;
            let agent_id = state
                .repository
                .resolve_printer_agent(tenant.workspace_id, tenant.environment_id, printer_id)
                .await?;
            let topology_route = state
                .destination_topology
                .list_all_routes(crate::destination_topology::tenant_scope(tenant))
                .await
                .map_err(crate::destination_topology::map_storage_error)?
                .into_iter()
                .find(|route| {
                    route.enabled
                        && route.printer_id == printer_id.to_string()
                        && route.agent_id == agent_id.to_string()
                });
            let mut metadata =
                BTreeMap::from([("piqae.route_agent_id".into(), agent_id.to_string())]);
            if let Some(route) = topology_route {
                metadata.insert("piqae.destination_id".into(), route.destination_id);
                metadata.insert("piqae.route_id".into(), route.id);
            }
            Ok(ResolvedJobDestination {
                printer_id,
                agent_id,
                metadata,
                binding: None,
            })
        }
        (None, Some(target_id)) => resolve_target_destination(state, tenant, target_id, true).await,
        _ => Err(AppError::invalid(
            "invalid_destination",
            "Provide exactly one printer_id or target_id.",
        )),
    }
}

async fn validate_encrypted_job(
    state: &AppState,
    tenant: TenantContext,
    request: &CreateJobRequest,
    destination: &ResolvedJobDestination,
) -> Result<(), AppError> {
    let ContentSource::EncryptedUpload { manifest, .. } = &request.content else {
        return Ok(());
    };
    let binding_expiry = chrono::DateTime::parse_from_rfc3339(&manifest.binding.expires_at)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|_| {
            AppError::invalid(
                "invalid_encrypted_job_binding",
                "Encrypted expiry is invalid.",
            )
        })?;
    let digest_valid = URL_SAFE_NO_PAD
        .decode(&manifest.ciphertext_sha256)
        .is_ok_and(|value| value.len() == 32);
    let iv_valid = URL_SAFE_NO_PAD
        .decode(&manifest.iv)
        .is_ok_and(|value| value.len() == 12);
    let recipient_ids = manifest
        .recipients
        .iter()
        .map(|recipient| recipient.key_id.as_str())
        .collect::<std::collections::HashSet<_>>();
    let recipients_valid = (1..=32).contains(&manifest.recipients.len())
        && recipient_ids.len() == manifest.recipients.len()
        && manifest.recipients.iter().all(encrypted_recipient_valid);
    if !target_binding_matches(request.target_id.as_deref(), &manifest.binding.target_id)
        || manifest.binding.workspace_id != tenant.workspace_id.to_string()
        || manifest.binding.environment_id != tenant.environment_id.to_string()
        || manifest.binding.printer_id != destination.printer_id.to_string()
        || manifest.binding.content_type != request.content_type
        || manifest.binding.options != request.options
        || manifest.binding.deliveries != request.deliveries
        || manifest.binding.raw_authorized != (request.content_type == ContentKind::Raw)
        || binding_expiry <= Utc::now()
        || binding_expiry > Utc::now() + Duration::days(14)
        || manifest.version != piqae_domain::ENCRYPTED_JOB_V3_VERSION
        || manifest.suite != piqae_domain::ENCRYPTED_JOB_V3_SUITE
        || !manifest.binding.envelope_id.starts_with("env_")
        || !(24..=259).contains(&manifest.binding.envelope_id.len())
        || !manifest.binding.envelope_id[4..]
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        || !digest_valid
        || !iv_valid
        || !recipients_valid
    {
        return Err(AppError::invalid(
            "invalid_encrypted_job_binding",
            "Encrypted content binding does not match this job.",
        ));
    }
    let binding = destination.binding.as_ref().ok_or_else(|| {
        AppError::invalid(
            "encrypted_job_requires_target",
            "Encrypted jobs require an immutable target binding.",
        )
    })?;
    let expected_revision = format!("{}:{}", binding.profile_id, binding.profile_revision);
    if manifest.binding.profile_revision != expected_revision {
        return Err(AppError::invalid(
            "encrypted_profile_mismatch",
            "The encrypted profile revision is not the selected target revision.",
        ));
    }
    let mut recipient_available = false;
    for recipient in &manifest.recipients {
        if state
            .repository
            .content_encryption_key_for_agent_recipient(
                tenant.workspace_id,
                tenant.environment_id,
                destination.agent_id,
                &recipient.key_id,
            )
            .await
            .is_ok()
        {
            recipient_available = true;
            break;
        }
    }
    if !recipient_available {
        return Err(AppError::invalid(
            "encrypted_recipient_unavailable",
            "The selected node cannot decrypt this envelope.",
        ));
    }
    Ok(())
}

fn encrypted_recipient_valid(recipient: &piqae_domain::EncryptedContentRecipient) -> bool {
    recipient.algorithm == piqae_domain::ENCRYPTED_JOB_V3_RECIPIENT_ALGORITHM
        && URL_SAFE_NO_PAD
            .decode(&recipient.ephemeral_public_key)
            .is_ok_and(|value| value.len() == 65 && value.first() == Some(&4))
        && URL_SAFE_NO_PAD
            .decode(&recipient.hkdf_salt)
            .is_ok_and(|value| value.len() == 32)
        && URL_SAFE_NO_PAD
            .decode(&recipient.key_wrap_iv)
            .is_ok_and(|value| value.len() == 12)
        && URL_SAFE_NO_PAD
            .decode(&recipient.encrypted_content_key)
            .is_ok_and(|value| value.len() == 48)
}

async fn resolve_target_destination(
    state: &AppState,
    tenant: TenantContext,
    target_id: &str,
    allow_offline: bool,
) -> Result<ResolvedJobDestination, AppError> {
    let target = state
        .repository
        .get_target(tenant.workspace_id, tenant.environment_id, target_id)
        .await?;
    if !target.enabled {
        return Err(AppError::conflict(
            "target_not_ready",
            "The target is disabled or has no ready binding.",
        ));
    }
    let agents = state
        .repository
        .list_agents(tenant.workspace_id, tenant.environment_id)
        .await?;
    let bindings = state
        .repository
        .list_target_bindings(tenant.workspace_id, tenant.environment_id, target_id)
        .await?;
    let topology_routes = state
        .destination_topology
        .list_all_routes(crate::destination_topology::tenant_scope(tenant))
        .await
        .map_err(crate::destination_topology::map_storage_error)?;
    let mut ranked_by_destination = HashMap::<String, Vec<String>>::new();
    let mut best_ready = None::<(usize, ResolvedJobDestination)>;
    let mut configured_fallback = None;
    for binding in bindings.into_iter().filter(|binding| binding.enabled) {
        let agent_exists = agents.iter().any(|agent| agent.id == binding.agent_id);
        if !agent_exists {
            continue;
        }
        let agent_ready = agents
            .iter()
            .any(|agent| agent.id == binding.agent_id && crate::routing::agent_is_connected(agent));
        let Ok(printer) = state
            .repository
            .get_printer(
                tenant.workspace_id,
                tenant.environment_id,
                binding.printer_id,
            )
            .await
        else {
            continue;
        };
        if printer.agent_id != binding.agent_id {
            continue;
        }
        let Some(profile) = printer.profiles.iter().find(|profile| {
            (profile.profile_id.as_str(), profile.revision)
                == (binding.profile_id.as_str(), binding.profile_revision)
                && profile.published
                && matches!(profile.status.as_deref(), None | Some("ready"))
                && target
                    .stock_id
                    .as_ref()
                    .is_none_or(|stock_id| profile.stock_id.as_ref() == Some(stock_id))
        }) else {
            continue;
        };
        let mut metadata = BTreeMap::from([
            ("piqae.target_id".into(), target.id.clone()),
            ("piqae.binding_id".into(), binding.id.clone()),
            ("piqae.profile_id".into(), profile.profile_id.clone()),
            ("piqae.route_agent_id".into(), printer.agent_id.to_string()),
            (
                "piqae.profile_revision".into(),
                profile.revision.to_string(),
            ),
        ]);
        let topology_route = topology_routes.iter().find(|route| {
            route.enabled
                && route.printer_id == printer.id.to_string()
                && route.agent_id == printer.agent_id.to_string()
        });
        if let Some(route) = topology_route {
            metadata.insert("piqae.destination_id".into(), route.destination_id.clone());
            metadata.insert("piqae.route_id".into(), route.id.clone());
        }
        if let Some(stock_id) = target.stock_id.as_ref().or(profile.stock_id.as_ref()) {
            metadata.insert("piqae.stock_id".into(), stock_id.clone());
        }
        let destination = ResolvedJobDestination {
            printer_id: printer.id,
            agent_id: printer.agent_id,
            metadata,
            binding: Some(binding),
        };
        if agent_ready && printer.state == PrinterState::Online {
            let rank = if let Some(route) = topology_route {
                if !ranked_by_destination.contains_key(&route.destination_id) {
                    let ranked = crate::destination_topology::ranked_ready_routes(
                        state,
                        crate::destination_topology::tenant_scope(tenant),
                        &route.destination_id,
                    )
                    .await?
                    .into_iter()
                    .map(|candidate| candidate.id)
                    .collect();
                    ranked_by_destination.insert(route.destination_id.clone(), ranked);
                }
                ranked_by_destination
                    .get(&route.destination_id)
                    .and_then(|ranked| ranked.iter().position(|id| id == &route.id))
            } else {
                Some(usize::MAX - 1)
            };
            if let Some(rank) = rank
                && best_ready
                    .as_ref()
                    .is_none_or(|(current, _)| rank < *current)
            {
                best_ready = Some((rank, destination));
            }
            continue;
        }
        if allow_offline && configured_fallback.is_none() {
            configured_fallback = Some(destination);
        }
    }
    if let Some((_, destination)) = best_ready {
        return Ok(destination);
    }
    if let Some(destination) = configured_fallback {
        return Ok(destination);
    }
    Err(AppError::conflict(
        "target_not_ready",
        if allow_offline {
            "The target has no valid configured binding."
        } else {
            "The target has no online ready binding."
        },
    ))
}

async fn schedule_waiting_destination_jobs(
    state: &AppState,
    tenant: TenantContext,
) -> Result<(), AppError> {
    let jobs = state
        .repository
        .list_reroutable_destination_jobs(tenant.workspace_id, tenant.environment_id, 1_000)
        .await?;
    for job in jobs {
        let Some(destination_id) = job.metadata.get("piqae.destination_id") else {
            continue;
        };
        let routes = crate::destination_topology::ranked_ready_routes(
            state,
            crate::destination_topology::tenant_scope(tenant),
            destination_id,
        )
        .await?;
        if routes.is_empty() {
            continue;
        }
        let target_id = job
            .metadata
            .get("piqae.target_id")
            .or_else(|| job.metadata.get("spool.target_id"));
        let bindings = if let Some(target_id) = target_id {
            state
                .repository
                .list_target_bindings(tenant.workspace_id, tenant.environment_id, target_id)
                .await?
        } else {
            Vec::new()
        };
        for route in routes {
            if job.metadata.get("piqae.route_id") == Some(&route.id) {
                break;
            }
            let binding = target_id.and_then(|_| {
                bindings.iter().find(|binding| {
                    binding.enabled
                        && binding.printer_id.to_string() == route.printer_id
                        && binding.agent_id.to_string() == route.agent_id
                })
            });
            if target_id.is_some() && binding.is_none() {
                continue;
            }
            let capability_revision = job
                .metadata
                .get("piqae.capability_revision")
                .and_then(|value| value.parse().ok());
            let route_ticket_key = format!("piqae.resolved_ticket_digest.{}", route.id);
            let resolved_ticket_digest = job
                .metadata
                .get(&route_ticket_key)
                .or_else(|| job.metadata.get("piqae.resolved_ticket_digest"));
            let rerouted = state
                .repository
                .reroute_job_to_destination_route_before_acceptance(
                    tenant.workspace_id,
                    tenant.environment_id,
                    piqae_storage_postgres::DestinationRouteReassignment {
                        job_id: job.id,
                        target_id: target_id.map(String::as_str),
                        route_id: &route.id,
                        profile_id: binding.map(|value| value.profile_id.as_str()),
                        profile_revision: binding.map(|value| value.profile_revision),
                        expected_capability_revision: capability_revision,
                        resolved_ticket_digest: resolved_ticket_digest.map(String::as_str),
                        reason: "standby_recovery",
                    },
                )
                .await;
            match rerouted {
                Ok(Some(rerouted)) => {
                    state
                        .publish(tenant, "job.routing_attempted", &rerouted)
                        .await?;
                    break;
                }
                Ok(None) | Err(RepositoryError::ConcurrentStateChange) => {}
                Err(error) => return Err(error.into()),
            }
        }
    }
    Ok(())
}

pub(crate) struct PersistedJobContent {
    pub source: ContentSource,
    pub owned_upload: Option<StoredUpload>,
}

#[allow(clippy::too_many_lines)]
pub(crate) async fn persist_job_content(
    state: &AppState,
    tenant: TenantContext,
    content_kind: ContentKind,
    content: ContentSource,
) -> Result<PersistedJobContent, AppError> {
    match content {
        ContentSource::Upload { upload_id } => {
            let upload = state
                .repository
                .get_upload(tenant.workspace_id, tenant.environment_id, &upload_id)
                .await?;
            let expected_media_type = match content_kind {
                ContentKind::Pdf => "application/pdf",
                ContentKind::Raw => "application/octet-stream",
            };
            if upload.state != "complete" || upload.media_type != expected_media_type {
                return Err(AppError::invalid(
                    "invalid_job_upload",
                    "The upload is incomplete or does not match the job content type.",
                ));
            }
            Ok(PersistedJobContent {
                source: ContentSource::Upload { upload_id },
                owned_upload: None,
            })
        }
        ContentSource::EncryptedUpload {
            upload_id,
            manifest,
        } => {
            let upload = state
                .repository
                .get_upload(tenant.workspace_id, tenant.environment_id, &upload_id)
                .await?;
            if upload.state != "complete"
                || upload.media_type != "application/octet-stream"
                || !decoded_digest_matches(&manifest.ciphertext_sha256, &upload.expected_sha256)
            {
                return Err(AppError::invalid(
                    "invalid_encrypted_upload",
                    "Ciphertext upload is incomplete or does not match its authenticated manifest.",
                ));
            }
            Ok(PersistedJobContent {
                source: ContentSource::EncryptedUpload {
                    upload_id,
                    manifest,
                },
                owned_upload: None,
            })
        }
        ContentSource::Base64 { data } => {
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(|_| AppError::invalid("invalid_base64_content", "Content is invalid."))?;
            if decoded.is_empty()
                || decoded.len() > usize::try_from(MAX_UPLOAD_BYTES).unwrap_or(50 * 1024 * 1024)
            {
                return Err(AppError::invalid(
                    "invalid_content_size",
                    "Content must contain between 1 byte and 50 MiB.",
                ));
            }
            let id = format!("upl_{}", ulid::Ulid::new());
            let sha256 = digest_hex(&decoded);
            let expected_bytes = i64::try_from(decoded.len())
                .map_err(|_| AppError::invalid("invalid_content_size", "Content is too large."))?;
            let upload = StoredUpload {
                id: id.clone(),
                object_key: format!("{}/{}/{}", tenant.workspace_id, tenant.environment_id, id),
                media_type: match content_kind {
                    ContentKind::Pdf => "application/pdf",
                    ContentKind::Raw => "application/octet-stream",
                }
                .into(),
                expected_sha256: sha256.clone(),
                expected_bytes,
                state: "pending".into(),
                expires_at: Utc::now() + Duration::days(14),
            };
            state
                .repository
                .create_upload(&upload, tenant.workspace_id, tenant.environment_id)
                .await?;
            state
                .object_store
                .put(&upload.object_key, Bytes::from(decoded), Some(&sha256))
                .await
                .map_err(|_| AppError::service_unavailable("object_store_unavailable"))?;
            state
                .repository
                .complete_upload(
                    tenant.workspace_id,
                    tenant.environment_id,
                    &id,
                    &sha256,
                    expected_bytes,
                )
                .await?;
            Ok(PersistedJobContent {
                source: ContentSource::Upload { upload_id: id },
                owned_upload: Some(upload),
            })
        }
        ContentSource::Uri {
            uri,
            authentication,
        } => {
            if authentication.is_some() {
                return Err(AppError::invalid(
                    "uri_credentials_not_supported",
                    "Authenticated URI content is not persisted; upload the content instead.",
                ));
            }
            Ok(PersistedJobContent {
                source: ContentSource::Uri {
                    uri,
                    authentication: None,
                },
                owned_upload: None,
            })
        }
    }
}

fn target_binding_matches(request_target_id: Option<&str>, manifest_target_id: &str) -> bool {
    request_target_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        == Some(manifest_target_id)
}

fn decoded_digest_matches(encoded_digest: &str, expected_sha256: &str) -> bool {
    URL_SAFE_NO_PAD
        .decode(encoded_digest)
        .ok()
        .map(hex::encode)
        .is_some_and(|digest| digest.eq_ignore_ascii_case(expected_sha256))
}

#[cfg(test)]
mod encrypted_binding_tests {
    use super::{
        decoded_digest_matches, target_binding_matches, valid_content_encryption_public_key,
    };
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use p256::pkcs8::EncodePublicKey as _;

    #[test]
    fn decoded_ciphertext_digest_accepts_canonical_hex_case_only() {
        let encoded = URL_SAFE_NO_PAD.encode([0xab_u8; 32]);
        assert!(decoded_digest_matches(&encoded, &"AB".repeat(32)));
        assert!(decoded_digest_matches(&encoded, &"ab".repeat(32)));
        assert!(!decoded_digest_matches(&encoded, &"AC".repeat(32)));
        assert!(!decoded_digest_matches("not+base64", &"ab".repeat(32)));
    }

    #[test]
    fn encrypted_target_binding_uses_the_normalized_request_identifier() {
        assert!(target_binding_matches(Some("  tgt_exact\n"), "tgt_exact"));
        assert!(!target_binding_matches(Some("tgt_other"), "tgt_exact"));
        assert!(!target_binding_matches(Some("   "), ""));
        assert!(!target_binding_matches(None, "tgt_exact"));
    }

    #[test]
    fn content_encryption_recipient_must_be_a_valid_p256_spki() {
        let secret = p256::SecretKey::random(&mut rand::rngs::OsRng);
        let der = secret.public_key().to_public_key_der();
        assert!(der.is_ok());
        let Some(der) = der.ok() else {
            return;
        };
        assert!(valid_content_encryption_public_key(
            &URL_SAFE_NO_PAD.encode(der.as_bytes())
        ));
        assert!(!valid_content_encryption_public_key(
            &URL_SAFE_NO_PAD.encode([0_u8; 91])
        ));
    }
}

pub(crate) async fn cleanup_owned_upload(
    state: &AppState,
    tenant: TenantContext,
    upload: Option<&StoredUpload>,
) {
    let Some(upload) = upload else {
        return;
    };
    if state.object_store.delete(&upload.object_key).await.is_err() {
        tracing::warn!(
            error.type = "job_upload_cleanup_object_failed",
            upload.id = %upload.id,
            "could not remove unreferenced inline job content"
        );
        return;
    }
    if state
        .repository
        .delete_upload(tenant.workspace_id, tenant.environment_id, &upload.id)
        .await
        .is_err()
    {
        tracing::warn!(
            error.type = "job_upload_cleanup_record_failed",
            upload.id = %upload.id,
            "could not remove unreferenced inline upload record"
        );
    }
}

pub async fn list_jobs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> Result<Json<Page<JobResponse>>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::JobsRead).await?;
    let limit = query.limit.clamp(1, 500);
    let after = query
        .after
        .as_deref()
        .map(JobId::from_str)
        .transpose()
        .map_err(|_| AppError::invalid("invalid_cursor", "The pagination cursor is invalid."))?;
    if query.metadata_value.is_some() && query.metadata_key.is_none() {
        return Err(AppError::invalid(
            "invalid_job_filter",
            "metadata_value requires metadata_key.",
        ));
    }
    let printer_id = query
        .printer_id
        .as_deref()
        .map(PrinterId::from_str)
        .transpose()
        .map_err(|_| AppError::invalid("invalid_job_filter", "printer_id is invalid."))?;
    let mut jobs = state
        .repository
        .list_jobs(tenant.workspace_id, tenant.environment_id, after, 500)
        .await?;
    jobs.retain(|job| {
        query.state.is_none_or(|value| job.state == value)
            && printer_id.is_none_or(|value| job.printer_id == value)
            && query
                .target_id
                .as_ref()
                .is_none_or(|value| job.metadata.get("piqae.target_id") == Some(value))
            && query.metadata_key.as_ref().is_none_or(|key| {
                job.metadata.get(key).is_some_and(|value| {
                    query
                        .metadata_value
                        .as_ref()
                        .is_none_or(|expected| value == expected)
                })
            })
    });
    let has_more = jobs.len() > usize::try_from(limit).unwrap_or(500);
    jobs.truncate(usize::try_from(limit).unwrap_or(500));
    let next_cursor = has_more
        .then(|| jobs.last().map(|job| job.id.to_string()))
        .flatten();
    Ok(Json(Page {
        data: jobs.into_iter().map(JobResponse::from).collect(),
        next_cursor,
        has_more,
    }))
}

pub async fn get_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(job_id): Path<String>,
) -> Result<Json<JobResponse>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::JobsRead).await?;
    let id = parse_job_id(&job_id)?;
    let job = state
        .repository
        .get_job(tenant.workspace_id, tenant.environment_id, id)
        .await?;
    Ok(Json(job.into()))
}

pub async fn list_job_events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(job_id): Path<String>,
) -> Result<Json<Vec<JobEvent>>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::JobsRead).await?;
    let events = state
        .repository
        .list_job_events(
            tenant.workspace_id,
            tenant.environment_id,
            parse_job_id(&job_id)?,
        )
        .await?;
    Ok(Json(events))
}

pub async fn cancel_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(job_id): Path<String>,
) -> Result<Response, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::JobsWrite).await?;
    let job = state
        .repository
        .request_job_cancellation(
            tenant.workspace_id,
            tenant.environment_id,
            parse_job_id(&job_id)?,
        )
        .await?;
    if job.state == JobState::Cancelled {
        state
            .publish_idempotently(
                &preaccept_cancellation_webhook_idempotency_key(
                    tenant.workspace_id,
                    tenant.environment_id,
                    job.id,
                ),
                tenant,
                "job.updated",
                &job,
            )
            .await?;
    } else {
        state.publish(tenant, "job.updated", &job).await?;
    }
    Ok((StatusCode::ACCEPTED, Json(JobResponse::from(job))).into_response())
}

#[allow(clippy::too_many_lines)]
pub async fn agent_sync(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<AgentSyncResponse>, AppError> {
    let identity = authenticate_agent(&state, &headers, "POST", "/v1/agent/sync", &body).await?;
    let request: AgentSyncRequest = serde_json::from_slice(&body)?;
    if request.agent_id != identity.agent_id {
        return Err(AppError::device_unauthorized("agent_identity_mismatch"));
    }
    if request.protocol_version != 1 || request.events.len() > 1_000 {
        return Err(AppError::invalid(
            "invalid_agent_sync",
            "The sync protocol or event batch is outside supported limits.",
        ));
    }
    if request.diagnostics.len() > 8
        || request.diagnostics.iter().any(|report| {
            !report.request_id.starts_with("diag_")
                || report.request_id.len() > 64
                || !matches!(report.state.as_str(), "complete" | "failed")
                || report.agent_version.len() > 64
                || report.platform.len() > 32
                || report.architecture.len() > 32
                || report
                    .last_error_code
                    .as_deref()
                    .is_some_and(|code| code.len() > 128 || !code.is_ascii())
                || report
                    .collection_error_code
                    .as_deref()
                    .is_some_and(|code| code.len() > 128 || !code.is_ascii())
                || serde_json::to_vec(report).map_or(true, |value| value.len() > 16_384)
        })
    {
        return Err(AppError::invalid(
            "invalid_agent_diagnostics",
            "The diagnostic report batch is outside supported limits.",
        ));
    }
    let now = Utc::now();
    if request.health.started_at > request.health.observed_at
        || request.health.observed_at > now + chrono::TimeDelta::minutes(5)
        || request.health.executor_crashes > i64::MAX as u64
        || request
            .health
            .last_error_code
            .as_deref()
            .is_some_and(|code| code.is_empty() || code.len() > 128 || !code.is_ascii())
    {
        return Err(AppError::invalid(
            "invalid_agent_health",
            "The reported agent health is outside supported limits.",
        ));
    }
    if request
        .acknowledged_command_cursor
        .as_deref()
        .is_some_and(|cursor| cursor.parse::<i64>().is_err())
    {
        return Err(AppError::invalid(
            "invalid_agent_command_cursor",
            "The acknowledged command cursor is invalid.",
        ));
    }
    let tenant = identity.tenant;
    let printers = request.printers.as_ref().map(|printers| {
        printers
            .iter()
            .map(|printer| SyncedPrinter {
                id: printer.id,
                native_id: printer.native_id.clone(),
                name: printer.name.clone(),
                state: printer.state,
                is_default: printer.is_default,
                capabilities: printer.capabilities.clone(),
                capability_revision: printer.capability_revision,
                native_options: printer.native_options.clone(),
                semantic_capabilities: printer.semantic_capabilities.clone(),
                profiles: printer
                    .profiles
                    .iter()
                    .map(|profile| piqae_storage_postgres::PrinterProfileSnapshot {
                        profile_id: profile.profile_id.clone(),
                        revision: profile.revision,
                        name: profile.name.clone(),
                        is_default: profile.is_default,
                        options: profile.options.clone(),
                        status: serde_json::to_value(profile.status)
                            .ok()
                            .and_then(|value| value.as_str().map(ToOwned::to_owned)),
                        native_kind: profile.native_kind.and_then(|kind| {
                            serde_json::to_value(kind)
                                .ok()
                                .and_then(|value| value.as_str().map(ToOwned::to_owned))
                        }),
                        native_digest: profile.native_digest.clone(),
                        driver_fingerprint: (!profile.driver_fingerprint.driver_name.is_empty()
                            || !profile.driver_fingerprint.native_queue_id.is_empty())
                        .then(|| serde_json::to_value(&profile.driver_fingerprint))
                        .transpose()
                        .ok()
                        .flatten(),
                        summary: Some(serde_json::to_value(&profile.summary).unwrap_or_default()),
                        stock_id: profile.stock_id.clone(),
                        safe_overrides: profile
                            .safe_overrides
                            .iter()
                            .filter_map(|value| {
                                serde_json::to_value(value)
                                    .ok()
                                    .and_then(|value| value.as_str().map(ToOwned::to_owned))
                            })
                            .collect(),
                        last_validated_at: profile
                            .last_validated_unix_ms
                            .and_then(chrono::DateTime::<Utc>::from_timestamp_millis),
                        last_test_job_id: profile.last_test_job_id.clone(),
                        published: profile.published,
                    })
                    .collect(),
            })
            .collect::<Vec<_>>()
    });
    state
        .repository
        .sync_agent_presence(
            tenant.workspace_id,
            tenant.environment_id,
            request.agent_id,
            &request.agent_version,
            &request.health,
            validate_document_render_capabilities(&request.document_render)?,
            printers.as_deref(),
        )
        .await?;
    let inventory_projection =
        crate::destination_topology::project_agent_topology(&state, tenant, &request).await?;
    let runtime_admission =
        crate::destination_topology::record_runtime_availability(&state, tenant, &request).await?;
    if let Some(projection) = inventory_projection.as_ref() {
        let invalidation = serde_json::json!({
            "agent_id": request.agent_id,
            "inventory_revision": projection.revision,
            "observed_at": request.health.observed_at,
        });
        state
            .publish(tenant, "route.updated", &invalidation)
            .await?;
        state
            .publish(tenant, "destination.updated", &invalidation)
            .await?;
    }
    let acknowledged_handoff_sequence = crate::destination_topology::ingest_native_handoffs(
        &state,
        tenant,
        request.agent_id,
        &request.native_handoffs,
    )
    .await?;
    let mut acknowledged_diagnostics = Vec::with_capacity(request.diagnostics.len());
    for report in &request.diagnostics {
        match state
            .repository
            .store_node_diagnostic(
                tenant.workspace_id,
                tenant.environment_id,
                request.agent_id,
                report,
            )
            .await
        {
            Ok(()) | Err(crate::repository::RepositoryError::NotFound) => {
                acknowledged_diagnostics.push(report.request_id.clone());
            }
            Err(error) => return Err(error.into()),
        }
    }
    schedule_waiting_destination_jobs(&state, tenant).await?;
    for event in &request.events {
        let should_reconcile = match state
            .repository
            .apply_agent_event(
                tenant.workspace_id,
                tenant.environment_id,
                request.agent_id,
                event,
            )
            .await
        {
            Ok(Some(job)) => {
                state.publish(tenant, "job.updated", &job).await?;
                // Uncertain delivery is the one outcome that cannot be proved
                // either way, so it gets its own event rather than making every
                // consumer subscribe to all job updates and filter. `event.state`
                // is the reported transition, so this fires once on the way in
                // rather than repeating for a job that is already uncertain.
                if event.state == JobState::DeliveryUncertain {
                    state
                        .publish(tenant, "job.delivery_uncertain", &job)
                        .await?;
                }
                true
            }
            Ok(None) => {
                // A replay can repair a crash between the ordinary job event
                // and the attempt projection, but only when this exact state
                // is already durable. A stale event must never advance the
                // physical delivery attempt.
                state
                    .repository
                    .get_job(tenant.workspace_id, tenant.environment_id, event.job_id)
                    .await
                    .is_ok_and(|job| job.state == event.state)
            }
            Err(RepositoryError::ConcurrentStateChange) => false,
            Err(error) => return Err(error.into()),
        };
        if should_reconcile {
            crate::destination_topology::reconcile_post_spooler_event(
                &state,
                tenant,
                request.agent_id,
                event,
            )
            .await?;
        }
    }
    let mut command_batch = state
        .repository
        .sync_agent_commands(
            tenant.workspace_id,
            tenant.environment_id,
            request.agent_id,
            request.acknowledged_command_cursor.as_deref(),
            100,
        )
        .await?;
    let mut deliverable_commands = Vec::with_capacity(command_batch.commands.len());
    for command in command_batch.commands {
        let retire = if let AgentCommand::CancelJob { job_id } = &command {
            state
                .repository
                .retire_terminal_absent_local_cancellation(
                    tenant.workspace_id,
                    tenant.environment_id,
                    request.agent_id,
                    *job_id,
                )
                .await?
        } else {
            false
        };
        if !retire {
            deliverable_commands.push(command);
        }
    }
    command_batch.commands = deliverable_commands;
    crate::destination_topology::finalize_acknowledged_uncertainty(
        &state,
        tenant,
        request.agent_id,
    )
    .await?;
    let leases = if request.queue.accepts_jobs && runtime_admission.eligible_for_offers {
        state
            .repository
            .claim_jobs(
                tenant.workspace_id,
                tenant.environment_id,
                request.agent_id,
                &format!("{}:{}", request.agent_id, request.agent_version),
                // The V1 agent materializes offers serially. Claiming a batch
                // would let later 30-second leases expire before the agent
                // reaches them, so offer one durable handoff per sync.
                1,
            )
            .await?
    } else {
        Vec::new()
    };
    let mut candidate_jobs = Vec::with_capacity(leases.len());
    for lease in leases {
        if lease.job.content_kind == ContentKind::Raw
            && !supports_printer_native_offer(
                &request.document_render,
                &lease.job.metadata,
                &lease.job.printer_id.to_string(),
            )
        {
            // Capabilities can change after registration. Never offer opaque
            // native bytes after the exact printer/language binding disappears.
            state
                .repository
                .release_agent_lease(
                    tenant.workspace_id,
                    tenant.environment_id,
                    request.agent_id,
                    lease.job.id,
                    lease.lease_id,
                    &lease.lease_token,
                )
                .await?;
            continue;
        }
        let route_reservation = match crate::destination_topology::reserve_job_route(
            &state,
            tenant,
            &lease.job,
            lease.lease_until,
        )
        .await?
        {
            crate::destination_topology::JobRouteReservation::Reserved(reservation) => {
                Some(reservation)
            }
            crate::destination_topology::JobRouteReservation::Busy => {
                state
                    .repository
                    .release_agent_lease(
                        tenant.workspace_id,
                        tenant.environment_id,
                        request.agent_id,
                        lease.job.id,
                        lease.lease_id,
                        &lease.lease_token,
                    )
                    .await?;
                continue;
            }
            crate::destination_topology::JobRouteReservation::ProjectionRequired {
                destination_id,
                route_id,
            } => {
                state
                    .repository
                    .release_agent_lease(
                        tenant.workspace_id,
                        tenant.environment_id,
                        request.agent_id,
                        lease.job.id,
                        lease.lease_id,
                        &lease.lease_token,
                    )
                    .await?;
                state
                    .publish(
                        tenant,
                        "route.updated",
                        &serde_json::json!({
                            "agent_id": request.agent_id,
                            "destination_id": destination_id,
                            "route_id": route_id,
                            "readiness": "projection_required",
                            "reason": "node_upgrade_required",
                            "observed_at": request.health.observed_at,
                        }),
                    )
                    .await?;
                continue;
            }
        };
        let mut content = match &lease.job.content {
            ContentSource::Upload { upload_id } => {
                let upload = state
                    .repository
                    .get_upload(tenant.workspace_id, tenant.environment_id, upload_id)
                    .await?;
                if upload.state != "complete" {
                    return Err(AppError::service_unavailable("job_upload_is_not_complete"));
                }
                ContentDescriptor::Download {
                    url: format!("/v1/agent/jobs/{}/content", lease.job.id),
                    sha256: upload.expected_sha256,
                    bytes: u64::try_from(upload.expected_bytes).map_err(|_| {
                        AppError::service_unavailable("invalid_stored_content_length")
                    })?,
                }
            }
            ContentSource::EncryptedUpload {
                upload_id,
                manifest,
            } => {
                let upload = state
                    .repository
                    .get_upload(tenant.workspace_id, tenant.environment_id, upload_id)
                    .await?;
                if upload.state != "complete" {
                    return Err(AppError::service_unavailable("job_upload_is_not_complete"));
                }
                ContentDescriptor::EncryptedDownload {
                    url: format!("/v1/agent/jobs/{}/content", lease.job.id),
                    sha256: upload.expected_sha256,
                    bytes: u64::try_from(upload.expected_bytes).map_err(|_| {
                        AppError::service_unavailable("invalid_stored_content_length")
                    })?,
                    manifest: manifest.clone(),
                }
            }
            ContentSource::Base64 { data } => {
                let decoded = base64::engine::general_purpose::STANDARD
                    .decode(data)
                    .map_err(|_| AppError::service_unavailable("invalid_stored_base64_content"))?;
                ContentDescriptor::InlineBase64 {
                    data: data.clone(),
                    sha256: Some(digest_hex(&decoded)),
                    bytes: Some(decoded.len() as u64),
                }
            }
            ContentSource::Uri {
                uri,
                authentication,
            } => ContentDescriptor::Uri {
                uri: uri.clone(),
                authentication: authentication.clone(),
                sha256: None,
                bytes: None,
            },
        };
        if lease
            .job
            .metadata
            .get("piqae.document.render_mode")
            .is_some_and(|mode| mode == "node_render")
        {
            let render_id = lease
                .job
                .metadata
                .get("piqae.document.render_id")
                .ok_or_else(|| AppError::service_unavailable("document_render_metadata_missing"))?;
            let policy = match lease
                .job
                .metadata
                .get("piqae.document.render_policy")
                .map(String::as_str)
            {
                Some("cloud_only") => piqae_protocol::agent::PrintPacketRenderPolicy::CloudOnly,
                Some("prefer_node") => piqae_protocol::agent::PrintPacketRenderPolicy::PreferNode,
                Some("require_node") => piqae_protocol::agent::PrintPacketRenderPolicy::RequireNode,
                _ => piqae_protocol::agent::PrintPacketRenderPolicy::Automatic,
            };
            let fallback_allowed =
                policy != piqae_protocol::agent::PrintPacketRenderPolicy::RequireNode;
            if matches!(content, ContentDescriptor::EncryptedDownload { .. }) {
                if !fallback_allowed {
                    return Err(AppError::service_unavailable(
                        "node_render_encrypted_content_unsupported",
                    ));
                }
                // Never attach cleartext specification/input beside encrypted
                // job content. Prefer/automatic keep the encrypted PDF offer.
                candidate_jobs.push(JobOffer {
                    expected_capability_revision: lease
                        .job
                        .metadata
                        .get("piqae.capability_revision")
                        .and_then(|revision| revision.parse().ok()),
                    resolved_ticket_digest: lease
                        .job
                        .metadata
                        .get("piqae.resolved_ticket_digest")
                        .cloned(),
                    job: lease.job,
                    lease_id: lease.lease_id,
                    lease_token: lease.lease_token,
                    lease_expires_at: lease.lease_until,
                    content,
                    route_reservation,
                });
                continue;
            }
            let render = crate::documents::node_render_payload(
                &state,
                tenant.workspace_id,
                tenant.environment_id,
                render_id,
            )
            .await?;
            if !supports_print_packet_offer(&request.document_render, &render) {
                if fallback_allowed {
                    // Capability changed after registration. Preserve the
                    // already-approved PDF without leaking a descriptor to a
                    // node that no longer satisfies the exact contract.
                    candidate_jobs.push(JobOffer {
                        expected_capability_revision: lease
                            .job
                            .metadata
                            .get("piqae.capability_revision")
                            .and_then(|revision| revision.parse().ok()),
                        resolved_ticket_digest: lease
                            .job
                            .metadata
                            .get("piqae.resolved_ticket_digest")
                            .cloned(),
                        job: lease.job,
                        lease_id: lease.lease_id,
                        lease_token: lease.lease_token,
                        lease_expires_at: lease.lease_until,
                        content,
                        route_reservation,
                    });
                    continue;
                }
                state
                    .repository
                    .release_agent_lease(
                        tenant.workspace_id,
                        tenant.environment_id,
                        request.agent_id,
                        lease.job.id,
                        lease.lease_id,
                        &lease.lease_token,
                    )
                    .await?;
                continue;
            }
            content = ContentDescriptor::PrintPacket {
                policy,
                render: Box::new(render),
                fallback: Box::new(content),
                fallback_allowed,
                decision_reason: lease
                    .job
                    .metadata
                    .get("piqae.document.render_decision_reason")
                    .cloned()
                    .unwrap_or_else(|| "unspecified".into()),
            };
        }
        candidate_jobs.push(JobOffer {
            expected_capability_revision: lease
                .job
                .metadata
                .get("piqae.capability_revision")
                .and_then(|revision| revision.parse().ok()),
            resolved_ticket_digest: lease
                .job
                .metadata
                .get("piqae.resolved_ticket_digest")
                .cloned(),
            job: lease.job,
            lease_id: lease.lease_id,
            lease_token: lease.lease_token,
            lease_expires_at: lease.lease_until,
            content,
            route_reservation,
        });
    }
    let has_immediate_work = !request.events.is_empty()
        || request.queue.queued_jobs > 0
        || request.queue.active_jobs > 0
        || !command_batch.commands.is_empty()
        || !candidate_jobs.is_empty();
    let next_poll_after_ms = adaptive_poll_after_ms(&request, has_immediate_work);
    Ok(Json(AgentSyncResponse {
        server_time: Utc::now(),
        acknowledged_event_cursor: request.events.last().map(|event| event.id),
        command_cursor: command_batch.cursor,
        commands: command_batch.commands,
        candidate_jobs,
        next_poll_after_ms,
        acknowledged_diagnostics,
        inventory_projection_acknowledgement_supported: true,
        inventory_projection,
        acknowledged_handoff_sequence,
        wake_hints: runtime_admission.wake_hints,
    }))
}

fn supports_print_packet_offer(
    capabilities: &piqae_protocol::agent::DocumentRenderCapabilities,
    render: &piqae_protocol::agent::PrintPacketNodeRender,
) -> bool {
    let Some(packet) = capabilities.print_packet.as_ref() else {
        return false;
    };
    let input_bytes = serde_json::to_vec(&render.input)
        .ok()
        .and_then(|value| u64::try_from(value.len()).ok())
        .unwrap_or(u64::MAX);
    let template_bytes = serde_json::to_vec(&render.specification)
        .ok()
        .and_then(|value| u64::try_from(value.len()).ok())
        .unwrap_or(u64::MAX);
    let resource_count = u32::try_from(render.resources.len()).unwrap_or(u32::MAX);
    let total_resource_bytes = render.resources.iter().fold(0_u64, |total, resource| {
        total.saturating_add(resource.byte_length)
    });
    render.negotiation_version == 2
        && render.packet_version == printpacket::DOCUMENT_V1
        && render.conformance_profile == printpacket::CONFORMANCE_CORE_V1
        && render.output_profile == printpacket::PDF_BASE14_V1
        && packet.negotiation_version == render.negotiation_version
        && packet
            .supported_packet_versions
            .contains(&render.packet_version)
        && render
            .required_feature_ids
            .iter()
            .all(|feature| packet.feature_ids.contains(feature))
        && packet
            .conformance_profiles
            .contains(&render.conformance_profile)
        && packet.output_profiles.iter().any(|profile| {
            matches!(
                profile,
                piqae_protocol::agent::PrintPacketOutputProfile::Pdf { id, media_type }
                    if id == &render.output_profile && media_type == "application/pdf"
            )
        })
        && packet.deterministic
        && capabilities.renderer_abi.as_deref() == Some(render.renderer_abi.as_str())
        && capabilities.resource_abi.as_deref() == Some(render.resource_abi.as_str())
        && template_bytes <= packet.limits.max_template_bytes
        && input_bytes <= packet.limits.max_input_bytes
        && render.expected_pdf_bytes <= packet.limits.max_output_bytes
        && render.expected_page_count > 0
        && render.expected_page_count <= packet.limits.max_pages
        && resource_count <= packet.limits.max_resource_count
        && total_resource_bytes <= packet.limits.max_total_resource_bytes
        && render.resources.iter().all(|resource| {
            resource.byte_length <= packet.limits.max_resource_bytes
                && packet.resource_types.contains(&resource.media_type)
                && capabilities
                    .image_media_types
                    .contains(&resource.media_type)
        })
}

fn supports_printer_native_offer(
    capabilities: &piqae_protocol::agent::DocumentRenderCapabilities,
    metadata: &BTreeMap<String, String>,
    printer_id: &str,
) -> bool {
    let (Some(output_id), Some(language_id), Some(packet)) = (
        metadata.get("piqae.printer_native.output_profile"),
        metadata.get("piqae.printer_native.language_profile"),
        capabilities.print_packet.as_ref(),
    ) else {
        return false;
    };
    let language = packet.native_language_profiles.iter().find(|profile| {
        profile.id == *language_id
            && profile
                .printer_ids
                .iter()
                .any(|supported| supported == printer_id)
    });
    packet.output_profiles.iter().any(|profile| {
        matches!(
            profile,
            piqae_protocol::agent::PrintPacketOutputProfile::PrinterNative {
                id,
                media_type,
                language_profile_id,
            } if id == output_id
                && language_profile_id == language_id
                && language.is_some_and(|language| &language.media_type == media_type)
        )
    })
}

fn validate_document_render_capabilities(
    value: &piqae_protocol::agent::DocumentRenderCapabilities,
) -> Result<&piqae_protocol::agent::DocumentRenderCapabilities, AppError> {
    let valid_text = |value: &Option<String>| {
        value
            .as_ref()
            .is_none_or(|v| !v.is_empty() && v.len() <= 128 && v.is_ascii())
    };
    let valid_media = |values: &[String]| {
        values.len() <= 16 && values.iter().all(|v| v.len() <= 64 && v.is_ascii())
    };
    let valid_id = |value: &str, slash: bool| {
        !value.is_empty()
            && value.len() <= 128
            && value.bytes().enumerate().all(|(index, byte)| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || (index > 0 && matches!(byte, b'.' | b'_' | b'-'))
                    || (slash && index > 0 && byte == b'/')
            })
            && value.as_bytes()[0].is_ascii_lowercase()
    };
    let valid_packet_version = |value: &str| {
        value.strip_prefix("printpacket/v").is_some_and(|version| {
            let mut bytes = version.bytes();
            bytes
                .next()
                .is_some_and(|first| matches!(first, b'1'..=b'9'))
                && bytes.all(|byte| byte.is_ascii_digit())
        })
    };
    let unique = |values: &[String]| values.iter().collect::<BTreeSet<_>>().len() == values.len();
    let valid_packet = value.print_packet.as_ref().is_none_or(|packet| {
        let output_ids = packet
            .output_profiles
            .iter()
            .map(piqae_protocol::agent::PrintPacketOutputProfile::id)
            .collect::<BTreeSet<_>>();
        let language_ids = packet
            .native_language_profiles
            .iter()
            .map(|profile| profile.id.as_str())
            .collect::<BTreeSet<_>>();
        packet.negotiation_version == 2
            && (1..=16).contains(&packet.supported_packet_versions.len())
            && unique(&packet.supported_packet_versions)
            && packet
                .supported_packet_versions
                .iter()
                .all(|version| valid_packet_version(version))
            && packet.feature_ids.len() <= 256
            && unique(&packet.feature_ids)
            && packet.feature_ids.iter().all(|id| valid_id(id, false))
            && packet.conformance_profiles.len() <= 32
            && unique(&packet.conformance_profiles)
            && packet
                .conformance_profiles
                .iter()
                .all(|id| valid_id(id, true))
            && (1..=32).contains(&packet.output_profiles.len())
            && output_ids.len() == packet.output_profiles.len()
            && packet.output_profiles.iter().all(|profile| match profile {
                piqae_protocol::agent::PrintPacketOutputProfile::Pdf { id, media_type } => {
                    valid_id(id, true) && media_type == "application/pdf"
                }
                piqae_protocol::agent::PrintPacketOutputProfile::PrinterNative {
                    id,
                    media_type,
                    language_profile_id,
                } => {
                    valid_id(id, true)
                        && valid_media_type(media_type)
                        && valid_id(language_profile_id, true)
                        && language_ids.contains(language_profile_id.as_str())
                }
            })
            && packet.limits.max_template_bytes <= 52_428_800
            && packet.limits.max_template_bytes > 0
            && packet.limits.max_input_bytes <= 52_428_800
            && packet.limits.max_input_bytes > 0
            && packet.limits.max_output_bytes <= 524_288_000
            && packet.limits.max_output_bytes > 0
            && (1..=100_000).contains(&packet.limits.max_pages)
            && packet.limits.max_resource_count <= 1_000
            && (1..=52_428_800).contains(&packet.limits.max_resource_bytes)
            && (1..=524_288_000).contains(&packet.limits.max_total_resource_bytes)
            && packet.limits.max_resource_bytes <= packet.limits.max_total_resource_bytes
            && packet.resource_types.len() <= 32
            && unique(&packet.resource_types)
            && packet
                .resource_types
                .iter()
                .all(|media| media == "image/jpeg")
            && packet.native_language_profiles.len() <= 32
            && language_ids.len() == packet.native_language_profiles.len()
            && packet.native_language_profiles.iter().all(|profile| {
                valid_id(&profile.id, true)
                    && valid_id(&profile.language, false)
                    && !profile.version.is_empty()
                    && profile.version.len() <= 64
                    && profile.version.is_ascii()
                    && valid_media_type(&profile.media_type)
                    && (1..=256).contains(&profile.printer_ids.len())
                    && unique(&profile.printer_ids)
                    && profile.printer_ids.iter().all(|id| {
                        !id.is_empty() && id.len() <= 128 && id.is_ascii() && id.starts_with("ptr_")
                    })
            })
            && !packet.implementation_version.is_empty()
            && packet.implementation_version.len() <= 128
            && packet.implementation_version.is_ascii()
    });
    if !valid_text(&value.renderer_abi)
        || !valid_text(&value.resource_abi)
        || !valid_media(&value.image_media_types)
        || value
            .image_media_types
            .iter()
            .any(|media| media != "image/jpeg")
        || !value.font_media_types.is_empty()
        || value.font_rendering
        || value.cached_resource_digests.len() > 256
        || value.cached_resource_digests.iter().any(|digest| {
            digest.len() != 64
                || !digest.bytes().all(|b| b.is_ascii_hexdigit())
                || digest != &digest.to_ascii_lowercase()
        })
        || !valid_packet
    {
        return Err(AppError::invalid(
            "invalid_document_render_capabilities",
            "Document render capabilities exceed protocol limits.",
        ));
    }
    Ok(value)
}

fn valid_media_type(value: &str) -> bool {
    let Some((kind, subtype)) = value.split_once('/') else {
        return false;
    };
    !kind.is_empty()
        && !subtype.is_empty()
        && value.len() <= 64
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'/' | b'.' | b'+' | b'-')
        })
}

fn adaptive_poll_after_ms(request: &AgentSyncRequest, has_immediate_work: bool) -> u64 {
    let uptime = request
        .health
        .observed_at
        .signed_duration_since(request.health.started_at)
        .num_seconds();
    // Stable per-agent/per-minute jitter avoids synchronized idle fleets without
    // requiring a new protocol field or nondeterministic test seam.
    let minute = request.health.observed_at.timestamp() / 60;
    let seed = request
        .agent_id
        .to_string()
        .bytes()
        .fold(minute.unsigned_abs(), |value, byte| {
            value.wrapping_mul(31).wrapping_add(u64::from(byte))
        });
    adaptive_poll_with_jitter(uptime, has_immediate_work, seed)
}

fn adaptive_poll_with_jitter(uptime_seconds: i64, has_immediate_work: bool, seed: u64) -> u64 {
    if has_immediate_work {
        return 1_000;
    }
    let base = if uptime_seconds < 15 * 60 {
        15_000_i64
    } else {
        60_000_i64
    };
    let jitter_percent = i64::try_from(seed % 41).unwrap_or(20) - 20;
    u64::try_from(base + (base * jitter_percent / 100))
        .unwrap_or(15_000)
        .clamp(1_000, 60_000)
}

#[cfg(test)]
#[allow(
    clippy::items_after_test_module,
    reason = "adaptive polling tests stay adjacent to the private policy helper"
)]
mod adaptive_poll_tests {
    use super::{
        adaptive_poll_with_jitter, supports_print_packet_offer, supports_printer_native_offer,
        validate_document_render_capabilities,
    };
    use piqae_protocol::agent::{
        DocumentRenderCapabilities, PrintPacketCapabilitiesV2, PrintPacketLimits,
        PrintPacketNodeRender, PrintPacketOutputProfile, PrintPacketResourceDescriptor,
        PrinterNativeLanguageProfile,
    };

    fn packet_capabilities() -> DocumentRenderCapabilities {
        DocumentRenderCapabilities {
            renderer_abi: Some("printpacket.pdf-renderer/v1".into()),
            resource_abi: Some("printpacket.resources/v1".into()),
            persistent_cache: true,
            font_rendering: false,
            image_media_types: vec!["image/jpeg".into()],
            font_media_types: vec![],
            cached_resource_digests: vec!["a".repeat(64)],
            print_packet: Some(PrintPacketCapabilitiesV2 {
                negotiation_version: 2,
                supported_packet_versions: vec!["printpacket/v1".into()],
                feature_ids: vec!["typography_base14_windows1252".into()],
                conformance_profiles: vec!["printpacket.conformance/core-v1".into()],
                output_profiles: vec![
                    PrintPacketOutputProfile::Pdf {
                        id: "printpacket.pdf-base14/v1".into(),
                        media_type: "application/pdf".into(),
                    },
                    PrintPacketOutputProfile::PrinterNative {
                        id: "escpos.generic/v1".into(),
                        media_type: "application/vnd.escpos".into(),
                        language_profile_id: "escpos.generic/v1".into(),
                    },
                ],
                deterministic: true,
                limits: PrintPacketLimits {
                    max_template_bytes: 1024,
                    max_input_bytes: 1024,
                    max_output_bytes: 4096,
                    max_pages: 2,
                    max_resource_count: 2,
                    max_resource_bytes: 1024,
                    max_total_resource_bytes: 1024,
                },
                resource_types: vec!["image/jpeg".into()],
                direct_offline: true,
                native_language_profiles: vec![PrinterNativeLanguageProfile {
                    id: "escpos.generic/v1".into(),
                    language: "escpos".into(),
                    version: "1".into(),
                    media_type: "application/vnd.escpos".into(),
                    printer_ids: vec!["ptr_fixture".into()],
                }],
                implementation_version: "0.1.22".into(),
            }),
        }
    }

    #[test]
    fn document_renderer_capabilities_are_truthful_and_bounded() {
        let supported = DocumentRenderCapabilities {
            renderer_abi: Some("printpacket.pdf-renderer/v1".into()),
            resource_abi: Some("printpacket.resources/v1".into()),
            persistent_cache: true,
            font_rendering: false,
            image_media_types: vec!["image/jpeg".into()],
            font_media_types: vec![],
            cached_resource_digests: vec!["a".repeat(64)],
            print_packet: None,
        };
        assert!(validate_document_render_capabilities(&supported).is_ok());
        let mut unsupported = supported.clone();
        unsupported.font_rendering = true;
        unsupported.font_media_types = vec!["font/ttf".into()];
        assert!(validate_document_render_capabilities(&unsupported).is_err());
        let mut unsupported = supported;
        unsupported.image_media_types = vec!["image/png".into()];
        assert!(validate_document_render_capabilities(&unsupported).is_err());
    }

    #[test]
    fn print_packet_capabilities_are_exact_bounded_and_printer_scoped() {
        let supported = packet_capabilities();
        assert!(validate_document_render_capabilities(&supported).is_ok());

        let metadata = std::collections::BTreeMap::from([
            (
                "piqae.printer_native.output_profile".into(),
                "escpos.generic/v1".into(),
            ),
            (
                "piqae.printer_native.language_profile".into(),
                "escpos.generic/v1".into(),
            ),
        ]);
        assert!(supports_printer_native_offer(
            &supported,
            &metadata,
            "ptr_fixture"
        ));
        assert!(!supports_printer_native_offer(
            &supported,
            &metadata,
            "ptr_other"
        ));

        let mut duplicate = supported.clone();
        if let Some(packet) = &mut duplicate.print_packet {
            packet.feature_ids = vec![
                "typography_base14_windows1252".into(),
                "typography_base14_windows1252".into(),
            ];
        }
        assert!(validate_document_render_capabilities(&duplicate).is_err());

        let mut retired_identifier = supported.clone();
        if let Some(packet) = &mut retired_identifier.print_packet {
            packet.supported_packet_versions = vec!["piqae.business-document/v1".into()];
        }
        assert!(validate_document_render_capabilities(&retired_identifier).is_err());

        let mut unbound = supported;
        if let Some(packet) = &mut unbound.print_packet {
            packet.native_language_profiles.clear();
        }
        assert!(validate_document_render_capabilities(&unbound).is_err());
    }

    #[test]
    fn print_packet_offer_intersects_template_and_aggregate_resource_limits() {
        let supported = packet_capabilities();
        let mut render = PrintPacketNodeRender {
            negotiation_version: 2,
            packet_version: "printpacket/v1".into(),
            required_feature_ids: vec!["typography_base14_windows1252".into()],
            conformance_profile: "printpacket.conformance/core-v1".into(),
            output_profile: "printpacket.pdf-base14/v1".into(),
            renderer_abi: "printpacket.pdf-renderer/v1".into(),
            resource_abi: "printpacket.resources/v1".into(),
            specification: serde_json::json!({
                "format": "printpacket/v1",
                "media": {"kind": "paged", "size": "a4"},
                "body": []
            }),
            input: serde_json::json!({}),
            resources: vec![
                PrintPacketResourceDescriptor {
                    digest: "a".repeat(64),
                    media_type: "image/jpeg".into(),
                    byte_length: 500,
                },
                PrintPacketResourceDescriptor {
                    digest: "b".repeat(64),
                    media_type: "image/jpeg".into(),
                    byte_length: 500,
                },
            ],
            expected_pdf_sha256: "c".repeat(64),
            expected_pdf_bytes: 4096,
            expected_page_count: 1,
        };
        assert!(supports_print_packet_offer(&supported, &render));

        render.resources[1].byte_length = 600;
        assert!(!supports_print_packet_offer(&supported, &render));

        render.resources[1].byte_length = 400;
        render.specification = serde_json::json!({"padding": "x".repeat(1024)});
        assert!(!supports_print_packet_offer(&supported, &render));
    }

    #[test]
    fn active_work_always_returns_the_fast_interval() {
        assert_eq!(adaptive_poll_with_jitter(86_400, true, u64::MAX), 1_000);
    }

    #[test]
    fn recent_idle_agents_poll_between_twelve_and_eighteen_seconds() {
        assert_eq!(adaptive_poll_with_jitter(60, false, 0), 12_000);
        assert_eq!(adaptive_poll_with_jitter(60, false, 40), 18_000);
    }

    #[test]
    fn long_idle_agents_back_off_to_at_most_one_minute() {
        assert_eq!(adaptive_poll_with_jitter(3_600, false, 0), 48_000);
        assert_eq!(adaptive_poll_with_jitter(3_600, false, 40), 60_000);
    }
}

pub async fn accept_agent_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(job_id): Path<String>,
    body: Bytes,
) -> Result<Json<AgentAcceptJobResponse>, AppError> {
    let path = format!("/v1/agent/jobs/{job_id}/accept");
    let identity = authenticate_agent(&state, &headers, "POST", &path, &body).await?;
    let request: AgentAcceptJobRequest = serde_json::from_slice(&body)?;
    let (Some(route_reservation_id), Some(route_generation), Some(route_fencing_token)) = (
        request.route_reservation_id,
        request.route_generation,
        request.route_fencing_token.as_deref(),
    ) else {
        return Err(AppError::conflict(
            "route_fence_required",
            "A complete physical-destination reservation proof is required.",
        ));
    };
    let reservation_id = route_reservation_id.to_string();
    let job = state
        .repository
        .accept_agent_job_with_delivery_attempt(
            identity.tenant.workspace_id,
            identity.tenant.environment_id,
            identity.agent_id,
            parse_job_id(&job_id)?,
            request.lease_id,
            &request.lease_token,
            Some(&request.content_sha256),
            request.local_sequence,
            piqae_storage_postgres::DeliveryAttemptProof {
                reservation_id: &reservation_id,
                generation: route_generation,
                fencing_token: route_fencing_token,
            },
        )
        .await?;
    state
        .publish_idempotently(
            &agent_acceptance_webhook_idempotency_key(
                identity.tenant.workspace_id,
                identity.tenant.environment_id,
                job.id,
            ),
            identity.tenant,
            "job.updated",
            &job,
        )
        .await?;
    Ok(Json(AgentAcceptJobResponse {
        accepted_at: Utc::now(),
        state: job.state,
    }))
}

pub async fn reconcile_agent_acceptance(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(job_id): Path<String>,
    body: Bytes,
) -> Result<Json<AgentAcceptanceReconciliationResponse>, AppError> {
    let path = format!("/v1/agent/jobs/{job_id}/acceptance/reconcile");
    let identity =
        authenticate_agent_for_revocation(&state, &headers, "POST", &path, &body).await?;
    let request: AgentAcceptJobRequest = serde_json::from_slice(&body)?;
    let (Some(route_reservation_id), Some(route_generation), Some(route_fencing_token)) = (
        request.route_reservation_id,
        request.route_generation,
        request.route_fencing_token.as_deref(),
    ) else {
        return Err(AppError::conflict(
            "route_fence_required",
            "A complete physical-destination reservation proof is required.",
        ));
    };
    let reservation_id = route_reservation_id.to_string();
    let (accepted, connector_revoked, fenced) = state
        .repository
        .reconcile_agent_acceptance(
            identity.tenant.workspace_id,
            identity.tenant.environment_id,
            identity.agent_id,
            parse_job_id(&job_id)?,
            request.lease_id,
            &request.lease_token,
            &request.content_sha256,
            request.local_sequence,
            piqae_storage_postgres::DeliveryAttemptProof {
                reservation_id: &reservation_id,
                generation: route_generation,
                fencing_token: route_fencing_token,
            },
        )
        .await?;
    if accepted {
        let job_id = parse_job_id(&job_id)?;
        let job = state
            .repository
            .get_job(
                identity.tenant.workspace_id,
                identity.tenant.environment_id,
                job_id,
            )
            .await?;
        state
            .publish_idempotently(
                &agent_acceptance_webhook_idempotency_key(
                    identity.tenant.workspace_id,
                    identity.tenant.environment_id,
                    job_id,
                ),
                identity.tenant,
                "job.updated",
                &job,
            )
            .await?;
    }
    Ok(Json(AgentAcceptanceReconciliationResponse {
        accepted,
        connector_revoked,
        fenced,
    }))
}

pub async fn abandon_agent_acceptance(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(job_id): Path<String>,
    body: Bytes,
) -> Result<Json<AgentAcceptanceAbandonResponse>, AppError> {
    let path = format!("/v1/agent/jobs/{job_id}/acceptance/abandon");
    let identity = authenticate_agent(&state, &headers, "POST", &path, &body).await?;
    let request: AgentAcceptJobRequest = serde_json::from_slice(&body)?;
    let (Some(route_reservation_id), Some(route_generation), Some(route_fencing_token)) = (
        request.route_reservation_id,
        request.route_generation,
        request.route_fencing_token.as_deref(),
    ) else {
        return Err(AppError::conflict(
            "route_fence_required",
            "A complete physical-destination reservation proof is required.",
        ));
    };
    let reservation_id = route_reservation_id.to_string();
    let abandoned = state
        .repository
        .abandon_agent_acceptance(
            identity.tenant.workspace_id,
            identity.tenant.environment_id,
            identity.agent_id,
            parse_job_id(&job_id)?,
            request.lease_id,
            &request.lease_token,
            &request.content_sha256,
            request.local_sequence,
            piqae_storage_postgres::DeliveryAttemptProof {
                reservation_id: &reservation_id,
                generation: route_generation,
                fencing_token: route_fencing_token,
            },
        )
        .await?;
    if abandoned {
        let job = state
            .repository
            .get_job(
                identity.tenant.workspace_id,
                identity.tenant.environment_id,
                parse_job_id(&job_id)?,
            )
            .await?;
        state.publish(identity.tenant, "job.updated", &job).await?;
    }
    Ok(Json(AgentAcceptanceAbandonResponse { abandoned }))
}

pub async fn renew_agent_lease(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(job_id): Path<String>,
    body: Bytes,
) -> Result<Json<AgentRenewLeaseResponse>, AppError> {
    let path = format!("/v1/agent/jobs/{job_id}/lease");
    let identity = authenticate_agent(&state, &headers, "POST", &path, &body).await?;
    let request: AgentRenewLeaseRequest = serde_json::from_slice(&body)?;
    let (Some(route_reservation_id), Some(route_generation), Some(route_fencing_token)) = (
        request.route_reservation_id,
        request.route_generation,
        request.route_fencing_token.as_deref(),
    ) else {
        return Err(AppError::conflict(
            "route_fence_required",
            "A complete physical-destination reservation proof is required.",
        ));
    };
    let reservation_id = route_reservation_id.to_string();
    let lease_expires_at = state
        .repository
        .renew_agent_lease_with_delivery_attempt(
            identity.tenant.workspace_id,
            identity.tenant.environment_id,
            identity.agent_id,
            parse_job_id(&job_id)?,
            request.lease_id,
            &request.lease_token,
            piqae_storage_postgres::DeliveryAttemptProof {
                reservation_id: &reservation_id,
                generation: route_generation,
                fencing_token: route_fencing_token,
            },
        )
        .await?;
    Ok(Json(AgentRenewLeaseResponse { lease_expires_at }))
}

pub async fn release_agent_lease(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(job_id): Path<String>,
    body: Bytes,
) -> Result<StatusCode, AppError> {
    let path = format!("/v1/agent/jobs/{job_id}/release");
    let identity = authenticate_agent(&state, &headers, "POST", &path, &body).await?;
    let request: AgentReleaseLeaseRequest = serde_json::from_slice(&body)?;
    if request.reason.trim().is_empty() {
        return Err(AppError::invalid(
            "invalid_lease_release",
            "A lease release reason is required.",
        ));
    }
    state
        .repository
        .release_agent_lease(
            identity.tenant.workspace_id,
            identity.tenant.environment_id,
            identity.agent_id,
            parse_job_id(&job_id)?,
            request.lease_id,
            &request.lease_token,
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn get_agent_content(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(job_id): Path<String>,
) -> Result<Response, AppError> {
    let path = format!("/v1/agent/jobs/{job_id}/content");
    let identity = authenticate_agent(&state, &headers, "GET", &path, &[]).await?;
    let lease_id = headers
        .get("x-piqae-lease-id")
        .or_else(|| headers.get("x-spool-lease-id"))
        .and_then(|value| value.to_str().ok())
        .and_then(|value| uuid::Uuid::parse_str(value).ok())
        .ok_or_else(|| AppError::device_unauthorized("missing_agent_lease"))?;
    let lease_token = headers
        .get("x-piqae-lease-token")
        .or_else(|| headers.get("x-spool-lease-token"))
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| AppError::device_unauthorized("missing_agent_lease"))?;
    let parsed_job_id = parse_job_id(&job_id)?;
    state
        .repository
        .validate_agent_lease(
            identity.tenant.workspace_id,
            identity.tenant.environment_id,
            identity.agent_id,
            parsed_job_id,
            lease_id,
            lease_token,
        )
        .await
        .map_err(|_| AppError::device_unauthorized("invalid_agent_lease"))?;
    let job = state
        .repository
        .get_job(
            identity.tenant.workspace_id,
            identity.tenant.environment_id,
            parsed_job_id,
        )
        .await?;
    let (ContentSource::Upload { upload_id } | ContentSource::EncryptedUpload { upload_id, .. }) =
        job.content
    else {
        return Err(AppError::invalid(
            "content_not_downloadable",
            "This job does not use uploaded content.",
        ));
    };
    let upload = state
        .repository
        .get_upload(
            identity.tenant.workspace_id,
            identity.tenant.environment_id,
            &upload_id,
        )
        .await?;
    if upload.state != "complete" {
        return Err(AppError::device_unauthorized("job_upload_is_not_complete"));
    }
    let content = state
        .object_store
        .get_stream(&upload.object_key)
        .await
        .map_err(|_| AppError::service_unavailable("object_store_unavailable"))?;
    let stream =
        content.map(|result| result.map_err(|error| std::io::Error::other(error.to_string())));
    Response::builder()
        .header(axum::http::header::CONTENT_TYPE, upload.media_type)
        .header(
            axum::http::header::CONTENT_LENGTH,
            upload.expected_bytes.to_string(),
        )
        .header("digest", format!("sha-256={}", upload.expected_sha256))
        .body(Body::from_stream(stream))
        .map_err(|_| AppError::service_unavailable("content_response_failed"))
}

pub async fn get_agent_document_resource(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((job_id, digest)): Path<(String, String)>,
) -> Result<Response, AppError> {
    let path = format!("/v1/agent/jobs/{job_id}/resources/{digest}");
    let identity = authenticate_agent(&state, &headers, "GET", &path, &[]).await?;
    let lease_id = headers
        .get("x-piqae-lease-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| uuid::Uuid::parse_str(value).ok())
        .ok_or_else(|| AppError::device_unauthorized("missing_agent_lease"))?;
    let lease_token = headers
        .get("x-piqae-lease-token")
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| AppError::device_unauthorized("missing_agent_lease"))?;
    let parsed_job_id = parse_job_id(&job_id)?;
    state
        .repository
        .validate_agent_lease(
            identity.tenant.workspace_id,
            identity.tenant.environment_id,
            identity.agent_id,
            parsed_job_id,
            lease_id,
            lease_token,
        )
        .await
        .map_err(|_| AppError::device_unauthorized("invalid_agent_lease"))?;
    let job = state
        .repository
        .get_job(
            identity.tenant.workspace_id,
            identity.tenant.environment_id,
            parsed_job_id,
        )
        .await?;
    let render_id = job
        .metadata
        .get("piqae.document.render_id")
        .ok_or_else(|| {
            AppError::invalid(
                "resource_not_authorized",
                "Job does not reference a PrintPacket render.",
            )
        })?;
    let payload = crate::documents::node_render_payload(
        &state,
        identity.tenant.workspace_id,
        identity.tenant.environment_id,
        render_id,
    )
    .await?;
    let resource = payload
        .resources
        .iter()
        .find(|resource| resource.digest == digest)
        .ok_or_else(|| {
            AppError::invalid(
                "document_resource_not_found",
                "Resource is not referenced by this job.",
            )
        })?;
    let object_key = crate::documents::document_resource_object_key(
        &identity.tenant.workspace_id.to_string(),
        &identity.tenant.environment_id.to_string(),
        &digest,
    );
    let stream = state
        .object_store
        .get_stream(&object_key)
        .await
        .map_err(|_| AppError::service_unavailable("document_resource_unavailable"))?;
    let stream =
        stream.map(|result| result.map_err(|error| std::io::Error::other(error.to_string())));
    Response::builder()
        .header(
            axum::http::header::CONTENT_TYPE,
            resource.media_type.as_str(),
        )
        .header("x-content-sha256", digest)
        .header("x-content-type-options", "nosniff")
        .body(Body::from_stream(stream))
        .map_err(|_| AppError::service_unavailable("content_response_failed"))
}

pub async fn stream_events(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Sse<impl futures::Stream<Item = Result<Event, Infallible>>>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::JobsRead).await?;
    let repository = state.repository.clone();
    let mut cursor = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let stream = async_stream::stream! {
        loop {
            match repository
                .list_tenant_events(
                    tenant.workspace_id,
                    tenant.environment_id,
                    cursor.as_deref(),
                    100,
                )
                .await
            {
                Ok(events) if events.is_empty() => {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
                Ok(events) => {
                    for event in events {
                        cursor = Some(event.id.clone());
                        yield Ok(Event::default()
                            .id(event.id)
                            .event(event.event_type)
                            .data(event.payload.to_string()));
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "event stream poll failed");
                    yield Ok(Event::default().event("resync_required").data("{}"));
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
            }
        }
    };
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

pub(crate) async fn authenticate_native(
    state: &AppState,
    headers: &HeaderMap,
    required_scope: Scope,
) -> Result<TenantContext, AppError> {
    let authorization = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(AppError::unauthorized)?;
    let platform_workspace =
        optional_product_header(headers, "x-piqae-workspace-id", "x-spool-workspace-id")?;
    let platform_environment =
        optional_product_header(headers, "x-piqae-environment-id", "x-spool-environment-id")?;
    let managed_workspace = optional_header(headers, "x-piqae-managed-workspace-id")?;
    let managed_environment = optional_header(headers, "x-piqae-managed-environment-id")?;
    let dashboard_request = optional_header(headers, "x-piqae-dashboard")? == Some("1");
    let tenant = match (
        platform_workspace,
        platform_environment,
        managed_workspace,
        managed_environment,
    ) {
        (None, None, None, None) => state.authenticator.authenticate_bearer(authorization).await,
        (Some(workspace), Some(environment), None, None) => {
            let workspace_id =
                WorkspaceId::from_str(workspace).map_err(|_| AppError::unauthorized())?;
            let environment_id =
                EnvironmentId::from_str(environment).map_err(|_| AppError::unauthorized())?;
            let request_id = crate::request_id::current();
            state
                .authenticator
                .authenticate_platform_bearer(
                    authorization,
                    workspace_id,
                    environment_id,
                    required_scope,
                    &request_id,
                )
                .await
        }
        (None, None, Some(workspace), Some(environment)) if dashboard_request => {
            let workspace_id =
                WorkspaceId::from_str(workspace).map_err(|_| AppError::unauthorized())?;
            let environment_id =
                EnvironmentId::from_str(environment).map_err(|_| AppError::unauthorized())?;
            state
                .authenticator
                .authenticate_managed_human(
                    authorization,
                    workspace_id,
                    environment_id,
                    required_scope,
                )
                .await
        }
        _ => Err(crate::authentication::AuthenticationError),
    }
    .map_err(|_| AppError::unauthorized())?;
    if !tenant.allows(required_scope) {
        return Err(AppError::forbidden());
    }
    Ok(tenant)
}

fn optional_header<'a>(headers: &'a HeaderMap, name: &str) -> Result<Option<&'a str>, AppError> {
    headers
        .get(name)
        .map(|value| value.to_str().map_err(|_| AppError::unauthorized()))
        .transpose()
}

fn optional_product_header<'a>(
    headers: &'a HeaderMap,
    name: &str,
    legacy_name: &str,
) -> Result<Option<&'a str>, AppError> {
    let canonical = optional_header(headers, name)?;
    let legacy = optional_header(headers, legacy_name)?;
    match (canonical, legacy) {
        (Some(canonical), Some(legacy)) if canonical != legacy => Err(AppError::unauthorized()),
        (Some(value), _) | (_, Some(value)) => Ok(Some(value)),
        (None, None) => Ok(None),
    }
}

pub(crate) async fn authenticate_compatibility(
    state: &AppState,
    headers: &HeaderMap,
    required_scope: Scope,
) -> Result<TenantContext, AppError> {
    let authorization = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(AppError::compatibility_unauthorized)?;
    let tenant = state
        .authenticator
        .authenticate_basic(authorization)
        .await
        .map_err(|_| AppError::compatibility_unauthorized())?;
    if !tenant.allows(required_scope) {
        return Err(AppError::forbidden().compatibility());
    }
    Ok(tenant)
}

fn validate_create(
    request: &CreateJobRequest,
    allow_reserved_metadata: bool,
) -> Result<(), AppError> {
    if !allow_reserved_metadata && request.metadata.keys().any(|key| key.starts_with("piqae.")) {
        return Err(AppError::invalid(
            "reserved_metadata_key",
            "Metadata keys beginning with piqae. are reserved by the control plane.",
        ));
    }
    let has_printer = request
        .printer_id
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty());
    let has_target = request
        .target_id
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty());
    if has_printer == has_target {
        return Err(AppError::invalid(
            "invalid_destination",
            "Provide exactly one printer_id or target_id.",
        ));
    }
    if request.title.trim().is_empty() || request.title.len() > 255 {
        return Err(AppError::invalid(
            "invalid_title",
            "Title must contain between 1 and 255 bytes.",
        ));
    }
    if !(1..=100).contains(&request.deliveries) {
        return Err(AppError::invalid(
            "invalid_deliveries",
            "Deliveries must be between 1 and 100.",
        ));
    }
    if !(1..=1_209_600).contains(&request.expire_after_seconds) {
        return Err(AppError::invalid(
            "invalid_expiry",
            "Expiry must be between 1 and 1209600 seconds.",
        ));
    }
    if request.content_type == ContentKind::Raw && request.options != JobOptions::default() {
        return Err(AppError::invalid(
            "raw_options_not_supported",
            "Native RAW jobs cannot include driver options.",
        ));
    }
    match (request.content_type, request.printer_native.as_ref()) {
        (ContentKind::Raw, None) => {
            return Err(AppError::invalid(
                "printer_native_descriptor_required",
                "Native RAW jobs require an exact printer-native output and language profile.",
            ));
        }
        (ContentKind::Pdf, Some(_)) => {
            return Err(AppError::invalid(
                "printer_native_descriptor_not_allowed",
                "A printer-native descriptor cannot be attached to a PDF job.",
            ));
        }
        (ContentKind::Raw, Some(_)) | (ContentKind::Pdf, None) => {}
    }
    Ok(())
}

async fn validate_printer_native_job(
    state: &AppState,
    tenant: TenantContext,
    request: &CreateJobRequest,
    destination: &ResolvedJobDestination,
) -> Result<BTreeMap<String, String>, AppError> {
    let Some(descriptor) = request.printer_native.as_ref() else {
        return Ok(BTreeMap::new());
    };
    if matches!(request.content, ContentSource::EncryptedUpload { .. }) {
        return Err(AppError::invalid(
            "encrypted_printer_native_unsupported",
            "Encrypted job v3 does not bind printer-native language profiles.",
        ));
    }
    let agent = state
        .repository
        .get_agent(
            tenant.workspace_id,
            tenant.environment_id,
            destination.agent_id,
        )
        .await?;
    if !crate::routing::agent_is_connected(&agent) {
        return Err(AppError::conflict(
            "printer_native_capability_unavailable",
            "The destination node must be online with a current printer-native capability report.",
        ));
    }
    let capabilities = match state
        .repository
        .document_render_capabilities_for_printer(
            tenant.workspace_id,
            tenant.environment_id,
            destination.printer_id,
        )
        .await
    {
        Ok(capabilities) => capabilities,
        Err(RepositoryError::NotFound) => {
            return Err(AppError::conflict(
                "printer_native_capability_unavailable",
                "The destination node must be online with a current printer-native capability report.",
            ));
        }
        Err(error) => return Err(error.into()),
    };
    let packet = capabilities.print_packet.as_ref().ok_or_else(|| {
        AppError::conflict(
            "node_update_required",
            "The destination node must be updated for language-bound printer-native jobs.",
        )
    })?;
    let output = packet
        .output_profiles
        .iter()
        .find_map(|profile| match profile {
            piqae_protocol::agent::PrintPacketOutputProfile::PrinterNative {
                id,
                media_type,
                language_profile_id,
            } if id == &descriptor.output_profile_id
                && language_profile_id == &descriptor.language_profile_id =>
            {
                Some(media_type)
            }
            _ => None,
        });
    let language = packet.native_language_profiles.iter().find(|profile| {
        profile.id == descriptor.language_profile_id
            && profile
                .printer_ids
                .iter()
                .any(|printer_id| printer_id == &destination.printer_id.to_string())
    });
    if output.is_none()
        || language.is_none()
        || output != language.map(|profile| &profile.media_type)
    {
        return Err(AppError::conflict(
            "printer_native_profile_unsupported",
            "The exact printer/output/language profile is not currently advertised by this destination.",
        ));
    }
    Ok(BTreeMap::from([
        (
            "piqae.printer_native.output_profile".into(),
            descriptor.output_profile_id.clone(),
        ),
        (
            "piqae.printer_native.language_profile".into(),
            descriptor.language_profile_id.clone(),
        ),
    ]))
}

async fn validate_resolved_ticket(
    state: &AppState,
    tenant: TenantContext,
    request: &CreateJobRequest,
    destination: &ResolvedJobDestination,
) -> Result<Option<piqae_storage_postgres::StoredResolvedPrintTicket>, AppError> {
    let Some(digest) = request.resolved_ticket_digest.as_deref() else {
        return Ok(None);
    };
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(AppError::invalid(
            "invalid_resolved_ticket",
            "Resolved ticket digest is invalid.",
        ));
    }
    let ticket = state
        .repository
        .get_resolved_print_ticket(tenant.workspace_id, tenant.environment_id, digest)
        .await?;
    if ticket.expires_at <= Utc::now() || ticket.printer_id != destination.printer_id {
        return Err(AppError::conflict(
            "resolved_ticket_stale",
            "The resolved ticket expired or targets a different printer.",
        ));
    }
    let current = state
        .repository
        .get_printer(
            tenant.workspace_id,
            tenant.environment_id,
            destination.printer_id,
        )
        .await?;
    if current.capability_revision != ticket.capability_revision {
        return Err(AppError::conflict(
            "stale_capability_revision",
            "Printer capabilities changed after resolution.",
        ));
    }
    let resolved: JobOptions = serde_json::from_value(
        ticket.display_ticket["resolved_options"].clone(),
    )
    .map_err(|_| {
        AppError::conflict(
            "invalid_resolved_ticket",
            "Stored resolved options are invalid.",
        )
    })?;
    if resolved != request.options {
        return Err(AppError::conflict(
            "resolved_options_mismatch",
            "Job options differ from the resolved ticket.",
        ));
    }
    Ok(Some(ticket))
}

pub(crate) fn parse_job_id(value: &str) -> Result<JobId, AppError> {
    JobId::from_str(value)
        .map_err(|_| AppError::invalid("invalid_job_id", "The job ID is invalid."))
}
