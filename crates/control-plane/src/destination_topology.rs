//! Tenant-scoped physical destinations, installed routes, and fenced delivery history.
//!
//! Public responses deliberately separate route telemetry, connector inventory
//! projection, and scheduling authority. A node heartbeat is never presented as
//! proof that a route inventory is current.

#![allow(clippy::missing_errors_doc)]

use crate::{AppState, api::authenticate_native, error::AppError};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use chrono::{DateTime, TimeDelta, Utc};
use hmac::{Hmac, Mac};
use piqae_auth::Scope;
use piqae_storage_postgres::{
    DeliveryAttempt, IdentityConfidence, IdentityDecision, IdentityDecisionKind,
    IdentityEvidence, ProjectionAcknowledgement, RouteObservation, RouteReservation,
    SchedulingAuthority, SiteCoordinatorMembership, StorageError, StoredPhysicalDestination,
    StoredPrinterRoute, TenantScope,
};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::collections::{HashMap, HashSet};

fn enum_name<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| "unknown".into())
}

fn protocol_confidence(
    value: piqae_protocol::agent::IdentityConfidence,
) -> IdentityConfidence {
    match value {
        piqae_protocol::agent::IdentityConfidence::Verified => IdentityConfidence::Verified,
        piqae_protocol::agent::IdentityConfidence::HighConfidence => {
            IdentityConfidence::High
        }
        piqae_protocol::agent::IdentityConfidence::PossibleMatch => {
            IdentityConfidence::Possible
        }
        piqae_protocol::agent::IdentityConfidence::Distinct => IdentityConfidence::Conflict,
        piqae_protocol::agent::IdentityConfidence::Unknown => IdentityConfidence::Unknown,
    }
}

fn evidence_is_strong(strength: &str) -> bool {
    matches!(strength, "strong" | "verified")
}

fn tenant_evidence_digest(
    state: &AppState,
    tenant_scope: TenantScope,
    normalized_node_digest: &str,
) -> Result<String, AppError> {
    if normalized_node_digest.len() != 64
        || normalized_node_digest != normalized_node_digest.to_ascii_lowercase()
        || !normalized_node_digest.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(AppError::invalid(
            "invalid_identity_evidence",
            "Identity evidence must be a normalized lowercase SHA-256 value.",
        ));
    }
    let mut mac = Hmac::<Sha256>::new_from_slice(&state.destination_identity_key)
        .map_err(|_| AppError::service_unavailable("destination_identity_key_unavailable"))?;
    mac.update(tenant_scope.workspace_id.to_string().as_bytes());
    mac.update(b"\0");
    mac.update(tenant_scope.environment_id.to_string().as_bytes());
    mac.update(b"\0");
    mac.update(normalized_node_digest.as_bytes());
    Ok(format!("hmac-sha256:{}", hex::encode(mac.finalize().into_bytes())))
}

fn scope(tenant: crate::authentication::TenantContext) -> TenantScope {
    TenantScope {
        workspace_id: tenant.workspace_id,
        environment_id: tenant.environment_id,
    }
}

fn storage_error(error: StorageError) -> AppError {
    match error {
        StorageError::NotFound => AppError::not_found(),
        StorageError::ConcurrentStateChange | StorageError::InvalidTransition => {
            AppError::conflict("destination_state_changed", "The destination or route state changed concurrently.")
        }
        StorageError::InvalidData(message) => AppError::invalid("invalid_destination_topology", message),
        _ => AppError::service_unavailable("destination_topology_unavailable"),
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct PhysicalDestinationResponse {
    pub id: String,
    pub display_name: String,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub identity_confidence: &'static str,
    pub status: String,
    pub route_count: usize,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize)]
pub struct RouteObservationResponse {
    pub id: String,
    pub route_id: String,
    pub sequence: u64,
    pub printer_state: String,
    pub state_reasons: Vec<String>,
    pub accepting_jobs: bool,
    pub total_jobs: u32,
    pub active_jobs: u32,
    pub held_jobs: u32,
    pub connector_jobs: u32,
    pub other_piqae_or_external_jobs: u32,
    pub unknown_jobs: u32,
    pub estimated_busy_seconds: Option<u64>,
    pub observed_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PrinterRouteResponse {
    pub id: String,
    pub physical_destination_id: String,
    pub printer_id: String,
    pub agent_id: String,
    pub native_queue_id: String,
    pub local_route_key: Option<String>,
    pub enabled: bool,
    pub health: &'static str,
    pub telemetry_freshness: &'static str,
    pub projection_health: &'static str,
    pub latest_observation: Option<RouteObservationResponse>,
    pub scheduling_authority_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize)]
pub struct IdentityEvidenceResponse {
    pub id: String,
    pub destination_id: String,
    pub route_id: String,
    pub kind: String,
    pub confidence: &'static str,
    pub observed_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize)]
