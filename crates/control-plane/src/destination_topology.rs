//! Tenant-scoped physical destinations, installed routes, and fenced delivery history.
//!
//! Public responses deliberately separate route telemetry, connector inventory
//! projection, and scheduling authority. A node heartbeat is never presented as
//! proof that a route inventory is current.

#![allow(
    clippy::missing_errors_doc,
    clippy::cognitive_complexity,
    clippy::too_many_lines,
    reason = "topology projection keeps evidence, route, observation, and acknowledgement ordering explicit"
)]

use crate::{AppState, api::authenticate_native, error::AppError};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, TimeDelta, Utc};
use hmac::{Hmac, Mac};
use piqae_auth::Scope;
use piqae_storage_postgres::{
    StorageError,
    destination_topology::{
        DeliveryAttempt, DeliveryAttemptState, IdentityConfidence, IdentityDecision,
        IdentityDecisionKind, IdentityEvidence, NewDeliveryAttempt,
        NodeRuntimeObservation as StoredNodeRuntimeObservation, NodeWakeHint,
        ProjectionAcknowledgement, RouteObservation, RouteReservation, SchedulingAuthority,
        SiteCoordinatorMembership, StoredPhysicalDestination, StoredPrinterRoute, TenantScope,
    },
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    str::FromStr,
};

const fn stored_observation_state(value: piqae_domain::PrinterState) -> &'static str {
    match value {
        piqae_domain::PrinterState::Online => "idle",
        piqae_domain::PrinterState::Busy => "processing",
        piqae_domain::PrinterState::Paused
        | piqae_domain::PrinterState::PaperOut
        | piqae_domain::PrinterState::Error => "stopped",
        piqae_domain::PrinterState::Offline => "unavailable",
        piqae_domain::PrinterState::Unknown => "unknown",
    }
}

fn public_observation_state(value: &str) -> String {
    match value {
        "idle" => "online",
        "processing" => "busy",
        "stopped" => "paused",
        "unavailable" => "offline",
        other => other,
    }
    .to_owned()
}

const fn stored_route_state(value: piqae_domain::PrinterState) -> &'static str {
    match value {
        piqae_domain::PrinterState::Online | piqae_domain::PrinterState::Busy => "available",
        piqae_domain::PrinterState::Offline => "unavailable",
        piqae_domain::PrinterState::Paused => "paused",
        piqae_domain::PrinterState::PaperOut | piqae_domain::PrinterState::Error => "rejecting",
        piqae_domain::PrinterState::Unknown => "unknown",
    }
}

fn public_destination_state(value: &str) -> String {
    match value {
        "available" => "active",
        "attention" | "unavailable" | "paused" | "unknown" => "needs_review",
        "retired" => "retired",
        other => other,
    }
    .to_owned()
}

const fn protocol_confidence(
    value: piqae_protocol::agent::IdentityConfidence,
) -> IdentityConfidence {
    match value {
        piqae_protocol::agent::IdentityConfidence::Verified => IdentityConfidence::Verified,
        piqae_protocol::agent::IdentityConfidence::High => IdentityConfidence::High,
        piqae_protocol::agent::IdentityConfidence::Possible => IdentityConfidence::Possible,
        piqae_protocol::agent::IdentityConfidence::Conflict => IdentityConfidence::Conflict,
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
        || !normalized_node_digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
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
    Ok(format!(
        "hmac-sha256:{}",
        hex::encode(mac.finalize().into_bytes())
    ))
}

const fn scope(tenant: crate::authentication::TenantContext) -> TenantScope {
    TenantScope {
        workspace_id: tenant.workspace_id,
        environment_id: tenant.environment_id,
    }
}

pub(crate) const fn tenant_scope(tenant: crate::authentication::TenantContext) -> TenantScope {
    scope(tenant)
}

fn storage_error(error: StorageError) -> AppError {
    match error {
        StorageError::NotFound => AppError::not_found(),
        StorageError::IdempotencyConflict
        | StorageError::ConcurrentStateChange
        | StorageError::InvalidTransition => AppError::conflict(
            "destination_state_changed",
            "The destination or route state changed concurrently.",
        ),
        StorageError::InvalidData(message) => {
            AppError::invalid("invalid_destination_topology", message)
        }
        _ => AppError::service_unavailable("destination_topology_unavailable"),
    }
}

pub(crate) fn map_storage_error(error: StorageError) -> AppError {
    storage_error(error)
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
    /// Preferred privacy-safe queue view. `piqae_owned_jobs` is scoped to the
    /// authenticated connector; `external_jobs` is only an opaque count and
    /// never reveals another tenant's job identity or content.
    pub queue_occupancy: PrivacySafeQueueOccupancyResponse,
    pub estimated_busy_seconds: Option<u64>,
    pub observed_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PrivacySafeQueueOccupancyResponse {
    pub piqae_owned_jobs: u32,
    pub external_jobs: u32,
    pub unknown_jobs: u32,
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
    pub capability_revision: u64,
    pub profile_revision: u64,
    pub profile_observed_at: Option<DateTime<Utc>>,
    pub stock_observed_at: Option<DateTime<Utc>>,
    pub stock_state: &'static str,
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

#[derive(Clone, Debug, Serialize)]
pub struct RouteReservationResponse {
    pub id: String,
    pub route_id: String,
    pub job_id: String,
    pub attempt_id: String,
    pub generation: u64,
    pub state: String,
    pub acquired_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub released_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DeliveryAttemptResponse {
    pub id: String,
    pub job_id: String,
    pub target_id: Option<String>,
    pub route_id: String,
    pub generation: u64,
    pub state: &'static str,
    pub native_spool_id: Option<String>,
    pub failure_reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct ResolveUncertainDeliveryRequest {
    resolution: String,
    note: String,
}

#[derive(Debug, Serialize)]
pub struct UncertainDeliveryResolutionResponse {
    job: piqae_domain::Job,
    resolution: String,
    state: &'static str,
    request_id: String,
    replacement_job: Option<piqae_domain::Job>,
    created_at: DateTime<Utc>,
    resolved_at: Option<DateTime<Utc>>,
}

const fn public_attempt_state(state: DeliveryAttemptState) -> &'static str {
    match state {
        DeliveryAttemptState::RouteLeased => "route_leased",
        DeliveryAttemptState::AcceptedByNode => "accepted_by_node",
        DeliveryAttemptState::QueuedLocal => "queued_local",
        DeliveryAttemptState::HandingToSpooler => "handing_to_spooler",
        DeliveryAttemptState::AcceptedBySpooler => "accepted_by_spooler",
        DeliveryAttemptState::PrintingReported => "printing_reported",
        DeliveryAttemptState::CompletedReported => "completed_reported",
        DeliveryAttemptState::Cancelled => "cancelled_before_handoff",
        DeliveryAttemptState::Failed => "failed",
        DeliveryAttemptState::DeliveryUncertain => "delivery_uncertain",
        DeliveryAttemptState::Superseded => "rejected_before_handoff",
    }
}

fn attempt_response(value: DeliveryAttempt) -> DeliveryAttemptResponse {
    DeliveryAttemptResponse {
        id: value.id,
        job_id: value.job_id,
        // Target IDs are control-plane presentation metadata and are not inferred
        // from a physical destination identifier.
        target_id: None,
        route_id: value.route_id,
        generation: value.generation,
        state: public_attempt_state(value.state),
        native_spool_id: None,
        failure_reason: None,
        created_at: value.created_at,
        updated_at: value.updated_at,
        completed_at: value.final_at,
    }
}

fn reservation_response(value: RouteReservation) -> RouteReservationResponse {
    RouteReservationResponse {
        id: value.id,
        route_id: value.route_id,
        job_id: value.job_id,
        attempt_id: value.attempt_id,
        generation: value.generation,
        state: match value.state.as_str() {
            "active" => "active",
            "released" => "released",
            "expired" => "expired",
            _ => "fenced",
        }
        .to_owned(),
        acquired_at: value.acquired_at,
        expires_at: value.lease_until,
        released_at: value.released_at,
    }
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

#[derive(Debug, Deserialize)]
pub struct NodeRuntimeListQuery {
    #[serde(default = "default_runtime_list_limit")]
    limit: u32,
    after: Option<String>,
}

const fn default_runtime_list_limit() -> u32 {
    100
}

#[derive(Clone, Debug, Serialize)]
pub struct NodeRuntimeObservationPage {
    pub data: Vec<NodeRuntimeObservationResponse>,
    pub next_cursor: Option<String>,
    pub has_more: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateWakeHintRequest {
    reason: String,
    #[serde(default = "default_wake_hint_ttl_seconds")]
    expires_in_seconds: u32,
}

const fn default_wake_hint_ttl_seconds() -> u32 {
    300
}

#[derive(Clone, Debug, Serialize)]
pub struct NodeRuntimeObservationResponse {
    pub node_id: String,
    pub sequence: u64,
    pub host_mode: String,
    pub availability_class: String,
    pub lifecycle_state: String,
    pub accepts_cloud_jobs: bool,
    pub execution_budget_ms: Option<u64>,
    pub wake_mechanisms: Vec<String>,
    pub observed_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub freshness: &'static str,
}

#[derive(Clone, Debug, Serialize)]
pub struct NodeWakeHintResponse {
    pub id: String,
    pub node_id: String,
    pub reason: String,
    pub delivery_channel: String,
    pub status: String,
    pub requested_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub observed_at: Option<DateTime<Utc>>,
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
        "conflict"
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
        printer_state: public_observation_state(&value.printer_state),
        state_reasons: value.state_reasons,
        accepting_jobs: value.accepting_jobs.unwrap_or(false),
        total_jobs: value.total_jobs,
        active_jobs: value.active_jobs,
        held_jobs: value.held_jobs,
        connector_jobs: value.connector_jobs,
        other_piqae_or_external_jobs: value.other_piqae_or_external_jobs,
        unknown_jobs: value.unknown_jobs,
        queue_occupancy: PrivacySafeQueueOccupancyResponse {
            piqae_owned_jobs: value.connector_jobs,
            // The N-1 aggregate includes unknown ownership. The preferred
            // projection partitions that aggregate so consumers never double
            // count the native queue.
            external_jobs: value
                .other_piqae_or_external_jobs
                .saturating_sub(value.unknown_jobs),
            unknown_jobs: value.unknown_jobs,
        },
        estimated_busy_seconds: value.estimated_busy_seconds.map(u64::from),
        observed_at: value.observed_at,
        expires_at: value.fresh_until,
    }
}

fn route_response_from_parts(
    route: StoredPrinterRoute,
    observation: Option<RouteObservation>,
    projection: Option<&ProjectionAcknowledgement>,
    authority: Option<String>,
) -> PrinterRouteResponse {
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
    let health = if !route.enabled || route.state == "unavailable" {
        "offline"
    } else if telemetry_freshness == "stale" || telemetry_freshness == "never" {
        "stale"
    } else {
        match observation
            .as_ref()
            .map(|value| value.printer_state.as_str())
        {
            Some("idle")
                if observation
                    .as_ref()
                    .is_some_and(|value| value.accepting_jobs == Some(true)) =>
            {
                "ready"
            }
            Some("processing") => "busy",
            Some("stopped") => "needs_operator",
            Some("unavailable") => "offline",
            _ => "unknown",
        }
    };
    let projection_health =
        projection
            .as_ref()
            .map_or("unsupported", |value| match value.status.as_str() {
                "acknowledged" => "current",
                "rejected" => "failed",
                _ => "pending",
            });
    let profile_observed_at = observation
        .as_ref()
        .and_then(|value| value.stock_state.get("profile_observed_at"))
        .and_then(serde_json::Value::as_str)
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc));
    let stock_observed_at = observation
        .as_ref()
        .and_then(|value| value.stock_state.get("stock_observed_at"))
        .and_then(serde_json::Value::as_str)
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc));
    let stock_state = stock_observed_at.map_or("unknown", |observed_at| {
        if observed_at + TimeDelta::minutes(5) >= now {
            "current"
        } else {
            "stale"
        }
    });
    PrinterRouteResponse {
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
        capability_revision: route.capability_revision,
        profile_revision: route.profile_revision,
        profile_observed_at,
        stock_observed_at,
        stock_state,
        latest_observation: observation.map(observation_response),
        scheduling_authority_id: authority,
        created_at: route.created_at,
        updated_at: route.updated_at,
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
    let authority = state
        .destination_topology
        .get_destination(tenant_scope, &route.destination_id)
        .await
        .map_err(storage_error)?
        .scheduling_authority_id;
    Ok(route_response_from_parts(
        route,
        observation,
        projection.as_ref(),
        authority,
    ))
}

fn route_responses_from_parts(
    routes: Vec<StoredPrinterRoute>,
    observations: &[RouteObservation],
    projections: &[ProjectionAcknowledgement],
    destinations: &[StoredPhysicalDestination],
) -> Result<Vec<PrinterRouteResponse>, AppError> {
    let observations = observations
        .iter()
        .map(|value| (value.route_id.as_str(), value))
        .collect::<HashMap<_, _>>();
    let projections = projections
        .iter()
        .map(|value| ((value.agent_id.as_str(), value.route_id.as_str()), value))
        .collect::<HashMap<_, _>>();
    let destinations = destinations
        .iter()
        .map(|value| (value.id.as_str(), value))
        .collect::<HashMap<_, _>>();
    routes
        .into_iter()
        .map(|route| {
            let destination = destinations
                .get(route.destination_id.as_str())
                .ok_or_else(|| storage_error(StorageError::NotFound))?;
            let observation = observations
                .get(route.id.as_str())
                .map(|value| (*value).clone());
            let projection = projections
                .get(&(route.agent_id.as_str(), route.id.as_str()))
                .copied();
            Ok(route_response_from_parts(
                route,
                observation,
                projection,
                destination.scheduling_authority_id.clone(),
            ))
        })
        .collect()
}

async fn route_responses(
    state: &AppState,
    tenant_scope: TenantScope,
    routes: Vec<StoredPrinterRoute>,
) -> Result<Vec<PrinterRouteResponse>, AppError> {
    let route_ids = routes
        .iter()
        .map(|route| route.id.clone())
        .collect::<Vec<_>>();
    let observations = state
        .destination_topology
        .latest_route_observations(tenant_scope, &route_ids)
        .await
        .map_err(storage_error)?;
    let projections = state
        .destination_topology
        .projection_acknowledgements_for_routes(tenant_scope, &route_ids)
        .await
        .map_err(storage_error)?;
    let destinations = state
        .destination_topology
        .list_destinations(tenant_scope)
        .await
        .map_err(storage_error)?;
    route_responses_from_parts(routes, &observations, &projections, &destinations)
}

fn destination_response_from_parts(
    destination: StoredPhysicalDestination,
    routes: &[StoredPrinterRoute],
    observations: &HashMap<&str, &RouteObservation>,
) -> PhysicalDestinationResponse {
    let routes = routes
        .iter()
        .filter(|route| route.destination_id == destination.id)
        .collect::<Vec<_>>();
    let route_count = routes.len();
    let now = Utc::now();
    let has_ready_route = routes.iter().any(|route| {
        route.enabled
            && route.state == "available"
            && observations
                .get(route.id.as_str())
                .is_some_and(|observation| {
                    observation.fresh_until >= now
                        && observation.accepting_jobs == Some(true)
                        && matches!(observation.printer_state.as_str(), "idle" | "processing")
                })
    });
    let public_status = if destination.state == "available" && !has_ready_route {
        "needs_review".into()
    } else {
        public_destination_state(&destination.state)
    };
    PhysicalDestinationResponse {
        id: destination.id,
        display_name: destination.name,
        manufacturer: None,
        model: None,
        identity_confidence: identity_confidence(destination.identity_confidence),
        status: public_status,
        route_count,
        created_at: destination.created_at,
        updated_at: destination.updated_at,
    }
}

async fn destination_response(
    state: &AppState,
    tenant_scope: TenantScope,
    destination: StoredPhysicalDestination,
) -> Result<PhysicalDestinationResponse, AppError> {
    let routes = state
        .destination_topology
        .list_routes(tenant_scope, &destination.id)
        .await
        .map_err(storage_error)?;
    let route_ids = routes
        .iter()
        .map(|route| route.id.clone())
        .collect::<Vec<_>>();
    let observations = state
        .destination_topology
        .latest_route_observations(tenant_scope, &route_ids)
        .await
        .map_err(storage_error)?;
    let observations = observations
        .iter()
        .map(|value| (value.route_id.as_str(), value))
        .collect::<HashMap<_, _>>();
    Ok(destination_response_from_parts(
        destination,
        &routes,
        &observations,
    ))
}

async fn destination_responses(
    state: &AppState,
    tenant_scope: TenantScope,
    destinations: Vec<StoredPhysicalDestination>,
) -> Result<Vec<PhysicalDestinationResponse>, AppError> {
    let routes = state
        .destination_topology
        .list_all_routes(tenant_scope)
        .await
        .map_err(storage_error)?;
    let route_ids = routes
        .iter()
        .map(|route| route.id.clone())
        .collect::<Vec<_>>();
    let observations = state
        .destination_topology
        .latest_route_observations(tenant_scope, &route_ids)
        .await
        .map_err(storage_error)?;
    let observations = observations
        .iter()
        .map(|value| (value.route_id.as_str(), value))
        .collect::<HashMap<_, _>>();
    Ok(destinations
        .into_iter()
        .map(|destination| destination_response_from_parts(destination, &routes, &observations))
        .collect())
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
    let all_destinations = state
        .destination_topology
        .list_destinations(tenant_scope)
        .await
        .map_err(storage_error)?;
    let mut destinations = all_destinations.clone();
    destinations.truncate(limit);
    let all_routes = state
        .destination_topology
        .list_all_routes(tenant_scope)
        .await
        .map_err(storage_error)?;
    let all_route_ids = all_routes
        .iter()
        .map(|route| route.id.clone())
        .collect::<Vec<_>>();
    let route_observations = state
        .destination_topology
        .latest_route_observations(tenant_scope, &all_route_ids)
        .await
        .map_err(storage_error)?;
    let observation_map = route_observations
        .iter()
        .map(|value| (value.route_id.as_str(), value))
        .collect::<HashMap<_, _>>();
    let destination_responses = destinations
        .iter()
        .cloned()
        .map(|destination| {
            destination_response_from_parts(destination, &all_routes, &observation_map)
        })
        .collect::<Vec<_>>();
    let mut routes = all_routes;
    routes.truncate(limit);
    let route_ids = routes
        .iter()
        .map(|route| route.id.clone())
        .collect::<Vec<_>>();
    let projections = state
        .destination_topology
        .projection_acknowledgements_for_routes(tenant_scope, &route_ids)
        .await
        .map_err(storage_error)?;
    let route_responses =
        route_responses_from_parts(routes, &route_observations, &projections, &all_destinations)?;
    let observations = route_responses
        .iter()
        .filter_map(|response| response.latest_observation.clone())
        .collect();
    Ok((destination_responses, route_responses, observations))
}

/// Returns only fresh, accepting execution routes in deterministic scheduling
/// order. The logical destination is unchanged; route choice is a disposable
/// pre-handoff decision.
pub(crate) async fn ranked_ready_routes(
    state: &AppState,
    tenant_scope: TenantScope,
    destination_id: &str,
) -> Result<Vec<StoredPrinterRoute>, AppError> {
    let now = Utc::now();
    let mut candidates = Vec::new();
    let routes = state
        .destination_topology
        .list_routes(tenant_scope, destination_id)
        .await
        .map_err(storage_error)?;
    let route_ids = routes
        .iter()
        .map(|route| route.id.clone())
        .collect::<Vec<_>>();
    let observations = state
        .destination_topology
        .latest_route_observations(tenant_scope, &route_ids)
        .await
        .map_err(storage_error)?
        .into_iter()
        .map(|observation| (observation.route_id.clone(), observation))
        .collect::<HashMap<_, _>>();
    for route in routes {
        if !route.enabled || route.state != "available" {
            continue;
        }
        let Some(observation) = observations.get(&route.id) else {
            continue;
        };
        if observation.fresh_until < now
            || observation.accepting_jobs != Some(true)
            || !matches!(observation.printer_state.as_str(), "idle" | "processing")
        {
            continue;
        }
        candidates.push((
            route,
            observation.total_jobs,
            observation.estimated_busy_seconds.unwrap_or(u32::MAX),
        ));
    }
    candidates.sort_by(
        |(left, left_jobs, left_busy), (right, right_jobs, right_busy)| {
            let role = |value: &str| u8::from(value != "primary");
            (
                role(&left.role),
                left.priority,
                *left_jobs,
                *left_busy,
                &left.id,
            )
                .cmp(&(
                    role(&right.role),
                    right.priority,
                    *right_jobs,
                    *right_busy,
                    &right.id,
                ))
        },
    );
    Ok(candidates.into_iter().map(|(route, _, _)| route).collect())
}

const fn evidence_kind(kind: piqae_protocol::agent::PhysicalIdentityEvidenceKind) -> &'static str {
    use piqae_protocol::agent::PhysicalIdentityEvidenceKind as Kind;
    match kind {
        Kind::IppPrinterUuid => "ipp_uuid",
        Kind::DeviceSerial => "device_serial",
        Kind::UsbSerial => "usb_serial",
        Kind::CertificateKey => "certificate_key",
        Kind::NetworkMac => "network_mac",
        Kind::NetworkEndpoint => "network_endpoint",
        Kind::ManufacturerModel => "manufacturer_model",
        Kind::CapabilityFingerprint => "capability_fingerprint",
        Kind::DriverFingerprint => "driver_fingerprint",
    }
}

const fn evidence_strength(
    strength: piqae_protocol::agent::IdentityEvidenceStrength,
) -> &'static str {
    use piqae_protocol::agent::IdentityEvidenceStrength as Strength;
    match strength {
        Strength::Strong => "strong",
        Strength::Medium => "medium",
        Strength::Weak => "weak",
    }
}

