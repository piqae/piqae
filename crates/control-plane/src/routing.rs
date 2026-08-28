#![allow(clippy::missing_errors_doc)]

use crate::{AppState, api::authenticate_native, error::AppError, repository::RepositoryError};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, TimeDelta, Utc};
use piqae_auth::Scope;
use piqae_domain::{PrinterId, PrinterState};
use piqae_storage_postgres::{
    PrinterProfileSnapshot, StoredAgent, StoredBindingReadiness, StoredLoadedMedia,
    StoredPrintWorkflow, StoredStock, StoredTarget, StoredTargetBinding, StoredTargetReadiness,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::str::FromStr;

#[derive(Debug, Deserialize)]
pub struct CreateStockRequest {
    name: String,
    sku: Option<String>,
    description: Option<String>,
    #[serde(default = "empty_object")]
    attributes: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct PatchStockRequest {
    name: Option<String>,
    sku: Option<String>,
    description: Option<String>,
    attributes: Option<serde_json::Value>,
    archived: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct CreateTargetRequest {
    name: String,
    description: Option<String>,
    stock_id: Option<String>,
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default = "default_routing_policy")]
    routing_policy: String,
}

#[derive(Debug, Deserialize)]
pub struct PatchTargetRequest {
    name: Option<String>,
    description: Option<String>,
    stock_id: Option<String>,
    clear_stock: Option<bool>,
    enabled: Option<bool>,
    routing_policy: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateBindingRequest {
    printer_id: String,
    profile_id: String,
    profile_revision: u64,
    role: String,
    #[serde(default = "default_true")]
    enabled: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreatePrintWorkflowRequest {
    name: String,
    printer_id: String,
    capability_revision: u64,
    profile: Option<ResourceRevision>,
    stock: Option<ResourceRevision>,
    definition: serde_json::Value,
    safe_overrides: Vec<String>,
    #[serde(default)]
    published: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResourceRevision {
    id: String,
    revision: u64,
}

fn empty_object() -> serde_json::Value {
    serde_json::json!({})
}

const fn default_true() -> bool {
    true
}

fn default_routing_policy() -> String {
    "primary_then_standby".into()
}

pub async fn list_stocks(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<StoredStock>>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::PrintersRead).await?;
    Ok(Json(
        state
            .repository
            .list_stocks(tenant.workspace_id, tenant.environment_id)
            .await?,
    ))
}

pub async fn list_print_workflows(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<StoredPrintWorkflow>>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::PrintersRead).await?;
    Ok(Json(
        state
            .repository
            .list_print_workflows(tenant.workspace_id, tenant.environment_id)
            .await?
            .into_iter()
            .filter(|workflow| workflow.published && !workflow.archived)
            .collect(),
    ))
}

#[allow(clippy::too_many_lines)]
pub async fn create_print_workflow(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreatePrintWorkflowRequest>,
) -> Result<Response, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::PrintersWrite).await?;
    let name = validate_name(&request.name, "invalid_print_workflow")?;
    let printer_id = PrinterId::from_str(&request.printer_id)
        .map_err(|_| AppError::invalid("invalid_print_workflow", "Printer ID is invalid."))?;
    let printer = state
        .repository
        .get_printer(tenant.workspace_id, tenant.environment_id, printer_id)
        .await?;
    if request.capability_revision == 0
        || printer.capability_revision != request.capability_revision
    {
        return Err(AppError::conflict(
            "stale_capability_revision",
            "The printer capability revision changed; refresh and validate again.",
        ));
    }
    if !request.definition.is_object()
        || request.safe_overrides.len() > 128
        || invalid_safe_override_set(&request.safe_overrides)
        || request.safe_overrides.iter().any(|value| {
            value.is_empty()
                || value.len() > 255
                || !(value.starts_with("portable_options.")
                    || value.starts_with("semantic_options."))
                || value.starts_with("driver.")
        })
    {
        return Err(AppError::invalid(
            "invalid_print_workflow",
            "Workflow definition and safe overrides must use bounded normalized fields.",
        ));
    }
    let definition =
        serde_json::from_value::<crate::print_intents::PrintIntent>(request.definition.clone())
            .map_err(|_| {
                AppError::invalid(
                    "workflow_definition_mismatch",
                    "Workflow definition must match the print-intent contract.",
                )
            })?;
    if definition.schema_version != 1
        || definition.printer_id != request.printer_id
        || definition.capability_revision != request.capability_revision
        || !definition.document_manifest.is_object()
        || definition.workflow.is_some()
    {
        return Err(AppError::invalid(
            "workflow_definition_mismatch",
            "Workflow definition must pin this printer and capability revision and cannot reference another workflow.",
        ));
    }
    if let Some(profile) = &request.profile {
        let requested_profile_id = profile.id.as_str();
        let requested_profile_revision = profile.revision;
        if profile.revision == 0
            || !printer.profiles.iter().any(|current| {
                current.profile_id == requested_profile_id
                    && current.revision == requested_profile_revision
                    && current.published
            })
        {
            return Err(AppError::conflict(
                "workflow_profile_unavailable",
                "The exact published printer profile revision is unavailable.",
            ));
        }
    }
    if let Some(stock) = &request.stock {
        let current = state
            .repository
            .get_stock(tenant.workspace_id, tenant.environment_id, &stock.id)
            .await?;
        if stock.revision == 0 || current.revision != stock.revision || current.archived {
            return Err(AppError::conflict(
                "workflow_stock_unavailable",
                "The exact active stock revision is unavailable.",
            ));
        }
        if request.definition["stock"]["id"] != stock.id
            || request.definition["stock"]["revision"] != stock.revision
        {
            return Err(AppError::invalid(
                "workflow_stock_mismatch",
                "Workflow definition stock must match the pinned stock revision.",
            ));
        }
    } else if !request
        .definition
        .get("stock")
        .is_none_or(serde_json::Value::is_null)
    {
        return Err(AppError::invalid(
            "workflow_stock_mismatch",
            "A workflow definition cannot use an unpinned stock.",
        ));
    }
    let now = Utc::now();
    let workflow = StoredPrintWorkflow {
        id: format!("pwf_{}", ulid::Ulid::new()),
        revision: 1,
        name,
        printer_id,
        capability_revision: request.capability_revision,
        profile_id: request.profile.as_ref().map(|value| value.id.clone()),
        profile_revision: request.profile.map(|value| value.revision),
        stock_id: request.stock.as_ref().map(|value| value.id.clone()),
        stock_revision: request.stock.map(|value| value.revision),
        definition: request.definition,
        safe_overrides: request.safe_overrides,
        published: request.published,
        archived: false,
        created_at: now,
        updated_at: now,
    };
    let workflow = state
        .repository
        .create_print_workflow(tenant.workspace_id, tenant.environment_id, &workflow)
        .await?;
    state
        .publish(tenant, "print_workflow.created", &workflow)
        .await?;
    Ok((StatusCode::CREATED, Json(workflow)).into_response())
}