pub struct IdentityDecisionResponse {
    pub id: String,
    pub destination_id: String,
    pub kind: &'static str,
    pub route_ids: Vec<String>,
    pub reason: String,
    pub actor_id: String,
    pub reverses_decision_id: Option<String>,
    pub reversed_by_decision_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateIdentityDecisionRequest {
    kind: DecisionRequestKind,
    route_ids: Vec<String>,
    reason: String,
    display_name: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DecisionRequestKind {
    Merge,
    Split,
}

#[derive(Debug, Deserialize)]
pub struct ObservationQuery {
    #[serde(default = "default_observation_limit")]
    limit: u32,
}

const fn default_observation_limit() -> u32 {
    100
}

const fn identity_confidence(value: IdentityConfidence) -> &'static str {
    match value {
        IdentityConfidence::Verified => "verified",
        IdentityConfidence::High => "high",
        IdentityConfidence::Possible => "possible",
        IdentityConfidence::Conflict => "conflict",
        IdentityConfidence::Unknown => "unknown",
    }
}

fn evidence_confidence(evidence: &IdentityEvidence) -> &'static str {
    if evidence.conflicts {
        "distinct"
    } else {
        match evidence.strength.as_str() {
            "verified" => "verified",
            "strong" => "high",
            "medium" => "possible",
            _ => "unknown",
        }
    }
}

fn observation_response(value: RouteObservation) -> RouteObservationResponse {
    RouteObservationResponse {
        id: value.id,
        route_id: value.route_id,
        sequence: value.sequence,
        printer_state: value.printer_state,
        state_reasons: value.state_reasons,
        accepting_jobs: value.accepting_jobs.unwrap_or(false),
        total_jobs: value.total_jobs,
        active_jobs: value.active_jobs,
        held_jobs: value.held_jobs,
        connector_jobs: value.connector_jobs,
        other_piqae_or_external_jobs: value.other_piqae_or_external_jobs,
        unknown_jobs: value.unknown_jobs,
        estimated_busy_seconds: value.estimated_busy_seconds.map(u64::from),
        observed_at: value.observed_at,
        expires_at: value.fresh_until,
    }
}

async fn route_response(
    state: &AppState,
    tenant_scope: TenantScope,
    route: StoredPrinterRoute,
) -> Result<PrinterRouteResponse, AppError> {
    let observation = match state
        .destination_topology
        .latest_route_observation(tenant_scope, &route.id)
        .await
    {
        Ok(value) => Some(value),
        Err(StorageError::NotFound) => None,
        Err(error) => return Err(storage_error(error)),
    };
    let projection = match state
        .destination_topology
        .get_projection_acknowledgement(tenant_scope, &route.agent_id, &route.id)
        .await
    {
        Ok(value) => Some(value),
        Err(StorageError::NotFound) => None,
        Err(error) => return Err(storage_error(error)),
    };
    let now = Utc::now();
    let telemetry_freshness = observation.as_ref().map_or("never", |value| {
        if value.fresh_until >= now {
            "live"
        } else if value.observed_at + TimeDelta::minutes(5) >= now {
            "recent"
        } else {
            "stale"
        }
    });
    let health = if !route.enabled || route.state == "offline" {
        "offline"
    } else if telemetry_freshness == "stale" || telemetry_freshness == "never" {
        "stale"
    } else {
        match observation.as_ref().map(|value| value.printer_state.as_str()) {
            Some("online") if observation.as_ref().is_some_and(|value| value.accepting_jobs == Some(true)) => "ready",
            Some("busy") | Some("printing") => "busy",
            Some("paused" | "paper_out" | "error") => "needs_operator",
            Some("offline") => "offline",
            _ => "unknown",
        }
    };
    let projection_health = projection.as_ref().map_or("unsupported", |value| {
        match value.status.as_str() {
            "current" | "projected" => "current",
            "failed" => "failed",
            _ => "pending",
        }
    });
    let authority = state
        .destination_topology
        .get_destination(tenant_scope, &route.destination_id)
        .await
        .map_err(storage_error)?
        .scheduling_authority_id;
    Ok(PrinterRouteResponse {
        id: route.id,
        physical_destination_id: route.destination_id,
        printer_id: route.printer_id,
        agent_id: route.agent_id,
        native_queue_id: route.native_queue_id,
        local_route_key: route.local_route_key,
        enabled: route.enabled,
        health,
        telemetry_freshness,
        projection_health,
        latest_observation: observation.map(observation_response),
        scheduling_authority_id: authority,
        created_at: route.updated_at,
        updated_at: route.updated_at,
    })
}

