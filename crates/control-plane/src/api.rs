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
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{
        IntoResponse, Response, Sse,
        sse::{Event, KeepAlive},
    },
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{Duration, Utc};
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use spool_domain::{
    ContentKind, ContentSource, Job, JobEvent, JobId, JobOptions, JobState, PrinterId,
};
use spool_object_store::digest_hex;
use spool_protocol::agent::{
    AgentAcceptJobRequest, AgentAcceptJobResponse, AgentReleaseLeaseRequest,
    AgentRenewLeaseRequest, AgentRenewLeaseResponse, AgentSyncRequest, AgentSyncResponse,
    ContentDescriptor, EnrolRequest, EnrolResponse, JobOffer,
};
use spool_storage_postgres::{
    StoredAgent, StoredPrinter, StoredUpload, StoredWebhook, StoredWebhookDelivery,
};
use std::{convert::Infallible, str::FromStr};

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

pub async fn ready(State(state): State<AppState>) -> Result<Json<HealthResponse>, AppError> {
    state.repository.ready().await?;
    Ok(health().await)
}

pub async fn list_agents(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<StoredAgent>>, AppError> {
    let tenant = authenticate_native(&state, &headers).await?;
    Ok(Json(
        state
            .repository
            .list_agents(tenant.workspace_id, tenant.environment_id)
            .await?,
    ))
}

pub async fn list_printers(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> Result<Json<Page<StoredPrinter>>, AppError> {
    let tenant = authenticate_native(&state, &headers).await?;
    let limit = query.limit.clamp(1, 500);
    let mut printers = state
        .repository
        .list_printers(tenant.workspace_id, tenant.environment_id, limit + 1)
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
    let tenant = authenticate_native(&state, &headers).await?;
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
    state.publish(
        tenant,
        "agent_enrolment.created",
        &serde_json::json!({"id": id, "name": request.name, "expires_at": expires_at}),
    );
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
    let tenant = authenticate_native(&state, &headers).await?;
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
    let tenant = authenticate_native(&state, &headers).await?;
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
    state.publish(tenant, "webhook.created", &webhook);
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
    let tenant = authenticate_native(&state, &headers).await?;
    state
        .repository
        .delete_webhook(tenant.workspace_id, tenant.environment_id, &webhook_id)
        .await?;
    state.publish(
        tenant,
        "webhook.deleted",
        &serde_json::json!({"id": webhook_id}),
    );
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_webhook_deliveries(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(webhook_id): Path<String>,
) -> Result<Json<Vec<StoredWebhookDelivery>>, AppError> {
    let tenant = authenticate_native(&state, &headers).await?;
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
    let tenant = authenticate_native(&state, &headers).await?;
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
    upload_url: Option<String>,
}

pub async fn create_upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateUploadRequest>,
) -> Result<Response, AppError> {
    let tenant = authenticate_native(&state, &headers).await?;
    if !matches!(
        request.media_type.as_str(),
        "application/pdf" | "application/octet-stream"
    ) || !(1..=52_428_800).contains(&request.byte_length)
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
            upload_url: Some(format!("/v1/uploads/{id}/content")),
            upload,
        }),
    )
        .into_response())
}