pub async fn create_stock(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateStockRequest>,
) -> Result<Response, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::PrintersWrite).await?;
    let name = validate_name(&request.name, "invalid_stock")?;
    if !request.attributes.is_object() {
        return Err(AppError::invalid(
            "invalid_stock",
            "Stock attributes must be a JSON object.",
        ));
    }
    let now = Utc::now();
    let stock = StoredStock {
        id: format!("stk_{}", ulid::Ulid::new()),
        revision: 1,
        name,
        sku: clean_optional(request.sku, 120, "invalid_stock")?,
        description: clean_optional(request.description, 2_000, "invalid_stock")?,
        attributes: request.attributes,
        archived: false,
        created_at: now,
        updated_at: now,
    };
    let stock = state
        .repository
        .create_stock(tenant.workspace_id, tenant.environment_id, &stock)
        .await?;
    state.publish(tenant, "stock.created", &stock).await?;
    Ok((StatusCode::CREATED, Json(stock)).into_response())
}

pub async fn patch_stock(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(stock_id): Path<String>,
    Json(request): Json<PatchStockRequest>,
) -> Result<Json<StoredStock>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::PrintersWrite).await?;
    let mut stock = state
        .repository
        .get_stock(tenant.workspace_id, tenant.environment_id, &stock_id)
        .await?;
    if let Some(name) = request.name {
        stock.name = validate_name(&name, "invalid_stock")?;
    }
    if let Some(sku) = request.sku {
        stock.sku = clean_optional(Some(sku), 120, "invalid_stock")?;
    }
    if let Some(description) = request.description {
        stock.description = clean_optional(Some(description), 2_000, "invalid_stock")?;
    }
    if let Some(attributes) = request.attributes {
        if !attributes.is_object() {
            return Err(AppError::invalid(
                "invalid_stock",
                "Stock attributes must be a JSON object.",
            ));
        }
        stock.attributes = attributes;
    }
    if let Some(archived) = request.archived {
        stock.archived = archived;
    }
    stock.updated_at = Utc::now();
    let stock = state
        .repository
        .update_stock(tenant.workspace_id, tenant.environment_id, &stock)
        .await?;
    state.publish(tenant, "stock.updated", &stock).await?;
    Ok(Json(stock))
}

pub async fn list_targets(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<StoredTarget>>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::PrintersRead).await?;
    Ok(Json(
        state
            .repository
            .list_targets(tenant.workspace_id, tenant.environment_id)
            .await?,
    ))
}

pub async fn create_target(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateTargetRequest>,
) -> Result<Response, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::PrintersWrite).await?;
    validate_routing_policy(&request.routing_policy)?;
    let now = Utc::now();
    let target = StoredTarget {
        id: format!("tgt_{}", ulid::Ulid::new()),
        name: validate_name(&request.name, "invalid_target")?,
        description: clean_optional(request.description, 2_000, "invalid_target")?,
        stock_id: clean_optional(request.stock_id, 80, "invalid_target")?,
        enabled: request.enabled,
        routing_policy: request.routing_policy,
        created_at: now,
        updated_at: now,
    };
    let target = state
        .repository
        .create_target(tenant.workspace_id, tenant.environment_id, &target)
        .await?;
    state.publish(tenant, "target.created", &target).await?;
    Ok((StatusCode::CREATED, Json(target)).into_response())
}

pub async fn patch_target(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(target_id): Path<String>,
    Json(request): Json<PatchTargetRequest>,
) -> Result<Json<StoredTarget>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::PrintersWrite).await?;
    let mut target = state
        .repository
        .get_target(tenant.workspace_id, tenant.environment_id, &target_id)
        .await?;
    if let Some(name) = request.name {
        target.name = validate_name(&name, "invalid_target")?;
    }
    if let Some(description) = request.description {
        target.description = clean_optional(Some(description), 2_000, "invalid_target")?;
    }
    if request.clear_stock.unwrap_or(false) {
        target.stock_id = None;
    } else if let Some(stock_id) = request.stock_id {
        target.stock_id = clean_optional(Some(stock_id), 80, "invalid_target")?;
    }
    if let Some(enabled) = request.enabled {
        target.enabled = enabled;
    }
    if let Some(routing_policy) = request.routing_policy {
        validate_routing_policy(&routing_policy)?;
        target.routing_policy = routing_policy;
    }
    target.updated_at = Utc::now();
    let target = state
        .repository
        .update_target(tenant.workspace_id, tenant.environment_id, &target)
        .await?;
    state.publish(tenant, "target.updated", &target).await?;
    Ok(Json(target))
}

pub async fn list_bindings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(target_id): Path<String>,
) -> Result<Json<Vec<StoredTargetBinding>>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::PrintersRead).await?;
    Ok(Json(
        state
            .repository
            .list_target_bindings(tenant.workspace_id, tenant.environment_id, &target_id)
            .await?,
    ))
}

