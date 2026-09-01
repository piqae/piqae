use crate::{
    AppState,
    api::{
        CreateJobRequest, authenticate_native, create_job_internal_with_target_binding,
        resolve_target_printpacket_printer,
    },
    authentication::TenantContext,
    error::AppError,
};
use axum::{
    Json, Router,
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use bytes::BytesMut;
use futures::StreamExt as _;
use piqae_auth::Scope;
use piqae_domain::{ContentKind, ContentSource, Job, JobOptions};
use piqae_protocol::agent::{PrintPacketNodeRender, PrintPacketResourceDescriptor};
use piqae_storage_postgres::{CreateDocumentResult, StoredDocumentPreview, StoredDocumentRender};
use printpacket_renderer::{PRINT_PACKET_DOCUMENT_FORMAT, PrintPacketV1};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum RenderPolicy {
    #[default]
    Automatic,
    CloudOnly,
    PreferNode,
    RequireNode,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RenderCost {
    document_count: u32,
    page_count: u32,
    pdf_bytes: u64,
    input_bytes: u64,
}

fn render_decision(
    policy: RenderPolicy,
    cost: Option<&RenderCost>,
) -> (&'static str, &'static str) {
    match policy {
        RenderPolicy::CloudOnly => ("cloud_pdf", "policy_cloud_only"),
        RenderPolicy::PreferNode => ("node_render", "policy_prefer_node"),
        RenderPolicy::RequireNode => ("node_render", "policy_require_node"),
        RenderPolicy::Automatic => {
            let Some(cost) = cost else {
                return ("cloud_pdf", "automatic_missing_measurements");
            };
            // Conservative deterministic model: a warm node saves transfer
            // only when the approved PDF is materially larger than compact
            // input. Constants are bounded and versioned here, not guessed
            // from ambient network state.
            let cloud_ms = 20_u64.saturating_add(cost.pdf_bytes.saturating_mul(1000) / 12_500_000);
            let node_ms = 35_u64
                .saturating_add(cost.input_bytes.saturating_mul(1000) / 12_500_000)
                .saturating_add(u64::from(cost.page_count).saturating_mul(4))
                .saturating_add(u64::from(cost.document_count).saturating_mul(2));
            if cost.pdf_bytes >= 2 * 1024 * 1024
                && cost.input_bytes.saturating_mul(2) < cost.pdf_bytes
                && node_ms.saturating_add(50) < cloud_ms
            {
                ("node_render", "automatic_measured_node_faster")
            } else {
                ("cloud_pdf", "automatic_measured_cloud_faster")
            }
        }
    }
}

fn normalized_resource_digest(value: &str) -> Option<String> {
    let value = value.strip_prefix("sha256:").unwrap_or(value);
    (value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| value.to_ascii_lowercase())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RenderReadinessRequest {
    printer_id: String,
    #[serde(default)]
    render_policy: RenderPolicy,
    #[serde(default)]
    render_cost: Option<RenderCost>,
}
#[derive(Debug, Serialize)]
struct RenderReadinessResponse {
    requested_policy: RenderPolicy,
    selected_mode: &'static str,
    status: &'static str,
    reason: &'static str,
    approved_pdf_fallback: bool,
    destination: DestinationReadiness,
    estimates: RenderEstimates,
}
#[derive(Debug, Serialize)]
struct DestinationReadiness {
    supported: bool,
    ready: bool,
    missing_features: Vec<String>,
    supported_packet_versions: Vec<String>,
    current_implementation: Option<String>,
    missing_resources: Vec<String>,
    reason: Option<&'static str>,
}
#[derive(Debug, Serialize)]
struct RenderEstimates {
    cloud_ms: u64,
    node_ms: u64,
}

const MAX_DOCUMENT_BYTES: usize = 1024 * 1024;
const MAX_JSON_DEPTH: usize = 64;
const MAX_JSON_NODES: usize = 50_000;
const PRINT_PACKET_NEGOTIATION_VERSION: u16 = 2;
const PRINT_PACKET_VERSION: &str = printpacket::DOCUMENT_V1;
const PRINT_PACKET_CONFORMANCE_PROFILE: &str = printpacket::CONFORMANCE_CORE_V2;
const PRINT_PACKET_OUTPUT_PROFILE: &str = printpacket::PDF_BASE14_V1;
const PRINT_PACKET_RENDERER_ABI: &str = "printpacket.pdf-renderer/v2";
const PRINT_PACKET_RESOURCE_ABI: &str = "printpacket.resources/v1";

fn print_packet_feature_id(feature: &printpacket::Feature) -> String {
    serde_json::to_value(feature)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "invalid_feature_serialization".into())
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/printpacket/templates", post(create_template))
        .route(
            "/v1/printpacket/resources/{digest}",
            put(put_printpacket_resource),
        )
        .route("/v1/printpacket/templates/{template_id}", get(get_template))
        .route(
            "/v1/printpacket/templates/{template_id}/publish",
            post(publish_template),
        )
        .route(
            "/v1/printpacket/template-revisions/{revision_id}",
            get(get_revision),
        )
        .route("/v1/printpacket/renders", post(register_render))
        .route(
            "/v1/printpacket/preview-renders",
            post(register_preview_render),
        )
        .route(
            "/v1/printpacket/preview-renders/{render_id}",
            get(get_preview_render),
        )
        .route(
            "/v1/printpacket/preview-renders/{render_id}/artifact",
            get(download_preview_render_artifact),
        )
        .route("/v1/printpacket/renders/{render_id}", get(get_render))
        .route(
            "/v1/printpacket/renders/{render_id}/readiness",
            post(render_readiness),
        )
        .route(
            "/v1/printpacket/renders/{render_id}/artifact",
            get(download_render_artifact),
        )
        .route(
            "/v1/printpacket/renders/{render_id}/print",
            post(print_render),
        )
        .route(
            "/v1/printpacket/renders/{render_id}/previews",
            post(create_preview),
        )
        .route("/v1/printpacket/previews/{preview_id}", get(get_preview))
        .route(
            "/v1/printpacket/previews/{preview_id}/artifact",
            get(download_preview_artifact),
        )
        .route(
            "/v1/printpacket/previews/{preview_id}/approve",
            post(approve_preview),
        )
        .route(
            "/v1/printpacket/previews/{preview_id}/cancel",
            post(cancel_preview),
        )
}

async fn put_printpacket_resource(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(digest): Path<String>,
    body: axum::body::Body,
) -> Result<StatusCode, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::JobsWrite).await?;
    if digest.len() != 64
        || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
        || digest != digest.to_ascii_lowercase()
    {
        return Err(AppError::invalid(
            "invalid_printpacket_resource",
            "Resource digest is invalid.",
        ));
    }
    let media_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::trim);
    if media_type != Some("image/jpeg") {
        return Err(AppError::invalid(
            "unsupported_printpacket_resource",
            "Renderer ABI v1 accepts image/jpeg resources only.",
        ));
    }
    let bytes = axum::body::to_bytes(body, 4 * 1024 * 1024)
        .await
        .map_err(|_| {
            AppError::invalid("printpacket_resource_too_large", "Resource exceeds 4 MiB.")
        })?;
    if bytes.is_empty() || hex::encode(Sha256::digest(&bytes)) != digest.to_ascii_lowercase() {
        return Err(AppError::invalid(
            "printpacket_resource_digest_mismatch",
            "Resource bytes do not match the URL digest.",
        ));
    }
    let object_key = document_resource_object_key(
        &tenant.workspace_id.to_string(),
        &tenant.environment_id.to_string(),
        &digest,
    );
    let byte_length = i64::try_from(bytes.len()).map_err(|_| {
        AppError::invalid("printpacket_resource_too_large", "Resource exceeds 4 MiB.")
    })?;
    state
        .object_store
        .put(&object_key, bytes, Some(&digest))
        .await
        .map_err(|_| AppError::service_unavailable("object_store_unavailable"))?;
    if let Err(error) = state
        .repository
        .register_printpacket_resource(
            tenant.workspace_id,
            tenant.environment_id,
            &digest.to_ascii_lowercase(),
            "image/jpeg",
            byte_length,
        )
        .await
    {
        let _ = state.object_store.delete(&object_key).await;
        return Err(error.into());
    }
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) fn document_resource_object_key(
    workspace: &str,
    environment: &str,
    digest: &str,
) -> String {
    format!("printpacket/resources/{workspace}/{environment}/{digest}")
}

#[derive(Debug, Deserialize)]
struct CreateTemplateRequest {
    name: String,
    #[serde(alias = "spec")]
    specification: Value,
}

