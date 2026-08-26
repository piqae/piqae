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
    StoredAgent, StoredBindingReadiness, StoredPrintWorkflow, StoredStock, StoredTarget,
    StoredTargetBinding, StoredTargetReadiness,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
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
    let target_stock_available = if let Some(stock_id) = target.stock_id.as_deref() {
        !state
            .repository
            .get_stock(tenant.workspace_id, tenant.environment_id, stock_id)
            .await?
            .archived
    } else {
        true
    };
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
        } else if !target_stock_available {
            reasons.push("target_stock_is_archived".into());
            "dependency_missing"
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
                Ok(printer) => binding_printer_readiness(&target, &binding, &printer, &mut reasons),
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
    binding: StoredTargetBinding,
    printer: piqae_storage_postgres::StoredPrinter,
    profile: piqae_storage_postgres::PrinterProfileSnapshot,
}

#[derive(Debug, Serialize)]
pub struct DesignSpecificationResponse {
    target: StoredTarget,
    stock: Option<StoredStock>,
    readiness: StoredTargetReadiness,
    destinations: Vec<DesignSpecificationDestination>,
    specification_revision: String,
}

pub async fn design_specification(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(target_id): Path<String>,
) -> Result<Json<DesignSpecificationResponse>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::PrintersRead).await?;
    let target = state
        .repository
        .get_target(tenant.workspace_id, tenant.environment_id, &target_id)
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
    let readiness = compute_target_readiness(&state, tenant, &target_id).await?;
    let bindings = state
        .repository
        .list_target_bindings(tenant.workspace_id, tenant.environment_id, &target_id)
        .await?;
    let mut destinations = Vec::with_capacity(bindings.len());
    for binding in bindings {
        let printer = state
            .repository
            .get_printer(
                tenant.workspace_id,
                tenant.environment_id,
                binding.printer_id,
            )
            .await?;
        let profile = printer
            .profiles
            .iter()
            .find(|profile| {
                (profile.profile_id.as_str(), profile.revision)
                    == (binding.profile_id.as_str(), binding.profile_revision)
            })
            .cloned()
            .ok_or(RepositoryError::NotFound)?;
        destinations.push(DesignSpecificationDestination {
            binding,
            printer,
            profile,
        });
    }
    let canonical = serde_json::to_vec(&(&target, &stock, &readiness, &destinations))
        .map_err(|_| AppError::service_unavailable("design_specification_serialization_failed"))?;
    let revision = format!("spec_{:x}", Sha256::digest(canonical));
    Ok(Json(DesignSpecificationResponse {
        target,
        stock,
        readiness,
        destinations,
        specification_revision: revision,
    }))
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
    target: &StoredTarget,
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
    if target.stock_id.is_some() && target.stock_id != profile.stock_id {
        reasons.push("profile_stock_does_not_match_target".into());
        return "dependency_missing";
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
            platform: "macos".into(),
            state: "connected".into(),
            version: "0.1.0".into(),
            last_seen_at: now - TimeDelta::seconds(89),
            health_started_at: None,
            health_observed_at: None,
            sqlite_integrity_ok: None,
            executor_crashes: 0,
            last_error_code: None,
        };
        assert!(agent_is_connected_at(&agent, now));
        agent.last_seen_at = now - TimeDelta::seconds(91);
        assert!(!agent_is_connected_at(&agent, now));
        agent.state = "paused".into();
        agent.last_seen_at = now;
        assert!(!agent_is_connected_at(&agent, now));
    }
}