pub async fn create_binding(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(target_id): Path<String>,
    Json(request): Json<CreateBindingRequest>,
) -> Result<Response, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::PrintersWrite).await?;
    if !matches!(request.role.as_str(), "primary" | "standby")
        || request.profile_id.trim().is_empty()
        || request.profile_id.chars().count() > 120
        || request.profile_revision == 0
    {
        return Err(AppError::invalid(
            "invalid_target_binding",
            "Binding role, profile ID, or revision is invalid.",
        ));
    }
    let printer_id = PrinterId::from_str(&request.printer_id)
        .map_err(|_| AppError::invalid("invalid_target_binding", "The printer ID is not valid."))?;
    let printer = state
        .repository
        .get_printer(tenant.workspace_id, tenant.environment_id, printer_id)
        .await?;
    let profile = printer
        .profiles
        .iter()
        .find(|profile| {
            (profile.profile_id.as_str(), profile.revision)
                == (request.profile_id.trim(), request.profile_revision)
        })
        .ok_or(RepositoryError::NotFound)?;
    if !profile.published {
        return Err(AppError::invalid(
            "invalid_target_binding",
            "Only a published profile revision can be bound to a target.",
        ));
    }
    let now = Utc::now();
    let binding = StoredTargetBinding {
        id: format!("tgb_{}", ulid::Ulid::new()),
        target_id,
        printer_id,
        agent_id: printer.agent_id,
        profile_id: request.profile_id.trim().to_owned(),
        profile_revision: request.profile_revision,
        role: request.role,
        enabled: request.enabled,
        created_at: now,
        updated_at: now,
    };
    let binding = state
        .repository
        .create_target_binding(tenant.workspace_id, tenant.environment_id, &binding)
        .await?;
    state
        .publish(tenant, "target.binding.created", &binding)
        .await?;
    Ok((StatusCode::CREATED, Json(binding)).into_response())
}

pub async fn delete_binding(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((target_id, binding_id)): Path<(String, String)>,
) -> Result<StatusCode, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::PrintersWrite).await?;
    state
        .repository
        .delete_target_binding(
            tenant.workspace_id,
            tenant.environment_id,
            &target_id,
            &binding_id,
        )
        .await?;
    state
        .publish(
            tenant,
            "target.binding.deleted",
            &serde_json::json!({"target_id": target_id, "binding_id": binding_id}),
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn target_readiness(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(target_id): Path<String>,
) -> Result<Json<StoredTargetReadiness>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::PrintersRead).await?;
    Ok(Json(
        compute_target_readiness(&state, tenant, &target_id).await?,
    ))
}

async fn compute_target_readiness(
    state: &AppState,
    tenant: crate::authentication::TenantContext,
    target_id: &str,
) -> Result<StoredTargetReadiness, AppError> {
    let target = state
        .repository
        .get_target(tenant.workspace_id, tenant.environment_id, target_id)
        .await?;
    let bindings = state
        .repository
        .list_target_bindings(tenant.workspace_id, tenant.environment_id, target_id)
        .await?;
    let agents = state
        .repository
        .list_agents(tenant.workspace_id, tenant.environment_id)
        .await?;
    let mut evaluated = Vec::with_capacity(bindings.len());
    for binding in bindings {
        let mut reasons = Vec::new();
        let status = if !target.enabled || !binding.enabled {
            "disabled"
        } else if !agents
            .iter()
            .any(|agent| agent.id == binding.agent_id && agent_is_connected(agent))
        {
            "node_offline"
        } else {
            match state
                .repository
                .get_printer(
                    tenant.workspace_id,
                    tenant.environment_id,
                    binding.printer_id,
                )
                .await
            {
                Ok(printer) => binding_printer_readiness(&binding, &printer, &mut reasons),
                Err(RepositoryError::NotFound) => "destination_missing",
                Err(error) => return Err(error.into()),
            }
        };
        evaluated.push(StoredBindingReadiness {
            binding,
            status: status.into(),
            reasons,
        });
    }
    let selected_binding_id = evaluated
        .iter()
        .find(|binding| binding.status == "ready")
        .map(|binding| binding.binding.id.clone());
    Ok(StoredTargetReadiness {
        target_id: target_id.to_owned(),
        status: if selected_binding_id.is_some() {
            "ready"
        } else {
            "target_has_no_ready_binding"
        }
        .into(),
        selected_binding_id,
        bindings: evaluated,
    })
}

#[derive(Debug, Serialize)]
pub struct DesignSpecificationDestination {
    pub(crate) binding: StoredTargetBinding,
    pub(crate) printer: piqae_storage_postgres::StoredPrinter,
    pub(crate) profile: piqae_storage_postgres::PrinterProfileSnapshot,
    pub(crate) media_compatibility: MediaCompatibility,
}

#[derive(Debug, Serialize)]
pub struct DesignSpecificationResponse {
    pub(crate) target: StoredTarget,
    pub(crate) stock: Option<StoredStock>,
    pub(crate) readiness: StoredTargetReadiness,
    pub(crate) destinations: Vec<DesignSpecificationDestination>,
    pub(crate) specification_revision: String,
}

pub async fn design_specification(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(target_id): Path<String>,
) -> Result<Json<DesignSpecificationResponse>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::PrintersRead).await?;
    Ok(Json(
        current_design_specification(&state, tenant, &target_id).await?,
    ))
}

pub(crate) async fn current_design_specification(
    state: &AppState,
    tenant: crate::authentication::TenantContext,
    target_id: &str,
) -> Result<DesignSpecificationResponse, AppError> {
    let target = state
        .repository
        .get_target(tenant.workspace_id, tenant.environment_id, target_id)
        .await?;
    let stock = match target.stock_id.as_deref() {
        Some(id) => Some(
            state
                .repository
                .get_stock(tenant.workspace_id, tenant.environment_id, id)
                .await?,
        ),
        None => None,
    };
    let readiness = compute_target_readiness(state, tenant, target_id).await?;
    let bindings = state
        .repository
        .list_target_bindings(tenant.workspace_id, tenant.environment_id, target_id)
        .await?;
    let mut destinations = Vec::with_capacity(bindings.len());
    for binding in &bindings {
        let printer = match state
            .repository
            .get_printer(
                tenant.workspace_id,
                tenant.environment_id,
                binding.printer_id,
            )
            .await
        {
            Ok(printer) => printer,
            Err(RepositoryError::NotFound) => continue,
            Err(error) => return Err(error.into()),
        };
        let Some(profile) = printer
            .profiles
            .iter()
            .find(|profile| {
                (profile.profile_id.as_str(), profile.revision)
                    == (binding.profile_id.as_str(), binding.profile_revision)
            })
            .cloned()
        else {
            continue;
        };
        let media_compatibility = evaluate_binding_media(
            state,
            tenant,
            stock.as_ref(),
            &printer,
            Some(&profile),
            Utc::now(),
            None,
        )
        .await?;
        destinations.push(DesignSpecificationDestination {
            binding: binding.clone(),
            printer,
            profile,
            media_compatibility,
        });
    }
    let canonical = serde_json::to_vec(&design_constraint_projection(
        &target,
        stock.as_ref(),
        &bindings,
    ))
    .map_err(|_| AppError::service_unavailable("design_specification_serialization_failed"))?;
    let revision = format!("spec_{:x}", Sha256::digest(canonical));
    Ok(DesignSpecificationResponse {
        target,
        stock,
        readiness,
        destinations,
        specification_revision: revision,
    })
}

