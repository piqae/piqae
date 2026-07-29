#![allow(clippy::missing_errors_doc)]

use crate::{
    AppState,
    authentication::TenantContext,
    device_auth::authenticate_agent,
    error::AppError,
    repository::{CreateResult, RepositoryError},
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
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{Duration, Utc};
use futures::StreamExt;
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use spool_auth::{Environment, Scope, generate_api_key};
use spool_domain::{
    AgentId, ContentKind, ContentSource, EnvironmentId, Job, JobEvent, JobId, JobOptions, JobState,
    PrinterId, PrinterState, WorkspaceId,
};
use spool_object_store::{ObjectByteStream, ObjectStoreError, digest_hex};
use spool_protocol::agent::{
    AgentAcceptJobRequest, AgentAcceptJobResponse, AgentReleaseLeaseRequest,
    AgentRenewLeaseRequest, AgentRenewLeaseResponse, AgentSyncRequest, AgentSyncResponse,
    ContentDescriptor, EnrolRequest, EnrolResponse, JobOffer,
};
use spool_storage_postgres::{
    StoredAgent, StoredApiKey, StoredPrinter, StoredTargetBinding, StoredUpload, StoredWebhook,
    StoredWebhookDelivery, SyncedPrinter,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    convert::Infallible,
    str::FromStr,
};

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    status: &'static str,
    version: &'static str,
}

pub async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
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
    Path(node_id): Path<AgentId>,
) -> Result<Json<StoredAgent>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::AgentsRead).await?;
    Ok(Json(
        state
            .repository
            .get_agent(tenant.workspace_id, tenant.environment_id, node_id)
            .await?,
    ))
}

#[derive(Debug, Deserialize)]
pub struct PatchNodeRequest {
    name: Option<String>,
}

pub async fn patch_node(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(node_id): Path<AgentId>,
    Json(request): Json<PatchNodeRequest>,
) -> Result<Json<StoredAgent>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::AgentsWrite).await?;
    let name = request.name.as_deref().map(str::trim).ok_or_else(|| {
        AppError::invalid("invalid_node", "A node name is required for this update.")
    })?;
    if name.is_empty() || name.chars().count() > 120 {
        return Err(AppError::invalid(
            "invalid_node",
            "The node name is outside the supported limits.",
        ));
    }
    let node = state
        .repository
        .rename_agent(tenant.workspace_id, tenant.environment_id, node_id, name)
        .await?;
    state.publish(tenant, "node.updated", &node).await?;
    Ok(Json(node))
}