async fn destination_response(
    state: &AppState,
    tenant_scope: TenantScope,
    destination: StoredPhysicalDestination,
) -> Result<PhysicalDestinationResponse, AppError> {
    let route_count = state
        .destination_topology
        .list_routes(tenant_scope, &destination.id)
        .await
        .map_err(storage_error)?
        .len();
    Ok(PhysicalDestinationResponse {
        id: destination.id,
        display_name: destination.name,
        manufacturer: None,
        model: None,
        identity_confidence: identity_confidence(destination.identity_confidence),
        status: destination.state,
        route_count,
        created_at: destination.updated_at,
        updated_at: destination.updated_at,
    })
}

/// Builds a bounded customer-attributed topology snapshot for the platform
/// operations endpoint. Callers retain the customer envelope; no row is ever
/// joined across tenant scopes.
pub(crate) async fn operational_snapshot(
    state: &AppState,
    tenant_scope: TenantScope,
    limit: usize,
) -> Result<
    (
        Vec<PhysicalDestinationResponse>,
        Vec<PrinterRouteResponse>,
        Vec<RouteObservationResponse>,
    ),
    AppError,
> {
    let mut destinations = state
        .destination_topology
        .list_destinations(tenant_scope)
        .await
        .map_err(storage_error)?;
    destinations.truncate(limit);
    let mut destination_responses = Vec::with_capacity(destinations.len());
    for destination in destinations {
        destination_responses.push(destination_response(state, tenant_scope, destination).await?);
    }
    let mut routes = state
        .destination_topology
        .list_all_routes(tenant_scope)
        .await
        .map_err(storage_error)?;
    routes.truncate(limit);
    let mut route_responses = Vec::with_capacity(routes.len());
    let mut observations = Vec::new();
    for route in routes {
        let response = route_response(state, tenant_scope, route).await?;
        if let Some(observation) = response.latest_observation.clone() {
            observations.push(observation);
        }
        route_responses.push(response);
    }
    Ok((destination_responses, route_responses, observations))
}

fn evidence_kind(kind: piqae_protocol::agent::PhysicalIdentityEvidenceKind) -> &'static str {
    use piqae_protocol::agent::PhysicalIdentityEvidenceKind as Kind;
    match kind {
        Kind::IppPrinterUuid => "ipp_uuid",
        Kind::DeviceSerial => "device_serial",
        Kind::UsbSerial => "usb_serial",
        Kind::NetworkCertificate => "certificate_key",
        Kind::MacAddress => "network_mac",
        Kind::MdnsEndpoint => "network_endpoint",
        Kind::DriverFingerprint => "driver_fingerprint",
        Kind::NativeQueue => "native_queue",
    }
}

fn evidence_strength(
    strength: piqae_protocol::agent::IdentityEvidenceStrength,
) -> &'static str {
    use piqae_protocol::agent::IdentityEvidenceStrength as Strength;
    match strength {
        Strength::Strong => "strong",
        Strength::Supporting => "medium",
        Strength::Weak => "weak",
    }
}

async fn destination_for_new_route(
    state: &AppState,
    tenant_scope: TenantScope,
    evidence: &[(String, String)],
) -> Result<(String, bool), AppError> {
    let strong = evidence
        .iter()
        .filter(|(strength, _)| evidence_is_strong(strength))
        .map(|(_, digest)| digest.as_str())
        .collect::<HashSet<_>>();
    if strong.is_empty() {
        return Ok((format!("pdst_{}", ulid::Ulid::new()), false));
    }
    let destinations = state
        .destination_topology
        .list_destinations(tenant_scope)
        .await
        .map_err(storage_error)?;
    let mut matches = Vec::new();
    for destination in destinations {
        let stored = state
            .destination_topology
            .list_identity_evidence(tenant_scope, &destination.id)
            .await
            .map_err(storage_error)?;
        if stored.iter().any(|item| {
            evidence_is_strong(&item.strength) && strong.contains(item.value_digest.as_str())
        }) {
            matches.push(destination.id);
        }
    }
    matches.sort();
    matches.dedup();
    match matches.as_slice() {
        [destination_id] => Ok((destination_id.clone(), false)),
        [] => Ok((format!("pdst_{}", ulid::Ulid::new()), false)),
        _ => Ok((format!("pdst_{}", ulid::Ulid::new()), true)),
    }
}