#[derive(Debug, Serialize)]
struct TemplateResponse {
    id: String,
    name: String,
    state: String,
    published_revision_id: Option<String>,
    specification: Value,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

async fn create_template(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateTemplateRequest>,
) -> Result<Response, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::JobsWrite).await?;
    let idempotency_key = required_idempotency_key(&headers)?;
    validate_name(&request.name)?;
    let plaintext = validate_document_spec(&request.specification)?;
    let normalized_specification = serde_json::from_slice(&plaintext)
        .map_err(|_| AppError::service_unavailable("invalid_normalized_document"))?;
    let id = stable_id(
        "dtpl",
        &tenant.workspace_id.to_string(),
        &tenant.environment_id.to_string(),
        &idempotency_key,
    );
    let input_aad = document_aad(
        &tenant.workspace_id.to_string(),
        &tenant.environment_id.to_string(),
        &id,
    );
    let encrypted = state
        .document_secrets
        .encrypt(&input_aad, &plaintext)
        .map_err(|_| AppError::service_unavailable("document_encryption_failed"))?;
    let digest = hex::encode(Sha256::digest(&plaintext));
    let result = state
        .repository
        .create_document_template(
            tenant.workspace_id,
            tenant.environment_id,
            &id,
            request.name.trim(),
            &encrypted,
            &digest,
        )
        .await?;
    let (status, stored) = match result {
        CreateDocumentResult::Created(value) => (StatusCode::CREATED, value),
        CreateDocumentResult::Existing(value) => (StatusCode::OK, value),
    };
    Ok((
        status,
        Json(TemplateResponse {
            id: stored.id,
            name: stored.name,
            state: stored.state,
            published_revision_id: stored.published_revision_id,
            specification: normalized_specification,
            created_at: stored.created_at,
            updated_at: stored.updated_at,
        }),
    )
        .into_response())
}

async fn get_template(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<TemplateResponse>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::JobsRead).await?;
    let stored = state
        .repository
        .get_document_template(tenant.workspace_id, tenant.environment_id, &id)
        .await?;
    let aad = document_aad(
        &tenant.workspace_id.to_string(),
        &tenant.environment_id.to_string(),
        &stored.id,
    );
    let plaintext = state
        .document_secrets
        .decrypt(&aad, &stored.draft_ciphertext)
        .map_err(|_| AppError::service_unavailable("document_decryption_failed"))?;
    let spec = serde_json::from_slice(&plaintext)
        .map_err(|_| AppError::service_unavailable("invalid_stored_document"))?;
    Ok(Json(TemplateResponse {
        id: stored.id,
        name: stored.name,
        state: stored.state,
        published_revision_id: stored.published_revision_id,
        specification: spec,
        created_at: stored.created_at,
        updated_at: stored.updated_at,
    }))
}

#[derive(Debug, Deserialize)]
struct PublishRequest {
    specification: Value,
}

async fn publish_template(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(template_id): Path<String>,
    Json(request): Json<PublishRequest>,
) -> Result<Response, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::JobsWrite).await?;
    let idempotency_key = required_idempotency_key(&headers)?;
    let plaintext = validate_document_spec(&request.specification)?;
    let digest = hex::encode(Sha256::digest(&plaintext));
    let aad = document_aad(
        &tenant.workspace_id.to_string(),
        &tenant.environment_id.to_string(),
        &template_id,
    );
    let ciphertext = state
        .document_secrets
        .encrypt(&aad, &plaintext)
        .map_err(|_| AppError::service_unavailable("document_encryption_failed"))?;
    state
        .repository
        .update_document_template_draft(
            tenant.workspace_id,
            tenant.environment_id,
            &template_id,
            &ciphertext,
            &digest,
        )
        .await?;
    let revision_id = stable_id(
        "drev",
        &tenant.workspace_id.to_string(),
        &template_id,
        &idempotency_key,
    );
    let result = state
        .repository
        .publish_document_template(
            tenant.workspace_id,
            tenant.environment_id,
            &template_id,
            &revision_id,
        )
        .await?;
    let (status, revision) = match result {
        CreateDocumentResult::Created(value) => (StatusCode::CREATED, value),
        CreateDocumentResult::Existing(value) => (StatusCode::OK, value),
    };
    if revision.spec_sha256 != digest {
        return Err(AppError::conflict(
            "idempotency_conflict",
            "The idempotency key was already used with a different document specification.",
        ));
    }
    let stored_plaintext = state
        .document_secrets
        .decrypt(&aad, &revision.spec_ciphertext)
        .map_err(|_| AppError::service_unavailable("document_decryption_failed"))?;
    let stored_specification = serde_json::from_slice(&stored_plaintext)
        .map_err(|_| AppError::service_unavailable("invalid_stored_document"))?;
    Ok((
        status,
        Json(RevisionResponse {
            id: revision.id,
            template_id: revision.template_id,
            revision: revision.revision,
            renderer_profile: revision.renderer_profile,
            specification: stored_specification,
            created_at: revision.created_at,
        }),
    )
        .into_response())
}

#[derive(Debug, Serialize)]
struct RevisionResponse {
    id: String,
    template_id: String,
    revision: i32,
    renderer_profile: String,
    specification: Value,
    created_at: chrono::DateTime<chrono::Utc>,
}

async fn get_revision(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<RevisionResponse>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::JobsRead).await?;
    let revision = state
        .repository
        .get_document_revision(tenant.workspace_id, tenant.environment_id, &id)
        .await?;
    let aad = document_aad(
        &tenant.workspace_id.to_string(),
        &tenant.environment_id.to_string(),
        &revision.template_id,
    );
    let plaintext = state
        .document_secrets
        .decrypt(&aad, &revision.spec_ciphertext)
        .map_err(|_| AppError::service_unavailable("document_decryption_failed"))?;
    let spec = serde_json::from_slice(&plaintext)
        .map_err(|_| AppError::service_unavailable("invalid_stored_document"))?;
    Ok(Json(RevisionResponse {
        id: revision.id,
        template_id: revision.template_id,
        revision: revision.revision,
        renderer_profile: revision.renderer_profile,
        specification: spec,
        created_at: revision.created_at,
    }))
}

#[derive(Debug, Deserialize)]
struct RegisterRenderRequest {
    template_revision_id: String,
    input: Value,
}