async fn destination_for_new_route(
    state: &AppState,
    tenant_scope: TenantScope,
    evidence: &[(String, String, String)],
) -> Result<(String, bool), AppError> {
    let strong = evidence
        .iter()
        .filter(|(_, strength, _)| evidence_is_strong(strength))
        .map(|(kind, _, digest)| (kind.as_str(), digest.as_str()))
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
    let mut conflicting_match = false;
    for destination in destinations {
        let stored = state
            .destination_topology
            .list_identity_evidence(tenant_scope, &destination.id)
            .await
            .map_err(storage_error)?;
        let stored_strong = stored
            .iter()
            .filter(|item| evidence_is_strong(&item.strength))
            .map(|item| (item.kind.as_str(), item.value_digest.as_str()))
            .collect::<HashSet<_>>();
        let has_match = strong.iter().any(|item| stored_strong.contains(item));
        let has_conflict = strong.iter().any(|(kind, digest)| {
            stored_strong
                .iter()
                .any(|(stored_kind, stored_digest)| stored_kind == kind && stored_digest != digest)
        });
        if has_match && has_conflict {
            conflicting_match = true;
        } else if has_match {
            matches.push(destination.id);
        }
    }
    matches.sort();
    matches.dedup();
    if conflicting_match {
        return Ok((format!("pdst_{}", ulid::Ulid::new()), true));
    }
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
                kind: if state.capabilities.deployment == "cloud" {
                    "hosted_control_plane".into()
                } else {
                    "self_hosted_control_plane".into()
                },
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
                state: "active".into(),
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
            if matches!(
                snapshot.topology_change,
                Some(piqae_protocol::agent::TopologyChange::Removed)
            ) {
                continue;
            }
            let evidence = snapshot
                .identity_evidence
                .iter()
                .map(|item| {
                    Ok((
                        evidence_kind(item.kind).to_owned(),
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
                        destination.state = "attention".into();
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
                            state: if conflicts { "attention" } else { "available" }.into(),
                            scheduling_authority_id: Some(authority_id.clone()),
                            identity_revision: snapshot.topology_revision.max(1),
                            created_at: snapshot.observed_at,
                            updated_at: snapshot.observed_at,
                        },
                    )
                    .await
                    .map_err(storage_error)?,
                Err(error) => return Err(storage_error(error)),
            }
            let server_route_id = existing.as_ref().map_or_else(
                || format!("rte_{}", ulid::Ulid::new()),
                |route| route.id.clone(),
            );
            let route_role = if let Some(route) = existing.as_ref() {
                route.role.clone()
            } else if state
                .destination_topology
                .list_routes(tenant_scope, &destination_id)
                .await
                .map_err(storage_error)?
                .into_iter()
                .any(|route| route.enabled && route.role == "primary")
            {
                "standby".into()
            } else {
                "primary".into()
            };
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
                        state: stored_route_state(printer.state).into(),
                        role: route_role,
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
                        created_at: existing
                            .as_ref()
                            .map_or(snapshot.observed_at, |route| route.created_at),
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
                            value_digest: evidence[index].2.clone(),
                            strength: evidence[index].1.clone(),
                            conflicts,
                            observed_at: snapshot.observed_at,
                            expires_at: None,
                            metadata: serde_json::json!({
                                "source": "node",
                                "schema_version": 1,
                                "normalization": "node_sha256_then_tenant_hmac",
                                "key_version": "v1"
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
                        agent_id: request.agent_id.to_string(),
                        route_id: server_route_id.clone(),
                        inventory_revision: snapshot.inventory_revision,
                        capability_revision: printer.capability_revision,
                        status: "acknowledged".into(),
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
        if !matches!(
            change.change,
            piqae_protocol::agent::TopologyChange::Removed
        ) {
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
                route.state = "unavailable".into();
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
        let classified_total = queue
            .connector_jobs
            .checked_add(queue.other_piqae_or_external_jobs);
        if observation.sequence == 0
            || observation.observed_at > Utc::now() + TimeDelta::minutes(5)
            || queue.active_jobs > queue.total_jobs
            || queue.held_jobs > queue.total_jobs
            || queue.unknown_jobs > queue.other_piqae_or_external_jobs
            || classified_total != Some(queue.total_jobs)
        {
            return Err(AppError::invalid(
                "invalid_route_observation",
                "Route sequence, timestamp, or privacy-safe queue counts are inconsistent.",
            ));
        }
        state
            .destination_topology
            .record_route_observation(
                tenant_scope,
                &RouteObservation {
                    // The node's durable route sequence is the idempotency key.
                    // Hashing the server route resource keeps the public storage
                    // identifier bounded while making an identical sync retry
                    // address the exact same observation.
                    id: format!(
                        "rob_{}",
                        &hex::encode(Sha256::digest(format!(
                            "{}\0{}",
                            route.id, observation.sequence
                        )))[..32]
                    ),
                    route_id: route.id,
                    sequence: observation.sequence,
                    printer_state: stored_observation_state(observation.state).into(),
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

pub(crate) struct RuntimeAdmission {
    pub eligible_for_offers: bool,
    pub wake_hints: Vec<piqae_protocol::agent::AgentWakeHint>,
}

const fn host_mode_name(value: piqae_protocol::agent::NodeHostMode) -> &'static str {
    use piqae_protocol::agent::NodeHostMode;
    match value {
        NodeHostMode::MachineService => "machine_service",
        NodeHostMode::UserAgent => "user_agent",
        NodeHostMode::EmbeddedApplication => "embedded_application",
        NodeHostMode::AttachedClient => "attached_client",
    }
}

const fn availability_class_name(
    value: piqae_protocol::agent::NodeAvailabilityClass,
) -> &'static str {
    use piqae_protocol::agent::NodeAvailabilityClass;
    match value {
        NodeAvailabilityClass::ContinuousWhileAwake => "continuous_while_awake",
        NodeAvailabilityClass::ForegroundOnly => "foreground_only",
        NodeAvailabilityClass::BackgroundOpportunistic => "background_opportunistic",
        NodeAvailabilityClass::ManagedKiosk => "managed_kiosk",
        NodeAvailabilityClass::WakeRelayCapable => "wake_relay_capable",
    }
}

const fn lifecycle_state_name(value: piqae_protocol::agent::NodeAvailability) -> &'static str {
    use piqae_protocol::agent::NodeAvailability;
    match value {
        NodeAvailability::Available => "available",
        NodeAvailability::Foreground => "foreground",
        NodeAvailability::Background => "background",
        NodeAvailability::Suspending => "suspending",
        NodeAvailability::Suspended => "suspended",
        NodeAvailability::Waking => "waking",
        NodeAvailability::Unavailable => "unavailable",
    }
}

const fn wake_mechanism_name(value: piqae_protocol::agent::WakeMechanism) -> &'static str {
    use piqae_protocol::agent::WakeMechanism;
    match value {
        WakeMechanism::LocalBroker => "local_broker",
        WakeMechanism::ApnsBackground => "apns_background",
        WakeMechanism::BluetoothAccessory => "bluetooth_accessory",
        WakeMechanism::ExternalAccessory => "external_accessory",
        WakeMechanism::WakeOnLan => "wake_on_lan",
        WakeMechanism::Manual => "manual",
    }
}

pub(crate) async fn record_runtime_availability(
    state: &AppState,
    tenant: crate::authentication::TenantContext,
    request: &piqae_protocol::agent::AgentSyncRequest,
) -> Result<RuntimeAdmission, AppError> {
    let tenant_scope = scope(tenant);
    let now = Utc::now();
    let eligible_for_offers = if let Some(runtime) = &request.runtime {
        let mut wake_mechanisms = runtime.wake_mechanisms.clone();
        wake_mechanisms.sort();
        wake_mechanisms.dedup();
        if runtime.sequence == 0
            || runtime.observed_at > now + TimeDelta::minutes(5)
            || runtime.fresh_until < runtime.observed_at
            || runtime.fresh_until > runtime.observed_at + TimeDelta::minutes(10)
            || runtime.wake_mechanisms.len() > 8
            || wake_mechanisms.len() != runtime.wake_mechanisms.len()
        {
            return Err(AppError::invalid(
                "invalid_runtime_observation",
                "Runtime availability is outside protocol bounds.",
            ));
        }
        let background_budget_ok = !matches!(
            runtime.availability_class,
            piqae_protocol::agent::NodeAvailabilityClass::BackgroundOpportunistic
        ) || !runtime.accepts_cloud_jobs
            || runtime
                .execution_budget_ms
                .is_some_and(|budget| budget >= 30_000);
        let attached_client_safe = !matches!(
            runtime.host_mode,
            piqae_protocol::agent::NodeHostMode::AttachedClient
        ) || !runtime.accepts_cloud_jobs;
        let lifecycle_acceptance_safe = !runtime.accepts_cloud_jobs
            || matches!(
                runtime.lifecycle_state,
                piqae_protocol::agent::NodeAvailability::Available
                    | piqae_protocol::agent::NodeAvailability::Foreground
                    | piqae_protocol::agent::NodeAvailability::Background
            );
        let availability_class_safe = !runtime.accepts_cloud_jobs
            || !matches!(
                runtime.availability_class,
                piqae_protocol::agent::NodeAvailabilityClass::ForegroundOnly
            )
            || matches!(
                runtime.lifecycle_state,
                piqae_protocol::agent::NodeAvailability::Foreground
            );
        // Persist a node's truthful availability class independently of relay
        // authorization. Nothing in route admission or wake dispatch treats
        // this self-report as proof of a registered external relay.
        if !background_budget_ok
            || !attached_client_safe
            || !lifecycle_acceptance_safe
            || !availability_class_safe
        {
            return Err(AppError::invalid(
                "unsafe_runtime_admission",
                "This host cannot safely accept cloud work in its current execution mode.",
            ));
        }
        let stored = StoredNodeRuntimeObservation {
            id: format!(
                "nro_{}",
                &hex::encode(Sha256::digest(format!(
                    "{}\0{}",
                    request.agent_id, runtime.sequence
                )))[..32]
            ),
            agent_id: request.agent_id.to_string(),
            sequence: runtime.sequence,
            host_mode: host_mode_name(runtime.host_mode).into(),
            availability_class: availability_class_name(runtime.availability_class).into(),
            lifecycle_state: lifecycle_state_name(runtime.lifecycle_state).into(),
            accepts_cloud_jobs: runtime.accepts_cloud_jobs,
            execution_budget_ms: runtime.execution_budget_ms,
            wake_mechanisms: runtime
                .wake_mechanisms
                .iter()
                .copied()
                .map(wake_mechanism_name)
                .map(str::to_owned)
                .collect(),
            observed_at: runtime.observed_at,
            fresh_until: runtime.fresh_until,
        };
        state
            .destination_topology
            .record_node_runtime_observation(tenant_scope, &stored)
            .await
            .map_err(storage_error)?;
        runtime.accepts_cloud_jobs
            && runtime.fresh_until >= now
            && matches!(
                runtime.lifecycle_state,
                piqae_protocol::agent::NodeAvailability::Available
                    | piqae_protocol::agent::NodeAvailability::Foreground
                    | piqae_protocol::agent::NodeAvailability::Background
            )
    } else {
        // Legacy desktop agents have no mobile execution budget. Their current
        // authenticated sync plus queue acceptance remains the N-1 admission.
        request.queue.accepts_jobs
    };
    let can_observe_hints = request.runtime.as_ref().is_none_or(|runtime| {
        !matches!(
            runtime.lifecycle_state,
            piqae_protocol::agent::NodeAvailability::Suspending
                | piqae_protocol::agent::NodeAvailability::Suspended
                | piqae_protocol::agent::NodeAvailability::Unavailable
        )
    });
    let wake_hints = if can_observe_hints {
        state
            .destination_topology
            .observe_pending_node_wake_hints(tenant_scope, &request.agent_id.to_string(), now, 32)
            .await
            .map_err(storage_error)?
            .into_iter()
            .map(|hint| {
                let delivery_channel = match hint.delivery_channel.as_str() {
                    "connected_session" => {
                        piqae_protocol::agent::WakeDeliveryChannel::ConnectedSession
                    }
                    "external_push" => piqae_protocol::agent::WakeDeliveryChannel::ExternalPush,
                    "local_relay" => piqae_protocol::agent::WakeDeliveryChannel::LocalRelay,
                    "manual" => piqae_protocol::agent::WakeDeliveryChannel::Manual,
                    other => {
                        return Err(StorageError::InvalidData(format!(
                            "unsupported wake delivery channel: {other}"
                        )));
                    }
                };
                Ok(piqae_protocol::agent::AgentWakeHint {
                    id: hint.id,
                    reason: hint.reason,
                    delivery_channel,
                    requested_at: hint.requested_at,
                    expires_at: hint.expires_at,
                })
            })
            .collect::<Result<Vec<_>, StorageError>>()
            .map_err(storage_error)?
    } else {
        Vec::new()
    };
    Ok(RuntimeAdmission {
        eligible_for_offers,
        wake_hints,
    })
}

/// Acquires the destination-wide native-handoff fence for an already leased
/// job. Route identity must have been durably projected before a job can be
/// offered: an unfenced compatibility handoff would defeat destination-wide
/// ordering during a rolling node upgrade.
pub(crate) enum JobRouteReservation {
    Busy,
    ProjectionRequired {
        destination_id: Option<String>,
        route_id: Option<String>,
    },
    Reserved(piqae_protocol::agent::CloudRouteReservation),
}

pub(crate) async fn reserve_job_route(
    state: &AppState,
    tenant: crate::authentication::TenantContext,
    job: &piqae_domain::Job,
    lease_until: DateTime<Utc>,
) -> Result<JobRouteReservation, AppError> {
    let tenant_scope = scope(tenant);
    let route = state
        .destination_topology
        .list_all_routes(tenant_scope)
        .await
        .map_err(storage_error)?
        .into_iter()
        .find(|route| {
            route.enabled
                && route.agent_id
                    == job
                        .metadata
                        .get("piqae.route_agent_id")
                        .map_or_else(String::new, Clone::clone)
                && route.printer_id == job.printer_id.to_string()
        });
    // Older jobs do not carry route_agent_id. Fall back to the route's
    // printer identity only when it is unambiguous within this tenant.
    let route = if route.is_some() {
        route
    } else {
        let matching = state
            .destination_topology
            .list_all_routes(tenant_scope)
            .await
            .map_err(storage_error)?
            .into_iter()
            .filter(|route| route.enabled && route.printer_id == job.printer_id.to_string())
            .collect::<Vec<_>>();
        (matching.len() == 1).then(|| matching[0].clone())
    };
    let Some(route) = route else {
        return Ok(JobRouteReservation::ProjectionRequired {
            destination_id: job.metadata.get("piqae.destination_id").cloned(),
            route_id: job.metadata.get("piqae.route_id").cloned(),
        });
    };
    let Some(local_route_key) = route.local_route_key.clone() else {
        return Ok(JobRouteReservation::ProjectionRequired {
            destination_id: Some(route.destination_id),
            route_id: Some(route.id),
        });
    };
    let observation = match state
        .destination_topology
        .latest_route_observation(tenant_scope, &route.id)
        .await
    {
        Ok(observation) => observation,
        Err(StorageError::NotFound) => return Ok(JobRouteReservation::Busy),
        Err(error) => return Err(storage_error(error)),
    };
    let now = Utc::now();
    if observation.fresh_until < now
        || observation.accepting_jobs != Some(true)
        || !matches!(observation.printer_state.as_str(), "idle" | "processing")
    {
        return Ok(JobRouteReservation::Busy);
    }
    let destination = state
        .destination_topology
        .get_destination(tenant_scope, &route.destination_id)
        .await
        .map_err(storage_error)?;
    if destination.state != "available" {
        return Ok(JobRouteReservation::Busy);
    }
    // Stable destination ordering and the single handoff fence are enforced in
    // the same storage transaction that begins the attempt. API-side bounded
    // scans cannot safely arbitrate concurrent schedulers.
    let reservation_id = uuid::Uuid::new_v4();
    let started = match state
        .destination_topology
        .begin_delivery_attempt(
            tenant_scope,
            NewDeliveryAttempt {
                attempt_id: &format!("datt_{}", ulid::Ulid::new()),
                reservation_id: &reservation_id.to_string(),
                job_id: &job.id.to_string(),
                destination_id: &route.destination_id,
                route_id: &route.id,
                lease_until,
            },
        )
        .await
    {
        Ok(value) => value,
        Err(StorageError::ConcurrentStateChange) => return Ok(JobRouteReservation::Busy),
        Err(error) => return Err(storage_error(error)),
    };
    Ok(JobRouteReservation::Reserved(
        piqae_protocol::agent::CloudRouteReservation {
            route_id: route.id,
            local_route_key,
            reservation_id,
            generation: started.attempt.generation,
            fencing_token: started.fencing_token,
            lease_expires_at: started.reservation.lease_until,
        },
    ))
}

pub(crate) async fn ingest_native_handoffs(
    state: &AppState,
    tenant: crate::authentication::TenantContext,
    agent_id: piqae_domain::AgentId,
    evidence: &[piqae_protocol::agent::NativeHandoffEvidence],
) -> Result<Option<u64>, AppError> {
    if evidence.len() > 1_000
        || evidence
            .windows(2)
            .any(|pair| pair[0].sequence >= pair[1].sequence)
    {
        return Err(AppError::invalid(
            "invalid_handoff_evidence",
            "Native handoff evidence must be bounded and strictly ordered.",
        ));
    }
    let tenant_scope = scope(tenant);
    let mut acknowledged = None;
    for item in evidence {
        let Some(route_id) = item.route_id.as_deref() else {
            return Err(AppError::conflict(
                "route_fence_required",
                "Native handoff evidence must carry its server route reservation.",
            ));
        };
        let route = state
            .destination_topology
            .get_route(tenant_scope, route_id)
            .await
            .map_err(storage_error)?;
        if route.agent_id != agent_id.to_string()
            || route.local_route_key.as_deref() != Some(item.local_route_key.as_str())
        {
            return Err(AppError::conflict(
                "stale_route_fence",
                "Native handoff evidence does not identify the authorized route.",
            ));
        }
        let mut attempt = state
            .destination_topology
            .get_delivery_attempt_by_reservation(tenant_scope, &item.reservation_id.to_string())
            .await
            .map_err(storage_error)?;
        if attempt.job_id != item.job_id.to_string()
            || attempt.route_id != route_id
            || attempt.generation != item.fencing_generation
        {
            return Err(AppError::conflict(
                "stale_route_fence",
                "Native handoff evidence was fenced by a newer delivery attempt.",
            ));
        }
        if attempt.state == DeliveryAttemptState::QueuedLocal {
            attempt = state
                .destination_topology
                .transition_delivery_attempt(
                    tenant_scope,
                    &attempt.id,
                    item.fencing_generation,
                    &item.fencing_token,
                    DeliveryAttemptState::HandingToSpooler,
                )
                .await
                .map_err(storage_error)?;
        }
        let next = match item.outcome {
            piqae_protocol::agent::NativeHandoffOutcome::Accepted => {
                DeliveryAttemptState::AcceptedBySpooler
            }
            piqae_protocol::agent::NativeHandoffOutcome::RejectedBeforeHandoff => {
                DeliveryAttemptState::Failed
            }
            piqae_protocol::agent::NativeHandoffOutcome::Ambiguous => {
                DeliveryAttemptState::DeliveryUncertain
            }
        };
        if attempt.state == DeliveryAttemptState::HandingToSpooler {
            attempt = state
                .destination_topology
                .transition_delivery_attempt(
                    tenant_scope,
                    &attempt.id,
                    item.fencing_generation,
                    &item.fencing_token,
                    next,
                )
                .await
                .map_err(storage_error)?;
        }
        if next == DeliveryAttemptState::DeliveryUncertain && attempt.state == next {
            let mut destination = state
                .destination_topology
                .get_destination(tenant_scope, &attempt.destination_id)
                .await
                .map_err(storage_error)?;
            destination.state = "attention".into();
            destination.updated_at = Utc::now();
            state
                .destination_topology
                .upsert_destination(tenant_scope, &destination)
                .await
                .map_err(storage_error)?;
        }
        state
            .publish(
                tenant,
                "attempt.updated",
                &serde_json::json!({
                    "attempt_id": attempt.id,
                    "job_id": attempt.job_id,
                    "destination_id": attempt.destination_id,
                    "state": public_attempt_state(attempt.state),
                    "updated_at": attempt.updated_at,
                }),
            )
            .await?;
        acknowledged = Some(item.sequence);
    }
    Ok(acknowledged)
}

/// Reconciles authenticated post-spooler job telemetry with the authoritative
/// delivery attempt. This is deliberately retried even when the ordinary job
/// event was already stored: a crash between the two projections must repair
/// the attempt rather than acknowledge an incomplete status projection.
pub(crate) async fn reconcile_post_spooler_event(
    state: &AppState,
    tenant: crate::authentication::TenantContext,
    agent_id: piqae_domain::AgentId,
    event: &piqae_domain::JobEvent,
) -> Result<Option<DeliveryAttempt>, AppError> {
    let next = match event.state {
        piqae_domain::JobState::Spooling
        | piqae_domain::JobState::Printing
        | piqae_domain::JobState::Blocked => Some(DeliveryAttemptState::PrintingReported),
        piqae_domain::JobState::CompletedReported => Some(DeliveryAttemptState::CompletedReported),
        piqae_domain::JobState::FailedTerminal => Some(DeliveryAttemptState::Failed),
        // Cancellation after native acceptance cannot prove the spooler did
        // not print. Preserve duplicate risk instead of claiming a clean stop.
        piqae_domain::JobState::Cancelled | piqae_domain::JobState::DeliveryUncertain => {
            Some(DeliveryAttemptState::DeliveryUncertain)
        }
        _ => None,
    };
    let Some(next) = next else {
        return Ok(None);
    };
    let tenant_scope = scope(tenant);
    let current = state
        .destination_topology
        .get_latest_delivery_attempt(tenant_scope, &event.job_id.to_string())
        .await
        .map_err(storage_error)?;
    let route = state
        .destination_topology
        .get_route(tenant_scope, &current.route_id)
        .await
        .map_err(storage_error)?;
    if route.agent_id != agent_id.to_string() {
        return Err(AppError::conflict(
            "stale_route_event",
            "The authenticated node does not own this delivery route.",
        ));
    }
    let already_projected = match next {
        DeliveryAttemptState::PrintingReported => matches!(
            current.state,
            DeliveryAttemptState::PrintingReported | DeliveryAttemptState::CompletedReported
        ),
        DeliveryAttemptState::CompletedReported => {
            current.state == DeliveryAttemptState::CompletedReported
        }
        DeliveryAttemptState::Failed => current.state == DeliveryAttemptState::Failed,
        DeliveryAttemptState::DeliveryUncertain => {
            current.state == DeliveryAttemptState::DeliveryUncertain
        }
        _ => false,
    };
    if already_projected {
        return Ok(Some(current));
    }
    if matches!(
        current.state,
        DeliveryAttemptState::CompletedReported
            | DeliveryAttemptState::Failed
            | DeliveryAttemptState::DeliveryUncertain
            | DeliveryAttemptState::Cancelled
            | DeliveryAttemptState::Superseded
    ) {
        // A stale or conflicting final report cannot rewrite a terminal
        // attempt. The ordinary job projection is the gate that decides which
        // report is durable; this path remains a safe no-op on replay.
        return Ok(Some(current));
    }
    let attempt = if next == DeliveryAttemptState::DeliveryUncertain {
        state
            .destination_topology
            .mark_post_spooler_attempt_uncertain(tenant_scope, &event.job_id.to_string())
            .await
    } else {
        state
            .destination_topology
            .transition_post_spooler_attempt(
                tenant_scope,
                &event.job_id.to_string(),
                &agent_id.to_string(),
                &current.route_id,
                next,
            )
            .await
    }
    .map_err(storage_error)?;
    state
        .publish(
            tenant,
            "attempt.updated",
            &serde_json::json!({
                "attempt_id": attempt.id,
                "job_id": attempt.job_id,
                "destination_id": attempt.destination_id,
                "state": public_attempt_state(attempt.state),
                "updated_at": attempt.updated_at,
            }),
        )
        .await?;
    if attempt.state == DeliveryAttemptState::DeliveryUncertain {
        state
            .publish(
                tenant,
                "destination.updated",
                &serde_json::json!({
                    "destination_id": attempt.destination_id,
                    "state": "needs_review",
                    "reason": "delivery_uncertain",
                    "updated_at": attempt.updated_at,
                }),
            )
            .await?;
    }
    Ok(Some(attempt))
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
    Ok(Json(
        destination_responses(&state, tenant_scope, stored).await?,
    ))
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
    Ok(Json(
        destination_response(&state, tenant_scope, destination).await?,
    ))
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
    Ok(Json(route_responses(&state, tenant_scope, stored).await?))
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
    Ok(Json(route_responses(&state, tenant_scope, stored).await?))
}

pub(crate) fn runtime_observation_response(
    value: StoredNodeRuntimeObservation,
) -> NodeRuntimeObservationResponse {
    let now = Utc::now();
    let freshness = if value.fresh_until >= now {
        "live"
    } else if value.observed_at + TimeDelta::minutes(5) >= now {
        "recent"
    } else {
        "stale"
    };
    NodeRuntimeObservationResponse {
        node_id: value.agent_id,
        sequence: value.sequence,
        host_mode: value.host_mode,
        availability_class: value.availability_class,
        lifecycle_state: value.lifecycle_state,
        accepts_cloud_jobs: value.accepts_cloud_jobs,
        execution_budget_ms: value.execution_budget_ms,
        wake_mechanisms: value.wake_mechanisms,
        observed_at: value.observed_at,
        expires_at: value.fresh_until,
        freshness,
    }
}

pub async fn list_node_runtime_observations(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<NodeRuntimeListQuery>,
) -> Result<Json<NodeRuntimeObservationPage>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::AgentsRead).await?;
    let limit = query.limit.clamp(1, 1_000);
    let after = query
        .after
        .as_deref()
        .map(parse_node_id)
        .transpose()?
        .map(|value| value.to_string());
    let mut observations = state
        .destination_topology
        .list_latest_node_runtime_observations(
            scope(tenant),
            after.as_deref(),
            limit.saturating_add(1),
        )
        .await
        .map_err(storage_error)?;
    let has_more = observations.len() > usize::try_from(limit).unwrap_or(1_000);
    observations.truncate(usize::try_from(limit).unwrap_or(1_000));
    let next_cursor = has_more
        .then(|| observations.last().map(|value| value.agent_id.clone()))
        .flatten();
    Ok(Json(NodeRuntimeObservationPage {
        data: observations
            .into_iter()
            .map(runtime_observation_response)
            .collect(),
        next_cursor,
        has_more,
    }))
}

pub(crate) fn wake_hint_response(value: NodeWakeHint) -> NodeWakeHintResponse {
    NodeWakeHintResponse {
        id: value.id,
        node_id: value.agent_id,
        reason: value.reason,
        delivery_channel: value.delivery_channel,
        status: value.status,
        requested_at: value.requested_at,
        expires_at: value.expires_at,
        observed_at: value.observed_at,
    }
}

pub async fn get_node_runtime(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(node_id): Path<String>,
) -> Result<Json<NodeRuntimeObservationResponse>, AppError> {
    let node_id = parse_node_id(&node_id)?;
    let tenant = authenticate_native(&state, &headers, Scope::AgentsRead).await?;
    state
        .repository
        .get_agent(tenant.workspace_id, tenant.environment_id, node_id)
        .await?;
    let observation = state
        .destination_topology
        .latest_node_runtime_observation(scope(tenant), &node_id.to_string())
        .await
        .map_err(storage_error)?;
    Ok(Json(runtime_observation_response(observation)))
}

pub async fn list_node_wake_hints(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(node_id): Path<String>,
    Query(query): Query<ObservationQuery>,
) -> Result<Json<Vec<NodeWakeHintResponse>>, AppError> {
    let node_id = parse_node_id(&node_id)?;
    let tenant = authenticate_native(&state, &headers, Scope::AgentsRead).await?;
    state
        .repository
        .get_agent(tenant.workspace_id, tenant.environment_id, node_id)
        .await?;
    Ok(Json(
        state
            .destination_topology
            .list_node_wake_hints(
                scope(tenant),
                &node_id.to_string(),
                query.limit.clamp(1, 100),
            )
            .await
            .map_err(storage_error)?
            .into_iter()
            .map(wake_hint_response)
            .collect(),
    ))
}

pub async fn create_node_wake_hint(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(node_id): Path<String>,
    Json(request): Json<CreateWakeHintRequest>,
) -> Result<Response, AppError> {
    let node_id = parse_node_id(&node_id)?;
    let tenant = authenticate_native(&state, &headers, Scope::AgentsWrite).await?;
    state
        .repository
        .get_agent(tenant.workspace_id, tenant.environment_id, node_id)
        .await?;
    let idempotency_key = headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .filter(|value| (8..=255).contains(&value.len()))
        .ok_or_else(|| {
            AppError::invalid(
                "invalid_idempotency_key",
                "Idempotency-Key must be between 8 and 255 bytes.",
            )
        })?;
    if !matches!(
        request.reason.as_str(),
        "job_available" | "operator_request" | "inventory_refresh" | "diagnostics"
    ) || !(30..=900).contains(&request.expires_in_seconds)
    {
        return Err(AppError::invalid(
            "invalid_wake_hint",
            "Wake reason or expiry is outside protocol bounds.",
        ));
    }
    let requested_at = Utc::now();
    let hint = state
        .destination_topology
        .create_node_wake_hint(
            scope(tenant),
            &NodeWakeHint {
                id: format!("wkh_{}", ulid::Ulid::new()),
                agent_id: node_id.to_string(),
                reason: request.reason,
                // No external dispatcher is configured yet. Returning this
                // through a later signed sync is explicitly not remote wake.
                delivery_channel: "connected_session".into(),
                status: "pending".into(),
                requested_at,
                expires_at: requested_at
                    + TimeDelta::seconds(i64::from(request.expires_in_seconds)),
                observed_at: None,
            },
            idempotency_key,
        )
        .await
        .map_err(storage_error)?;
    let response = wake_hint_response(hint);
    state
        .publish(tenant, "node.wake_hint.requested", &response)
        .await?;
    Ok((StatusCode::ACCEPTED, Json(response)).into_response())
}

fn parse_node_id(value: &str) -> Result<piqae_domain::AgentId, AppError> {
    piqae_domain::AgentId::from_str(value).map_err(|_| {
        AppError::invalid(
            "invalid_node_id",
            "Node ID must be an agt_<ULID> identifier.",
        )
    })
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
                confidence: evidence_confidence(&value),
                id: value.id,
                destination_id: value.destination_id,
                route_id: value.route_id,
                kind: value.kind,
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
            IdentityDecisionKind::Merge | IdentityDecisionKind::Confirm => "merge",
            IdentityDecisionKind::Split | IdentityDecisionKind::RejectMatch => "split",
            IdentityDecisionKind::Reverse => "reversal",
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
    Ok(Json(
        stored_decisions(&state, tenant_scope, &destination_id).await?,
    ))
}

pub async fn create_identity_decision(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(destination_id): Path<String>,
    Json(request): Json<CreateIdentityDecisionRequest>,
) -> Result<(axum::http::StatusCode, Json<IdentityDecisionResponse>), AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::PrintersWrite).await?;
    let tenant_scope = scope(tenant);
    let destination = state
        .destination_topology
        .get_destination(tenant_scope, &destination_id)
        .await
        .map_err(storage_error)?;
    if request.route_ids.is_empty() || request.route_ids.len() > 100 {
        return Err(AppError::invalid(
            "invalid_route_selection",
            "Select between one and 100 routes.",
        ));
    }
    if request.reason.trim().is_empty() || request.reason.chars().count() > 2_000 {
        return Err(AppError::invalid(
            "invalid_identity_reason",
            "A reason of at most 2,000 characters is required.",
        ));
    }
    if request
        .display_name
        .as_ref()
        .is_some_and(|name| name.trim().is_empty() || name.chars().count() > 255)
    {
        return Err(AppError::invalid(
            "invalid_destination_name",
            "A destination name must contain at most 255 characters.",
        ));
    }
    let unique = request.route_ids.iter().collect::<HashSet<_>>();
    if unique.len() != request.route_ids.len() {
        return Err(AppError::invalid(
            "invalid_route_selection",
            "Route IDs must be unique.",
        ));
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
    let (kind, decision_destination_id) = match request.kind {
        DecisionRequestKind::Merge => {
            if related_destination_ids.len() != 1
                || routes
                    .iter()
                    .any(|route| route.destination_id == destination_id)
            {
                return Err(AppError::conflict(
                    "merge_not_reversible",
                    "Select routes from exactly one other destination to merge into this destination.",
                ));
            }
            (IdentityDecisionKind::Merge, destination_id.clone())
        }
        DecisionRequestKind::Split => {
            if routes
                .iter()
                .any(|route| route.destination_id != destination_id)
            {
                return Err(AppError::conflict(
                    "split_route_mismatch",
                    "Every split route must currently belong to this destination.",
                ));
            }
            let new_id = format!("pdst_{}", ulid::Ulid::new());
            related_destination_ids = vec![destination_id.clone()];
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
                            .map_or_else(
                                || format!("{} (split)", destination.name),
                                ToOwned::to_owned,
                            ),
                        identity_confidence: IdentityConfidence::Conflict,
                        state: "available".into(),
                        scheduling_authority_id: destination.scheduling_authority_id.clone(),
                        identity_revision: destination.identity_revision.saturating_add(1),
                        created_at: now,
                        updated_at: now,
                    },
                )
                .await
                .map_err(storage_error)?;
            (IdentityDecisionKind::Split, new_id)
        }
    };
    let decision = IdentityDecision {
        id: format!("idd_{}", ulid::Ulid::new()),
        kind,
        destination_id: decision_destination_id,
        related_destination_ids,
        route_ids: request.route_ids,
        evidence_ids: Vec::new(),
        actor_kind: "operator".into(),
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
    state
        .publish(
            tenant,
            "destination.updated",
            &serde_json::json!({
                "destination_id": decision.destination_id,
                "decision_id": decision.id,
                "updated_at": now,
            }),
        )
        .await?;
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
        return Err(AppError::conflict(
            "identity_decision_reversed",
            "This decision was already reversed.",
        ));
    }
    match original.kind {
        IdentityDecisionKind::Split | IdentityDecisionKind::Merge => {}
        _ => {
            return Err(AppError::conflict(
                "identity_decision_not_reversible",
                "Only merge and split decisions can be reversed.",
            ));
        }
    }
    let now = Utc::now();
    let reversal = IdentityDecision {
        id: format!("idd_{}", ulid::Ulid::new()),
        kind: IdentityDecisionKind::Reverse,
        destination_id,
        related_destination_ids: original.related_destination_ids,
        route_ids: original.route_ids,
        evidence_ids: original.evidence_ids,
        actor_kind: "operator".into(),
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
    state
        .publish(
            tenant,
            "destination.updated",
            &serde_json::json!({
                "destination_id": reversal.destination_id,
                "decision_id": reversal.id,
                "updated_at": now,
            }),
        )
        .await?;
    Ok(Json(decision_response(reversal, &HashMap::new())))
}