/// Durably projects one authenticated connector's installation topology.
/// Inventory acknowledgement is returned only after every destination, route,
/// evidence record and route observation has committed.
pub(crate) async fn project_agent_topology(
    state: &AppState,
    tenant: crate::authentication::TenantContext,
    request: &piqae_protocol::agent::AgentSyncRequest,
) -> Result<Option<piqae_protocol::agent::InventoryProjectionAcknowledgement>, AppError> {
    let tenant_scope = scope(tenant);
    let now = Utc::now();
    let authority_id = "sched_primary".to_owned();
    state
        .destination_topology
        .upsert_scheduling_authority(
            tenant_scope,
            &SchedulingAuthority {
                id: authority_id.clone(),
                kind: state.capabilities.deployment.clone(),
                authority_key: format!(
                    "{}:{}:{}",
                    state.capabilities.deployment, tenant.workspace_id, tenant.environment_id
                ),
                display_name: if state.capabilities.deployment == "cloud" {
                    "Piqae Cloud".into()
                } else {
                    "Self-hosted Piqae".into()
                },
                active: true,
            },
        )
        .await
        .map_err(storage_error)?;
    state
        .destination_topology
        .upsert_site_membership(
            tenant_scope,
            &SiteCoordinatorMembership {
                authority_id: authority_id.clone(),
                agent_id: request.agent_id.to_string(),
                site_id: request.agent_id.to_string(),
                state: "online".into(),
                last_seen_at: Some(request.health.observed_at),
            },
        )
        .await
        .map_err(storage_error)?;

    let mut projected_revision = None::<u64>;
    let mut local_to_server = HashMap::new();
    if let Some(printers) = request.printers.as_ref() {
        for printer in printers {
            let Some(snapshot) = printer.route.as_ref() else {
                continue;
            };
            if matches!(snapshot.topology_change, Some(piqae_protocol::agent::TopologyChange::Removed)) {
                continue;
            }
            let evidence = snapshot
                .identity_evidence
                .iter()
                .map(|item| {
                    Ok((
                        evidence_strength(item.strength).to_owned(),
                        tenant_evidence_digest(state, tenant_scope, &item.value_sha256)?,
                    ))
                })
                .collect::<Result<Vec<_>, AppError>>()?;
            let existing = match state
                .destination_topology
                .get_route_by_local_key(
                    tenant_scope,
                    &request.agent_id.to_string(),
                    &snapshot.local_route_key,
                )
                .await
            {
                Ok(route) => Some(route),
                Err(StorageError::NotFound) => state
                    .destination_topology
                    .list_all_routes(tenant_scope)
                    .await
                    .map_err(storage_error)?
                    .into_iter()
                    .find(|route| {
                        route.agent_id == request.agent_id.to_string()
                            && route.printer_id == printer.id.to_string()
                    }),
                Err(error) => return Err(storage_error(error)),
            };
            let (destination_id, conflicts) = if let Some(route) = existing.as_ref() {
                (route.destination_id.clone(), false)
            } else {
                destination_for_new_route(state, tenant_scope, &evidence).await?
            };
            let confidence = if conflicts {
                IdentityConfidence::Conflict
            } else {
                protocol_confidence(snapshot.identity_confidence)
            };
            match state
                .destination_topology
                .get_destination(tenant_scope, &destination_id)
                .await
            {
                Ok(mut destination) => {
                    if conflicts {
                        destination.state = "needs_review".into();
                        destination.identity_confidence = IdentityConfidence::Conflict;
                    }
                    destination.updated_at = snapshot.observed_at;
                    state
                        .destination_topology
                        .upsert_destination(tenant_scope, &destination)
                        .await
                        .map_err(storage_error)?;
                }
                Err(StorageError::NotFound) => state
                    .destination_topology
                    .upsert_destination(
                        tenant_scope,
                        &StoredPhysicalDestination {
                            id: destination_id.clone(),
                            name: printer.name.clone(),
                            identity_confidence: confidence,
                            state: if conflicts { "needs_review" } else { "active" }.into(),
                            scheduling_authority_id: Some(authority_id.clone()),
                            identity_revision: snapshot.topology_revision.max(1),
                            updated_at: snapshot.observed_at,
                        },
                    )
                    .await
                    .map_err(storage_error)?,
                Err(error) => return Err(storage_error(error)),
            }
            let server_route_id = existing
                .as_ref()
                .map(|route| route.id.clone())
                .unwrap_or_else(|| format!("rte_{}", ulid::Ulid::new()));
            state
                .destination_topology
                .upsert_route(
                    tenant_scope,
                    &StoredPrinterRoute {
                        id: server_route_id.clone(),
                        destination_id: destination_id.clone(),
                        local_route_key: Some(snapshot.local_route_key.clone()),
                        printer_id: printer.id.to_string(),
                        agent_id: request.agent_id.to_string(),
                        native_queue_id: printer.native_id.clone(),
                        state: enum_name(&printer.state),
                        role: existing.as_ref().map_or("standby", |route| route.role.as_str()).into(),
                        priority: existing.as_ref().map_or(100, |route| route.priority),
                        enabled: true,
                        capability_revision: printer.capability_revision,
                        profile_revision: printer
                            .profiles
                            .iter()
                            .map(|profile| profile.revision)
                            .max()
                            .unwrap_or(0),
                        last_seen_at: Some(snapshot.observed_at),
                        updated_at: snapshot.observed_at,
                    },
                )
                .await
                .map_err(storage_error)?;
            for (index, item) in snapshot.identity_evidence.iter().enumerate() {
                state
                    .destination_topology
                    .record_identity_evidence(
                        tenant_scope,
                        &IdentityEvidence {
                            id: format!("ide_{}", ulid::Ulid::new()),
                            destination_id: destination_id.clone(),
                            route_id: server_route_id.clone(),
                            kind: evidence_kind(item.kind).into(),
                            value_digest: evidence[index].1.clone(),
                            strength: evidence[index].0.clone(),
                            conflicts,
                            observed_at: snapshot.observed_at,
                            expires_at: None,
                            metadata: serde_json::json!({
                                "source": "agent_sync",
                                "schema_version": 1,
                                "normalization": "node_sha256_then_tenant_hmac"
                            }),
                        },
                    )
                    .await
                    .map_err(storage_error)?;
            }
            state
                .destination_topology
                .acknowledge_projection(
                    tenant_scope,
                    &ProjectionAcknowledgement {
                        connector_id: request.agent_id.to_string(),
                        route_id: server_route_id.clone(),
                        inventory_revision: snapshot.inventory_revision,
                        capability_revision: printer.capability_revision,
                        status: "current".into(),
                        error_code: None,
                        observed_at: snapshot.observed_at,
                        acknowledged_at: Some(now),
                    },
                )
                .await
                .map_err(storage_error)?;
            local_to_server.insert(snapshot.local_route_key.clone(), server_route_id);
            projected_revision = Some(
                projected_revision.map_or(snapshot.inventory_revision, |revision| {
                    revision.max(snapshot.inventory_revision)
                }),
            );
        }
    }

    for change in &request.topology_changes {
        if !matches!(change.change, piqae_protocol::agent::TopologyChange::Removed) {
            continue;
        }
        match state
            .destination_topology
            .get_route_by_local_key(
                tenant_scope,
                &request.agent_id.to_string(),
                &change.local_route_key,
            )
            .await
        {
            Ok(mut route) => {
                route.enabled = false;
                route.state = "offline".into();
                route.last_seen_at = Some(change.observed_at);
                route.updated_at = change.observed_at;
                state
                    .destination_topology
                    .upsert_route(tenant_scope, &route)
                    .await
                    .map_err(storage_error)?;
            }
            Err(StorageError::NotFound) => {}
            Err(error) => return Err(storage_error(error)),
        }
    }

    for observation in &request.route_observations {
        let route = if let Some(server_id) = local_to_server.get(&observation.local_route_key) {
            state
                .destination_topology
                .get_route(tenant_scope, server_id)
                .await
        } else {
            state
                .destination_topology
                .get_route_by_local_key(
                    tenant_scope,
                    &request.agent_id.to_string(),
                    &observation.local_route_key,
                )
                .await
        };
        let route = match route {
            Ok(route) => route,
            Err(StorageError::NotFound) => continue,
            Err(error) => return Err(storage_error(error)),
        };
        if observation.state_reasons.len() > 64
            || observation
                .state_reasons
                .iter()
                .any(|reason| reason.len() > 255 || !reason.is_ascii())
        {
            return Err(AppError::invalid(
                "invalid_route_observation",
                "Route observation reasons exceed protocol bounds.",
            ));
        }
        let queue = observation.queue.clone().unwrap_or_default();
        state
            .destination_topology
            .record_route_observation(
                tenant_scope,
                &RouteObservation {
                    id: format!("rob_{}", ulid::Ulid::new()),
                    route_id: route.id,
                    sequence: observation.inventory_revision.max(1),
                    printer_state: enum_name(&observation.state),
                    accepting_jobs: Some(observation.accepts_jobs),
                    state_reasons: observation.state_reasons.clone(),
                    total_jobs: queue.total_jobs,
                    connector_jobs: queue.connector_jobs,
                    other_piqae_or_external_jobs: queue.other_piqae_or_external_jobs,
                    unknown_jobs: queue.unknown_jobs,
                    active_jobs: queue.active_jobs,
                    held_jobs: queue.held_jobs,
                    estimated_busy_seconds: None,
                    privacy_level: "counts_only".into(),
                    stock_state: serde_json::json!({
                        "profile_observed_at": observation.profile_observed_at,
                        "stock_observed_at": observation.stock_observed_at
                    }),
                    observed_at: observation.observed_at,
                    fresh_until: observation.observed_at + TimeDelta::seconds(90),
                },
            )
            .await
            .map_err(storage_error)?;
    }

    Ok(projected_revision.map(|revision| {
        piqae_protocol::agent::InventoryProjectionAcknowledgement {
            revision,
            projected_at: now,
        }
    }))
}

