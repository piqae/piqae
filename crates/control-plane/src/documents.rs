use crate::{
    AppState,
    api::{CreateJobRequest, authenticate_native, create_job},
    error::AppError,
};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use piqae_auth::Scope;
use piqae_document_renderer::DocumentSpecV1;
use piqae_domain::{ContentKind, ContentSource, JobOptions};
use piqae_storage_postgres::{CreateDocumentResult, StoredDocumentRender};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

const MAX_DOCUMENT_BYTES: usize = 1024 * 1024;
const MAX_JSON_DEPTH: usize = 64;
const MAX_JSON_NODES: usize = 50_000;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/document-templates", post(create_template))
        .route("/v1/document-templates/{template_id}", get(get_template))
        .route(
            "/v1/document-templates/{template_id}/publish",
            post(publish_template),
        )
        .route(
            "/v1/document-template-revisions/{revision_id}",
            get(get_revision),
        )
        .route("/v1/document-renders", post(register_render))
        .route("/v1/document-conversions", post(create_conversion))
        .route(
            "/v1/document-conversions/{conversion_id}",
            get(get_conversion),
        )
        .route("/v1/document-renders/{render_id}", get(get_render))
        .route("/v1/document-renders/{render_id}/print", post(print_render))
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateConversionRequest {
    adapter: String,
    adapter_version: String,
    source: Value,
    strict: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct PersistedConversionResult {
    document: Value,
    warnings: Value,
}

#[derive(Debug, Serialize)]
struct ConversionResponse {
    id: String,
    adapter: String,
    adapter_version: String,
    adapter_api_version: String,
    source_format: String,
    source_sha256: String,
    strict: bool,
    fidelity: String,
    renderer_version: String,
    document: Value,
    warnings: Value,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[allow(clippy::too_many_lines)]
async fn create_conversion(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateConversionRequest>,
) -> Result<Response, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::JobsWrite).await?;
    let key = required_idempotency_key(&headers)?;
    if request.adapter != "pdfme"
        || request.adapter_version != crate::document_adapters::PDFME_ADAPTER_VERSION
    {
        return Err(AppError::invalid(
            "unsupported_document_adapter",
            "Only exact adapter pdfme@1.0.0 is supported.",
        ));
    }
    let mut canonical_source = request.source.clone();
    canonical_source.sort_all_objects();
    let source = validate_json(&canonical_source, true)?;
    let source_sha256 = hex::encode(Sha256::digest(&source));
    let converted = crate::document_adapters::convert_pdfme(&canonical_source, request.strict)
        .map_err(|_| {
            AppError::invalid(
                "document_conversion_incompatible",
                "The source uses unsupported or lossy adapter features.",
            )
        })?;
    validate_document_spec(&converted.document)?;
    let warnings = serde_json::to_value(converted.warnings).map_err(|_| {
        AppError::invalid(
            "invalid_document_payload",
            "Conversion diagnostics are invalid.",
        )
    })?;
    let result = PersistedConversionResult {
        document: converted.document,
        warnings,
    };
    let result_plaintext = serde_json::to_vec(&result).map_err(|_| {
        AppError::invalid("invalid_document_payload", "Converted document is invalid.")
    })?;
    if result_plaintext.len() > MAX_DOCUMENT_BYTES {
        return Err(AppError::payload_too_large(
            "document_conversion_too_large",
            "Converted document and diagnostics exceed 1 MiB.",
        ));
    }
    let result_sha256 = hex::encode(Sha256::digest(&result_plaintext));
    let request_sha256 = hex::encode(Sha256::digest(
        [
            request.adapter.as_bytes(),
            b"\0",
            request.adapter_version.as_bytes(),
            b"\0",
            if request.strict {
                b"strict".as_slice()
            } else {
                b"lossy".as_slice()
            },
            b"\0",
            &source,
        ]
        .concat(),
    ));
    let id = stable_id(
        "dcnv",
        &tenant.workspace_id.to_string(),
        &tenant.environment_id.to_string(),
        &key,
    );
    let aad = conversion_result_aad(
        &tenant.workspace_id.to_string(),
        &tenant.environment_id.to_string(),
        &id,
    );
    let ciphertext = state
        .document_secrets
        .encrypt(&aad, &result_plaintext)
        .map_err(|_| AppError::service_unavailable("document_encryption_failed"))?;
    let stored_result = state
        .repository
        .create_document_conversion(
            tenant.workspace_id,
            tenant.environment_id,
            &id,
            &request.adapter,
            &request.adapter_version,
            &source_sha256,
            request.strict,
            converted.fidelity,
            piqae_document_renderer::RENDERER_VERSION,
            &ciphertext,
            &result_sha256,
            &key,
            &request_sha256,
        )
        .await?;
    let (status, stored) = match stored_result {
        CreateDocumentResult::Created(v) => (StatusCode::CREATED, v),
        CreateDocumentResult::Existing(v) => (StatusCode::OK, v),
    };
    let response = conversion_response(
        &state,
        &tenant.workspace_id.to_string(),
        &tenant.environment_id.to_string(),
        stored,
    )?;
    Ok((status, Json(response)).into_response())
}

