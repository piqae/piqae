use crate::{
    AppState,
    api::{CreateJobRequest, authenticate_native, create_job},
    error::AppError,
};
use axum::{
    Json, Router,
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use bytes::BytesMut;
use futures::StreamExt as _;
use piqae_auth::Scope;
use piqae_document_renderer::BusinessDocumentV1;
use piqae_domain::{ContentKind, ContentSource, JobOptions};
use piqae_storage_postgres::{CreateDocumentResult, StoredDocumentPreview, StoredDocumentRender};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

const MAX_DOCUMENT_BYTES: usize = 1024 * 1024;
const MAX_JSON_DEPTH: usize = 64;
const MAX_JSON_NODES: usize = 50_000;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/business-document-templates", post(create_template))
        .route(
            "/v1/business-document-templates/{template_id}",
            get(get_template),
        )
        .route(
            "/v1/business-document-templates/{template_id}/publish",
            post(publish_template),
        )
        .route(
            "/v1/business-document-template-revisions/{revision_id}",
            get(get_revision),
        )
        .route("/v1/business-document-renders", post(register_render))
        .route("/v1/business-document-renders/{render_id}", get(get_render))
        .route(
            "/v1/business-document-renders/{render_id}/artifact",
            get(download_render_artifact),
        )
        .route(
            "/v1/business-document-renders/{render_id}/print",
            post(print_render),
        )
        .route(
            "/v1/business-document-renders/{render_id}/previews",
            post(create_preview),
        )
        .route(
            "/v1/business-document-previews/{preview_id}",
            get(get_preview),
        )
        .route(
            "/v1/business-document-previews/{preview_id}/artifact",
            get(download_preview_artifact),
        )
        .route(
            "/v1/business-document-previews/{preview_id}/approve",
            post(approve_preview),
        )
        .route(
            "/v1/business-document-previews/{preview_id}/cancel",
            post(cancel_preview),
        )
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
            specification: request.specification,
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
    failure_code: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<StoredDocumentRender> for RenderResponse {
    fn from(value: StoredDocumentRender) -> Self {
        Self {
            id: value.id,
            template_revision_id: value.template_revision_id,
            state: value.state,
            artifact_sha256: value.artifact_sha256,
            artifact_byte_length: value.artifact_byte_length,
            artifact_media_type: value.artifact_media_type,
            failure_code: value.failure_code,
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
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
    Ok((status, Json(RenderResponse::from(stored))).into_response())
}

async fn get_render(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<RenderResponse>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::JobsRead).await?;
    Ok(Json(RenderResponse::from(
        state
            .repository
            .get_document_render(tenant.workspace_id, tenant.environment_id, &id)
            .await?,
    )))
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

async fn print_render(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(request): Json<PrintRenderRequest>,
) -> Result<Response, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::JobsWrite).await?;
    let idempotency_key = required_idempotency_key(&headers)?;
    let render = state
        .repository
        .get_document_render(tenant.workspace_id, tenant.environment_id, &id)
        .await?;
    if render.state != "completed" {
        return Err(AppError::conflict(
            "document_render_not_completed",
            "Only a completed document render can be printed.",
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
    metadata.insert("document_render_id".into(), render.id);
    create_job(
        State(state),
        headers,
        Json(CreateJobRequest {
            printer_id: request.printer_id,
            target_id: request.target_id,
            title: request.title,
            source: Some("piqae.documents".into()),
            content_type: ContentKind::Pdf,
            content: ContentSource::Upload { upload_id },
            options: request.options,
            deliveries: request.deliveries,
            expire_after_seconds: default_print_expiry(),
            metadata,
            resolved_ticket_digest: None,
        }),
    )
    .await
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PrintRenderRequest {
    printer_id: Option<String>,
    target_id: Option<String>,
    title: String,
    #[serde(default)]
    options: JobOptions,
    #[serde(default = "default_print_deliveries")]
    deliveries: u16,
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
    let encoded = serde_json::to_vec(&request)
        .map_err(|_| AppError::invalid("invalid_document_preview", "Approval is invalid."))?;
    let hash = hex::encode(Sha256::digest(&encoded));
    let preview = state
        .repository
        .begin_document_preview_approval(t.workspace_id, t.environment_id, &id, &key, &hash)
        .await?;
    let response = print_render(
        State(state.clone()),
        headers,
        Path(preview.render_id.clone()),
        Json(request),
    )
    .await?;
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
pub(crate) fn artifact_key_aad(workspace: &str, environment: &str, resource: &str) -> Vec<u8> {
    document_aad_for("render-artifact-key", workspace, environment, resource)
}
fn document_aad_for(domain: &str, workspace: &str, environment: &str, resource: &str) -> Vec<u8> {
    format!("piqae.business-documents/v1\0{domain}\0{workspace}\0{environment}\0{resource}")
        .into_bytes()
}
fn validate_document_spec(value: &Value) -> Result<Vec<u8>, AppError> {
    if value.get("format").and_then(Value::as_str) != Some("piqae.business-document/v1") {
        return Err(AppError::invalid(
            "invalid_document_spec",
            "format must be piqae.business-document/v1.",
        ));
    }
    let encoded = validate_json(value, true)?;
    serde_json::from_slice::<BusinessDocumentV1>(&encoded).map_err(|_| {
        AppError::invalid("invalid_document_spec", "Document structure is invalid.")
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
    fn preview_ids_are_environment_scoped() {
        assert_ne!(
            preview_stable_id("workspace", "environment_a", "render", "key"),
            preview_stable_id("workspace", "environment_b", "render", "key")
        );
    }

    #[test]
    fn document_specs_are_bounded_and_reject_runtime_urls() {
        assert!(
            validate_document_spec(&serde_json::json!({
                "format": "piqae.business-document/v1", "media": {"kind": "paged", "size": "a4"},
                "body": [{"type": "paragraph", "content": [{"type": "text", "value": "Receipt"}]}]
            }))
            .is_ok()
        );
        assert!(
            validate_document_spec(&serde_json::json!({
                "format": "piqae.business-document/v1", "media": {"kind": "paged", "size": "a4"},
                "body": [{"type": "paragraph", "content": [{"type": "text", "value": "https://example.test/logo.png"}]}]
            }))
            .is_err()
        );
        assert!(validate_document_spec(&serde_json::json!({"format": "other/v1"})).is_err());
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