pub async fn list_destinations(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<PhysicalDestinationResponse>>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::PrintersRead).await?;
    let tenant_scope = scope(tenant);
    let stored = state
        .destination_topology
        .list_destinations(tenant_scope)
        .await
        .map_err(storage_error)?;
    let mut data = Vec::with_capacity(stored.len());
    for destination in stored {
        data.push(destination_response(&state, tenant_scope, destination).await?);
    }
    Ok(Json(data))
}

pub async fn get_destination(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(destination_id): Path<String>,
) -> Result<Json<PhysicalDestinationResponse>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::PrintersRead).await?;
    let tenant_scope = scope(tenant);
    let destination = state
        .destination_topology
        .get_destination(tenant_scope, &destination_id)
        .await
        .map_err(storage_error)?;
    Ok(Json(destination_response(&state, tenant_scope, destination).await?))
}

pub async fn list_destination_routes(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(destination_id): Path<String>,
) -> Result<Json<Vec<PrinterRouteResponse>>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::PrintersRead).await?;
    let tenant_scope = scope(tenant);
    state
        .destination_topology
        .get_destination(tenant_scope, &destination_id)
        .await
        .map_err(storage_error)?;
    let stored = state
        .destination_topology
        .list_routes(tenant_scope, &destination_id)
        .await
        .map_err(storage_error)?;
    let mut data = Vec::with_capacity(stored.len());
    for route in stored {
        data.push(route_response(&state, tenant_scope, route).await?);
    }
    Ok(Json(data))
}