pub async fn list_route_reservations(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<RouteReservationResponse>>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::JobsRead).await?;
    Ok(Json(
        state
            .destination_topology
            .list_route_reservations(scope(tenant), 100)
            .await
            .map_err(storage_error)?
            .into_iter()
            .map(reservation_response)
            .collect(),
    ))
}

pub async fn list_delivery_attempts(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(job_id): Path<String>,
) -> Result<Json<Vec<DeliveryAttemptResponse>>, AppError> {
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
            .map_err(storage_error)?
            .into_iter()
            .map(attempt_response)
            .collect(),
    ))
}

fn stored_uncertainty_resolution(value: &str) -> Option<&'static str> {
    match value {
        "acknowledge_printed" => Some("confirmed_delivered"),
        "acknowledge_missing" => Some("accept_missing"),
        "cancelled" => Some("cancelled"),
        "reprint" => Some("reprint_authorized"),
        _ => None,
    }
}

fn public_uncertainty_resolution(value: &str) -> &'static str {
    match value {
        "confirmed_delivered" => "acknowledge_printed",
        "accept_missing" => "acknowledge_missing",
        "reprint_authorized" => "reprint",
        _ => "cancelled",
    }
}

async fn validate_reprintable_job(
    state: &AppState,
    job: &piqae_domain::Job,
) -> Result<(), AppError> {
    if job.expires_at <= Utc::now() {
        return Err(AppError::conflict(
            "reprint_content_expired",
            "The retained content has expired; submit a fresh print job instead.",
        ));
    }
    match &job.content {
        piqae_domain::ContentSource::Base64 { .. } => Ok(()),
        piqae_domain::ContentSource::Upload { upload_id } => {
            let upload = state
                .repository
                .get_upload(job.workspace_id, job.environment_id, upload_id)
                .await?;
            if upload.state == "complete" {
                Ok(())
            } else {
                Err(AppError::conflict(
                    "reprint_content_unavailable",
                    "The original retained content is no longer complete.",
                ))
            }
        }
        piqae_domain::ContentSource::EncryptedUpload { .. } => Err(AppError::conflict(
            "encrypted_reprint_requires_new_envelope",
            "Encrypted content requires a fresh job envelope and cannot be cloned.",
        )),
        piqae_domain::ContentSource::Uri { .. } => Err(AppError::conflict(
            "uri_reprint_requires_new_job",
            "External content may have changed; submit a fresh print job instead.",
        )),
    }
}