/// Explicit public projection. Persistence-only ciphertext and worker lease
/// fields must never become part of the HTTP response through derive changes.
#[derive(Debug, Serialize)]
struct RenderResponse {
    id: String,
    template_revision_id: String,
    state: String,
    artifact_sha256: Option<String>,
    artifact_byte_length: Option<i64>,
    artifact_media_type: Option<String>,
    page_count: Option<i32>,
    failure_code: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl TryFrom<StoredDocumentRender> for RenderResponse {
    type Error = AppError;

    fn try_from(value: StoredDocumentRender) -> Result<Self, Self::Error> {
        if value.purpose != "printable" {
            return Err(AppError::not_found());
        }
        let template_revision_id = value
            .template_revision_id
            .ok_or_else(|| AppError::service_unavailable("invalid_stored_document"))?;
        Ok(Self {
            id: value.id,
            template_revision_id,
            state: value.state,
            artifact_sha256: value.artifact_sha256,
            artifact_byte_length: value.artifact_byte_length,
            artifact_media_type: value.artifact_media_type,
            page_count: value.page_count,
            failure_code: value.failure_code,
            created_at: value.created_at,
            updated_at: value.updated_at,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RegisterPreviewRenderRequest {
    specification: Value,
    input: Value,
    #[serde(default = "default_preview_render_ttl")]
    expires_in_seconds: i64,
}

const fn default_preview_render_ttl() -> i64 {
    900
}

#[derive(Debug, Serialize)]
struct PreviewRenderResponse {
    id: String,
    purpose: &'static str,
    state: String,
    artifact_sha256: Option<String>,
    artifact_byte_length: Option<i64>,
    artifact_media_type: Option<String>,
    page_count: Option<i32>,
    failure_code: Option<String>,
    expires_at: chrono::DateTime<chrono::Utc>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl TryFrom<StoredDocumentRender> for PreviewRenderResponse {
    type Error = AppError;

    fn try_from(value: StoredDocumentRender) -> Result<Self, Self::Error> {
        if value.purpose != "preview" {
            return Err(AppError::not_found());
        }
        Ok(Self {
            id: value.id,
            purpose: "preview",
            state: value.state,
            artifact_sha256: value.artifact_sha256,
            artifact_byte_length: value.artifact_byte_length,
            artifact_media_type: value.artifact_media_type,
            page_count: value.page_count,
            failure_code: value.failure_code,
            expires_at: value.expires_at,
            created_at: value.created_at,
            updated_at: value.updated_at,
        })
    }
}

#[allow(clippy::too_many_lines)]
async fn register_render(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<RegisterRenderRequest>,
) -> Result<Response, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::JobsWrite).await?;
    let idempotency_key = required_idempotency_key(&headers)?;
    if request.template_revision_id.len() > 80 {
        return Err(AppError::invalid(
            "invalid_document_render",
            "Template revision is invalid.",
        ));
    }
    if !request.input.is_object() {
        return Err(AppError::invalid(
            "invalid_document_input",
            "PrintPacket render input must be a JSON object.",
        ));
    }
    let plaintext = validate_json(&request.input, false)?;
    let input_sha256 = hex::encode(Sha256::digest(&plaintext));
    let request_sha256 = hex::encode(Sha256::digest(
        [request.template_revision_id.as_bytes(), b"\0", &plaintext].concat(),
    ));
    let id = format!("drnd_{}", uuid::Uuid::now_v7());
    let aad = render_input_aad(
        &tenant.workspace_id.to_string(),
        &tenant.environment_id.to_string(),
        &id,
    );
    let encrypted = state
        .document_secrets
        .encrypt(&aad, &plaintext)
        .map_err(|_| AppError::service_unavailable("document_encryption_failed"))?;
    let result = state
        .repository
        .register_document_render(
            tenant.workspace_id,
            tenant.environment_id,
            &id,
            &request.template_revision_id,
            &encrypted,
            &input_sha256,
            &idempotency_key,
            &request_sha256,
        )
        .await?;
    let (status, stored) = match result {
        CreateDocumentResult::Created(value) => (StatusCode::ACCEPTED, value),
        CreateDocumentResult::Existing(value) => (StatusCode::OK, value),
    };
    let revision = state
        .repository
        .get_document_revision(
            tenant.workspace_id,
            tenant.environment_id,
            stored
                .template_revision_id
                .as_deref()
                .ok_or_else(|| AppError::service_unavailable("invalid_stored_document"))?,
        )
        .await?;
    let specification = state
        .document_secrets
        .decrypt(
            &document_aad(
                &tenant.workspace_id.to_string(),
                &tenant.environment_id.to_string(),
                &revision.template_id,
            ),
            &revision.spec_ciphertext,
        )
        .map_err(|_| AppError::service_unavailable("document_decryption_failed"))?;
    let specification: PrintPacketV1 = serde_json::from_slice(&specification)
        .map_err(|_| AppError::service_unavailable("invalid_stored_document"))?;
    let resource_digests = specification
        .resources
        .values()
        .map(|resource| match resource {
            printpacket_renderer::Resource::Image { digest, .. } => {
                normalized_resource_digest(digest)
                    .ok_or_else(|| AppError::service_unavailable("invalid_stored_document"))
            }
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    state
        .repository
        .link_printpacket_render_resources(
            tenant.workspace_id,
            tenant.environment_id,
            &stored.id,
            &resource_digests,
        )
        .await?;
    Ok((status, Json(RenderResponse::try_from(stored)?)).into_response())
}

async fn get_render(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<RenderResponse>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::JobsRead).await?;
    Ok(Json(RenderResponse::try_from(
        state
            .repository
            .get_document_render(tenant.workspace_id, tenant.environment_id, &id)
            .await?,
    )?))
}

async fn register_preview_render(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<RegisterPreviewRenderRequest>,
) -> Result<Response, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::JobsWrite).await?;
    let idempotency_key = required_idempotency_key(&headers)?;
    if !(60..=1800).contains(&request.expires_in_seconds) {
        return Err(AppError::invalid(
            "invalid_preview_expiry",
            "Preview expiry must be between 60 and 1800 seconds.",
        ));
    }
    let specification = validate_document_spec(&request.specification)?;
    if !request.input.is_object() {
        return Err(AppError::invalid(
            "invalid_document_input",
            "PrintPacket render input must be a JSON object.",
        ));
    }
    let input = validate_json(&request.input, false)?;
    let spec_sha256 = hex::encode(Sha256::digest(&specification));
    let input_sha256 = hex::encode(Sha256::digest(&input));
    let parsed_specification: PrintPacketV1 = serde_json::from_slice(&specification)
        .map_err(|_| AppError::service_unavailable("invalid_normalized_document"))?;
    let resource_digests = parsed_specification
        .resources
        .values()
        .map(|resource| match resource {
            printpacket_renderer::Resource::Image { digest, .. } => {
                normalized_resource_digest(digest)
                    .ok_or_else(|| AppError::service_unavailable("invalid_normalized_document"))
            }
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    let request_sha256 = hex::encode(Sha256::digest(
        [
            b"preview\0".as_slice(),
            specification.as_slice(),
            b"\0",
            input.as_slice(),
            b"\0",
            request.expires_in_seconds.to_string().as_bytes(),
        ]
        .concat(),
    ));
    let id = format!("dprv_{}", uuid::Uuid::now_v7());
    let workspace = tenant.workspace_id.to_string();
    let environment = tenant.environment_id.to_string();
    let encrypted_specification = state
        .document_secrets
        .encrypt(
            &preview_render_spec_aad(&workspace, &environment, &id),
            &specification,
        )
        .map_err(|_| AppError::service_unavailable("document_encryption_failed"))?;
    let encrypted_input = state
        .document_secrets
        .encrypt(
            &preview_render_input_aad(&workspace, &environment, &id),
            &input,
        )
        .map_err(|_| AppError::service_unavailable("document_encryption_failed"))?;
    let result = state
        .repository
        .register_preview_document_render(
            tenant.workspace_id,
            tenant.environment_id,
            &id,
            &encrypted_specification,
            &spec_sha256,
            &encrypted_input,
            &input_sha256,
            &idempotency_key,
            &request_sha256,
            request.expires_in_seconds,
            &resource_digests,
        )
        .await?;
    let (status, stored) = match result {
        CreateDocumentResult::Created(value) => (StatusCode::ACCEPTED, value),
        CreateDocumentResult::Existing(value) => (StatusCode::OK, value),
    };
    Ok((status, Json(PreviewRenderResponse::try_from(stored)?)).into_response())
}

async fn get_preview_render(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<PreviewRenderResponse>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::JobsRead).await?;
    let render = state
        .repository
        .get_document_render(tenant.workspace_id, tenant.environment_id, &id)
        .await?;
    if render.expires_at <= chrono::Utc::now() || render.state == "expired" {
        return Err(AppError::not_found());
    }
    Ok(Json(PreviewRenderResponse::try_from(render)?))
}

// This is intentionally kept as one capability matrix so every readiness
// response is derived from the same atomic view of a node report and payload.
#[allow(clippy::too_many_lines)]
async fn evaluate_readiness(
    state: &AppState,
    workspace_id: piqae_domain::WorkspaceId,
    environment_id: piqae_domain::EnvironmentId,
    printer_id: &str,
    render_id: &str,
    policy: RenderPolicy,
    cost: Option<&RenderCost>,
) -> Result<RenderReadinessResponse, AppError> {
    validate_render_cost(cost)?;
    let printer_id = printer_id
        .parse()
        .map_err(|_| AppError::invalid("invalid_printer_id", "Printer ID is invalid."))?;
    let capabilities = state
        .repository
        .document_render_capabilities_for_printer(workspace_id, environment_id, printer_id)
        .await?;
    let payload = node_render_payload(state, workspace_id, environment_id, render_id).await?;
    let stored_render = state
        .repository
        .get_document_render(workspace_id, environment_id, render_id)
        .await?;
    if stored_render.purpose != "printable" {
        return Err(AppError::not_found());
    }
    let input_bytes = u64::try_from(
        serde_json::to_vec(&payload.input)
            .map_err(|_| AppError::service_unavailable("invalid_stored_document"))?
            .len(),
    )
    .map_err(|_| AppError::service_unavailable("invalid_stored_content_length"))?;
    let template_bytes = u64::try_from(
        serde_json::to_vec(&payload.specification)
            .map_err(|_| AppError::service_unavailable("invalid_stored_document"))?
            .len(),
    )
    .map_err(|_| AppError::service_unavailable("invalid_stored_content_length"))?;
    let measured = RenderCost {
        document_count: cost.map_or(1, |value| value.document_count),
        page_count: stored_render
            .page_count
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(0),
        pdf_bytes: payload.expected_pdf_bytes,
        input_bytes,
    };
    let packet = capabilities.print_packet.as_ref();
    let supported_packet_versions = packet
        .map(|packet| packet.supported_packet_versions.clone())
        .unwrap_or_default();
    let current_implementation = packet.map(|packet| packet.implementation_version.clone());
    let missing_features = payload
        .required_feature_ids
        .iter()
        .filter(|feature| {
            packet.is_none_or(|packet| {
                !packet
                    .feature_ids
                    .iter()
                    .any(|supported| supported == *feature)
            })
        })
        .cloned()
        .collect::<Vec<_>>();
    let negotiation_supported =
        packet.is_some_and(|packet| packet.negotiation_version == PRINT_PACKET_NEGOTIATION_VERSION);
    let packet_version_supported = packet.is_some_and(|packet| {
        packet
            .supported_packet_versions
            .iter()
            .any(|version| version == &payload.packet_version)
    });
    let conformance_supported = packet.is_some_and(|packet| {
        packet
            .conformance_profiles
            .iter()
            .any(|profile| profile == &payload.conformance_profile)
    });
    let output_supported = packet.is_some_and(|packet| {
        packet.output_profiles.iter().any(|profile| {
            matches!(
                profile,
                piqae_protocol::agent::PrintPacketOutputProfile::Pdf { id, media_type }
                    if id == &payload.output_profile && media_type == "application/pdf"
            )
        })
    });
    let deterministic = packet.is_some_and(|packet| packet.deterministic);
    let abi_supported = capabilities.renderer_abi.as_deref() == Some(payload.renderer_abi.as_str())
        && capabilities.resource_abi.as_deref() == Some(payload.resource_abi.as_str());
    let media_supported = packet.is_some_and(|packet| {
        payload.resources.iter().all(|resource| {
            packet.resource_types.contains(&resource.media_type)
                && capabilities
                    .image_media_types
                    .contains(&resource.media_type)
        })
    });
    let resource_count = u32::try_from(payload.resources.len()).unwrap_or(u32::MAX);
    let total_resource_bytes = payload.resources.iter().fold(0_u64, |total, resource| {
        total.saturating_add(resource.byte_length)
    });
    let limits_supported = packet.is_some_and(|packet| {
        template_bytes <= packet.limits.max_template_bytes
            && input_bytes <= packet.limits.max_input_bytes
            && payload.expected_pdf_bytes <= packet.limits.max_output_bytes
            && payload.expected_page_count <= packet.limits.max_pages
            && resource_count <= packet.limits.max_resource_count
            && total_resource_bytes <= packet.limits.max_total_resource_bytes
            && payload
                .resources
                .iter()
                .all(|resource| resource.byte_length <= packet.limits.max_resource_bytes)
    });
    let missing_resources = payload
        .resources
        .iter()
        .filter(|resource| {
            !capabilities
                .cached_resource_digests
                .contains(&resource.digest)
        })
        .map(|resource| resource.digest.clone())
        .collect::<Vec<_>>();
    // Missing cache entries are downloadable through the authenticated lease;
    // they mean a cold/warming node, not an incompatible node.
    let supported = negotiation_supported
        && packet_version_supported
        && missing_features.is_empty()
        && conformance_supported
        && output_supported
        && deterministic
        && abi_supported
        && media_supported
        && limits_supported;
    let ready = supported;
    let (mut selected_mode, mut reason) =
        if measured.page_count == 0 && matches!(policy, RenderPolicy::Automatic) {
            ("cloud_pdf", "automatic_missing_authoritative_page_count")
        } else {
            render_decision(policy, Some(&measured))
        };
    let approved_pdf_fallback = !matches!(policy, RenderPolicy::RequireNode);
    if selected_mode == "node_render" && !ready && approved_pdf_fallback {
        selected_mode = "cloud_pdf";
        reason = if packet.is_none() {
            "unsupported_old_node_pdf_fallback"
        } else {
            "node_not_ready_pdf_fallback"
        };
    }
    let (cloud_ms, node_ms) = Some(&measured).map_or((0, 0), |value| {
        (
            20_u64.saturating_add(value.pdf_bytes.saturating_mul(1000) / 12_500_000),
            35_u64
                .saturating_add(value.input_bytes.saturating_mul(1000) / 12_500_000)
                .saturating_add(u64::from(value.page_count).saturating_mul(4))
                .saturating_add(u64::from(value.document_count).saturating_mul(2)),
        )
    });
    let destination_reason = if packet.is_none() {
        Some("unsupported_old_node")
    } else if !negotiation_supported {
        Some("negotiation_version_unsupported")
    } else if !packet_version_supported {
        Some("packet_version_unsupported")
    } else if !missing_features.is_empty() {
        Some("packet_features_unsupported")
    } else if !conformance_supported {
        Some("conformance_profile_unsupported")
    } else if !output_supported {
        Some("output_profile_unsupported")
    } else if !deterministic {
        Some("deterministic_output_unavailable")
    } else if !abi_supported {
        Some("renderer_abi_unavailable")
    } else if !media_supported {
        Some("resource_media_type_unsupported")
    } else if !limits_supported {
        Some("packet_limits_exceeded")
    } else if ready && missing_resources.is_empty() {
        None
    } else if ready {
        Some("resources_warming")
    } else {
        Some("resources_not_cached")
    };
    let requires_node_update = packet.is_none()
        || !negotiation_supported
        || !packet_version_supported
        || !missing_features.is_empty()
        || !conformance_supported
        || !output_supported
        || !deterministic
        || !abi_supported
        || !media_supported;
    let status = if ready {
        "ready"
    } else if requires_node_update || !approved_pdf_fallback {
        "node_update_required"
    } else {
        "fallback_ready"
    };
    Ok(RenderReadinessResponse {
        requested_policy: policy,
        selected_mode,
        status,
        reason,
        approved_pdf_fallback,
        destination: DestinationReadiness {
            supported,
            ready,
            missing_features,
            supported_packet_versions,
            current_implementation,
            missing_resources,
            reason: destination_reason,
        },
        estimates: RenderEstimates { cloud_ms, node_ms },
    })
}

/// Revalidates the exact immutable `PrintPacket` payload against a replacement
/// printer before a waiting node-render job is moved to that route. A cloud-PDF
/// job has no renderer capability dependency and is therefore unaffected.
pub(crate) async fn reroute_candidate_supports_rendered_job(
    state: &AppState,
    tenant: TenantContext,
    job: &Job,
    printer_id: &str,
) -> Result<bool, AppError> {
    if job
        .metadata
        .get("piqae.document.render_mode")
        .map(String::as_str)
        != Some("node_render")
    {
        return Ok(true);
    }
    let Some(render_id) = job.metadata.get("piqae.document.render_id") else {
        return Ok(false);
    };
    let policy = match job
        .metadata
        .get("piqae.document.render_policy")
        .map(String::as_str)
    {
        Some("automatic") => "automatic",
        Some("prefer_node") => "prefer_node",
        Some("require_node") => "require_node",
        _ => return Ok(false),
    };
    printer_supports_exact_render(state, tenant, printer_id, render_id, policy).await
}

pub(crate) async fn printer_supports_exact_render(
    state: &AppState,
    tenant: TenantContext,
    printer_id: &str,
    render_id: &str,
    policy: &str,
) -> Result<bool, AppError> {
    let policy = match policy {
        "automatic" => RenderPolicy::Automatic,
        "prefer_node" => RenderPolicy::PreferNode,
        "require_node" => RenderPolicy::RequireNode,
        _ => return Ok(false),
    };
    let readiness = evaluate_readiness(
        state,
        tenant.workspace_id,
        tenant.environment_id,
        printer_id,
        render_id,
        policy,
        None,
    )
    .await?;
    Ok(readiness.destination.ready)
}

async fn render_readiness(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(request): Json<RenderReadinessRequest>,
) -> Result<Json<RenderReadinessResponse>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::JobsRead).await?;
    Ok(Json(
        evaluate_readiness(
            &state,
            tenant.workspace_id,
            tenant.environment_id,
            &request.printer_id,
            &id,
            request.render_policy,
            request.render_cost.as_ref(),
        )
        .await?,
    ))
}

async fn download_render_artifact(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::JobsRead).await?;
    let render = state
        .repository
        .get_document_render(tenant.workspace_id, tenant.environment_id, &id)
        .await?;
    if render.purpose != "printable" {
        return Err(AppError::not_found());
    }
    download_render_artifact_inner(&state, &tenant, render).await
}

async fn download_preview_render_artifact(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::JobsRead).await?;
    let render = state
        .repository
        .get_document_render(tenant.workspace_id, tenant.environment_id, &id)
        .await?;
    if render.purpose != "preview" {
        return Err(AppError::not_found());
    }
    if render.expires_at <= chrono::Utc::now() || render.state == "expired" {
        return Err(AppError::not_found());
    }
    download_render_artifact_inner(&state, &tenant, render).await
}