pub async fn list_routes(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<PrinterRouteResponse>>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::PrintersRead).await?;
    let tenant_scope = scope(tenant);
    let stored = state
        .destination_topology
        .list_all_routes(tenant_scope)
        .await
        .map_err(storage_error)?;
    let mut data = Vec::with_capacity(stored.len());
    for route in stored {
        data.push(route_response(&state, tenant_scope, route).await?);
    }
    Ok(Json(data))
}

pub async fn get_route(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(route_id): Path<String>,
) -> Result<Json<PrinterRouteResponse>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::PrintersRead).await?;
    let tenant_scope = scope(tenant);
    let route = state
        .destination_topology
        .get_route(tenant_scope, &route_id)
        .await
        .map_err(storage_error)?;
    Ok(Json(route_response(&state, tenant_scope, route).await?))
}

pub async fn list_route_observations(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(route_id): Path<String>,
    Query(query): Query<ObservationQuery>,
) -> Result<Json<Vec<RouteObservationResponse>>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::PrintersRead).await?;
    let tenant_scope = scope(tenant);
    state
        .destination_topology
        .get_route(tenant_scope, &route_id)
        .await
        .map_err(storage_error)?;
    Ok(Json(
        state
            .destination_topology
            .list_route_observations(tenant_scope, &route_id, query.limit.clamp(1, 100))
            .await
            .map_err(storage_error)?
            .into_iter()
            .map(observation_response)
            .collect(),
    ))
}

pub async fn list_identity_evidence(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(destination_id): Path<String>,
) -> Result<Json<Vec<IdentityEvidenceResponse>>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::PrintersRead).await?;
    let tenant_scope = scope(tenant);
    state
        .destination_topology
        .get_destination(tenant_scope, &destination_id)
        .await
        .map_err(storage_error)?;
    Ok(Json(
        state
            .destination_topology
            .list_identity_evidence(tenant_scope, &destination_id)
            .await
            .map_err(storage_error)?
            .into_iter()
            .map(|value| IdentityEvidenceResponse {
                id: value.id,
                destination_id: value.destination_id,
                route_id: value.route_id,
                kind: value.kind,
                confidence: evidence_confidence(&value),
                observed_at: value.observed_at,
            })
            .collect(),
    ))
}

fn decision_response(
    decision: IdentityDecision,
    reversals: &HashMap<String, String>,
) -> IdentityDecisionResponse {
    IdentityDecisionResponse {
        reversed_by_decision_id: reversals.get(&decision.id).cloned(),
        id: decision.id,
        destination_id: decision.destination_id,
        kind: match decision.kind {
            IdentityDecisionKind::Merge => "merge",
            IdentityDecisionKind::Split => "split",
            IdentityDecisionKind::Reverse => "reversal",
            IdentityDecisionKind::Confirm => "merge",
            IdentityDecisionKind::RejectMatch => "split",
        },
        route_ids: decision.route_ids,
        reason: decision.reason,
        actor_id: decision.actor_id.unwrap_or(decision.actor_kind),
        reverses_decision_id: decision.reverses_decision_id,
        created_at: decision.created_at,
    }
}