pub async fn delete_node(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(node_id): Path<AgentId>,
) -> Result<StatusCode, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::AgentsWrite).await?;
    state
        .repository
        .revoke_agent(tenant.workspace_id, tenant.environment_id, node_id)
        .await?;
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
        spool_protocol::agent::AgentCommand::Pause,
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
        spool_protocol::agent::AgentCommand::Resume,
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
        .enqueue_agent_command(
            tenant.workspace_id,
            tenant.environment_id,
            node_id,
            &spool_protocol::agent::AgentCommand::CollectDiagnostics {
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

async fn enqueue_node_command(
    state: &AppState,
    headers: &HeaderMap,
    node_id: AgentId,
    command: spool_protocol::agent::AgentCommand,
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
    let token = format!("spl_enr_{}", URL_SAFE_NO_PAD.encode(secret));
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
    let public_key = URL_SAFE_NO_PAD
        .decode(&request.public_key)
        .or_else(|_| base64::engine::general_purpose::STANDARD.decode(&request.public_key))
        .map_err(|_| AppError::invalid("invalid_agent_public_key", "Public key is invalid."))?;
    if public_key.len() != 32 {
        return Err(AppError::invalid(
            "invalid_agent_public_key",
            "Public key must contain exactly 32 bytes.",
        ));
    }
    let secret_hash = format!("{:x}", Sha256::digest(request.token.as_bytes()));
    let enrolled = state
        .repository
        .enrol_agent(
            &secret_hash,
            &public_key,
            &request.name,
            &request.hostname,
            &request.platform,
            &request.architecture,
            &request.agent_version,
            request.protocol_version,
        )
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(EnrolResponse {
            agent_id: enrolled.agent_id,
            environment: enrolled.environment_id.to_string(),
            server_time: Utc::now(),
            sync_after_ms: 0,
        }),
    )
        .into_response())
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
    pub content: ContentSource,
    #[serde(default)]
    pub options: JobOptions,
    #[serde(default = "default_deliveries")]
    pub deliveries: u16,
    #[serde(default = "default_expiry")]
    pub expire_after_seconds: i64,
    #[serde(default)]
    pub metadata: std::collections::BTreeMap<String, String>,
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
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    #[serde(default = "default_limit")]
    limit: i64,
    after: Option<String>,
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
    let tenant = authenticate_native(&state, &headers, Scope::JobsWrite).await?;
    validate_create(&request)?;
    let destination = resolve_job_destination(&state, tenant, &request).await?;
    let request_bytes = serde_json::to_vec(&request)?;
    let now = Utc::now();
    let content =
        persist_job_content(&state, tenant, request.content_type, request.content).await?;
    let mut metadata = request.metadata;
    metadata.extend(destination.metadata);
    let job = Job {
        id: JobId::new(),
        workspace_id: tenant.workspace_id,
        environment_id: tenant.environment_id,
        printer_id: destination.printer_id,
        title: request.title,
        source: request.source,
        content_kind: request.content_type,
        content,
        options: request.options,
        metadata,
        deliveries: request.deliveries,
        state: JobState::Registered,
        created_at: now,
        expires_at: now + Duration::seconds(request.expire_after_seconds),
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
    match state
        .repository
        .create_job(&job, destination.agent_id, idempotency, &request_bytes)
        .await?
    {
        CreateResult::Existing(existing) => {
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
            Ok(ResolvedJobDestination {
                printer_id,
                agent_id,
                metadata: BTreeMap::new(),
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
            ("spool.target_id".into(), target.id.clone()),
            ("spool.binding_id".into(), binding.id.clone()),
            ("spool.profile_id".into(), profile.profile_id.clone()),
            (
                "spool.profile_revision".into(),
                profile.revision.to_string(),
            ),
        ]);
        if let Some(stock_id) = target.stock_id.as_ref().or(profile.stock_id.as_ref()) {
            metadata.insert("spool.stock_id".into(), stock_id.clone());
        }
        let destination = ResolvedJobDestination {
            printer_id: printer.id,
            agent_id: printer.agent_id,
            metadata,
            binding: Some(binding),
        };
        if agent_ready && printer.state == PrinterState::Online {
            return Ok(destination);
        }
        if allow_offline && configured_fallback.is_none() {
            configured_fallback = Some(destination);
        }
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

async fn recover_waiting_target_jobs(
    state: &AppState,
    tenant: TenantContext,
) -> Result<(), AppError> {
    let jobs = state
        .repository
        .list_reroutable_target_jobs(tenant.workspace_id, tenant.environment_id, 100)
        .await?;
    for job in jobs {
        let Some(target_id) = job.metadata.get("spool.target_id") else {
            continue;
        };
        let Ok(destination) = resolve_target_destination(state, tenant, target_id, false).await
        else {
            continue;
        };
        let Some(binding) = destination.binding.as_ref() else {
            continue;
        };
        match state
            .repository
            .reroute_job_before_acceptance(
                tenant.workspace_id,
                tenant.environment_id,
                job.id,
                target_id,
                binding,
                "standby_recovery",
            )
            .await
        {
            Ok(Some(rerouted)) => {
                state
                    .publish(tenant, "job.routing_attempted", &rerouted)
                    .await?;
            }
            Ok(None) | Err(RepositoryError::ConcurrentStateChange) => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

pub(crate) async fn persist_job_content(
    state: &AppState,
    tenant: TenantContext,
    content_kind: ContentKind,
    content: ContentSource,
) -> Result<ContentSource, AppError> {
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
            Ok(ContentSource::Upload { upload_id })
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
            Ok(ContentSource::Upload { upload_id: id })
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
            Ok(ContentSource::Uri {
                uri,
                authentication: None,
            })
        }
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
    let mut jobs = state
        .repository
        .list_jobs(tenant.workspace_id, tenant.environment_id, after, limit + 1)
        .await?;
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
    state.publish(tenant, "job.updated", &job).await?;
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
                profiles: printer
                    .profiles
                    .iter()
                    .map(|profile| spool_storage_postgres::PrinterProfileSnapshot {
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
            printers.as_deref(),
        )
        .await?;
    recover_waiting_target_jobs(&state, tenant).await?;
    for event in &request.events {
        match state
            .repository
            .apply_agent_event(
                tenant.workspace_id,
                tenant.environment_id,
                request.agent_id,
                event,
            )
            .await
        {
            Ok(Some(job)) => state.publish(tenant, "job.updated", &job).await?,
            Ok(None) | Err(RepositoryError::ConcurrentStateChange) => {}
            Err(error) => return Err(error.into()),
        }
    }
    let command_batch = state
        .repository
        .sync_agent_commands(
            tenant.workspace_id,
            tenant.environment_id,
            request.agent_id,
            request.acknowledged_command_cursor.as_deref(),
            100,
        )
        .await?;
    let leases = if request.queue.accepts_jobs {
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
        let content = match &lease.job.content {
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
        candidate_jobs.push(JobOffer {
            job: lease.job,
            lease_id: lease.lease_id,
            lease_token: lease.lease_token,
            lease_expires_at: lease.lease_until,
            content,
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
    }))
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
    use super::adaptive_poll_with_jitter;

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
    let job = state
        .repository
        .accept_agent_job(
            identity.tenant.workspace_id,
            identity.tenant.environment_id,
            identity.agent_id,
            parse_job_id(&job_id)?,
            request.lease_id,
            &request.lease_token,
            Some(&request.content_sha256),
            request.local_sequence,
        )
        .await?;
    state.publish(identity.tenant, "job.updated", &job).await?;
    Ok(Json(AgentAcceptJobResponse {
        accepted_at: Utc::now(),
        state: job.state,
    }))
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
    let lease_expires_at = state
        .repository
        .renew_agent_lease(
            identity.tenant.workspace_id,
            identity.tenant.environment_id,
            identity.agent_id,
            parse_job_id(&job_id)?,
            request.lease_id,
            &request.lease_token,
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
        .get("x-spool-lease-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| uuid::Uuid::parse_str(value).ok())
        .ok_or_else(|| AppError::device_unauthorized("missing_agent_lease"))?;
    let lease_token = headers
        .get("x-spool-lease-token")
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
    let ContentSource::Upload { upload_id } = job.content else {
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
    let platform_workspace = headers
        .get("x-spool-workspace-id")
        .and_then(|value| value.to_str().ok());
    let platform_environment = headers
        .get("x-spool-environment-id")
        .and_then(|value| value.to_str().ok());
    let tenant = match (platform_workspace, platform_environment) {
        (None, None) => state.authenticator.authenticate_bearer(authorization).await,
        (Some(workspace), Some(environment)) => {
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
        _ => Err(crate::authentication::AuthenticationError),
    }
    .map_err(|_| AppError::unauthorized())?;
    if !tenant.allows(required_scope) {
        return Err(AppError::forbidden());
    }
    Ok(tenant)
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

fn validate_create(request: &CreateJobRequest) -> Result<(), AppError> {
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
    Ok(())
}

pub(crate) fn parse_job_id(value: &str) -> Result<JobId, AppError> {
    JobId::from_str(value)
        .map_err(|_| AppError::invalid("invalid_job_id", "The job ID is invalid."))
}