async fn download_render_artifact_inner(
    state: &AppState,
    tenant: &TenantContext,
    render: StoredDocumentRender,
) -> Result<Response, AppError> {
    if render.state != "completed" {
        return Err(AppError::conflict(
            "document_render_not_completed",
            "Only a completed document render has a downloadable artifact.",
        ));
    }
    let expected_sha256 = render
        .artifact_sha256
        .as_deref()
        .ok_or_else(|| AppError::service_unavailable("document_artifact_unavailable"))?;
    let expected_bytes = render
        .artifact_byte_length
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| (1..=50 * 1024 * 1024).contains(value))
        .ok_or_else(|| AppError::service_unavailable("document_artifact_unavailable"))?;
    let _buffer_permit = state
        .document_artifact_downloads
        .clone()
        .try_acquire_owned()
        .map_err(|_| AppError::service_unavailable("document_artifact_download_busy"))?;
    let encrypted_key = render
        .artifact_object_key_ciphertext
        .as_deref()
        .ok_or_else(|| AppError::service_unavailable("document_artifact_unavailable"))?;
    let aad = artifact_key_aad(
        &tenant.workspace_id.to_string(),
        &tenant.environment_id.to_string(),
        &render.id,
    );
    let object_key = state
        .document_secrets
        .decrypt(&aad, encrypted_key)
        .map_err(|_| AppError::service_unavailable("document_decryption_failed"))?;
    let object_key = String::from_utf8(object_key)
        .map_err(|_| AppError::service_unavailable("invalid_stored_document"))?;
    // Validate the complete bounded object before constructing the response so
    // a corrupt prefix can never be exposed to a browser or POS proxy.
    let mut stream = state
        .object_store
        .get_stream(&object_key)
        .await
        .map_err(|_| AppError::service_unavailable("document_artifact_unavailable"))?;
    let mut content = BytesMut::new();
    while let Some(chunk) = stream.next().await {
        let chunk =
            chunk.map_err(|_| AppError::service_unavailable("document_artifact_unavailable"))?;
        if chunk.len() > expected_bytes.saturating_sub(content.len()) {
            return Err(AppError::service_unavailable(
                "document_artifact_integrity_failed",
            ));
        }
        content.extend_from_slice(&chunk);
    }
    let actual_digest = Sha256::digest(&content);
    if content.len() != expected_bytes
        || !hex::encode(actual_digest).eq_ignore_ascii_case(expected_sha256)
    {
        return Err(AppError::service_unavailable(
            "document_artifact_integrity_failed",
        ));
    }
    Response::builder()
        .header(axum::http::header::CONTENT_TYPE, "application/pdf")
        .header(axum::http::header::CONTENT_LENGTH, expected_bytes)
        .header(
            axum::http::header::CONTENT_DISPOSITION,
            "attachment; filename=\"document.pdf\"",
        )
        .header(axum::http::header::CACHE_CONTROL, "private, no-store")
        .header("x-content-type-options", "nosniff")
        .header(
            "digest",
            format!("sha-256={}", STANDARD.encode(actual_digest)),
        )
        .body(Body::from(content.freeze()))
        .map_err(|_| AppError::service_unavailable("content_response_failed"))
}