async fn stored_decisions(
    state: &AppState,
    tenant_scope: TenantScope,
    destination_id: &str,
) -> Result<Vec<IdentityDecisionResponse>, AppError> {
    let decisions = state
        .destination_topology
        .list_identity_decisions(tenant_scope, destination_id)
        .await
        .map_err(storage_error)?;
    let reversals = decisions
        .iter()
        .filter_map(|decision| {
            decision
                .reverses_decision_id
                .as_ref()
                .map(|original| (original.clone(), decision.id.clone()))
        })
        .collect::<HashMap<_, _>>();
    Ok(decisions
        .into_iter()
        .map(|decision| decision_response(decision, &reversals))
        .collect())
}

pub async fn list_identity_decisions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(destination_id): Path<String>,
) -> Result<Json<Vec<IdentityDecisionResponse>>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::PrintersRead).await?;
    let tenant_scope = scope(tenant);
    state
        .destination_topology
        .get_destination(tenant_scope, &destination_id)
        .await
        .map_err(storage_error)?;
    Ok(Json(stored_decisions(&state, tenant_scope, &destination_id).await?))
}

pub async fn create_identity_decision(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(destination_id): Path<String>,
    Json(request): Json<CreateIdentityDecisionRequest>,
) -> Result<(axum::http::StatusCode, Json<IdentityDecisionResponse>), AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::PrintersWrite).await?;
    let tenant_scope = scope(tenant);
    let mut destination = state
        .destination_topology
        .get_destination(tenant_scope, &destination_id)
        .await
        .map_err(storage_error)?;
    if request.route_ids.is_empty() || request.route_ids.len() > 100 {
        return Err(AppError::invalid("invalid_route_selection", "Select between one and 100 routes."));
    }
    if request.reason.trim().is_empty() || request.reason.chars().count() > 2_000 {
        return Err(AppError::invalid("invalid_identity_reason", "A reason of at most 2,000 characters is required."));
    }
    if request.display_name.as_ref().is_some_and(|name| {
        name.trim().is_empty() || name.chars().count() > 255
    }) {
        return Err(AppError::invalid(
            "invalid_destination_name",
            "A destination name must contain at most 255 characters.",
        ));
    }
    let unique = request.route_ids.iter().collect::<HashSet<_>>();
    if unique.len() != request.route_ids.len() {
        return Err(AppError::invalid("invalid_route_selection", "Route IDs must be unique."));
    }
    let mut routes = Vec::with_capacity(request.route_ids.len());
    for route_id in &request.route_ids {
        routes.push(
            state
                .destination_topology
                .get_route(tenant_scope, route_id)
                .await
                .map_err(storage_error)?,
        );
    }
    let mut related_destination_ids = routes
        .iter()
        .map(|route| route.destination_id.clone())
        .filter(|id| id != &destination_id)
        .collect::<Vec<_>>();
    related_destination_ids.sort();
    related_destination_ids.dedup();
    let now = Utc::now();
    let (kind, moved_destination_id) = match request.kind {
        DecisionRequestKind::Merge => {
            if related_destination_ids.len() > 1 {
                return Err(AppError::conflict(
                    "merge_not_reversible",
                    "A single decision can merge routes from only one other destination.",
                ));
            }
            (IdentityDecisionKind::Merge, destination_id.clone())
        }
        DecisionRequestKind::Split => {
            if routes.iter().any(|route| route.destination_id != destination_id) {
                return Err(AppError::conflict(
                    "split_route_mismatch",
                    "Every split route must currently belong to this destination.",
                ));
            }
            let new_id = format!("pdst_{}", ulid::Ulid::new());
            related_destination_ids = vec![new_id.clone()];
            state
                .destination_topology
                .upsert_destination(
                    tenant_scope,
                    &StoredPhysicalDestination {
                        id: new_id.clone(),
                        name: request
                            .display_name
                            .as_deref()
                            .map(str::trim)
                            .filter(|name| !name.is_empty())
                            .map(ToOwned::to_owned)
                            .unwrap_or_else(|| format!("{} (split)", destination.name)),
                        identity_confidence: IdentityConfidence::Conflict,
                        state: "active".into(),
                        scheduling_authority_id: destination.scheduling_authority_id.clone(),
                        identity_revision: destination.identity_revision.saturating_add(1),
                        updated_at: now,
                    },
                )
                .await
                .map_err(storage_error)?;
            (IdentityDecisionKind::Split, new_id)
        }
    };
    for route in &mut routes {
        route.destination_id.clone_from(&moved_destination_id);
        route.updated_at = now;
        state
            .destination_topology
            .upsert_route(tenant_scope, route)
            .await
            .map_err(storage_error)?;
    }
    destination.identity_confidence = IdentityConfidence::Verified;
    destination.state = "active".into();
    destination.identity_revision = destination.identity_revision.saturating_add(1);
    destination.updated_at = now;
    state
        .destination_topology
        .upsert_destination(tenant_scope, &destination)
        .await
        .map_err(storage_error)?;
    let decision = IdentityDecision {
        id: format!("idd_{}", ulid::Ulid::new()),
        kind,
        destination_id,
        related_destination_ids,
        route_ids: request.route_ids,
        evidence_ids: Vec::new(),
        actor_kind: "api_principal".into(),
        actor_id: tenant.platform_service_account_id.map(|id| id.to_string()),
        reason: request.reason.trim().to_owned(),
        reverses_decision_id: None,
        request_id: Some(crate::request_id::current()),
        created_at: now,
    };
    state
        .destination_topology
        .record_identity_decision(tenant_scope, &decision)
        .await
        .map_err(storage_error)?;
    Ok((
        axum::http::StatusCode::CREATED,
        Json(decision_response(decision, &HashMap::new())),
    ))
}