fn design_constraint_projection(
    target: &StoredTarget,
    stock: Option<&StoredStock>,
    bindings: &[StoredTargetBinding],
) -> serde_json::Value {
    let mut binding_constraints = bindings
        .iter()
        .map(|binding| {
            serde_json::json!({
                "id": binding.id,
                "target_id": binding.target_id,
                "printer_id": binding.printer_id,
                "agent_id": binding.agent_id,
                "profile_id": binding.profile_id,
                "profile_revision": binding.profile_revision,
                "role": binding.role,
                "enabled": binding.enabled,
            })
        })
        .collect::<Vec<_>>();
    binding_constraints.sort_by(|left, right| left["id"].as_str().cmp(&right["id"].as_str()));
    serde_json::json!({
        "target": {
            "id": target.id,
            "stock_id": target.stock_id,
            "enabled": target.enabled,
            "routing_policy": target.routing_policy,
        },
        "stock": stock.map(|stock| serde_json::json!({
            "id": stock.id,
            "revision": stock.revision,
            "attributes": stock.attributes,
            "archived": stock.archived,
        })),
        "bindings": binding_constraints,
    })
}

/// Loaded-media observations are deliberately short-lived. They describe an
/// operator or device observation, not an indefinitely valid printer default.
pub(crate) const LOADED_MEDIA_FRESHNESS_SECONDS: i64 = 15 * 60;
const MEDIA_DIMENSION_TOLERANCE_MM: f64 = 0.5;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct MediaDimensions {
    pub(crate) width_mm: f64,
    pub(crate) height_mm: f64,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct LoadedMediaStock {
    pub(crate) id: String,
    pub(crate) revision: u64,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct LoadedMediaEvidence {
    pub(crate) source: String,
    pub(crate) confidence: String,
    pub(crate) observed_at: DateTime<Utc>,
    pub(crate) fresh_until: DateTime<Utc>,
    pub(crate) stock: Option<LoadedMediaStock>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct MediaCompatibility {
    pub(crate) status: &'static str,
    pub(crate) reasons: Vec<String>,
    pub(crate) profile_dimensions_mm: Option<MediaDimensions>,
    pub(crate) loaded_media: Option<LoadedMediaEvidence>,
}

#[derive(Clone, Debug)]
pub(crate) struct PrintPacketMediaFence {
    pub(crate) specification_revision: String,
    pub(crate) stock_id: String,
    pub(crate) stock_revision: u64,
    pub(crate) loaded_media_snapshot: String,
}

pub(crate) async fn validate_printpacket_media_fence(
    state: &AppState,
    tenant: crate::authentication::TenantContext,
    target_id: &str,
    binding_id: &str,
    expected_specification_revision: &str,
    document_media: &printpacket_renderer::Media,
    requested_source: Option<&str>,
) -> Result<PrintPacketMediaFence, AppError> {
    let specification = current_design_specification(state, tenant, target_id).await?;
    if specification.specification_revision != expected_specification_revision {
        return Err(AppError::conflict(
            "design_specification_changed",
            "The target design specification changed; refresh and preflight the template again.",
        ));
    }
    if specification.readiness.status != "ready" {
        return Err(AppError::conflict(
            "target_not_ready",
            "The target has no binding with fresh, trusted loaded-stock evidence.",
        ));
    }
    let destination = specification
        .destinations
        .iter()
        .find(|destination| destination.binding.id == binding_id)
        .ok_or_else(|| {
            AppError::conflict(
                "target_binding_changed",
                "The selected target binding is no longer available.",
            )
        })?;
    let stock = specification.stock.as_ref().ok_or_else(|| {
        AppError::conflict(
            "target_stock_not_configured",
            "PrintPacket target printing requires an active stock revision.",
        )
    })?;
    let media_compatibility = evaluate_binding_media(
        state,
        tenant,
        Some(stock),
        &destination.printer,
        Some(&destination.profile),
        Utc::now(),
        requested_source,
    )
    .await?;
    if media_compatibility.status != "ready" {
        return Err(AppError::conflict(
            "target_media_not_ready",
            "The selected binding does not have fresh, trusted loaded-stock evidence for the effective profile source.",
        ));
    }
    if !document_media_compatible(
        document_media,
        stock,
        media_compatibility.profile_dimensions_mm.as_ref(),
    ) {
        return Err(AppError::conflict(
            "document_media_incompatible",
            "The PrintPacket media does not match the target stock and immutable profile dimensions.",
        ));
    }
    let loaded = media_compatibility
        .loaded_media
        .as_ref()
        .ok_or_else(|| AppError::conflict("target_media_not_ready", "Loaded stock is unknown."))?;
    let loaded_media_snapshot = serde_json::to_string(loaded)
        .map_err(|_| AppError::service_unavailable("loaded_media_serialization_failed"))?;
    Ok(PrintPacketMediaFence {
        specification_revision: specification.specification_revision,
        stock_id: stock.id.clone(),
        stock_revision: stock.revision,
        loaded_media_snapshot,
    })
}

pub(crate) fn document_media_compatible(
    media: &printpacket_renderer::Media,
    stock: &StoredStock,
    profile: Option<&MediaDimensions>,
) -> bool {
    let Some(profile) = profile else {
        return false;
    };
    let stock_width = stock
        .attributes
        .get("width_mm")
        .and_then(serde_json::Value::as_f64);
    let stock_height = stock
        .attributes
        .get("height_mm")
        .or_else(|| stock.attributes.get("length_mm"))
        .and_then(serde_json::Value::as_f64);
    let kind = stock
        .attributes
        .get("kind")
        .and_then(serde_json::Value::as_str);
    match media {
        printpacket_renderer::Media::Paged {
            size, orientation, ..
        } => {
            let (mut width, mut height) = match size {
                printpacket_renderer::PageSize::A4 => (210.0, 297.0),
                printpacket_renderer::PageSize::A5 => (148.0, 210.0),
                printpacket_renderer::PageSize::Letter => (215.9, 279.4),
            };
            if matches!(orientation, printpacket_renderer::Orientation::Landscape) {
                std::mem::swap(&mut width, &mut height);
            }
            let orientation_name =
                if matches!(orientation, printpacket_renderer::Orientation::Landscape) {
                    "landscape"
                } else {
                    "portrait"
                };
            kind == Some("sheet")
                && stock_height.is_some_and(|stock_height| {
                    dimensions_match_unordered(stock_width, Some(stock_height), width, height)
                })
                && dimensions_match_unordered(
                    Some(profile.width_mm),
                    Some(profile.height_mm),
                    width,
                    height,
                )
                && stock_orientation_allows(stock, orientation_name)
        }
        printpacket_renderer::Media::Continuous { width_mm, .. } => {
            let width = f64::from(*width_mm);
            matches!(kind, Some("roll" | "continuous" | "receipt"))
                && stock_width.is_some_and(|value| dimension_close(value, width))
                && dimension_close(profile.width_mm, width)
        }
        printpacket_renderer::Media::Label {
            width_mm,
            height_mm,
            ..
        } => {
            let width = f64::from(*width_mm);
            let height = f64::from(*height_mm);
            let ordered = dimensions_match_ordered(stock_width, stock_height, width, height)
                && dimensions_match_ordered(
                    Some(profile.width_mm),
                    Some(profile.height_mm),
                    width,
                    height,
                );
            let rotated = stock_label_rotatable(stock)
                && dimensions_match_unordered(stock_width, stock_height, width, height)
                && dimensions_match_unordered(
                    Some(profile.width_mm),
                    Some(profile.height_mm),
                    width,
                    height,
                );
            matches!(kind, Some("label" | "roll_label")) && (ordered || rotated)
        }
    }
}

fn dimension_close(left: f64, right: f64) -> bool {
    (left - right).abs() <= MEDIA_DIMENSION_TOLERANCE_MM
}

fn dimensions_match_ordered(
    left_width: Option<f64>,
    left_height: Option<f64>,
    right_width: f64,
    right_height: f64,
) -> bool {
    left_width.is_some_and(|width| dimension_close(width, right_width))
        && left_height.is_some_and(|height| dimension_close(height, right_height))
}

fn dimensions_match_unordered(
    left_width: Option<f64>,
    left_height: Option<f64>,
    right_width: f64,
    right_height: f64,
) -> bool {
    dimensions_match_ordered(left_width, left_height, right_width, right_height)
        || dimensions_match_ordered(left_width, left_height, right_height, right_width)
}

fn stock_orientation_allows(stock: &StoredStock, document_orientation: &str) -> bool {
    match stock
        .attributes
        .get("orientation")
        .and_then(serde_json::Value::as_str)
    {
        None | Some("any") => true,
        Some(orientation) => orientation == document_orientation,
    }
}

fn stock_label_rotatable(stock: &StoredStock) -> bool {
    stock
        .attributes
        .get("rotatable")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

pub(crate) async fn printpacket_execution_failure(
    state: &AppState,
    tenant: crate::authentication::TenantContext,
    job: &piqae_domain::Job,
) -> Result<Option<(piqae_domain::JobFailureReason, &'static str)>, AppError> {
    let Some(pins) = printpacket_execution_pins(&job.metadata) else {
        return Ok(None);
    };
    let pins = match pins {
        Ok(pins) => pins,
        Err(failure) => return Ok(Some(failure)),
    };
    let Ok(media) = serde_json::from_str::<printpacket_renderer::Media>(pins.encoded_media) else {
        return Ok(Some((
            piqae_domain::JobFailureReason::DocumentMediaIncompatible,
            "The pinned PrintPacket media is invalid; publish and submit a valid template revision.",
        )));
    };
    let specification = match current_design_specification(state, tenant, pins.target_id).await {
        Ok(specification) => specification,
        Err(error) if error.is_not_found() => {
            return Ok(Some((
                piqae_domain::JobFailureReason::TargetConfigurationChanged,
                "The pinned target, stock, printer, or profile disappeared before native acceptance.",
            )));
        }
        Err(error) => return Err(error),
    };
    let Some(stock) = specification.stock.as_ref() else {
        return Ok(Some((
            piqae_domain::JobFailureReason::TargetConfigurationChanged,
            "The target stock is no longer configured; restore it and submit a new print attempt.",
        )));
    };
    let destination = specification.destinations.iter().find(|destination| {
        destination.binding.id == *pins.binding_id
            && destination.binding.profile_id == *pins.profile_id
            && destination.binding.profile_revision == pins.profile_revision
            && destination.printer.id == job.printer_id
    });
    let Some(destination) = destination else {
        return Ok(Some((
            piqae_domain::JobFailureReason::TargetConfigurationChanged,
            "The pinned target, printer, or immutable profile revision changed before native acceptance.",
        )));
    };
    let target_or_binding_disabled = !specification.target.enabled || !destination.binding.enabled;
    if target_or_binding_disabled
        || destination.binding.agent_id != destination.printer.agent_id
        || !destination.profile.published
        || !matches!(destination.profile.status.as_deref(), Some("ready") | None)
    {
        return Ok(Some((
            piqae_domain::JobFailureReason::TargetConfigurationChanged,
            "The target, binding, driver, or immutable profile is no longer ready for native acceptance.",
        )));
    }
    if stock.id != *pins.stock_id || stock.revision != pins.stock_revision {
        return Ok(Some((
            piqae_domain::JobFailureReason::TargetConfigurationChanged,
            "The pinned stock revision changed before native acceptance; preflight and submit a new attempt.",
        )));
    }
    let media_compatibility = evaluate_binding_media(
        state,
        tenant,
        Some(stock),
        &destination.printer,
        Some(&destination.profile),
        Utc::now(),
        job.options.bin.as_deref(),
    )
    .await?;
    if media_compatibility.status != "ready" {
        return Ok(Some((
            piqae_domain::JobFailureReason::StockNotLoaded,
            "Fresh trusted evidence no longer confirms the target stock is loaded in the pinned source.",
        )));
    }
    if !document_media_compatible(
        &media,
        stock,
        media_compatibility.profile_dimensions_mm.as_ref(),
    ) {
        return Ok(Some((
            piqae_domain::JobFailureReason::DocumentMediaIncompatible,
            "The PrintPacket media no longer matches the target stock and immutable profile dimensions.",
        )));
    }
    if specification.specification_revision != *pins.specification_revision {
        return Ok(Some((
            piqae_domain::JobFailureReason::TargetConfigurationChanged,
            "The target design specification changed before native acceptance; preflight and submit a new attempt.",
        )));
    }
    Ok(None)
}

type ExecutionFailure = (piqae_domain::JobFailureReason, &'static str);

struct PrintPacketExecutionPins<'a> {
    target_id: &'a str,
    binding_id: &'a str,
    profile_id: &'a str,
    profile_revision: u64,
    specification_revision: &'a str,
    stock_id: &'a str,
    stock_revision: u64,
    encoded_media: &'a str,
}

fn printpacket_execution_pins(
    metadata: &BTreeMap<String, String>,
) -> Option<Result<PrintPacketExecutionPins<'_>, ExecutionFailure>> {
    let target_id = metadata.get("piqae.target_id")?;
    let encoded_media = metadata.get("piqae.document.media")?;
    let pins = (
        metadata.get("piqae.binding_id"),
        metadata.get("piqae.profile_id"),
        metadata
            .get("piqae.profile_revision")
            .and_then(|value| value.parse::<u64>().ok()),
        metadata.get("piqae.design_specification_revision"),
        metadata.get("piqae.stock_id"),
        metadata
            .get("piqae.stock_revision")
            .and_then(|value| value.parse::<u64>().ok()),
    );
    let (
        Some(binding_id),
        Some(profile_id),
        Some(profile_revision),
        Some(specification_revision),
        Some(stock_id),
        Some(stock_revision),
    ) = pins
    else {
        return Some(Err((
            piqae_domain::JobFailureReason::TargetConfigurationChanged,
            "PrintPacket target execution pins are incomplete; refresh the destination and submit a new print attempt.",
        )));
    };
    Some(Ok(PrintPacketExecutionPins {
        target_id,
        binding_id,
        profile_id,
        profile_revision,
        specification_revision,
        stock_id,
        stock_revision,
        encoded_media,
    }))
}

pub(crate) async fn evaluate_binding_media(
    state: &AppState,
    tenant: crate::authentication::TenantContext,
    stock: Option<&StoredStock>,
    printer: &piqae_storage_postgres::StoredPrinter,
    profile: Option<&PrinterProfileSnapshot>,
    now: DateTime<Utc>,
    requested_source: Option<&str>,
) -> Result<MediaCompatibility, AppError> {
    let profile_dimensions_mm = profile.and_then(profile_dimensions);
    let mut result = MediaCompatibility {
        status: "incompatible",
        reasons: Vec::new(),
        profile_dimensions_mm,
        loaded_media: None,
    };
    let Some(stock) = stock else {
        result.reasons.push("target_stock_not_configured".into());
        return Ok(result);
    };
    if stock.archived {
        result.reasons.push("target_stock_is_archived".into());
        return Ok(result);
    }
    let Some(profile) = profile else {
        result.reasons.push("profile_revision_unavailable".into());
        return Ok(result);
    };
    if profile.stock_id.as_deref() != Some(stock.id.as_str()) {
        result
            .reasons
            .push("profile_stock_does_not_match_target".into());
        return Ok(result);
    }
    if !stock_profile_dimensions_compatible(stock, result.profile_dimensions_mm.as_ref()) {
        result
            .reasons
            .push("profile_dimensions_do_not_match_stock".into());
        return Ok(result);
    }

    let observations = state
        .repository
        .list_loaded_media(tenant.workspace_id, tenant.environment_id, printer.id)
        .await?;
    let expected_source = match effective_profile_media_source(profile, requested_source) {
        Ok(source) => source,
        Err(reason) => {
            result.reasons.push(reason.into());
            return Ok(result);
        }
    };
    let selected = match expected_source.as_deref() {
        Some(source) => observations
            .iter()
            .find(|observation| observation.source == source),
        None if observations.len() == 1 => observations.first(),
        None => None,
    };
    let Some(selected) = selected else {
        result.status = "not_reported";
        result.reasons.push(if expected_source.is_some() {
            "loaded_media_not_reported_for_profile_source".into()
        } else {
            "profile_media_source_not_configured".into()
        });
        return Ok(result);
    };
    result.loaded_media = Some(loaded_media_evidence(selected));
    if selected.observed_at + TimeDelta::seconds(LOADED_MEDIA_FRESHNESS_SECONDS) < now {
        result.status = "stale";
        result.reasons.push("loaded_media_observation_stale".into());
        return Ok(result);
    }
    if !matches!(
        selected.confidence.as_str(),
        "reported" | "operator_confirmed"
    ) {
        result.status = "untrusted";
        result
            .reasons
            .push("loaded_media_confidence_untrusted".into());
        return Ok(result);
    }
    if selected.calibration_state != "current" {
        result.status = "untrusted";
        result
            .reasons
            .push("loaded_media_calibration_not_current".into());
        return Ok(result);
    }
    if selected.stock_id.as_deref() != Some(stock.id.as_str())
        || selected.stock_revision != Some(stock.revision)
    {
        result.status = "incompatible";
        result
            .reasons
            .push("loaded_stock_revision_does_not_match_target".into());
        return Ok(result);
    }
    result.status = "ready";
    Ok(result)
}

fn profile_media_source(profile: &PrinterProfileSnapshot) -> Option<String> {
    profile
        .summary
        .as_ref()
        .and_then(|summary| summary.get("source"))
        .and_then(serde_json::Value::as_str)
        .or(profile.options.bin.as_deref())
        .map(str::to_owned)
}

pub(crate) fn effective_profile_media_source(
    profile: &PrinterProfileSnapshot,
    requested_source: Option<&str>,
) -> Result<Option<String>, &'static str> {
    let configured = profile_media_source(profile);
    let Some(requested) = requested_source else {
        return Ok(configured);
    };
    if configured.as_deref() == Some(requested) {
        return Ok(configured);
    }
    if profile
        .safe_overrides
        .iter()
        .any(|override_name| override_name == "bin")
    {
        return Ok(Some(requested.to_owned()));
    }
    Err("media_source_override_not_allowed")
}

pub(crate) fn profile_dimensions(profile: &PrinterProfileSnapshot) -> Option<MediaDimensions> {
    let dimensions = profile.summary.as_ref()?.get("dimensions_mm")?.as_array()?;
    let width_mm = dimensions.first()?.as_f64()?;
    let height_mm = dimensions.get(1)?.as_f64()?;
    (width_mm.is_finite() && height_mm.is_finite() && width_mm > 0.0 && height_mm > 0.0).then_some(
        MediaDimensions {
            width_mm,
            height_mm,
        },
    )
}

pub(crate) fn stock_profile_dimensions_compatible(
    stock: &StoredStock,
    profile: Option<&MediaDimensions>,
) -> bool {
    let Some(profile) = profile else {
        return false;
    };
    let Some(width) = stock
        .attributes
        .get("width_mm")
        .and_then(serde_json::Value::as_f64)
    else {
        return false;
    };
    let height = stock
        .attributes
        .get("height_mm")
        .or_else(|| stock.attributes.get("length_mm"))
        .and_then(serde_json::Value::as_f64);
    match stock
        .attributes
        .get("kind")
        .and_then(serde_json::Value::as_str)
    {
        Some("roll" | "continuous" | "receipt") => dimension_close(width, profile.width_mm),
        Some("sheet" | "card" | "envelope") => {
            dimensions_match_unordered(Some(width), height, profile.width_mm, profile.height_mm)
        }
        Some("label" | "roll_label") if stock_label_rotatable(stock) => {
            dimensions_match_unordered(Some(width), height, profile.width_mm, profile.height_mm)
        }
        Some("label" | "roll_label") => {
            dimensions_match_ordered(Some(width), height, profile.width_mm, profile.height_mm)
        }
        _ => false,
    }
}

fn loaded_media_evidence(observation: &StoredLoadedMedia) -> LoadedMediaEvidence {
    LoadedMediaEvidence {
        source: observation.source.clone(),
        confidence: observation.confidence.clone(),
        observed_at: observation.observed_at,
        fresh_until: observation.observed_at + TimeDelta::seconds(LOADED_MEDIA_FRESHNESS_SECONDS),
        stock: observation
            .stock_id
            .clone()
            .zip(observation.stock_revision)
            .map(|(id, revision)| LoadedMediaStock { id, revision }),
    }
}

const NODE_HEARTBEAT_STALE_AFTER_SECONDS: i64 = 90;

pub(crate) fn agent_is_connected(agent: &StoredAgent) -> bool {
    agent_is_connected_at(agent, Utc::now())
}

fn agent_is_connected_at(agent: &StoredAgent, now: DateTime<Utc>) -> bool {
    agent.state == "connected"
        && agent.last_seen_at >= now - TimeDelta::seconds(NODE_HEARTBEAT_STALE_AFTER_SECONDS)
}

fn binding_printer_readiness(
    binding: &StoredTargetBinding,
    printer: &piqae_storage_postgres::StoredPrinter,
    reasons: &mut Vec<String>,
) -> &'static str {
    if printer.agent_id != binding.agent_id {
        reasons.push("binding_agent_changed".into());
        return "destination_missing";
    }
    let Some(profile) = printer.profiles.iter().find(|profile| {
        (profile.profile_id.as_str(), profile.revision)
            == (binding.profile_id.as_str(), binding.profile_revision)
    }) else {
        reasons.push("profile_revision_not_in_current_snapshot".into());
        return "profile_stale";
    };
    if !profile.published {
        return "profile_stale";
    }
    match profile.status.as_deref() {
        Some("ready") | None => printer_readiness(printer.state),
        Some("driver_mismatch") => "driver_mismatch",
        Some("dependency_missing") => "dependency_missing",
        Some("destination_missing") => "destination_missing",
        Some("interactive_only") => "needs_operator",
        Some(_) => "profile_stale",
    }
}

const fn printer_readiness(state: PrinterState) -> &'static str {
    match state {
        PrinterState::Online => "ready",
        PrinterState::Busy => "busy",
        PrinterState::PaperOut | PrinterState::Paused | PrinterState::Error => "needs_operator",
        PrinterState::Offline | PrinterState::Unknown => "destination_offline",
    }
}