#[allow(clippy::too_many_lines)]
async fn print_render(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(request): Json<PrintRenderRequest>,
) -> Result<Response, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::JobsWrite).await?;
    let idempotency_key = required_idempotency_key(&headers)?;
    validate_render_cost(request.render_cost.as_ref())?;
    let render = state
        .repository
        .get_document_render(tenant.workspace_id, tenant.environment_id, &id)
        .await?;
    if render.purpose != "printable" {
        return Err(AppError::not_found());
    }
    if render.state != "completed" {
        return Err(AppError::conflict(
            "document_render_not_completed",
            "Only a completed document render can be printed.",
        ));
    }
    let document_media =
        render_document_media(&state, tenant.workspace_id, tenant.environment_id, &render).await?;
    if request.target_id.is_some() && request.specification_revision.is_none() {
        return Err(AppError::conflict(
            "design_specification_revision_required",
            "Target printing requires the current design specification revision.",
        ));
    }
    if request.printer_id.is_some() && request.specification_revision.is_some() {
        return Err(AppError::invalid(
            "design_specification_revision_not_allowed",
            "Direct printer printing must not include a target design specification revision.",
        ));
    }
    let encrypted_key = render
        .artifact_object_key_ciphertext
        .as_deref()
        .ok_or_else(|| AppError::service_unavailable("document_artifact_unavailable"))?;
    let aad = artifact_key_aad(
        &tenant.workspace_id.to_string(),
        &tenant.environment_id.to_string(),
        &render.id,
    );
    let object_key = state
        .document_secrets
        .decrypt(&aad, encrypted_key)
        .map_err(|_| AppError::service_unavailable("document_decryption_failed"))?;
    let object_key = String::from_utf8(object_key)
        .map_err(|_| AppError::service_unavailable("invalid_stored_document"))?;
    let artifact_sha256 = render
        .artifact_sha256
        .as_deref()
        .ok_or_else(|| AppError::service_unavailable("document_artifact_unavailable"))?;
    let artifact_bytes = render
        .artifact_byte_length
        .ok_or_else(|| AppError::service_unavailable("document_artifact_unavailable"))?;
    let measured_cost =
        if matches!(request.render_policy, RenderPolicy::CloudOnly) {
            None
        } else {
            let payload = node_render_payload(
                &state,
                tenant.workspace_id,
                tenant.environment_id,
                &render.id,
            )
            .await?;
            let input_bytes = u64::try_from(
                serde_json::to_vec(&payload.input)
                    .map_err(|_| AppError::service_unavailable("invalid_stored_document"))?
                    .len(),
            )
            .map_err(|_| AppError::service_unavailable("invalid_stored_content_length"))?;
            Some(RenderCost {
                // The caller may describe how many Shopify orders were combined,
                // but byte and page measurements always come from the immutable
                // completed render held by the control plane.
                document_count: request
                    .render_cost
                    .as_ref()
                    .map_or(1, |cost| cost.document_count),
                page_count: u32::try_from(render.page_count.ok_or_else(|| {
                    AppError::service_unavailable("document_page_count_unavailable")
                })?)
                .map_err(|_| AppError::service_unavailable("document_page_count_invalid"))?,
                pdf_bytes: u64::try_from(artifact_bytes)
                    .map_err(|_| AppError::service_unavailable("invalid_stored_content_length"))?,
                input_bytes,
            })
        };
    let (mut selected_mode, mut decision_reason) =
        render_decision(request.render_policy, measured_cost.as_ref());
    let readiness_destination = if matches!(request.render_policy, RenderPolicy::CloudOnly) {
        None
    } else {
        resolve_request_readiness_printer(
            &state,
            tenant,
            &request,
            &document_media,
            &render.id,
            selected_mode == "node_render",
        )
        .await?
    };
    if !matches!(request.render_policy, RenderPolicy::CloudOnly) {
        if let Some(readiness_destination) = readiness_destination.as_ref() {
            let readiness = evaluate_readiness(
                &state,
                tenant.workspace_id,
                tenant.environment_id,
                &readiness_destination.printer_id,
                &render.id,
                request.render_policy,
                measured_cost.as_ref(),
            )
            .await?;
            selected_mode = readiness.selected_mode;
            decision_reason = readiness.reason;
            if !readiness.destination.ready
                && matches!(request.render_policy, RenderPolicy::RequireNode)
            {
                return Err(AppError::conflict(
                    "node_render_not_ready",
                    "The selected node cannot render this exact document and require_node fails closed.",
                ));
            }
        } else {
            if matches!(request.render_policy, RenderPolicy::RequireNode) {
                return Err(AppError::conflict(
                    "node_render_destination_unresolved",
                    "Node rendering requires an exact printer destination before approval.",
                ));
            }
            selected_mode = "cloud_pdf";
            decision_reason = "node_destination_unresolved_pdf_fallback";
        }
    }
    // Idempotency is based only on the immutable render plus the caller's
    // semantic request. Node readiness, target selection, and measured routing
    // metadata may legitimately drift between retries and must not turn the
    // same external operation into an idempotency conflict.
    let canonical_print_request = serde_json::to_vec(&(render.id.as_str(), &request))
        .map_err(|_| AppError::invalid("invalid_print_request", "Print request is invalid."))?;
    let acquisition_sha256 = hex::encode(Sha256::digest(
        [render.id.as_bytes(), b"\0", idempotency_key.as_bytes()].concat(),
    ));
    let upload_id = format!("dua_{acquisition_sha256}");
    state
        .repository
        .acquire_document_artifact_upload(
            &upload_id,
            tenant.workspace_id,
            tenant.environment_id,
            &render.id,
            &object_key,
            artifact_sha256,
            artifact_bytes,
            &acquisition_sha256,
            chrono::Utc::now() + chrono::Duration::seconds(default_print_expiry()),
        )
        .await?;
    let mut metadata = std::collections::BTreeMap::new();
    metadata.insert("piqae.document.render_id".into(), render.id);
    metadata.insert(
        "piqae.document.render_policy".into(),
        serde_json::to_string(&request.render_policy)
            .unwrap_or_else(|_| "\"automatic\"".into())
            .trim_matches('"')
            .to_owned(),
    );
    metadata.insert("piqae.document.render_mode".into(), selected_mode.into());
    metadata.insert(
        "piqae.document.media".into(),
        serde_json::to_string(&document_media)
            .map_err(|_| AppError::service_unavailable("document_media_serialization_failed"))?,
    );
    if let Some(revision) = request.specification_revision {
        metadata.insert("piqae.design_specification_revision".into(), revision);
    }
    metadata.insert(
        "piqae.document.render_decision_reason".into(),
        decision_reason.into(),
    );
    metadata.insert(
        "piqae.document.pdf_bytes".into(),
        artifact_bytes.to_string(),
    );
    if let Some(cost) = &measured_cost {
        metadata.insert(
            "piqae.document.input_bytes".into(),
            cost.input_bytes.to_string(),
        );
        metadata.insert(
            "piqae.document.page_count".into(),
            cost.page_count.to_string(),
        );
        metadata.insert(
            "piqae.document.document_count".into(),
            cost.document_count.to_string(),
        );
    }
    create_job_internal_with_target_binding(
        state,
        headers,
        CreateJobRequest {
            printer_id: request.printer_id,
            target_id: request.target_id,
            title: request.title,
            source: Some("piqae.documents".into()),
            content_type: ContentKind::Pdf,
            printer_native: None,
            content: ContentSource::Upload { upload_id },
            options: request.options,
            deliveries: request.deliveries,
            expire_after_seconds: default_print_expiry(),
            metadata,
            resolved_ticket_digest: None,
        },
        readiness_destination.and_then(|destination| destination.binding_id),
        canonical_print_request,
    )
    .await
}