fn replacement_metadata(
    mut metadata: BTreeMap<String, String>,
    resolution: &piqae_storage_postgres::destination_topology::DeliveryUncertaintyResolution,
) -> BTreeMap<String, String> {
    metadata.retain(|key, _| {
        !key.starts_with("piqae.delivery_")
            && !key.starts_with("piqae.uncertainty_")
            && !key.starts_with("piqae.attempt_")
            && !key.starts_with("piqae.reprint_")
            && !key.starts_with("spool.delivery_")
    });
    metadata.insert("piqae.reprint_of".into(), resolution.job_id.clone());
    metadata.insert(
        "piqae.uncertainty_resolution_id".into(),
        resolution.id.clone(),
    );
    metadata.insert(
        "piqae.reprint_authorization_request_id".into(),
        resolution.request_id.clone(),
    );
    metadata
}

async fn create_authorized_reprint(
    state: &AppState,
    tenant: crate::authentication::TenantContext,
    resolution: &piqae_storage_postgres::destination_topology::DeliveryUncertaintyResolution,
) -> Result<Option<piqae_domain::Job>, AppError> {
    if resolution.resolution != "reprint_authorized" {
        return Ok(None);
    }
    let original_id = resolution
        .job_id
        .parse()
        .map_err(|_| AppError::service_unavailable("invalid_uncertain_job_id"))?;
    let original = state
        .repository
        .get_job(tenant.workspace_id, tenant.environment_id, original_id)
        .await?;
    validate_reprintable_job(state, &original).await?;
    let attempt = state
        .destination_topology
        .get_latest_delivery_attempt(scope(tenant), &resolution.job_id)
        .await
        .map_err(storage_error)?;
    let route = state
        .destination_topology
        .get_route(scope(tenant), &attempt.route_id)
        .await
        .map_err(storage_error)?;
    let agent_id = route
        .agent_id
        .parse()
        .map_err(|_| AppError::service_unavailable("invalid_uncertain_route_agent"))?;
    let now = Utc::now();
    let metadata = replacement_metadata(original.metadata.clone(), resolution);
    let replacement = piqae_domain::Job {
        id: piqae_domain::JobId::new(),
        workspace_id: tenant.workspace_id,
        environment_id: tenant.environment_id,
        printer_id: original.printer_id,
        title: format!("Reprint: {}", original.title)
            .chars()
            .take(180)
            .collect(),
        source: original.source.clone(),
        content_kind: original.content_kind,
        content: original.content.clone(),
        options: original.options.clone(),
        metadata,
        deliveries: original.deliveries,
        state: piqae_domain::JobState::Registered,
        created_at: now,
        expires_at: now + TimeDelta::hours(1),
        delivery_uncertain_since: None,
    };
    let digest = Sha256::digest(resolution.request_id.as_bytes());
    let idempotency_key = format!("uncertainty-reprint-{}", &hex::encode(digest)[..32]);
    let request_bytes = serde_json::to_vec(&serde_json::json!({
        "original_job_id": resolution.job_id,
        "resolution_request_id": resolution.request_id,
    }))
    .map_err(|_| AppError::service_unavailable("reprint_idempotency_failed"))?;
    let stored = state
        .repository
        .create_cloud_job(
            &replacement,
            agent_id,
            Some(&idempotency_key),
            &request_bytes,
            state.capabilities.billing.enabled,
        )
        .await?;
    let replacement = match stored {
        crate::repository::CreateResult::Created(created)
        | crate::repository::CreateResult::Existing(created) => created,
    };
    let replacement = if replacement.state == piqae_domain::JobState::Registered {
        state
            .repository
            .transition_job(
                tenant.workspace_id,
                tenant.environment_id,
                replacement.id,
                piqae_domain::JobState::WaitingForAgent,
                None,
                Some("Explicit reprint authorized after uncertain delivery".into()),
                None,
                None,
            )
            .await?
    } else {
        replacement
    };
    state.publish(tenant, "job.updated", &replacement).await?;
    Ok(Some(replacement))
}