fn validate_name(value: &str, code: &'static str) -> Result<String, AppError> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 120 {
        return Err(AppError::invalid(
            code,
            "The resource name must contain between 1 and 120 characters.",
        ));
    }
    Ok(value.to_owned())
}

fn clean_optional(
    value: Option<String>,
    max: usize,
    code: &'static str,
) -> Result<Option<String>, AppError> {
    value
        .map(|value| {
            let value = value.trim();
            if value.chars().count() > max {
                Err(AppError::invalid(
                    code,
                    "A field exceeds its maximum length.",
                ))
            } else if value.is_empty() {
                Ok(None)
            } else {
                Ok(Some(value.to_owned()))
            }
        })
        .transpose()
        .map(Option::flatten)
}

fn validate_routing_policy(value: &str) -> Result<(), AppError> {
    if value != "primary_then_standby" {
        return Err(AppError::invalid(
            "invalid_target",
            "The routing policy is not supported.",
        ));
    }
    Ok(())
}

fn invalid_safe_override_set(values: &[String]) -> bool {
    let mut seen = HashSet::new();
    values.iter().any(|value| !seen.insert(value.as_str()))
        || values.iter().enumerate().any(|(index, left)| {
            values.iter().skip(index + 1).any(|right| {
                left.strip_prefix(right)
                    .is_some_and(|suffix| suffix.starts_with('.'))
                    || right
                        .strip_prefix(left)
                        .is_some_and(|suffix| suffix.starts_with('.'))
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stock(kind: &str, width_mm: f64, height_mm: Option<f64>) -> StoredStock {
        StoredStock {
            id: "stk_test".into(),
            revision: 3,
            name: "Test stock".into(),
            sku: None,
            description: None,
            attributes: serde_json::json!({
                "kind": kind,
                "width_mm": width_mm,
                "height_mm": height_mm,
            }),
            archived: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn printpacket_media_must_match_stock_and_immutable_profile_geometry() {
        let profile = MediaDimensions {
            width_mm: 100.0,
            height_mm: 50.0,
        };
        let label = printpacket_renderer::Media::Label {
            width_mm: 100.0,
            height_mm: 50.0,
            margins: printpacket_renderer::Edges {
                top_mm: 0.0,
                right_mm: 0.0,
                bottom_mm: 0.0,
                left_mm: 0.0,
            },
        };
        assert!(document_media_compatible(
            &label,
            &stock("label", 100.0, Some(50.0)),
            Some(&profile),
        ));
        assert!(!document_media_compatible(
            &label,
            &stock("label", 102.0, Some(50.0)),
            Some(&profile),
        ));
        assert!(!document_media_compatible(
            &label,
            &stock("sheet", 100.0, Some(50.0)),
            Some(&profile),
        ));
        let mut incomplete_stock = stock("label", 100.0, None);
        incomplete_stock.attributes = serde_json::json!({
            "kind": "label",
            "width_mm": 100.0,
        });
        assert!(!document_media_compatible(
            &label,
            &incomplete_stock,
            Some(&profile),
        ));
    }

    #[test]
    fn continuous_media_compares_width_without_inventing_a_cut_length() {
        let profile = MediaDimensions {
            width_mm: 80.0,
            height_mm: 200.0,
        };
        let receipt = printpacket_renderer::Media::Continuous {
            width_mm: 80.0,
            margins: printpacket_renderer::Edges {
                top_mm: 2.0,
                right_mm: 2.0,
                bottom_mm: 2.0,
                left_mm: 2.0,
            },
        };
        assert!(document_media_compatible(
            &receipt,
            &stock("receipt", 80.0, None),
            Some(&profile),
        ));
    }

    #[test]
    fn paged_sheet_geometry_is_physical_and_explicit_orientation_is_enforced() {
        for (size, width_mm, height_mm) in [
            (printpacket_renderer::PageSize::A4, 210.0, 297.0),
            (printpacket_renderer::PageSize::A5, 148.0, 210.0),
            (printpacket_renderer::PageSize::Letter, 215.9, 279.4),
        ] {
            let profile = MediaDimensions {
                width_mm,
                height_mm,
            };
            let landscape = printpacket_renderer::Media::Paged {
                size,
                orientation: printpacket_renderer::Orientation::Landscape,
                margins: printpacket_renderer::Edges {
                    top_mm: 0.0,
                    right_mm: 0.0,
                    bottom_mm: 0.0,
                    left_mm: 0.0,
                },
            };
            let mut physical_stock = stock("sheet", width_mm, Some(height_mm));
            assert!(document_media_compatible(
                &landscape,
                &physical_stock,
                Some(&profile),
            ));
            physical_stock.attributes["orientation"] = serde_json::json!("portrait");
            assert!(!document_media_compatible(
                &landscape,
                &physical_stock,
                Some(&profile),
            ));
            physical_stock.attributes["orientation"] = serde_json::json!("landscape");
            assert!(document_media_compatible(
                &landscape,
                &physical_stock,
                Some(&profile),
            ));
        }
    }

    #[test]
    fn label_rotation_requires_an_explicit_stock_declaration() {
        let profile = MediaDimensions {
            width_mm: 100.0,
            height_mm: 50.0,
        };
        let rotated_label = printpacket_renderer::Media::Label {
            width_mm: 50.0,
            height_mm: 100.0,
            margins: printpacket_renderer::Edges {
                top_mm: 0.0,
                right_mm: 0.0,
                bottom_mm: 0.0,
                left_mm: 0.0,
            },
        };
        let mut fixed = stock("label", 100.0, Some(50.0));
        assert!(!document_media_compatible(
            &rotated_label,
            &fixed,
            Some(&profile),
        ));
        fixed.attributes["rotatable"] = serde_json::json!(true);
        assert!(document_media_compatible(
            &rotated_label,
            &fixed,
            Some(&profile),
        ));
    }

    #[test]
    fn name_limits_count_unicode_characters_not_bytes() {
        let valid = "🖨".repeat(120);
        assert_eq!(
            validate_name(&valid, "invalid_target").ok().as_deref(),
            Some(valid.as_str())
        );
        assert!(validate_name(&"🖨".repeat(121), "invalid_target").is_err());
    }

    #[test]
    fn optional_field_limits_count_unicode_characters_not_bytes() {
        let valid = "é".repeat(2_000);
        assert_eq!(
            clean_optional(Some(valid.clone()), 2_000, "invalid_target").ok(),
            Some(Some(valid))
        );
        assert!(clean_optional(Some("é".repeat(2_001)), 2_000, "invalid_target").is_err());
    }

    #[test]
    fn safe_overrides_reject_duplicates_and_parent_child_ambiguity() {
        assert!(invalid_safe_override_set(&[
            "semantic_options.media".into(),
            "semantic_options.media.sensing".into(),
        ]));
        assert!(invalid_safe_override_set(&[
            "portable_options.copies".into(),
            "portable_options.copies".into(),
        ]));
        assert!(!invalid_safe_override_set(&[
            "portable_options.copies".into(),
            "semantic_options.media.sensing".into(),
        ]));
    }

    #[test]
    fn connected_nodes_are_fenced_when_heartbeats_go_stale() {
        let now = Utc::now();
        let mut agent = StoredAgent {
            id: piqae_domain::AgentId::new(),
            name: "Node".into(),
            site: None,
            location: None,
            labels: Vec::new(),
            identity_revision: 1,
            platform: "macos".into(),
            state: "connected".into(),
            version: "0.1.0".into(),
            last_seen_at: now - TimeDelta::seconds(89),
            health_started_at: None,
            health_observed_at: None,
            sqlite_integrity_ok: None,
            executor_crashes: 0,
            last_error_code: None,
            document_render: piqae_protocol::agent::DocumentRenderCapabilities::default(),
        };
        assert!(agent_is_connected_at(&agent, now));
        agent.last_seen_at = now - TimeDelta::seconds(91);
        assert!(!agent_is_connected_at(&agent, now));
        agent.state = "paused".into();
        agent.last_seen_at = now;
        assert!(!agent_is_connected_at(&agent, now));
    }
}