async fn render_document_media(
    state: &AppState,
    workspace_id: piqae_domain::WorkspaceId,
    environment_id: piqae_domain::EnvironmentId,
    render: &StoredDocumentRender,
) -> Result<printpacket_renderer::Media, AppError> {
    if render.purpose != "printable" {
        return Err(AppError::not_found());
    }
    let revision_id = render
        .template_revision_id
        .as_deref()
        .ok_or_else(|| AppError::service_unavailable("invalid_stored_document"))?;
    let revision = state
        .repository
        .get_document_revision(workspace_id, environment_id, revision_id)
        .await?;
    let bytes = state
        .document_secrets
        .decrypt(
            &document_aad(
                &workspace_id.to_string(),
                &environment_id.to_string(),
                &revision.template_id,
            ),
            &revision.spec_ciphertext,
        )
        .map_err(|_| AppError::service_unavailable("document_decryption_failed"))?;
    let document: PrintPacketV1 = serde_json::from_slice(&bytes)
        .map_err(|_| AppError::service_unavailable("invalid_stored_document"))?;
    Ok(document.media)
}

fn validate_render_cost(cost: Option<&RenderCost>) -> Result<(), AppError> {
    let Some(cost) = cost else {
        return Ok(());
    };
    if !(1..=10_000).contains(&cost.document_count)
        || !(1..=100_000).contains(&cost.page_count)
        || !(1..=524_288_000).contains(&cost.pdf_bytes)
        || !(1..=52_428_800).contains(&cost.input_bytes)
    {
        return Err(AppError::invalid(
            "invalid_document_render_cost",
            "Render cost measurements are outside the supported limits.",
        ));
    }
    Ok(())
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PrintRenderRequest {
    printer_id: Option<String>,
    target_id: Option<String>,
    specification_revision: Option<String>,
    title: String,
    #[serde(default)]
    options: JobOptions,
    #[serde(default = "default_print_deliveries")]
    deliveries: u16,
    #[serde(default)]
    render_policy: RenderPolicy,
    #[serde(default)]
    render_cost: Option<RenderCost>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreatePreviewRequest {
    #[serde(default = "default_preview_ttl")]
    expires_in_seconds: i64,
}
const fn default_preview_ttl() -> i64 {
    600
}

#[derive(Debug, Serialize)]
struct PreviewResponse {
    id: String,
    render_id: String,
    state: String,
    job_id: Option<String>,
    expires_at: chrono::DateTime<chrono::Utc>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}
impl From<StoredDocumentPreview> for PreviewResponse {
    fn from(v: StoredDocumentPreview) -> Self {
        Self {
            id: v.id,
            render_id: v.render_id,
            state: v.state,
            job_id: v.job_id,
            expires_at: v.expires_at,
            created_at: v.created_at,
            updated_at: v.updated_at,
        }
    }
}

async fn create_preview(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(render_id): Path<String>,
    Json(request): Json<CreatePreviewRequest>,
) -> Result<Response, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::JobsWrite).await?;
    let key = required_idempotency_key(&headers)?;
    let render = state
        .repository
        .get_document_render(tenant.workspace_id, tenant.environment_id, &render_id)
        .await?;
    if render.purpose != "printable" {
        return Err(AppError::not_found());
    }
    if !(60..=1800).contains(&request.expires_in_seconds) {
        return Err(AppError::invalid(
            "invalid_document_preview",
            "Preview expiry must be between 60 and 1800 seconds.",
        ));
    }
    let hash = hex::encode(Sha256::digest(
        [
            render_id.as_bytes(),
            b"\0",
            request.expires_in_seconds.to_string().as_bytes(),
        ]
        .concat(),
    ));
    let id = preview_stable_id(
        &tenant.workspace_id.to_string(),
        &tenant.environment_id.to_string(),
        &render_id,
        &key,
    );
    let result = state
        .repository
        .create_document_preview(
            tenant.workspace_id,
            tenant.environment_id,
            &id,
            &render_id,
            &key,
            &hash,
            chrono::Utc::now() + chrono::Duration::seconds(request.expires_in_seconds),
        )
        .await?;
    let (status, p) = match result {
        CreateDocumentResult::Created(v) => (StatusCode::CREATED, v),
        CreateDocumentResult::Existing(v) => (StatusCode::OK, v),
    };
    Ok((status, Json(PreviewResponse::from(p))).into_response())
}
async fn get_preview(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<PreviewResponse>, AppError> {
    let t = authenticate_native(&state, &headers, Scope::JobsRead).await?;
    Ok(Json(
        state
            .repository
            .get_document_preview(t.workspace_id, t.environment_id, &id)
            .await?
            .into(),
    ))
}
async fn download_preview_artifact(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let t = authenticate_native(&state, &headers, Scope::JobsRead).await?;
    let p = state
        .repository
        .get_document_preview(t.workspace_id, t.environment_id, &id)
        .await?;
    if !matches!(
        p.state.as_str(),
        "awaiting_approval" | "approving" | "approved"
    ) {
        return Err(AppError::conflict(
            "document_preview_unavailable",
            "The preview is no longer available.",
        ));
    }
    download_render_artifact(State(state), headers, Path(p.render_id)).await
}
async fn cancel_preview(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<PreviewResponse>, AppError> {
    let t = authenticate_native(&state, &headers, Scope::JobsWrite).await?;
    let _ = required_idempotency_key(&headers)?;
    Ok(Json(
        state
            .repository
            .cancel_document_preview(t.workspace_id, t.environment_id, &id)
            .await?
            .into(),
    ))
}

#[derive(Debug, Serialize)]
struct ApprovedPreviewResponse {
    preview: PreviewResponse,
    job: Value,
}
async fn approve_preview(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(request): Json<PrintRenderRequest>,
) -> Result<Response, AppError> {
    let t = authenticate_native(&state, &headers, Scope::JobsWrite).await?;
    let key = required_idempotency_key(&headers)?;
    if matches!(request.render_policy, RenderPolicy::RequireNode) {
        let pending = state
            .repository
            .get_document_preview(t.workspace_id, t.environment_id, &id)
            .await?;
        let render = state
            .repository
            .get_document_render(t.workspace_id, t.environment_id, &pending.render_id)
            .await?;
        let document_media =
            render_document_media(&state, t.workspace_id, t.environment_id, &render).await?;
        let readiness_destination = resolve_request_readiness_printer(
            &state,
            t,
            &request,
            &document_media,
            &pending.render_id,
            true,
        )
        .await?
        .ok_or_else(|| {
            AppError::conflict(
                "node_render_destination_unresolved",
                "require_node requires an exact printer or target destination.",
            )
        })?;
        let readiness = evaluate_readiness(
            &state,
            t.workspace_id,
            t.environment_id,
            &readiness_destination.printer_id,
            &pending.render_id,
            request.render_policy,
            request.render_cost.as_ref(),
        )
        .await?;
        if !readiness.destination.ready {
            return Err(AppError::conflict(
                "node_render_not_ready",
                "The selected node cannot render this exact document and require_node fails closed.",
            ));
        }
    }
    let encoded = serde_json::to_vec(&request)
        .map_err(|_| AppError::invalid("invalid_document_preview", "Approval is invalid."))?;
    let hash = hex::encode(Sha256::digest(&encoded));
    let preview = state
        .repository
        .begin_document_preview_approval(t.workspace_id, t.environment_id, &id, &key, &hash)
        .await?;
    // `print_render` owns the complete render-to-job handoff state machine.
    // Polling it from this already stateful HTTP handler nests both generated
    // futures on one Tokio worker call stack and can overflow the bounded
    // production worker stack. Merely pinning the child future does not break
    // that poll chain. Schedule the handoff as its own task so the executor
    // polls it from a fresh task boundary, while this request still awaits the
    // exact result before completing the durable preview approval.
    let response = tokio::spawn(print_render(
        State(state.clone()),
        headers,
        Path(preview.render_id.clone()),
        Json(request),
    ))
    .await
    .map_err(|error| {
        tracing::error!(
            error.type = "document_preview_approval_task_failed",
            task.cancelled = error.is_cancelled(),
            task.panicked = error.is_panic(),
            "document preview approval handoff task failed"
        );
        AppError::service_unavailable("document_preview_approval_failed")
    })??;
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .map_err(|_| AppError::service_unavailable("document_preview_approval_failed"))?;
    let job: Value = serde_json::from_slice(&bytes)
        .map_err(|_| AppError::service_unavailable("document_preview_approval_failed"))?;
    let job_id = job
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::service_unavailable("document_preview_approval_failed"))?;
    let preview = state
        .repository
        .complete_document_preview_approval(t.workspace_id, t.environment_id, &id, &key, job_id)
        .await?;
    Ok((
        status,
        Json(ApprovedPreviewResponse {
            preview: preview.into(),
            job,
        }),
    )
        .into_response())
}

struct ReadinessDestination {
    printer_id: String,
    binding_id: Option<String>,
}

async fn resolve_request_readiness_printer(
    state: &AppState,
    tenant: TenantContext,
    request: &PrintRenderRequest,
    document_media: &printpacket_renderer::Media,
    render_id: &str,
    require_renderer: bool,
) -> Result<Option<ReadinessDestination>, AppError> {
    if let Some(printer_id) = request.printer_id.as_ref() {
        return Ok(Some(ReadinessDestination {
            printer_id: printer_id.clone(),
            binding_id: None,
        }));
    }
    let Some(target_id) = request.target_id.as_deref() else {
        return Ok(None);
    };
    let policy = match request.render_policy {
        RenderPolicy::Automatic => "automatic",
        RenderPolicy::CloudOnly => "cloud_only",
        RenderPolicy::PreferNode => "prefer_node",
        RenderPolicy::RequireNode => "require_node",
    };
    let resolved = resolve_target_printpacket_printer(
        state,
        tenant,
        target_id,
        document_media,
        request.options.bin.as_deref(),
        require_renderer.then_some((render_id, policy)),
    )
    .await;
    let (printer_id, binding_id) = match resolved {
        Ok(destination) => destination,
        Err(error)
            if require_renderer
                && !matches!(request.render_policy, RenderPolicy::RequireNode)
                && error.is_conflict() =>
        {
            resolve_target_printpacket_printer(
                state,
                tenant,
                target_id,
                document_media,
                request.options.bin.as_deref(),
                None,
            )
            .await?
        }
        Err(error) => return Err(error),
    };
    Ok(Some(ReadinessDestination {
        printer_id,
        binding_id: Some(binding_id),
    }))
}

const fn default_print_deliveries() -> u16 {
    1
}
const fn default_print_expiry() -> i64 {
    1_209_600
}

fn validate_name(name: &str) -> Result<(), AppError> {
    if name.trim().is_empty() || name.chars().count() > 120 {
        return Err(AppError::invalid(
            "invalid_document_template",
            "Name is outside the supported limits.",
        ));
    }
    Ok(())
}
fn validate_key(key: &str) -> Result<(), AppError> {
    if !(8..=200).contains(&key.len()) || !key.is_ascii() {
        return Err(AppError::invalid(
            "invalid_idempotency_key",
            "Idempotency key is outside the supported limits.",
        ));
    }
    Ok(())
}
fn required_idempotency_key(headers: &HeaderMap) -> Result<String, AppError> {
    let key = headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            AppError::invalid(
                "missing_idempotency_key",
                "Idempotency-Key header is required.",
            )
        })?;
    validate_key(key)?;
    Ok(key.to_owned())
}
fn stable_id(prefix: &str, a: &str, b: &str, key: &str) -> String {
    let digest = hex::encode(Sha256::digest(
        [a.as_bytes(), b"\0", b.as_bytes(), b"\0", key.as_bytes()].concat(),
    ));
    format!("{prefix}_{}", &digest[..32])
}
fn preview_stable_id(workspace: &str, environment: &str, render: &str, key: &str) -> String {
    stable_id("dpvw", workspace, environment, &format!("{render}\0{key}"))
}
pub(crate) fn document_aad(workspace: &str, environment: &str, resource: &str) -> Vec<u8> {
    document_aad_for("template-spec", workspace, environment, resource)
}
pub(crate) fn render_input_aad(workspace: &str, environment: &str, resource: &str) -> Vec<u8> {
    document_aad_for("render-input", workspace, environment, resource)
}
pub(crate) fn preview_render_spec_aad(
    workspace: &str,
    environment: &str,
    resource: &str,
) -> Vec<u8> {
    document_aad_for("preview-render-spec", workspace, environment, resource)
}
pub(crate) fn preview_render_input_aad(
    workspace: &str,
    environment: &str,
    resource: &str,
) -> Vec<u8> {
    document_aad_for("preview-render-input", workspace, environment, resource)
}
pub(crate) fn artifact_key_aad(workspace: &str, environment: &str, resource: &str) -> Vec<u8> {
    document_aad_for("render-artifact-key", workspace, environment, resource)
}