pub async fn upload_content(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(upload_id): Path<String>,
    body: Bytes,
) -> Result<Json<StoredUpload>, AppError> {
    let tenant = authenticate_native(&state, &headers).await?;
    let upload = state
        .repository
        .get_upload(tenant.workspace_id, tenant.environment_id, &upload_id)
        .await?;
    if upload.state != "pending"
        || upload.expires_at <= Utc::now()
        || i64::try_from(body.len()).unwrap_or(i64::MAX) != upload.expected_bytes
    {
        return Err(AppError::invalid(
            "upload_not_writable",
            "Upload is expired, complete, or has the wrong byte length.",
        ));
    }
    state
        .object_store
        .put(&upload.object_key, body, Some(&upload.expected_sha256))
        .await
        .map_err(|error| match error {
            spool_object_store::ObjectStoreError::DigestMismatch => {
                AppError::invalid("upload_digest_mismatch", "Upload digest does not match.")
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
    let tenant = authenticate_native(&state, &headers).await?;
    let upload = state
        .repository
        .get_upload(tenant.workspace_id, tenant.environment_id, &upload_id)
        .await?;
    let bytes = state
        .object_store
        .get(&upload.object_key)
        .await
        .map_err(|_| AppError::service_unavailable("object_store_unavailable"))?;
    let actual_sha256 = digest_hex(&bytes);
    let actual_bytes = i64::try_from(bytes.len()).unwrap_or(i64::MAX);
    if !request.sha256.eq_ignore_ascii_case(&actual_sha256) || request.byte_length != actual_bytes {
        return Err(AppError::invalid(
            "upload_verification_failed",
            "Stored object does not match completion metadata.",
        ));
    }
    Ok(Json(
        state
            .repository
            .complete_upload(
                tenant.workspace_id,
                tenant.environment_id,
                &upload_id,
                &actual_sha256,
                actual_bytes,
            )
            .await?,
    ))
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CreateJobRequest {
    pub printer_id: String,
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
    #[allow(dead_code)]
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
    let tenant = authenticate_native(&state, &headers).await?;
    validate_create(&request)?;
    let printer_id = PrinterId::from_str(&request.printer_id)
        .map_err(|_| AppError::invalid("invalid_printer_id", "The printer ID is invalid."))?;
    let agent_id = state
        .repository
        .resolve_printer_agent(tenant.workspace_id, tenant.environment_id, printer_id)
        .await?;
    let request_bytes = serde_json::to_vec(&request)?;
    let now = Utc::now();
    let job = Job {
        id: JobId::new(),
        workspace_id: tenant.workspace_id,
        environment_id: tenant.environment_id,
        printer_id,
        title: request.title,
        source: request.source,
        content_kind: request.content_type,
        content: request.content,
        options: request.options,
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
        .create_job(&job, agent_id, idempotency, &request_bytes)
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
            state.publish(tenant, "job.updated", &queued);
            Ok((StatusCode::CREATED, Json(JobResponse::from(created))).into_response())
        }
    }
}

pub async fn list_jobs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> Result<Json<Page<JobResponse>>, AppError> {
    let tenant = authenticate_native(&state, &headers).await?;
    let limit = query.limit.clamp(1, 500);
    let mut jobs = state
        .repository
        .list_jobs(tenant.workspace_id, tenant.environment_id, limit + 1)
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
    let tenant = authenticate_native(&state, &headers).await?;
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
    let tenant = authenticate_native(&state, &headers).await?;
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
    let tenant = authenticate_native(&state, &headers).await?;
    let job = state
        .repository
        .transition_job(
            tenant.workspace_id,
            tenant.environment_id,
            parse_job_id(&job_id)?,
            JobState::CancelRequested,
            None,
            Some("Cancellation requested by API caller".into()),
            None,
            None,
        )
        .await?;
    state.publish(tenant, "job.updated", &job);
    Ok((StatusCode::ACCEPTED, Json(JobResponse::from(job))).into_response())
}

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
    let tenant = identity.tenant;
    for event in &request.events {
        match state
            .repository
            .transition_job(
                tenant.workspace_id,
                tenant.environment_id,
                event.job_id,
                event.state,
                event.reason.clone(),
                event.message.clone(),
                Some(request.agent_id),
                event.native_job_id.clone(),
            )
            .await
        {
            Ok(job) => state.publish(tenant, "job.updated", &job),
            Err(RepositoryError::ConcurrentStateChange) => {}
            Err(error) => return Err(error.into()),
        }
    }
    let leases = if request.queue.accepts_jobs {
        state
            .repository
            .claim_jobs(
                tenant.workspace_id,
                tenant.environment_id,
                request.agent_id,
                &format!("{}:{}", request.agent_id, request.agent_version),
                20,
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
    Ok(Json(AgentSyncResponse {
        server_time: Utc::now(),
        acknowledged_event_cursor: request.events.last().map(|event| event.id),
        command_cursor: request.acknowledged_command_cursor,
        commands: Vec::new(),
        candidate_jobs,
        next_poll_after_ms: 250,
    }))
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
    state.publish(identity.tenant, "job.updated", &job);
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
    let job = state
        .repository
        .get_job(
            identity.tenant.workspace_id,
            identity.tenant.environment_id,
            parse_job_id(&job_id)?,
        )
        .await?;
    let target_agent = state
        .repository
        .resolve_printer_agent(
            identity.tenant.workspace_id,
            identity.tenant.environment_id,
            job.printer_id,
        )
        .await?;
    if target_agent != identity.agent_id {
        return Err(AppError::device_unauthorized("agent_job_mismatch"));
    }
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
    let content = state
        .object_store
        .get(&upload.object_key)
        .await
        .map_err(|_| AppError::service_unavailable("object_store_unavailable"))?;
    Ok((
        [
            (axum::http::header::CONTENT_TYPE, upload.media_type),
            (
                axum::http::HeaderName::from_static("digest"),
                format!("sha-256={}", upload.expected_sha256),
            ),
        ],
        content,
    )
        .into_response())
}

pub async fn stream_events(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Sse<impl futures::Stream<Item = Result<Event, Infallible>>>, AppError> {
    let tenant = authenticate_native(&state, &headers).await?;
    let mut receiver = state.events.subscribe();
    let stream = async_stream::stream! {
        loop {
            match receiver.recv().await {
                Ok(event) if event.tenant.workspace_id == tenant.workspace_id
                    && event.tenant.environment_id == tenant.environment_id => {
                        yield Ok(Event::default()
                            .id(event.id)
                            .event(event.event_type)
                            .data(event.data.to_string()));
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    yield Ok(Event::default().event("resync_required").data("{}"));
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    };
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

pub(crate) async fn authenticate_native(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<TenantContext, AppError> {
    let authorization = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(AppError::unauthorized)?;
    state
        .authenticator
        .authenticate_bearer(authorization)
        .await
        .map_err(|_| AppError::unauthorized())
}

pub(crate) async fn authenticate_compatibility(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<TenantContext, AppError> {
    let authorization = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(AppError::compatibility_unauthorized)?;
    state
        .authenticator
        .authenticate_basic(authorization)
        .await
        .map_err(|_| AppError::compatibility_unauthorized())
}

fn validate_create(request: &CreateJobRequest) -> Result<(), AppError> {
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