async fn get_conversion(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<ConversionResponse>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::JobsRead).await?;
    let stored = state
        .repository
        .get_document_conversion(tenant.workspace_id, tenant.environment_id, &id)
        .await?;
    Ok(Json(conversion_response(
        &state,
        &tenant.workspace_id.to_string(),
        &tenant.environment_id.to_string(),
        stored,
    )?))
}

fn conversion_response(
    state: &AppState,
    workspace: &str,
    environment: &str,
    stored: piqae_storage_postgres::StoredDocumentConversion,
) -> Result<ConversionResponse, AppError> {
    let aad = conversion_result_aad(workspace, environment, &stored.id);
    let plaintext = state
        .document_secrets
        .decrypt(&aad, &stored.result_ciphertext)
        .map_err(|_| AppError::service_unavailable("document_decryption_failed"))?;
    if hex::encode(Sha256::digest(&plaintext)) != stored.result_sha256 {
        return Err(AppError::service_unavailable("invalid_stored_document"));
    }
    let result: PersistedConversionResult = serde_json::from_slice(&plaintext)
        .map_err(|_| AppError::service_unavailable("invalid_stored_document"))?;
    Ok(ConversionResponse {
        id: stored.id,
        adapter: stored.adapter_id,
        adapter_version: stored.adapter_version,
        adapter_api_version: stored.adapter_api_version,
        source_format: stored.source_format,
        source_sha256: stored.source_sha256,
        strict: stored.strict,
        fidelity: stored.fidelity,
        renderer_version: stored.renderer_version,
        document: result.document,
        warnings: result.warnings,
        created_at: stored.created_at,
    })
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
    metadata.insert("piqae.document_render_id".into(), render.id);
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
        }),
    )
    .await
}

#[derive(Debug, Deserialize)]
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
pub(crate) fn document_aad(workspace: &str, environment: &str, resource: &str) -> Vec<u8> {
    document_aad_for("template-spec", workspace, environment, resource)
}
pub(crate) fn render_input_aad(workspace: &str, environment: &str, resource: &str) -> Vec<u8> {
    document_aad_for("render-input", workspace, environment, resource)
}
pub(crate) fn artifact_key_aad(workspace: &str, environment: &str, resource: &str) -> Vec<u8> {
    document_aad_for("render-artifact-key", workspace, environment, resource)
}
pub(crate) fn conversion_result_aad(workspace: &str, environment: &str, resource: &str) -> Vec<u8> {
    document_aad_for(
        "adapter-conversion-result",
        workspace,
        environment,
        resource,
    )
}
fn document_aad_for(domain: &str, workspace: &str, environment: &str, resource: &str) -> Vec<u8> {
    format!("piqae.documents/v1\0{domain}\0{workspace}\0{environment}\0{resource}").into_bytes()
}
fn validate_document_spec(value: &Value) -> Result<Vec<u8>, AppError> {
    if value.get("spec_version").and_then(Value::as_str) != Some("piqae.document/v1") {
        return Err(AppError::invalid(
            "invalid_document_spec",
            "spec_version must be piqae.document/v1.",
        ));
    }
    let encoded = validate_json(value, true)?;
    serde_json::from_slice::<DocumentSpecV1>(&encoded).map_err(|_| {
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
    fn document_specs_are_bounded_and_reject_runtime_urls() {
        assert!(
            validate_document_spec(&serde_json::json!({
                "spec_version": "piqae.document/v1", "page": {"size": "a4"},
                "body": [{"type": "text", "value": "Receipt"}]
            }))
            .is_ok()
        );
        assert!(
            validate_document_spec(&serde_json::json!({
                "spec_version": "piqae.document/v1", "page": {"size": "a4"},
                "body": [{"type": "text", "value": "https://example.test/logo.png"}]
            }))
            .is_err()
        );
        assert!(validate_document_spec(&serde_json::json!({"spec_version": "other/v1"})).is_err());
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
}