pub async fn reverse_identity_decision(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((destination_id, decision_id)): Path<(String, String)>,
) -> Result<Json<IdentityDecisionResponse>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::PrintersWrite).await?;
    let tenant_scope = scope(tenant);
    let decisions = state
        .destination_topology
        .list_identity_decisions(tenant_scope, &destination_id)
        .await
        .map_err(storage_error)?;
    let original = decisions
        .iter()
        .find(|decision| decision.id == decision_id)
        .cloned()
        .ok_or_else(AppError::not_found)?;
    if decisions
        .iter()
        .any(|decision| decision.reverses_decision_id.as_deref() == Some(&decision_id))
    {
        return Err(AppError::conflict("identity_decision_reversed", "This decision was already reversed."));
    }
    let restore_destination = match original.kind {
        IdentityDecisionKind::Split => original.destination_id.clone(),
        IdentityDecisionKind::Merge => original
            .related_destination_ids
            .first()
            .cloned()
            .unwrap_or_else(|| original.destination_id.clone()),
        _ => return Err(AppError::conflict("identity_decision_not_reversible", "Only merge and split decisions can be reversed.")),
    };
    let now = Utc::now();
    for route_id in &original.route_ids {
        let mut route = state
            .destination_topology
            .get_route(tenant_scope, route_id)
            .await
            .map_err(storage_error)?;
        route.destination_id.clone_from(&restore_destination);
        route.updated_at = now;
        state
            .destination_topology
            .upsert_route(tenant_scope, &route)
            .await
            .map_err(storage_error)?;
    }
    let reversal = IdentityDecision {
        id: format!("idd_{}", ulid::Ulid::new()),
        kind: IdentityDecisionKind::Reverse,
        destination_id,
        related_destination_ids: original.related_destination_ids,
        route_ids: original.route_ids,
        evidence_ids: original.evidence_ids,
        actor_kind: "api_principal".into(),
        actor_id: tenant.platform_service_account_id.map(|id| id.to_string()),
        reason: format!("Reversed decision {decision_id}"),
        reverses_decision_id: Some(decision_id),
        request_id: Some(crate::request_id::current()),
        created_at: now,
    };
    state
        .destination_topology
        .reverse_identity_decision(tenant_scope, &reversal)
        .await
        .map_err(storage_error)?;
    Ok(Json(decision_response(reversal, &HashMap::new())))
}

pub async fn list_route_reservations(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<RouteReservation>>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::JobsRead).await?;
    Ok(Json(
        state
            .destination_topology
            .list_route_reservations(scope(tenant), 100)
            .await
            .map_err(storage_error)?,
    ))
}

pub async fn list_delivery_attempts(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(job_id): Path<String>,
) -> Result<Json<Vec<DeliveryAttempt>>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::JobsRead).await?;
    let job_id_parsed = job_id
        .parse()
        .map_err(|_| AppError::invalid("invalid_job_id", "The job ID is invalid."))?;
    state
        .repository
        .get_job(tenant.workspace_id, tenant.environment_id, job_id_parsed)
        .await?;
    Ok(Json(
        state
            .destination_topology
            .list_delivery_attempts(scope(tenant), &job_id)
            .await
            .map_err(storage_error)?,
    ))
}