pub(crate) async fn node_render_payload(
    state: &AppState,
    workspace_id: piqae_domain::WorkspaceId,
    environment_id: piqae_domain::EnvironmentId,
    render_id: &str,
) -> Result<PrintPacketNodeRender, AppError> {
    let render = state
        .repository
        .get_document_render(workspace_id, environment_id, render_id)
        .await?;
    if render.purpose != "printable" {
        return Err(AppError::not_found());
    }
    let revision_id = render
        .template_revision_id
        .as_deref()
        .ok_or_else(|| AppError::service_unavailable("invalid_stored_document"))?;
    let revision = state
        .repository
        .get_document_revision(workspace_id, environment_id, revision_id)
        .await?;
    let spec_bytes = state
        .document_secrets
        .decrypt(
            &document_aad(
                &workspace_id.to_string(),
                &environment_id.to_string(),
                &revision.template_id,
            ),
            &revision.spec_ciphertext,
        )
        .map_err(|_| AppError::service_unavailable("document_decryption_failed"))?;
    let input_bytes = state
        .document_secrets
        .decrypt(
            &render_input_aad(
                &workspace_id.to_string(),
                &environment_id.to_string(),
                &render.id,
            ),
            render
                .input_ciphertext
                .as_deref()
                .ok_or_else(|| AppError::service_unavailable("invalid_stored_document"))?,
        )
        .map_err(|_| AppError::service_unavailable("document_decryption_failed"))?;
    let specification: PrintPacketV1 = serde_json::from_slice(&spec_bytes)
        .map_err(|_| AppError::service_unavailable("invalid_stored_document"))?;
    let resources = specification
        .resources
        .values()
        .map(|resource| match resource {
            printpacket_renderer::Resource::Image {
                digest,
                media_type,
                byte_length,
            } => Ok(PrintPacketResourceDescriptor {
                digest: normalized_resource_digest(digest)
                    .ok_or_else(|| AppError::service_unavailable("invalid_stored_document"))?,
                media_type: media_type.clone(),
                byte_length: *byte_length,
            }),
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    Ok(PrintPacketNodeRender {
        negotiation_version: PRINT_PACKET_NEGOTIATION_VERSION,
        packet_version: PRINT_PACKET_VERSION.into(),
        required_feature_ids: printpacket::required_features(&specification)
            .iter()
            .map(print_packet_feature_id)
            .collect(),
        conformance_profile: PRINT_PACKET_CONFORMANCE_PROFILE.into(),
        output_profile: PRINT_PACKET_OUTPUT_PROFILE.into(),
        renderer_abi: PRINT_PACKET_RENDERER_ABI.into(),
        resource_abi: PRINT_PACKET_RESOURCE_ABI.into(),
        specification: serde_json::from_slice(&spec_bytes)
            .map_err(|_| AppError::service_unavailable("invalid_stored_document"))?,
        input: serde_json::from_slice(&input_bytes)
            .map_err(|_| AppError::service_unavailable("invalid_stored_document"))?,
        resources,
        expected_pdf_sha256: render
            .artifact_sha256
            .ok_or_else(|| AppError::service_unavailable("document_artifact_unavailable"))?,
        expected_pdf_bytes: u64::try_from(
            render
                .artifact_byte_length
                .ok_or_else(|| AppError::service_unavailable("document_artifact_unavailable"))?,
        )
        .map_err(|_| AppError::service_unavailable("invalid_stored_content_length"))?,
        expected_page_count: u32::try_from(
            render
                .page_count
                .ok_or_else(|| AppError::service_unavailable("document_page_count_unavailable"))?,
        )
        .ok()
        .filter(|page_count| *page_count > 0)
        .ok_or_else(|| AppError::service_unavailable("document_page_count_invalid"))?,
    })
}
fn document_aad_for(domain: &str, workspace: &str, environment: &str, resource: &str) -> Vec<u8> {
    format!("printpacket/v1\0{domain}\0{workspace}\0{environment}\0{resource}").into_bytes()
}
fn validate_document_spec(value: &Value) -> Result<Vec<u8>, AppError> {
    if value.get("format").and_then(Value::as_str) != Some(PRINT_PACKET_DOCUMENT_FORMAT) {
        return Err(AppError::invalid(
            "invalid_document_spec",
            "format must be printpacket/v1.",
        ));
    }
    let encoded = validate_json(value, true)?;
    let specification = serde_json::from_slice::<PrintPacketV1>(&encoded).map_err(|_| {
        AppError::invalid("invalid_document_spec", "Document structure is invalid.")
    })?;
    printpacket_renderer::validate(
        &specification,
        printpacket_renderer::RenderLimits::default(),
    )
    .map_err(|_| {
        AppError::invalid(
            "invalid_document_spec",
            "Document structure, version, or declared content is invalid.",
        )
    })?;
    Ok(encoded)
}
fn validate_json(value: &Value, reject_remote_urls: bool) -> Result<Vec<u8>, AppError> {
    fn visit(value: &Value, depth: usize, nodes: &mut usize, reject_urls: bool) -> bool {
        *nodes += 1;
        if depth > MAX_JSON_DEPTH || *nodes > MAX_JSON_NODES {
            return false;
        }
        match value {
            Value::Array(values) => values
                .iter()
                .all(|value| visit(value, depth + 1, nodes, reject_urls)),
            Value::Object(values) => {
                values.len() <= 1_000
                    && values.iter().all(|(key, value)| {
                        key.len() <= 120 && visit(value, depth + 1, nodes, reject_urls)
                    })
            }
            Value::String(value) => {
                value.len() <= MAX_DOCUMENT_BYTES
                    && (!reject_urls
                        || !(value.starts_with("http://") || value.starts_with("https://")))
            }
            _ => true,
        }
    }
    let mut nodes = 0;
    if !visit(value, 0, &mut nodes, reject_remote_urls) {
        return Err(AppError::invalid(
            "invalid_document_payload",
            "Document data exceeds structural limits or contains a remote asset URL.",
        ));
    }
    let encoded = serde_json::to_vec(value)
        .map_err(|_| AppError::invalid("invalid_document_payload", "Document data is invalid."))?;
    if encoded.len() > MAX_DOCUMENT_BYTES {
        return Err(AppError::payload_too_large(
            "document_payload_too_large",
            "Document data exceeds 1 MiB.",
        ));
    }
    Ok(encoded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository::{MemoryRepository, Repository};
    use piqae_domain::{EnvironmentId, WorkspaceId};

    #[test]
    fn resource_digest_normalization_rejects_malformed_values() {
        assert_eq!(
            normalized_resource_digest(&format!("sha256:{}", "a".repeat(64))),
            Some("a".repeat(64))
        );
        assert_eq!(
            normalized_resource_digest(&"A".repeat(64)),
            Some("a".repeat(64))
        );
        assert_eq!(normalized_resource_digest("sha256:not-a-digest"), None);
        assert_eq!(normalized_resource_digest(""), None);
    }

    #[test]
    fn automatic_policy_requires_measured_costs() {
        assert_eq!(
            render_decision(RenderPolicy::Automatic, None),
            ("cloud_pdf", "automatic_missing_measurements")
        );
    }

    #[test]
    fn preview_ids_are_environment_scoped() {
        assert_ne!(
            preview_stable_id("workspace", "environment_a", "render", "key"),
            preview_stable_id("workspace", "environment_b", "render", "key")
        );
    }

    #[test]
    fn document_specs_are_bounded_and_reject_runtime_urls() -> Result<(), String> {
        let canonical = validate_document_spec(&serde_json::json!({
            "format": "printpacket/v1", "media": {"kind": "paged", "size": "a4"},
            "body": [{"type": "paragraph", "content": [{"type": "text", "value": "Receipt"}]}]
        }))
        .map_err(|error| format!("canonical PrintPacket validation failed: {error:?}"))?;
        let normalized: PrintPacketV1 = serde_json::from_slice(&canonical)
            .map_err(|error| format!("canonical PrintPacket decoding failed: {error}"))?;
        assert_eq!(normalized.format, PRINT_PACKET_DOCUMENT_FORMAT);
        assert!(
            validate_document_spec(&serde_json::json!({
                "format": "piqae.business-document/v1", "media": {"kind": "paged", "size": "a4"},
                "body": [{"type": "paragraph", "content": [{"type": "text", "value": "https://example.test/logo.png"}]}]
            }))
            .is_err(),
            "experimental Piqae format identifier must not be normalized or migrated"
        );
        assert!(
            validate_document_spec(&serde_json::json!({
                "format": "printpacket/v1",
                "media": {
                    "kind": "continuous",
                    "width_mm": 58.0,
                    "margins": {"top_mm": 2.0, "right_mm": 2.0, "bottom_mm": 2.0, "left_mm": 2.0}
                },
                "body": [{"type": "page_break"}]
            }))
            .is_err(),
            "continuous media must reject recursive page breaks before publishing"
        );
        assert!(
            validate_document_spec(&serde_json::json!({
                "format": "printpacket/v1", "media": {"kind": "paged", "size": "a4"},
                "header": {"last": [{"type": "paragraph", "content": []}]},
                "footer": {"first": [{"type": "paragraph", "content": []}]},
                "body": []
            }))
            .is_ok(),
            "all bounded first/default/last region variants must be publishable"
        );
        assert!(validate_document_spec(&serde_json::json!({"format": "other/v1"})).is_err());
        Ok(())
    }

    #[test]
    fn encryption_contexts_are_domain_separated() {
        let template = document_aad("w", "e", "same");
        let input = render_input_aad("w", "e", "same");
        let artifact = artifact_key_aad("w", "e", "same");
        assert_ne!(template, input);
        assert_ne!(input, artifact);
        assert_ne!(template, artifact);
    }

    #[tokio::test]
    async fn memory_repository_hides_document_ids_from_other_tenants()
    -> Result<(), crate::repository::RepositoryError> {
        let repository = MemoryRepository::default();
        let workspace_a = WorkspaceId::new();
        let environment_a = EnvironmentId::new();
        let workspace_b = WorkspaceId::new();
        let environment_b = EnvironmentId::new();
        repository
            .create_document_template(
                workspace_a,
                environment_a,
                "dtpl_probe",
                "Receipt",
                b"encrypted",
                &"a".repeat(64),
            )
            .await?;
        assert!(
            repository
                .get_document_template(workspace_a, environment_a, "dtpl_probe")
                .await
                .is_ok()
        );
        assert!(matches!(
            repository
                .get_document_template(workspace_b, environment_b, "dtpl_probe")
                .await,
            Err(crate::repository::RepositoryError::NotFound)
        ));
        Ok(())
    }

    #[tokio::test]
    async fn preview_gate_is_tenant_scoped_and_creates_no_job_before_approval()
    -> Result<(), crate::repository::RepositoryError> {
        let repository = MemoryRepository::default();
        let workspace = WorkspaceId::new();
        let environment = EnvironmentId::new();
        repository
            .create_document_template(
                workspace,
                environment,
                "template",
                "Receipt",
                b"encrypted",
                &"a".repeat(64),
            )
            .await?;
        repository
            .publish_document_template(workspace, environment, "template", "revision")
            .await?;
        repository
            .register_document_render(
                workspace,
                environment,
                "render",
                "revision",
                b"input",
                &"b".repeat(64),
                "render-key",
                &"c".repeat(64),
            )
            .await?;
        repository
            .complete_document_render(
                workspace,
                environment,
                "render",
                b"object",
                &"d".repeat(64),
                10,
            )
            .await?;
        let preview = repository
            .create_document_preview(
                workspace,
                environment,
                "preview",
                "render",
                "preview-key",
                &"e".repeat(64),
                chrono::Utc::now() + chrono::Duration::minutes(10),
            )
            .await?;
        let preview = match preview {
            CreateDocumentResult::Created(value) => value,
            CreateDocumentResult::Existing(_) => panic!("new preview"),
        };
        assert_eq!(preview.state, "awaiting_approval");
        assert!(preview.job_id.is_none());
        assert!(matches!(
            repository
                .get_document_preview(WorkspaceId::new(), EnvironmentId::new(), "preview")
                .await,
            Err(crate::repository::RepositoryError::NotFound)
        ));
        let cancelled = repository
            .cancel_document_preview(workspace, environment, "preview")
            .await?;
        assert_eq!(cancelled.state, "cancelled");
        assert!(matches!(
            repository
                .begin_document_preview_approval(
                    workspace,
                    environment,
                    "preview",
                    "approval-key",
                    &"f".repeat(64)
                )
                .await,
            Err(crate::repository::RepositoryError::ConcurrentStateChange)
        ));
        Ok(())
    }
}