pub(crate) async fn finalize_acknowledged_uncertainty(
    state: &AppState,
    tenant: crate::authentication::TenantContext,
    agent_id: piqae_domain::AgentId,
) -> Result<(), AppError> {
    let resolutions = state
        .destination_topology
        .finalize_acknowledged_uncertainty_resolutions(scope(tenant), &agent_id.to_string(), 100)
        .await
        .map_err(storage_error)?;
    for resolution in resolutions {
        let replacement = create_authorized_reprint(state, tenant, &resolution).await?;
        for event_type in ["attempt.updated", "destination.updated"] {
            state
                .publish(
                    tenant,
                    event_type,
                    &serde_json::json!({
                        "job_id": resolution.job_id,
                        "destination_id": resolution.destination_id,
                        "state": "resolved",
                        "request_id": resolution.request_id,
                        "replacement_job_id": replacement.as_ref().map(|job| job.id),
                        "updated_at": resolution.created_at,
                    }),
                )
                .await?;
        }
    }
    Ok(())
}

/// Queues an exact node-local ambiguity-fence resolution. The decision remains
/// pending until the node durably acknowledges the command cursor.
pub async fn resolve_uncertain_delivery(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(job_id): Path<String>,
    Json(request): Json<ResolveUncertainDeliveryRequest>,
) -> Result<Response, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::JobsWrite).await?;
    let request_id = headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .filter(|value| (8..=255).contains(&value.len()))
        .ok_or_else(|| {
            AppError::invalid(
                "invalid_idempotency_key",
                "Idempotency-Key must be between 8 and 255 bytes.",
            )
        })?;
    if request.note.trim().is_empty() || request.note.len() > 2_000 {
        return Err(AppError::invalid(
            "invalid_uncertainty_note",
            "An operator note between 1 and 2000 bytes is required.",
        ));
    }
    let resolution = stored_uncertainty_resolution(&request.resolution).ok_or_else(|| {
        AppError::invalid(
            "invalid_uncertainty_resolution",
            "Choose acknowledge_printed, acknowledge_missing, cancelled, or reprint.",
        )
    })?;
    let parsed_job_id = job_id
        .parse()
        .map_err(|_| AppError::invalid("invalid_job_id", "The job identifier is invalid."))?;
    let job = state
        .repository
        .get_job(tenant.workspace_id, tenant.environment_id, parsed_job_id)
        .await?;
    if job.state != piqae_domain::JobState::DeliveryUncertain {
        return Err(AppError::conflict(
            "job_not_delivery_uncertain",
            "Only a delivery-uncertain job can be resolved.",
        ));
    }
    if resolution == "reprint_authorized" {
        validate_reprintable_job(&state, &job).await?;
    }
    let actor_id = tenant.platform_service_account_id.map_or_else(
        || format!("operator:{}", tenant.workspace_id),
        |id| format!("platform:{id}"),
    );
    let pending = state
        .destination_topology
        .enqueue_delivery_uncertainty_resolution(
            scope(tenant),
            &job_id,
            resolution,
            Some(request.note.trim()),
            &actor_id,
            request_id,
        )
        .await
        .map_err(storage_error)?;
    if let Some(finalized) = state
        .destination_topology
        .finalize_delivery_uncertainty_resolution(scope(tenant), request_id)
        .await
        .map_err(storage_error)?
    {
        let resolved_at = finalized.created_at;
        let replacement_job = create_authorized_reprint(&state, tenant, &finalized).await?;
        let job = state
            .repository
            .get_job(tenant.workspace_id, tenant.environment_id, parsed_job_id)
            .await?;
        let response = UncertainDeliveryResolutionResponse {
            job,
            resolution: public_uncertainty_resolution(&finalized.resolution).into(),
            state: "resolved",
            request_id: finalized.request_id,
            replacement_job,
            created_at: finalized.created_at,
            resolved_at: Some(resolved_at),
        };
        for event_type in ["attempt.updated", "destination.updated"] {
            state
                .publish(
                    tenant,
                    event_type,
                    &serde_json::json!({
                        "job_id": job_id,
                        "destination_id": finalized.destination_id,
                        "state": "resolved",
                        "request_id": response.request_id,
                        "updated_at": resolved_at,
                    }),
                )
                .await?;
        }
        return Ok((StatusCode::OK, Json(response)).into_response());
    }
    Ok((
        StatusCode::ACCEPTED,
        Json(UncertainDeliveryResolutionResponse {
            job,
            resolution: request.resolution,
            state: "pending_node_ack",
            request_id: pending.request_id,
            replacement_job: None,
            created_at: pending.created_at,
            resolved_at: None,
        }),
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::too_many_lines)]
    use super::*;
    use crate::{authentication::StaticAuthenticator, repository::MemoryRepository};
    use axum::{body::Body, http::Request};
    use base64::{Engine as _, engine::general_purpose::STANDARD_NO_PAD};
    use ed25519_dalek::{Signer as _, SigningKey};
    use http_body_util::BodyExt as _;
    use piqae_domain::{EnvironmentId, WorkspaceId};
    use piqae_storage_postgres::destination_topology::{
        DestinationTopologyRepository, MemoryDestinationTopologyRepository,
    };
    use std::{collections::BTreeMap, sync::Arc};
    use tower::ServiceExt as _;

    fn signed_agent_request(
        agent_id: piqae_domain::AgentId,
        signing_key: &SigningKey,
        body: Vec<u8>,
    ) -> Request<Body> {
        let path = "/v1/agent/sync";
        let timestamp = Utc::now().timestamp_millis();
        let nonce = uuid::Uuid::new_v4();
        let digest = format!("{:x}", Sha256::digest(&body));
        let canonical = format!("POST\n{path}\n{timestamp}\n{nonce}\n{digest}");
        let signature = signing_key.sign(canonical.as_bytes());
        Request::builder()
            .method("POST")
            .uri(path)
            .header("content-type", "application/json")
            .header("x-piqae-agent-id", agent_id.to_string())
            .header("x-piqae-timestamp", timestamp.to_string())
            .header("x-piqae-nonce", nonce.to_string())
            .header("x-piqae-body-sha256", digest)
            .header(
                "x-piqae-signature",
                STANDARD_NO_PAD.encode(signature.to_bytes()),
            )
            .body(Body::from(body))
            .expect("signed sync request")
    }

    fn state_with_topology(topology: Arc<MemoryDestinationTopologyRepository>) -> AppState {
        AppState::new_for_tests(
            Arc::new(MemoryRepository::with_destination_topology(
                topology.as_ref().clone(),
            )),
            Arc::new(StaticAuthenticator::default()),
        )
        .with_destination_topology(topology)
        .with_destination_identity_key([7; 32])
    }

    #[test]
    fn evidence_digest_is_tenant_scoped_and_versioned() {
        let state = state_with_topology(Arc::new(MemoryDestinationTopologyRepository::default()));
        let first = TenantScope {
            workspace_id: WorkspaceId::new(),
            environment_id: EnvironmentId::new(),
        };
        let second = TenantScope {
            workspace_id: WorkspaceId::new(),
            environment_id: EnvironmentId::new(),
        };
        let raw = "0".repeat(64);
        let first_digest = tenant_evidence_digest(&state, first, &raw).expect("first digest");
        let second_digest = tenant_evidence_digest(&state, second, &raw).expect("second digest");
        assert!(first_digest.starts_with("hmac-sha256:"));
        assert_ne!(first_digest, second_digest);
    }

    #[test]
    fn public_evidence_uses_conflict_not_a_decision_value() {
        let evidence = IdentityEvidence {
            id: "ide_test".into(),
            destination_id: "pdst_test".into(),
            route_id: "rte_test".into(),
            kind: "device_serial".into(),
            value_digest: format!("hmac-sha256:{}", "0".repeat(64)),
            strength: "strong".into(),
            conflicts: true,
            observed_at: Utc::now(),
            expires_at: None,
            metadata: serde_json::json!({}),
        };
        assert_eq!(evidence_confidence(&evidence), "conflict");
    }

    #[test]
    fn authorized_reprint_metadata_drops_prior_delivery_outcomes() {
        let resolution =
            piqae_storage_postgres::destination_topology::DeliveryUncertaintyResolution {
                id: "dur_new".into(),
                job_id: "01J00000000000000000000000".into(),
                attempt_id: "attempt_old".into(),
                destination_id: "destination_old".into(),
                resolution: "reprint_authorized".into(),
                note: None,
                actor_id: "operator".into(),
                request_id: "request_new".into(),
                created_at: Utc::now(),
            };
        let metadata = replacement_metadata(
            BTreeMap::from([
                (
                    "piqae.delivery_resolution".into(),
                    "reprint_authorized".into(),
                ),
                ("piqae.delivery_resolution_request_id".into(), "old".into()),
                ("piqae.delivery_result".into(), "uncertain".into()),
                ("piqae.attempt_id".into(), "attempt_old".into()),
                ("piqae.route_id".into(), "route_kept".into()),
            ]),
            &resolution,
        );
        assert!(!metadata.contains_key("piqae.delivery_resolution"));
        assert!(!metadata.contains_key("piqae.delivery_result"));
        assert!(!metadata.contains_key("piqae.attempt_id"));
        assert_eq!(
            metadata.get("piqae.route_id").map(String::as_str),
            Some("route_kept")
        );
        assert_eq!(
            metadata.get("piqae.reprint_of").map(String::as_str),
            Some(resolution.job_id.as_str())
        );
        assert_eq!(
            metadata
                .get("piqae.reprint_authorization_request_id")
                .map(String::as_str),
            Some("request_new")
        );
    }

    #[tokio::test]
    async fn topology_responses_preserve_resource_creation_times() {
        let topology = Arc::new(MemoryDestinationTopologyRepository::default());
        let state = state_with_topology(topology.clone());
        let tenant_scope = TenantScope {
            workspace_id: WorkspaceId::new(),
            environment_id: EnvironmentId::new(),
        };
        let created_at = Utc::now() - TimeDelta::hours(1);
        let updated_at = Utc::now();
        let destination = StoredPhysicalDestination {
            id: "pdst_01J00000000000000000000000".into(),
            name: "Printer".into(),
            identity_confidence: IdentityConfidence::High,
            state: "available".into(),
            scheduling_authority_id: None,
            identity_revision: 1,
            created_at,
            updated_at,
        };
        topology
            .upsert_destination(tenant_scope, &destination)
            .await
            .expect("destination");
        let route = StoredPrinterRoute {
            id: "rte_01J00000000000000000000000".into(),
            destination_id: destination.id.clone(),
            printer_id: "ptr_test".into(),
            agent_id: "agt_test".into(),
            native_queue_id: "native-test".into(),
            local_route_key: Some("rte_local_test".into()),
            state: "available".into(),
            role: "primary".into(),
            priority: 0,
            enabled: true,
            capability_revision: 1,
            profile_revision: 1,
            last_seen_at: Some(updated_at),
            created_at,
            updated_at,
        };
        topology
            .upsert_route(tenant_scope, &route)
            .await
            .expect("route");

        let destination_response = destination_response(&state, tenant_scope, destination)
            .await
            .expect("destination response");
        let route_response = route_response(&state, tenant_scope, route)
            .await
            .expect("route response");
        assert_eq!(destination_response.created_at, created_at);
        assert_eq!(destination_response.updated_at, updated_at);
        assert_eq!(route_response.created_at, created_at);
        assert_eq!(route_response.updated_at, updated_at);
    }

    #[tokio::test]
    async fn matching_requires_same_kind_and_rejects_conflicting_strong_evidence() {
        let topology = Arc::new(MemoryDestinationTopologyRepository::default());
        let state = state_with_topology(topology.clone());
        let tenant_scope = TenantScope {
            workspace_id: WorkspaceId::new(),
            environment_id: EnvironmentId::new(),
        };
        let destination_id = "pdst_01J00000000000000000000000";
        topology
            .upsert_destination(
                tenant_scope,
                &StoredPhysicalDestination {
                    id: destination_id.into(),
                    name: "Printer".into(),
                    identity_confidence: IdentityConfidence::High,
                    state: "available".into(),
                    scheduling_authority_id: None,
                    identity_revision: 1,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                },
            )
            .await
            .expect("destination");
        topology
            .upsert_route(
                tenant_scope,
                &StoredPrinterRoute {
                    id: "rte_01J00000000000000000000000".into(),
                    destination_id: destination_id.into(),
                    printer_id: "ptr_test".into(),
                    agent_id: "agt_test".into(),
                    native_queue_id: "native-test".into(),
                    local_route_key: Some("rte_local_test".into()),
                    state: "available".into(),
                    role: "primary".into(),
                    priority: 0,
                    enabled: true,
                    capability_revision: 1,
                    profile_revision: 1,
                    last_seen_at: Some(Utc::now()),
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                },
            )
            .await
            .expect("route");
        for (id, kind, digest) in [
            ("ide_uuid", "ipp_uuid", "a".repeat(64)),
            ("ide_serial", "device_serial", "b".repeat(64)),
        ] {
            topology
                .record_identity_evidence(
                    tenant_scope,
                    &IdentityEvidence {
                        id: id.into(),
                        destination_id: destination_id.into(),
                        route_id: "rte_01J00000000000000000000000".into(),
                        kind: kind.into(),
                        value_digest: digest,
                        strength: "strong".into(),
                        conflicts: false,
                        observed_at: Utc::now(),
                        expires_at: None,
                        metadata: serde_json::json!({}),
                    },
                )
                .await
                .expect("evidence");
        }
        let cross_kind = destination_for_new_route(
            &state,
            tenant_scope,
            &[("device_serial".into(), "strong".into(), "a".repeat(64))],
        )
        .await
        .expect("cross-kind result");
        assert_ne!(cross_kind.0, destination_id);
        assert!(!cross_kind.1);

        let conflicting = destination_for_new_route(
            &state,
            tenant_scope,
            &[
                ("ipp_uuid".into(), "strong".into(), "a".repeat(64)),
                ("device_serial".into(), "strong".into(), "c".repeat(64)),
            ],
        )
        .await
        .expect("conflict result");
        assert_ne!(conflicting.0, destination_id);
        assert!(conflicting.1);
    }

    #[tokio::test]
    async fn postgres_agent_projection_is_idempotent_and_tenant_isolated() {
        use crate::authentication::TenantContext;
        use piqae_domain::{PrinterCapabilities, PrinterId, PrinterState};
        use piqae_protocol::agent::{
            AgentHealth, AgentProtocolCapabilities, AgentSyncRequest, IdentityEvidenceStrength,
            PhysicalIdentityEvidence, PhysicalIdentityEvidenceKind, PrinterRouteSnapshot,
            PrinterSnapshot, PrivacySafeQueueObservation, QueueSnapshot,
            RouteObservation as AgentRouteObservation,
        };
        use piqae_storage_postgres::PostgresStore;
        use sqlx::postgres::PgPoolOptions;

        let Ok(database_url) = std::env::var("PIQAE_TEST_DATABASE_URL") else {
            eprintln!("skipped: set PIQAE_TEST_DATABASE_URL for control-plane topology evidence");
            return;
        };
        let schema = format!("piqae_control_topology_{}", ulid::Ulid::new()).to_ascii_lowercase();
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&database_url)
            .await
            .expect("connect test postgres");
        sqlx::query(&format!("CREATE SCHEMA {schema}"))
            .execute(&admin)
            .await
            .expect("create disposable schema");
        let schema_for_connection = schema.clone();
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .after_connect(move |connection, _| {
                let statement = format!("SET search_path TO {schema_for_connection}");
                Box::pin(async move {
                    sqlx::query(&statement).execute(connection).await?;
                    Ok(())
                })
            })
            .connect(&database_url)
            .await
            .expect("connect schema pool");
        let store = PostgresStore::from_pool(pool.clone());
        store.migrate().await.expect("migrate disposable schema");
        let authenticator = StaticAuthenticator::default();
        let state =
            AppState::new_for_tests(Arc::new(store.clone()), Arc::new(authenticator.clone()))
                .with_destination_topology(Arc::new(store.clone()))
                .with_destination_identity_key([9; 32]);
        let application = crate::router(state.clone());

        let mut destination_ids = Vec::new();
        let mut stored_digests = Vec::new();
        let mut tenant_tokens = Vec::new();
        for index in 1_u8..=2 {
            let workspace_id = WorkspaceId::new();
            let environment_id = EnvironmentId::new();
            let tenant = TenantContext::unrestricted(workspace_id, environment_id);
            store
                .ensure_bootstrap_tenant(workspace_id, environment_id)
                .await
                .expect("bootstrap tenant");
            let agent_id = piqae_domain::AgentId::new();
            let printer_id = PrinterId::new();
            let signing_key = SigningKey::from_bytes(&[index; 32]);
            sqlx::query("INSERT INTO agents (id,workspace_id,environment_id,name,installation_id,public_key,os,architecture,version,protocol_version) VALUES ($1,$2,$3,'Node',$4,$5,'linux','x86_64','test',1)")
                .bind(agent_id.to_string()).bind(workspace_id.to_string()).bind(environment_id.to_string())
                .bind(format!("installation-{index}")).bind(signing_key.verifying_key().to_bytes().to_vec()).execute(&pool).await.expect("agent");
            sqlx::query("INSERT INTO printers (id,workspace_id,environment_id,agent_id,native_id,name,state,capabilities_revision) VALUES ($1,$2,$3,$4,$5,'Printer','online',1)")
                .bind(printer_id.to_string()).bind(workspace_id.to_string()).bind(environment_id.to_string())
                .bind(agent_id.to_string()).bind(format!("native-{index}")).execute(&pool).await.expect("printer");
            let now = Utc::now();
            let request = AgentSyncRequest {
                agent_id,
                protocol_version: 1,
                agent_version: "test".into(),
                printer_revision: 1,
                acknowledged_command_cursor: None,
                event_cursor: None,
                queue: QueueSnapshot {
                    queued_jobs: 0,
                    active_jobs: 0,
                    content_bytes: 0,
                    accepts_jobs: true,
                },
                health: AgentHealth {
                    started_at: now,
                    observed_at: now,
                    sqlite_integrity_ok: true,
                    executor_crashes: 0,
                    last_error_code: None,
                },
                printers: Some(vec![PrinterSnapshot {
                    id: printer_id,
                    native_id: format!("native-{index}"),
                    name: "Printer".into(),
                    state: PrinterState::Online,
                    is_default: true,
                    capabilities: PrinterCapabilities::default(),
                    exposed: true,
                    capability_revision: 1,
                    native_options: BTreeMap::default(),
                    semantic_capabilities: piqae_domain::SemanticPrinterCapabilities::default(),
                    profiles: Vec::new(),
                    route: Some(PrinterRouteSnapshot {
                        local_route_key: format!("rte_{}", format!("{index:x}").repeat(32)),
                        inventory_revision: 1,
                        topology_revision: 1,
                        observed_at: now,
                        identity_evidence: vec![PhysicalIdentityEvidence {
                            kind: PhysicalIdentityEvidenceKind::DeviceSerial,
                            value_sha256: "a".repeat(64),
                            strength: IdentityEvidenceStrength::Strong,
                        }],
                        identity_confidence: piqae_protocol::agent::IdentityConfidence::High,
                        topology_change: None,
                        profile_observed_at: Some(now),
                        stock_observed_at: Some(now),
                    }),
                }]),
                events: Vec::new(),
                diagnostics: Vec::new(),
                document_render: piqae_protocol::agent::DocumentRenderCapabilities::default(),
                capabilities: AgentProtocolCapabilities::default(),
                route_observations: vec![AgentRouteObservation {
                    local_route_key: format!("rte_{}", format!("{index:x}").repeat(32)),
                    sequence: 1,
                    observed_at: now,
                    inventory_revision: 1,
                    state: PrinterState::Online,
                    accepts_jobs: true,
                    state_reasons: Vec::new(),
                    queue: Some(PrivacySafeQueueObservation::default()),
                    profile_observed_at: Some(now),
                    stock_observed_at: Some(now),
                }],
                topology_changes: Vec::new(),
                native_handoffs: Vec::new(),
                runtime: None,
            };
            let token = format!("piq_destination_test_{index}");
            authenticator.insert(&token, tenant).await;
            tenant_tokens.push(token.clone());
            for label in ["first", "retry"] {
                let response = application
                    .clone()
                    .oneshot(signed_agent_request(
                        agent_id,
                        &signing_key,
                        serde_json::to_vec(&request).expect("sync JSON"),
                    ))
                    .await
                    .expect("agent sync response");
                assert_eq!(response.status(), StatusCode::OK, "{label} sync");
                let payload: piqae_protocol::agent::AgentSyncResponse = serde_json::from_slice(
                    &response
                        .into_body()
                        .collect()
                        .await
                        .expect("sync body")
                        .to_bytes(),
                )
                .expect("sync response JSON");
                assert_eq!(
                    payload
                        .inventory_projection
                        .as_ref()
                        .map(|ack| ack.revision),
                    Some(1),
                    "projection is acknowledged only after durable persistence"
                );
            }
            let listed = application
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/v1/physical-destinations")
                        .header("authorization", format!("Bearer {token}"))
                        .body(Body::empty())
                        .expect("destination list request"),
                )
                .await
                .expect("destination list response");
            assert_eq!(listed.status(), StatusCode::OK);
            let tenant_scope = scope(tenant);
            let destinations = store
                .list_destinations(tenant_scope)
                .await
                .expect("destinations");
            assert_eq!(destinations.len(), 1);
            let evidence = store
                .list_identity_evidence(tenant_scope, &destinations[0].id)
                .await
                .expect("evidence");
            assert_eq!(evidence.len(), 1);
            let routes = store.list_all_routes(tenant_scope).await.expect("routes");
            let observations = store
                .list_route_observations(tenant_scope, &routes[0].id, 10)
                .await
                .expect("observations");
            assert_eq!(observations.len(), 1, "a sync retry is idempotent");

            if index == 1 {
                use piqae_domain::{ContentKind, ContentSource, Job, JobId, JobOptions, JobState};
                use piqae_storage_postgres::destination_topology::{
                    DeliveryAttemptState, NewDeliveryAttempt,
                };

                let job_id = JobId::new();
                let uncertain_job = Job {
                    id: job_id,
                    workspace_id,
                    environment_id,
                    printer_id,
                    title: "Uncertain fenced handoff".into(),
                    source: None,
                    content_kind: ContentKind::Pdf,
                    content: ContentSource::Base64 {
                        data: "JVBERi0=".into(),
                    },
                    options: JobOptions::default(),
                    metadata: BTreeMap::from([
                        ("piqae.destination_id".into(), destinations[0].id.clone()),
                        ("piqae.route_id".into(), routes[0].id.clone()),
                    ]),
                    deliveries: 1,
                    state: JobState::WaitingForAgent,
                    created_at: now,
                    expires_at: now + TimeDelta::hours(1),
                    delivery_uncertain_since: None,
                };
                store
                    .create_job(&uncertain_job, agent_id, None, b"uncertain-fence-test")
                    .await
                    .expect("uncertain job fixture");
                let started = store
                    .begin_delivery_attempt(
                        tenant_scope,
                        NewDeliveryAttempt {
                            attempt_id: "attempt_control_uncertain",
                            reservation_id: "00000000-0000-0000-0000-000000000001",
                            job_id: &job_id.to_string(),
                            destination_id: &destinations[0].id,
                            route_id: &routes[0].id,
                            lease_until: now + TimeDelta::minutes(2),
                        },
                    )
                    .await
                    .expect("begin fenced uncertain attempt");
                for next in [
                    DeliveryAttemptState::AcceptedByNode,
                    DeliveryAttemptState::QueuedLocal,
                    DeliveryAttemptState::HandingToSpooler,
                    DeliveryAttemptState::DeliveryUncertain,
                ] {
                    store
                        .transition_delivery_attempt(
                            tenant_scope,
                            "attempt_control_uncertain",
                            started.attempt.generation,
                            &started.fencing_token,
                            next,
                        )
                        .await
                        .expect("advance uncertain attempt");
                }
                sqlx::query("UPDATE jobs SET state='delivery_uncertain',delivery_uncertain_since=now() WHERE workspace_id=$1 AND environment_id=$2 AND id=$3")
                    .bind(workspace_id.to_string()).bind(environment_id.to_string()).bind(job_id.to_string()).execute(&pool).await.expect("mark uncertain job");
                let resolution_path = format!("/v1/jobs/{job_id}/resolve-uncertain");
                let resolution_body = serde_json::to_vec(&serde_json::json!({
                    "resolution": "acknowledge_missing",
                    "note": "Operator checked the output tray and accepts the missing document"
                }))
                .expect("resolution JSON");
                let pending = application
                    .clone()
                    .oneshot(
                        Request::builder()
                            .method("POST")
                            .uri(&resolution_path)
                            .header("authorization", format!("Bearer {token}"))
                            .header("content-type", "application/json")
                            .header("idempotency-key", "resolve-control-uncertain-1")
                            .body(Body::from(resolution_body.clone()))
                            .expect("resolution request"),
                    )
                    .await
                    .expect("pending resolution response");
                assert_eq!(pending.status(), StatusCode::ACCEPTED);
                let stored_command: serde_json::Value = sqlx::query_scalar(
                    "SELECT command FROM agent_commands WHERE workspace_id=$1 AND environment_id=$2 AND agent_id=$3 ORDER BY cursor DESC LIMIT 1",
                )
                .bind(workspace_id.to_string())
                .bind(environment_id.to_string())
                .bind(agent_id.to_string())
                .fetch_one(&pool)
                .await
                .expect("stored uncertainty command");
                serde_json::from_value::<piqae_protocol::agent::AgentCommand>(
                    stored_command.clone(),
                )
                .unwrap_or_else(|error| {
                    panic!("uncertainty command {stored_command} matches node protocol: {error}")
                });

                let command_response = application
                    .clone()
                    .oneshot(signed_agent_request(
                        agent_id,
                        &signing_key,
                        serde_json::to_vec(&request).expect("command sync JSON"),
                    ))
                    .await
                    .expect("command sync response");
                let command_status = command_response.status();
                let command_body = command_response
                    .into_body()
                    .collect()
                    .await
                    .expect("command sync body")
                    .to_bytes();
                assert_eq!(
                    command_status,
                    StatusCode::OK,
                    "command sync failed: {}",
                    String::from_utf8_lossy(&command_body)
                );
                let command_sync: piqae_protocol::agent::AgentSyncResponse =
                    serde_json::from_slice(&command_body).expect("command sync JSON");
                assert!(matches!(
                    command_sync.commands.as_slice(),
                    [piqae_protocol::agent::AgentCommand::ResolveAmbiguousHandoff { .. }]
                ));
                let mut acknowledged = request.clone();
                acknowledged.acknowledged_command_cursor = command_sync.command_cursor;
                let acknowledged_response = application
                    .clone()
                    .oneshot(signed_agent_request(
                        agent_id,
                        &signing_key,
                        serde_json::to_vec(&acknowledged).expect("acknowledgement sync JSON"),
                    ))
                    .await
                    .expect("acknowledgement sync response");
                assert_eq!(acknowledged_response.status(), StatusCode::OK);
                let resolved = application
                    .clone()
                    .oneshot(
                        Request::builder()
                            .method("POST")
                            .uri(&resolution_path)
                            .header("authorization", format!("Bearer {token}"))
                            .header("content-type", "application/json")
                            .header("idempotency-key", "resolve-control-uncertain-1")
                            .body(Body::from(resolution_body))
                            .expect("resolved replay request"),
                    )
                    .await
                    .expect("resolved replay response");
                assert_eq!(resolved.status(), StatusCode::OK);

                let crash_resolution =
                    piqae_storage_postgres::destination_topology::DeliveryUncertaintyResolution {
                        id: "dur_crash_reprint".into(),
                        job_id: job_id.to_string(),
                        attempt_id: "attempt_control_uncertain".into(),
                        destination_id: destinations[0].id.clone(),
                        resolution: "reprint_authorized".into(),
                        note: Some("simulate crash after registered replacement".into()),
                        actor_id: "operator".into(),
                        request_id: "resolve-control-reprint-crash".into(),
                        created_at: Utc::now(),
                    };
                let mut crash_replacement = uncertain_job.clone();
                crash_replacement.id = JobId::new();
                crash_replacement.state = JobState::Registered;
                crash_replacement.metadata =
                    replacement_metadata(uncertain_job.metadata.clone(), &crash_resolution);
                let reprint_digest = Sha256::digest(crash_resolution.request_id.as_bytes());
                let reprint_key =
                    format!("uncertainty-reprint-{}", &hex::encode(reprint_digest)[..32]);
                let reprint_request = serde_json::to_vec(&serde_json::json!({
                    "original_job_id": crash_resolution.job_id,
                    "resolution_request_id": crash_resolution.request_id,
                }))
                .expect("reprint request JSON");
                store
                    .create_cloud_job(
                        &crash_replacement,
                        agent_id,
                        Some(&reprint_key),
                        &reprint_request,
                        false,
                    )
                    .await
                    .expect("crash-left registered reprint");
                let recovered_reprint =
                    create_authorized_reprint(&state, tenant, &crash_resolution)
                        .await
                        .expect("recover registered reprint")
                        .expect("authorized replacement");
                assert_eq!(recovered_reprint.id, crash_replacement.id);
                assert_eq!(recovered_reprint.state, JobState::WaitingForAgent);
                assert!(
                    !recovered_reprint
                        .metadata
                        .contains_key("piqae.delivery_resolution")
                );
                sqlx::query("UPDATE jobs SET state='completed_reported' WHERE workspace_id=$1 AND environment_id=$2 AND id=$3")
                    .bind(workspace_id.to_string()).bind(environment_id.to_string()).bind(recovered_reprint.id.to_string()).execute(&pool).await.expect("retire recovered reprint fixture");

                // A signed telemetry replay repairs a crash between the job
                // and attempt projections, while a stale report after a final
                // state cannot rewrite the completed attempt.
                let lifecycle_job_id = JobId::new();
                let lifecycle_job = Job {
                    id: lifecycle_job_id,
                    title: "Replay-safe post-spooler lifecycle".into(),
                    metadata: BTreeMap::from([
                        ("piqae.destination_id".into(), destinations[0].id.clone()),
                        ("piqae.route_id".into(), routes[0].id.clone()),
                    ]),
                    ..uncertain_job.clone()
                };
                store
                    .create_job(&lifecycle_job, agent_id, None, b"lifecycle-replay-test")
                    .await
                    .expect("lifecycle job fixture");
                sqlx::query("UPDATE jobs SET state='accepted_by_spooler' WHERE workspace_id=$1 AND environment_id=$2 AND id=$3")
                    .bind(workspace_id.to_string()).bind(environment_id.to_string()).bind(lifecycle_job_id.to_string()).execute(&pool).await.expect("seed accepted lifecycle job");
                let lifecycle = store
                    .begin_delivery_attempt(
                        tenant_scope,
                        NewDeliveryAttempt {
                            attempt_id: "attempt_control_lifecycle",
                            reservation_id: "00000000-0000-0000-0000-000000000002",
                            job_id: &lifecycle_job_id.to_string(),
                            destination_id: &destinations[0].id,
                            route_id: &routes[0].id,
                            lease_until: now + TimeDelta::minutes(2),
                        },
                    )
                    .await
                    .expect("begin lifecycle attempt");
                for next in [
                    DeliveryAttemptState::AcceptedByNode,
                    DeliveryAttemptState::QueuedLocal,
                    DeliveryAttemptState::HandingToSpooler,
                    DeliveryAttemptState::AcceptedBySpooler,
                ] {
                    store
                        .transition_delivery_attempt(
                            tenant_scope,
                            "attempt_control_lifecycle",
                            lifecycle.attempt.generation,
                            &lifecycle.fencing_token,
                            next,
                        )
                        .await
                        .expect("advance lifecycle attempt");
                }
                let mut last_sync = None;
                for (sequence, state_name) in [
                    JobState::Spooling,
                    JobState::Printing,
                    JobState::CompletedReported,
                ]
                .into_iter()
                .enumerate()
                {
                    let event = piqae_domain::JobEvent {
                        id: piqae_domain::EventId::new(),
                        job_id: lifecycle_job_id,
                        sequence: u64::try_from(sequence + 1).expect("event sequence"),
                        state: state_name,
                        reason: None,
                        message: None,
                        agent_id: Some(agent_id),
                        native_job_id: None,
                        occurred_at: Utc::now(),
                    };
                    let mut lifecycle_sync = request.clone();
                    lifecycle_sync.events = vec![event];
                    let body = serde_json::to_vec(&lifecycle_sync).expect("lifecycle sync JSON");
                    let response = application
                        .clone()
                        .oneshot(signed_agent_request(agent_id, &signing_key, body.clone()))
                        .await
                        .expect("lifecycle sync response");
                    let status = response.status();
                    let response_body = response
                        .into_body()
                        .collect()
                        .await
                        .expect("lifecycle response body")
                        .to_bytes();
                    assert_eq!(
                        status,
                        StatusCode::OK,
                        "lifecycle sync failed: {}",
                        String::from_utf8_lossy(&response_body)
                    );
                    last_sync = Some(body);
                }
                let replay = application
                    .clone()
                    .oneshot(signed_agent_request(
                        agent_id,
                        &signing_key,
                        last_sync.expect("completed sync body"),
                    ))
                    .await
                    .expect("completed replay response");
                assert_eq!(replay.status(), StatusCode::OK);
                let attempt = store
                    .get_latest_delivery_attempt(tenant_scope, &lifecycle_job_id.to_string())
                    .await
                    .expect("completed lifecycle attempt");
                assert_eq!(attempt.state, DeliveryAttemptState::CompletedReported);

                let mut stale_sync = request.clone();
                stale_sync.events = vec![piqae_domain::JobEvent {
                    id: piqae_domain::EventId::new(),
                    job_id: lifecycle_job_id,
                    sequence: 99,
                    state: JobState::Printing,
                    reason: None,
                    message: None,
                    agent_id: Some(agent_id),
                    native_job_id: None,
                    occurred_at: Utc::now(),
                }];
                let stale_response = application
                    .clone()
                    .oneshot(signed_agent_request(
                        agent_id,
                        &signing_key,
                        serde_json::to_vec(&stale_sync).expect("stale sync JSON"),
                    ))
                    .await
                    .expect("stale sync response");
                assert_ne!(stale_response.status(), StatusCode::OK);
                assert_eq!(
                    store
                        .get_latest_delivery_attempt(tenant_scope, &lifecycle_job_id.to_string(),)
                        .await
                        .expect("attempt after stale report")
                        .state,
                    DeliveryAttemptState::CompletedReported
                );
            }

            let mut conflicting_retry = request.clone();
            conflicting_retry.route_observations[0].state = PrinterState::Busy;
            let error = project_agent_topology(&state, tenant, &conflicting_retry)
                .await
                .expect_err("a reused route sequence cannot change its payload");
            assert_eq!(
                axum::response::IntoResponse::into_response(error).status(),
                axum::http::StatusCode::CONFLICT
            );
            destination_ids.push(destinations[0].id.clone());
            stored_digests.push(evidence[0].value_digest.clone());
        }
        assert_ne!(destination_ids[0], destination_ids[1]);
        assert_ne!(stored_digests[0], stored_digests[1]);
        let cross_tenant = application
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/physical-destinations/{}", destination_ids[1]))
                    .header("authorization", format!("Bearer {}", tenant_tokens[0]))
                    .body(Body::empty())
                    .expect("cross-tenant destination request"),
            )
            .await
            .expect("cross-tenant destination response");
        assert_eq!(cross_tenant.status(), StatusCode::NOT_FOUND);

        pool.close().await;
        sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
            .execute(&admin)
            .await
            .expect("drop disposable schema");
    }
}
