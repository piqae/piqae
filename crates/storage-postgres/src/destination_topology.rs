//! Tenant-isolated physical destination, route, observation, and fenced-attempt storage.
//!
//! A destination is a tenant's view of a real printer. A route is one installed
//! operating-system queue on one node that can reach it. Local device evidence
//! can be projected into several tenants, but this repository never performs a
//! cross-tenant lookup or mutation.

#![allow(
    clippy::cognitive_complexity,
    clippy::significant_drop_tightening,
    clippy::suspicious_operation_groupings,
    reason = "short in-memory critical sections mirror transactional repository operations"
)]

use crate::{PostgresStore, StorageError};
use async_trait::async_trait;
use base64::Engine as _;
use chrono::{DateTime, Utc};
use piqae_domain::{EnvironmentId, JobId, WorkspaceId};
use piqae_protocol::agent::{AgentCommand, AmbiguousHandoffResolution};
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::Row;
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, RwLock},
};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TenantScope {
    pub workspace_id: WorkspaceId,
    pub environment_id: EnvironmentId,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentityConfidence {
    Unknown,
    Possible,
    High,
    Verified,
    Conflict,
}

impl IdentityConfidence {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Possible => "possible",
            Self::High => "high",
            Self::Verified => "verified",
            Self::Conflict => "conflict",
        }
    }

    fn parse(value: &str) -> Result<Self, StorageError> {
        match value {
            "unknown" => Ok(Self::Unknown),
            "possible" => Ok(Self::Possible),
            "high" => Ok(Self::High),
            "verified" => Ok(Self::Verified),
            "conflict" => Ok(Self::Conflict),
            other => Err(StorageError::InvalidData(format!(
                "unknown identity confidence {other}"
            ))),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StoredPhysicalDestination {
    pub id: String,
    pub name: String,
    pub identity_confidence: IdentityConfidence,
    pub state: String,
    pub scheduling_authority_id: Option<String>,
    pub identity_revision: u64,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StoredPrinterRoute {
    pub id: String,
    pub destination_id: String,
    pub printer_id: String,
    pub agent_id: String,
    pub native_queue_id: String,
    pub local_route_key: Option<String>,
    pub state: String,
    pub role: String,
    pub priority: i32,
    pub enabled: bool,
    pub capability_revision: u64,
    pub profile_revision: u64,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct IdentityEvidence {
    pub id: String,
    pub destination_id: String,
    pub route_id: String,
    pub kind: String,
    /// A one-way digest. Raw serials, addresses, and device identifiers are not stored here.
    pub value_digest: String,
    pub strength: String,
    pub conflicts: bool,
    pub observed_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub metadata: serde_json::Value,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentityDecisionKind {
    Merge,
    Split,
    Confirm,
    RejectMatch,
    Reverse,
}

impl IdentityDecisionKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Split => "split",
            Self::Confirm => "confirm",
            Self::RejectMatch => "reject_match",
            Self::Reverse => "reverse",
        }
    }

    fn parse(value: &str) -> Result<Self, StorageError> {
        match value {
            "merge" => Ok(Self::Merge),
            "split" => Ok(Self::Split),
            "confirm" => Ok(Self::Confirm),
            "reject_match" => Ok(Self::RejectMatch),
            "reverse" => Ok(Self::Reverse),
            other => Err(StorageError::InvalidData(format!(
                "unknown identity decision kind {other}"
            ))),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct IdentityDecision {
    pub id: String,
    pub kind: IdentityDecisionKind,
    pub destination_id: String,
    pub related_destination_ids: Vec<String>,
    pub route_ids: Vec<String>,
    pub evidence_ids: Vec<String>,
    pub actor_kind: String,
    pub actor_id: Option<String>,
    pub reason: String,
    pub reverses_decision_id: Option<String>,
    pub request_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RouteObservation {
    pub id: String,
    pub route_id: String,
    pub sequence: u64,
    pub printer_state: String,
    pub accepting_jobs: Option<bool>,
    pub state_reasons: Vec<String>,
    pub total_jobs: u32,
    pub connector_jobs: u32,
    /// Work not owned by the observing connector. No titles, users, or tenant IDs are exposed.
    pub other_piqae_or_external_jobs: u32,
    pub unknown_jobs: u32,
    pub active_jobs: u32,
    pub held_jobs: u32,
    pub estimated_busy_seconds: Option<u32>,
    pub privacy_level: String,
    pub stock_state: serde_json::Value,
    pub observed_at: DateTime<Utc>,
    pub fresh_until: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectionAcknowledgement {
    pub agent_id: String,
    pub route_id: String,
    pub inventory_revision: u64,
    pub capability_revision: u64,
    pub status: String,
    pub error_code: Option<String>,
    pub observed_at: DateTime<Utc>,
    pub acknowledged_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SchedulingAuthority {
    pub id: String,
    pub kind: String,
    pub authority_key: String,
    pub display_name: String,
    pub active: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SiteCoordinatorMembership {
    pub authority_id: String,
    pub agent_id: String,
    pub site_id: String,
    pub state: String,
    pub last_seen_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryAttemptState {
    RouteLeased,
    AcceptedByNode,
    QueuedLocal,
    HandingToSpooler,
    AcceptedBySpooler,
    PrintingReported,
    CompletedReported,
    Cancelled,
    Failed,
    DeliveryUncertain,
    Superseded,
}

impl DeliveryAttemptState {
    const fn as_str(self) -> &'static str {
        match self {
            Self::RouteLeased => "route_leased",
            Self::AcceptedByNode => "accepted_by_node",
            Self::QueuedLocal => "queued_local",
            Self::HandingToSpooler => "handing_to_spooler",
            Self::AcceptedBySpooler => "accepted_by_spooler",
            Self::PrintingReported => "printing_reported",
            Self::CompletedReported => "completed_reported",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
            Self::DeliveryUncertain => "delivery_uncertain",
            Self::Superseded => "superseded",
        }
    }

    fn parse(value: &str) -> Result<Self, StorageError> {
        match value {
            "route_leased" => Ok(Self::RouteLeased),
            "accepted_by_node" => Ok(Self::AcceptedByNode),
            "queued_local" => Ok(Self::QueuedLocal),
            "handing_to_spooler" => Ok(Self::HandingToSpooler),
            "accepted_by_spooler" => Ok(Self::AcceptedBySpooler),
            "printing_reported" => Ok(Self::PrintingReported),
            "completed_reported" => Ok(Self::CompletedReported),
            "cancelled" => Ok(Self::Cancelled),
            "failed" => Ok(Self::Failed),
            "delivery_uncertain" => Ok(Self::DeliveryUncertain),
            "superseded" => Ok(Self::Superseded),
            other => Err(StorageError::InvalidData(format!(
                "unknown delivery attempt state {other}"
            ))),
        }
    }

    #[must_use]
    pub const fn is_final(self) -> bool {
        matches!(
            self,
            Self::CompletedReported
                | Self::Cancelled
                | Self::Failed
                | Self::DeliveryUncertain
                | Self::Superseded
        )
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DeliveryAttempt {
    pub id: String,
    pub job_id: String,
    pub destination_id: String,
    pub route_id: String,
    pub generation: u64,
    pub state: DeliveryAttemptState,
    pub lease_until: DateTime<Utc>,
    pub accepted_at: Option<DateTime<Utc>>,
    pub handoff_started_at: Option<DateTime<Utc>>,
    pub spooler_accepted_at: Option<DateTime<Utc>>,
    pub final_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RouteReservation {
    pub id: String,
    pub route_id: String,
    pub destination_id: String,
    pub job_id: String,
    pub attempt_id: String,
    pub generation: u64,
    pub state: String,
    pub lease_until: DateTime<Utc>,
    pub released_at: Option<DateTime<Utc>>,
    pub acquired_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StartedDeliveryAttempt {
    pub attempt: DeliveryAttempt,
    pub reservation: RouteReservation,
    /// Returned once to the scheduler/node boundary. Only its SHA-256 digest is stored.
    pub fencing_token: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DeliveryUncertaintyResolution {
    pub id: String,
    pub job_id: String,
    pub attempt_id: String,
    pub destination_id: String,
    pub resolution: String,
    pub note: Option<String>,
    pub actor_id: String,
    pub request_id: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PendingDeliveryUncertaintyResolution {
    pub request_id: String,
    pub job_id: String,
    pub attempt_id: String,
    pub destination_id: String,
    pub route_id: String,
    pub agent_id: String,
    pub reservation_id: String,
    pub generation: u64,
    pub resolution: String,
    pub note: Option<String>,
    pub actor_id: String,
    pub agent_command_cursor: u64,
    pub command: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub finalized_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug)]
pub struct NewDeliveryAttempt<'a> {
    pub attempt_id: &'a str,
    pub reservation_id: &'a str,
    pub job_id: &'a str,
    pub destination_id: &'a str,
    pub route_id: &'a str,
    pub lease_until: DateTime<Utc>,
}

#[async_trait]
pub trait DestinationTopologyRepository: Send + Sync {
    async fn upsert_scheduling_authority(
        &self,
        scope: TenantScope,
        authority: &SchedulingAuthority,
    ) -> Result<(), StorageError>;
    async fn upsert_destination(
        &self,
        scope: TenantScope,
        destination: &StoredPhysicalDestination,
    ) -> Result<(), StorageError>;
    async fn get_destination(
        &self,
        scope: TenantScope,
        id: &str,
    ) -> Result<StoredPhysicalDestination, StorageError>;
    async fn list_destinations(
        &self,
        scope: TenantScope,
    ) -> Result<Vec<StoredPhysicalDestination>, StorageError>;
    async fn upsert_route(
        &self,
        scope: TenantScope,
        route: &StoredPrinterRoute,
    ) -> Result<(), StorageError>;
    async fn get_route(
        &self,
        scope: TenantScope,
        route_id: &str,
    ) -> Result<StoredPrinterRoute, StorageError>;
    async fn get_route_by_local_key(
        &self,
        scope: TenantScope,
        agent_id: &str,
        local_route_key: &str,
    ) -> Result<StoredPrinterRoute, StorageError>;
    async fn list_all_routes(
        &self,
        scope: TenantScope,
    ) -> Result<Vec<StoredPrinterRoute>, StorageError>;
    async fn list_routes(
        &self,
        scope: TenantScope,
        destination_id: &str,
    ) -> Result<Vec<StoredPrinterRoute>, StorageError>;
    async fn record_identity_evidence(
        &self,
        scope: TenantScope,
        evidence: &IdentityEvidence,
    ) -> Result<(), StorageError>;
    async fn list_identity_evidence(
        &self,
        scope: TenantScope,
        destination_id: &str,
    ) -> Result<Vec<IdentityEvidence>, StorageError>;
    async fn record_identity_decision(
        &self,
        scope: TenantScope,
        decision: &IdentityDecision,
    ) -> Result<(), StorageError>;
    async fn reverse_identity_decision(
        &self,
        scope: TenantScope,
        reversal: &IdentityDecision,
    ) -> Result<(), StorageError>;
    async fn list_identity_decisions(
        &self,
        scope: TenantScope,
        destination_id: &str,
    ) -> Result<Vec<IdentityDecision>, StorageError>;
    async fn record_route_observation(
        &self,
        scope: TenantScope,
        observation: &RouteObservation,
    ) -> Result<(), StorageError>;
    async fn latest_route_observation(
        &self,
        scope: TenantScope,
        route_id: &str,
    ) -> Result<RouteObservation, StorageError>;
    async fn list_route_observations(
        &self,
        scope: TenantScope,
        route_id: &str,
        limit: u32,
    ) -> Result<Vec<RouteObservation>, StorageError>;
    async fn acknowledge_projection(
        &self,
        scope: TenantScope,
        acknowledgement: &ProjectionAcknowledgement,
    ) -> Result<(), StorageError>;
    async fn get_projection_acknowledgement(
        &self,
        scope: TenantScope,
        agent_id: &str,
        route_id: &str,
    ) -> Result<ProjectionAcknowledgement, StorageError>;
    async fn upsert_site_membership(
        &self,
        scope: TenantScope,
        membership: &SiteCoordinatorMembership,
    ) -> Result<(), StorageError>;
    async fn begin_delivery_attempt(
        &self,
        scope: TenantScope,
        request: NewDeliveryAttempt<'_>,
    ) -> Result<StartedDeliveryAttempt, StorageError>;
    async fn transition_delivery_attempt(
        &self,
        scope: TenantScope,
        attempt_id: &str,
        generation: u64,
        fencing_token: &str,
        next: DeliveryAttemptState,
    ) -> Result<DeliveryAttempt, StorageError>;
    /// Applies a trusted node event after durable spooler acceptance. The
    /// caller has already authenticated the agent; no scheduler fencing token
    /// is available on this later event path.
    async fn transition_post_spooler_attempt(
        &self,
        scope: TenantScope,
        job_id: &str,
        agent_id: &str,
        route_id: &str,
        next: DeliveryAttemptState,
    ) -> Result<DeliveryAttempt, StorageError>;
    /// Converts a timed-out post-spooler attempt to uncertain delivery and
    /// marks its physical destination for operator attention atomically.
    async fn mark_post_spooler_attempt_uncertain(
        &self,
        scope: TenantScope,
        job_id: &str,
    ) -> Result<DeliveryAttempt, StorageError>;
    async fn renew_delivery_attempt(
        &self,
        scope: TenantScope,
        reservation_id: &str,
        generation: u64,
        fencing_token: &str,
        lease_until: DateTime<Utc>,
    ) -> Result<StartedDeliveryAttempt, StorageError>;
    async fn enqueue_delivery_uncertainty_resolution(
        &self,
        scope: TenantScope,
        job_id: &str,
        resolution: &str,
        note: Option<&str>,
        actor_id: &str,
        request_id: &str,
    ) -> Result<PendingDeliveryUncertaintyResolution, StorageError>;
    async fn finalize_delivery_uncertainty_resolution(
        &self,
        scope: TenantScope,
        request_id: &str,
    ) -> Result<Option<DeliveryUncertaintyResolution>, StorageError>;
    async fn finalize_acknowledged_uncertainty_resolutions(
        &self,
        scope: TenantScope,
        agent_id: &str,
        limit: u32,
    ) -> Result<Vec<DeliveryUncertaintyResolution>, StorageError>;
    /// Memory adapter hook corresponding to durable agent-command cursor ACK.
    /// PostgreSQL rejects this direct hook; its existing command-sync path is
    /// the sole authority for acknowledgement.
    async fn acknowledge_uncertainty_resolution_command(
        &self,
        scope: TenantScope,
        agent_command_cursor: u64,
    ) -> Result<(), StorageError>;
    async fn has_unresolved_destination_uncertainty(
        &self,
        scope: TenantScope,
        destination_id: &str,
    ) -> Result<bool, StorageError>;
    async fn recompute_destination_attention(
        &self,
        scope: TenantScope,
        destination_id: &str,
    ) -> Result<StoredPhysicalDestination, StorageError>;
    async fn list_delivery_attempts(
        &self,
        scope: TenantScope,
        job_id: &str,
    ) -> Result<Vec<DeliveryAttempt>, StorageError>;
    async fn get_latest_delivery_attempt(
        &self,
        scope: TenantScope,
        job_id: &str,
    ) -> Result<DeliveryAttempt, StorageError>;
    async fn get_delivery_attempt_by_reservation(
        &self,
        scope: TenantScope,
        reservation_id: &str,
    ) -> Result<DeliveryAttempt, StorageError>;
    async fn list_route_reservations(
        &self,
        scope: TenantScope,
        limit: u32,
    ) -> Result<Vec<RouteReservation>, StorageError>;
}

fn token_digest(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

fn reservation_write_error(error: sqlx::Error) -> StorageError {
    if error
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
        .is_some_and(|code| code == "23505")
    {
        StorageError::ConcurrentStateChange
    } else {
        StorageError::Database(error)
    }
}

fn new_fencing_token() -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

const fn valid_attempt_transition(from: DeliveryAttemptState, to: DeliveryAttemptState) -> bool {
    match from {
        DeliveryAttemptState::RouteLeased => matches!(
            to,
            DeliveryAttemptState::AcceptedByNode
                | DeliveryAttemptState::Cancelled
                | DeliveryAttemptState::Failed
                | DeliveryAttemptState::Superseded
        ),
        DeliveryAttemptState::AcceptedByNode => matches!(
            to,
            DeliveryAttemptState::QueuedLocal
                | DeliveryAttemptState::Cancelled
                | DeliveryAttemptState::Failed
        ),
        DeliveryAttemptState::QueuedLocal => matches!(
            to,
            DeliveryAttemptState::HandingToSpooler
                | DeliveryAttemptState::Cancelled
                | DeliveryAttemptState::Failed
        ),
        DeliveryAttemptState::HandingToSpooler => matches!(
            to,
            DeliveryAttemptState::AcceptedBySpooler
                | DeliveryAttemptState::Failed
                | DeliveryAttemptState::DeliveryUncertain
        ),
        DeliveryAttemptState::AcceptedBySpooler => matches!(
            to,
            DeliveryAttemptState::PrintingReported
                | DeliveryAttemptState::CompletedReported
                | DeliveryAttemptState::Failed
                | DeliveryAttemptState::DeliveryUncertain
        ),
        DeliveryAttemptState::PrintingReported => matches!(
            to,
            DeliveryAttemptState::CompletedReported
                | DeliveryAttemptState::Failed
                | DeliveryAttemptState::DeliveryUncertain
        ),
        DeliveryAttemptState::CompletedReported
        | DeliveryAttemptState::Cancelled
        | DeliveryAttemptState::Failed
        | DeliveryAttemptState::DeliveryUncertain
        | DeliveryAttemptState::Superseded => false,
    }
}

fn i64_to_u64(value: i64, field: &str) -> Result<u64, StorageError> {
    u64::try_from(value).map_err(|_| StorageError::InvalidData(format!("negative {field} stored")))
}

fn i32_to_u32(value: i32, field: &str) -> Result<u32, StorageError> {
    u32::try_from(value).map_err(|_| StorageError::InvalidData(format!("negative {field} stored")))
}

fn map_destination(row: &sqlx::postgres::PgRow) -> Result<StoredPhysicalDestination, StorageError> {
    Ok(StoredPhysicalDestination {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        identity_confidence: IdentityConfidence::parse(row.try_get("identity_confidence")?)?,
        state: row.try_get("state")?,
        scheduling_authority_id: row.try_get("scheduling_authority_id")?,
        identity_revision: i64_to_u64(row.try_get("identity_revision")?, "identity_revision")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn map_route(row: &sqlx::postgres::PgRow) -> Result<StoredPrinterRoute, StorageError> {
    Ok(StoredPrinterRoute {
        id: row.try_get("id")?,
        destination_id: row.try_get("destination_id")?,
        printer_id: row.try_get("printer_id")?,
        agent_id: row.try_get("agent_id")?,
        native_queue_id: row.try_get("native_queue_id")?,
        local_route_key: row.try_get("local_route_key")?,
        state: row.try_get("state")?,
        role: row.try_get("role")?,
        priority: row.try_get("priority")?,
        enabled: row.try_get("enabled")?,
        capability_revision: i64_to_u64(
            row.try_get("capability_revision")?,
            "capability_revision",
        )?,
        profile_revision: i64_to_u64(row.try_get("profile_revision")?, "profile_revision")?,
        last_seen_at: row.try_get("last_seen_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn map_evidence(row: &sqlx::postgres::PgRow) -> Result<IdentityEvidence, StorageError> {
    Ok(IdentityEvidence {
        id: row.try_get("id")?,
        destination_id: row.try_get("destination_id")?,
        route_id: row.try_get("route_id")?,
        kind: row.try_get("kind")?,
        value_digest: row.try_get("value_digest")?,
        strength: row.try_get("strength")?,
        conflicts: row.try_get("conflicts")?,
        observed_at: row.try_get("observed_at")?,
        expires_at: row.try_get("expires_at")?,
        metadata: row.try_get("metadata")?,
    })
}

fn map_decision(row: &sqlx::postgres::PgRow) -> Result<IdentityDecision, StorageError> {
    Ok(IdentityDecision {
        id: row.try_get("id")?,
        kind: IdentityDecisionKind::parse(row.try_get("kind")?)?,
        destination_id: row.try_get("destination_id")?,
        related_destination_ids: serde_json::from_value(row.try_get("related_destination_ids")?)?,
        route_ids: serde_json::from_value(row.try_get("route_ids")?)?,
        evidence_ids: serde_json::from_value(row.try_get("evidence_ids")?)?,
        actor_kind: row.try_get("actor_kind")?,
        actor_id: row.try_get("actor_id")?,
        reason: row.try_get("reason")?,
        reverses_decision_id: row.try_get("reverses_decision_id")?,
        request_id: row.try_get("request_id")?,
        created_at: row.try_get("created_at")?,
    })
}

fn map_observation(row: &sqlx::postgres::PgRow) -> Result<RouteObservation, StorageError> {
    Ok(RouteObservation {
        id: row.try_get("id")?,
        route_id: row.try_get("route_id")?,
        sequence: i64_to_u64(row.try_get("sequence")?, "observation sequence")?,
        printer_state: row.try_get("printer_state")?,
        accepting_jobs: row.try_get("accepting_jobs")?,
        state_reasons: row.try_get("state_reasons")?,
        total_jobs: i32_to_u32(row.try_get("total_jobs")?, "total_jobs")?,
        connector_jobs: i32_to_u32(row.try_get("connector_jobs")?, "connector_jobs")?,
        other_piqae_or_external_jobs: i32_to_u32(
            row.try_get("other_piqae_or_external_jobs")?,
            "other_piqae_or_external_jobs",
        )?,
        unknown_jobs: i32_to_u32(row.try_get("unknown_jobs")?, "unknown_jobs")?,
        active_jobs: i32_to_u32(row.try_get("active_jobs")?, "active_jobs")?,
        held_jobs: i32_to_u32(row.try_get("held_jobs")?, "held_jobs")?,
        estimated_busy_seconds: row
            .try_get::<Option<i32>, _>("estimated_busy_seconds")?
            .map(|value| i32_to_u32(value, "estimated_busy_seconds"))
            .transpose()?,
        privacy_level: row.try_get("privacy_level")?,
        stock_state: row.try_get("stock_state")?,
        observed_at: row.try_get("observed_at")?,
        fresh_until: row.try_get("fresh_until")?,
    })
}

fn map_projection_acknowledgement(
    row: &sqlx::postgres::PgRow,
) -> Result<ProjectionAcknowledgement, StorageError> {
    Ok(ProjectionAcknowledgement {
        agent_id: row.try_get("agent_id")?,
        route_id: row.try_get("route_id")?,
        inventory_revision: i64_to_u64(row.try_get("inventory_revision")?, "inventory_revision")?,
        capability_revision: i64_to_u64(
            row.try_get("capability_revision")?,
            "capability_revision",
        )?,
        status: row.try_get("status")?,
        error_code: row.try_get("error_code")?,
        observed_at: row.try_get("observed_at")?,
        acknowledged_at: row.try_get("acknowledged_at")?,
    })
}

fn map_attempt(row: &sqlx::postgres::PgRow) -> Result<DeliveryAttempt, StorageError> {
    Ok(DeliveryAttempt {
        id: row.try_get("id")?,
        job_id: row.try_get("job_id")?,
        destination_id: row.try_get("destination_id")?,
        route_id: row.try_get("route_id")?,
        generation: i64_to_u64(row.try_get("generation")?, "generation")?,
        state: DeliveryAttemptState::parse(row.try_get("state")?)?,
        lease_until: row.try_get("lease_until")?,
        accepted_at: row.try_get("accepted_at")?,
        handoff_started_at: row.try_get("handoff_started_at")?,
        spooler_accepted_at: row.try_get("spooler_accepted_at")?,
        final_at: row.try_get("final_at")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn map_reservation(row: &sqlx::postgres::PgRow) -> Result<RouteReservation, StorageError> {
    Ok(RouteReservation {
        id: row.try_get("id")?,
        route_id: row.try_get("route_id")?,
        destination_id: row.try_get("destination_id")?,
        job_id: row.try_get("job_id")?,
        attempt_id: row.try_get("attempt_id")?,
        generation: i64_to_u64(row.try_get("generation")?, "reservation generation")?,
        state: row.try_get("state")?,
        lease_until: row.try_get("lease_until")?,
        released_at: row.try_get("released_at")?,
        acquired_at: row.try_get("created_at")?,
    })
}

fn map_uncertainty_resolution(
    row: &sqlx::postgres::PgRow,
) -> Result<DeliveryUncertaintyResolution, StorageError> {
    Ok(DeliveryUncertaintyResolution {
        id: row.try_get("id")?,
        job_id: row.try_get("job_id")?,
        attempt_id: row.try_get("attempt_id")?,
        destination_id: row.try_get("destination_id")?,
        resolution: row.try_get("resolution")?,
        note: row.try_get("note")?,
        actor_id: row.try_get("actor_id")?,
        request_id: row.try_get("request_id")?,
        created_at: row.try_get("created_at")?,
    })
}

fn pending_uncertainty_resolution_from_row(
    row: &sqlx::postgres::PgRow,
) -> Result<PendingDeliveryUncertaintyResolution, StorageError> {
    let generation = i64_to_u64(row.try_get("generation")?, "generation")?;
    let cursor = i64_to_u64(row.try_get("agent_command_cursor")?, "agent command cursor")?;
    Ok(PendingDeliveryUncertaintyResolution {
        request_id: row.try_get("request_id")?,
        job_id: row.try_get("job_id")?,
        attempt_id: row.try_get("attempt_id")?,
        destination_id: row.try_get("destination_id")?,
        route_id: row.try_get("route_id")?,
        agent_id: row.try_get("agent_id")?,
        reservation_id: row.try_get("reservation_id")?,
        generation,
        resolution: row.try_get("resolution")?,
        note: row.try_get("note")?,
        actor_id: row.try_get("actor_id")?,
        agent_command_cursor: cursor,
        command: row.try_get("command")?,
        created_at: row.try_get("created_at")?,
        finalized_at: row.try_get("finalized_at")?,
    })
}

#[async_trait]
impl DestinationTopologyRepository for PostgresStore {
    async fn upsert_scheduling_authority(
        &self,
        scope: TenantScope,
        authority: &SchedulingAuthority,
    ) -> Result<(), StorageError> {
        sqlx::query("INSERT INTO scheduling_authorities (workspace_id,environment_id,id,kind,authority_key,display_name,active) VALUES ($1,$2,$3,$4,$5,$6,$7) ON CONFLICT (workspace_id,environment_id,id) DO UPDATE SET kind=EXCLUDED.kind,authority_key=EXCLUDED.authority_key,display_name=EXCLUDED.display_name,active=EXCLUDED.active,updated_at=now()")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(&authority.id).bind(&authority.kind).bind(&authority.authority_key).bind(&authority.display_name).bind(authority.active).execute(self.pool()).await?;
        Ok(())
    }

    async fn get_route(
        &self,
        scope: TenantScope,
        route_id: &str,
    ) -> Result<StoredPrinterRoute, StorageError> {
        let row = sqlx::query("SELECT id,destination_id,printer_id,agent_id,native_queue_id,local_route_key,state,role,priority,enabled,capability_revision,profile_revision,last_seen_at,updated_at FROM printer_routes WHERE workspace_id=$1 AND environment_id=$2 AND id=$3 AND retired_at IS NULL")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(route_id).fetch_optional(self.pool()).await?.ok_or(StorageError::NotFound)?;
        map_route(&row)
    }

    async fn get_route_by_local_key(
        &self,
        scope: TenantScope,
        agent_id: &str,
        local_route_key: &str,
    ) -> Result<StoredPrinterRoute, StorageError> {
        let row = sqlx::query("SELECT id,destination_id,printer_id,agent_id,native_queue_id,local_route_key,state,role,priority,enabled,capability_revision,profile_revision,last_seen_at,updated_at FROM printer_routes WHERE workspace_id=$1 AND environment_id=$2 AND agent_id=$3 AND local_route_key=$4 AND retired_at IS NULL")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(agent_id).bind(local_route_key).fetch_optional(self.pool()).await?.ok_or(StorageError::NotFound)?;
        map_route(&row)
    }

    async fn list_all_routes(
        &self,
        scope: TenantScope,
    ) -> Result<Vec<StoredPrinterRoute>, StorageError> {
        let rows = sqlx::query("SELECT id,destination_id,printer_id,agent_id,native_queue_id,local_route_key,state,role,priority,enabled,capability_revision,profile_revision,last_seen_at,updated_at FROM printer_routes WHERE workspace_id=$1 AND environment_id=$2 AND retired_at IS NULL ORDER BY destination_id,priority,id")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).fetch_all(self.pool()).await?;
        rows.iter().map(map_route).collect()
    }

    async fn upsert_destination(
        &self,
        scope: TenantScope,
        destination: &StoredPhysicalDestination,
    ) -> Result<(), StorageError> {
        let revision = i64::try_from(destination.identity_revision).map_err(|_| {
            StorageError::InvalidData("identity revision exceeds PostgreSQL bigint".into())
        })?;
        sqlx::query("INSERT INTO physical_destinations (workspace_id,environment_id,id,name,identity_confidence,state,scheduling_authority_id,identity_revision,updated_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9) ON CONFLICT (workspace_id,environment_id,id) DO UPDATE SET name=EXCLUDED.name,identity_confidence=EXCLUDED.identity_confidence,state=EXCLUDED.state,scheduling_authority_id=EXCLUDED.scheduling_authority_id,identity_revision=EXCLUDED.identity_revision,updated_at=EXCLUDED.updated_at")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(&destination.id).bind(&destination.name).bind(destination.identity_confidence.as_str()).bind(&destination.state).bind(&destination.scheduling_authority_id).bind(revision).bind(destination.updated_at).execute(self.pool()).await?;
        Ok(())
    }

    async fn get_destination(
        &self,
        scope: TenantScope,
        id: &str,
    ) -> Result<StoredPhysicalDestination, StorageError> {
        let row = sqlx::query("SELECT id,name,identity_confidence,state,scheduling_authority_id,identity_revision,updated_at FROM physical_destinations WHERE workspace_id=$1 AND environment_id=$2 AND id=$3")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(id).fetch_optional(self.pool()).await?.ok_or(StorageError::NotFound)?;
        map_destination(&row)
    }

    async fn list_destinations(
        &self,
        scope: TenantScope,
    ) -> Result<Vec<StoredPhysicalDestination>, StorageError> {
        let rows = sqlx::query("SELECT id,name,identity_confidence,state,scheduling_authority_id,identity_revision,updated_at FROM physical_destinations WHERE workspace_id=$1 AND environment_id=$2 AND retired_at IS NULL ORDER BY updated_at DESC,id")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).fetch_all(self.pool()).await?;
        rows.iter().map(map_destination).collect()
    }

    async fn upsert_route(
        &self,
        scope: TenantScope,
        route: &StoredPrinterRoute,
    ) -> Result<(), StorageError> {
        let capabilities = i64::try_from(route.capability_revision).map_err(|_| {
            StorageError::InvalidData("capability revision exceeds PostgreSQL bigint".into())
        })?;
        let profiles = i64::try_from(route.profile_revision).map_err(|_| {
            StorageError::InvalidData("profile revision exceeds PostgreSQL bigint".into())
        })?;
        sqlx::query("INSERT INTO printer_routes (workspace_id,environment_id,id,destination_id,printer_id,agent_id,native_queue_id,local_route_key,state,role,priority,enabled,capability_revision,profile_revision,last_seen_at,updated_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16) ON CONFLICT (workspace_id,environment_id,printer_id,agent_id) DO UPDATE SET destination_id=EXCLUDED.destination_id,native_queue_id=EXCLUDED.native_queue_id,local_route_key=COALESCE(EXCLUDED.local_route_key,printer_routes.local_route_key),state=EXCLUDED.state,role=EXCLUDED.role,priority=EXCLUDED.priority,enabled=EXCLUDED.enabled,capability_revision=EXCLUDED.capability_revision,profile_revision=EXCLUDED.profile_revision,last_seen_at=EXCLUDED.last_seen_at,updated_at=EXCLUDED.updated_at")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(&route.id).bind(&route.destination_id).bind(&route.printer_id).bind(&route.agent_id).bind(&route.native_queue_id).bind(&route.local_route_key).bind(&route.state).bind(&route.role).bind(route.priority).bind(route.enabled).bind(capabilities).bind(profiles).bind(route.last_seen_at).bind(route.updated_at).execute(self.pool()).await?;
        Ok(())
    }

    async fn list_routes(
        &self,
        scope: TenantScope,
        destination_id: &str,
    ) -> Result<Vec<StoredPrinterRoute>, StorageError> {
        let rows = sqlx::query("SELECT id,destination_id,printer_id,agent_id,native_queue_id,local_route_key,state,role,priority,enabled,capability_revision,profile_revision,last_seen_at,updated_at FROM printer_routes WHERE workspace_id=$1 AND environment_id=$2 AND destination_id=$3 AND retired_at IS NULL ORDER BY priority,id")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(destination_id).fetch_all(self.pool()).await?;
        rows.iter().map(map_route).collect()
    }

    async fn record_identity_evidence(
        &self,
        scope: TenantScope,
        evidence: &IdentityEvidence,
    ) -> Result<(), StorageError> {
        let mut tx = self.pool().begin().await?;
        let route_matches: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM printer_routes WHERE workspace_id=$1 AND environment_id=$2 AND id=$3 AND destination_id=$4)")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(&evidence.route_id).bind(&evidence.destination_id).fetch_one(&mut *tx).await?;
        if !route_matches {
            return Err(StorageError::NotFound);
        }
        sqlx::query("INSERT INTO destination_identity_evidence (workspace_id,environment_id,id,destination_id,route_id,kind,value_digest,strength,conflicts,observed_at,expires_at,metadata) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12) ON CONFLICT (workspace_id,environment_id,route_id,kind,value_digest) DO UPDATE SET destination_id=EXCLUDED.destination_id,strength=EXCLUDED.strength,conflicts=EXCLUDED.conflicts,observed_at=EXCLUDED.observed_at,expires_at=EXCLUDED.expires_at,metadata=EXCLUDED.metadata")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(&evidence.id).bind(&evidence.destination_id).bind(&evidence.route_id).bind(&evidence.kind).bind(&evidence.value_digest).bind(&evidence.strength).bind(evidence.conflicts).bind(evidence.observed_at).bind(evidence.expires_at).bind(&evidence.metadata).execute(&mut *tx).await?;
        if evidence.conflicts {
            sqlx::query("UPDATE physical_destinations SET identity_confidence='conflict',identity_revision=identity_revision+1,updated_at=now() WHERE workspace_id=$1 AND environment_id=$2 AND id=$3")
                .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(&evidence.destination_id).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    async fn list_identity_evidence(
        &self,
        scope: TenantScope,
        destination_id: &str,
    ) -> Result<Vec<IdentityEvidence>, StorageError> {
        let rows = sqlx::query("SELECT id,destination_id,route_id,kind,value_digest,strength,conflicts,observed_at,expires_at,metadata FROM destination_identity_evidence WHERE workspace_id=$1 AND environment_id=$2 AND destination_id=$3 ORDER BY observed_at DESC,id")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(destination_id).fetch_all(self.pool()).await?;
        rows.iter().map(map_evidence).collect()
    }

    async fn record_identity_decision(
        &self,
        scope: TenantScope,
        decision: &IdentityDecision,
    ) -> Result<(), StorageError> {
        if decision.kind == IdentityDecisionKind::Reverse {
            return Err(StorageError::InvalidData(
                "reverse decisions must use reverse_identity_decision".into(),
            ));
        }
        apply_identity_decision(self, scope, decision).await
    }

    async fn reverse_identity_decision(
        &self,
        scope: TenantScope,
        reversal: &IdentityDecision,
    ) -> Result<(), StorageError> {
        if reversal.kind != IdentityDecisionKind::Reverse || reversal.reverses_decision_id.is_none()
        {
            return Err(StorageError::InvalidData(
                "reversal must identify the decision it reverses".into(),
            ));
        }
        reverse_applied_identity_decision(self, scope, reversal).await
    }

    async fn list_identity_decisions(
        &self,
        scope: TenantScope,
        destination_id: &str,
    ) -> Result<Vec<IdentityDecision>, StorageError> {
        let rows = sqlx::query("SELECT decision.id,decision.kind,decision.destination_id,decision.related_destination_ids,COALESCE((SELECT jsonb_agg(link.route_id ORDER BY link.route_id) FROM destination_identity_decision_routes link WHERE link.workspace_id=decision.workspace_id AND link.environment_id=decision.environment_id AND link.decision_id=decision.id),'[]'::jsonb) AS route_ids,decision.evidence_ids,decision.actor_kind,decision.actor_id,decision.reason,decision.reverses_decision_id,decision.request_id,decision.created_at FROM destination_identity_decisions decision WHERE decision.workspace_id=$1 AND decision.environment_id=$2 AND decision.destination_id=$3 ORDER BY decision.created_at,decision.id")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(destination_id).fetch_all(self.pool()).await?;
        rows.iter().map(map_decision).collect()
    }

    async fn record_route_observation(
        &self,
        scope: TenantScope,
        observation: &RouteObservation,
    ) -> Result<(), StorageError> {
        let mut tx = self.pool().begin().await?;
        sqlx::query("SELECT id FROM printer_routes WHERE workspace_id=$1 AND environment_id=$2 AND id=$3 FOR UPDATE")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(&observation.route_id).fetch_optional(&mut *tx).await?.ok_or(StorageError::NotFound)?;
        let sequence = i64::try_from(observation.sequence)
            .map_err(|_| StorageError::InvalidData("observation sequence exceeds bigint".into()))?;
        if let Some(existing) = sqlx::query("SELECT id,route_id,sequence,printer_state,accepting_jobs,state_reasons,total_jobs,connector_jobs,other_piqae_or_external_jobs,unknown_jobs,active_jobs,held_jobs,estimated_busy_seconds,privacy_level,stock_state,observed_at,fresh_until FROM route_observations WHERE workspace_id=$1 AND environment_id=$2 AND route_id=$3 AND sequence=$4")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(&observation.route_id).bind(sequence).fetch_optional(&mut *tx).await? {
            let mut stored = map_observation(&existing)?;
            stored.id.clone_from(&observation.id);
            if stored == *observation {
                tx.commit().await?;
                return Ok(());
            }
            return Err(StorageError::IdempotencyConflict);
        }
        let latest: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(sequence),0) FROM route_observations WHERE workspace_id=$1 AND environment_id=$2 AND route_id=$3")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(&observation.route_id).fetch_one(&mut *tx).await?;
        if sequence <= latest {
            return Err(StorageError::ConcurrentStateChange);
        }
        sqlx::query("INSERT INTO route_observations (workspace_id,environment_id,id,route_id,sequence,printer_state,accepting_jobs,state_reasons,total_jobs,connector_jobs,other_piqae_or_external_jobs,unknown_jobs,active_jobs,held_jobs,estimated_busy_seconds,privacy_level,stock_state,observed_at,fresh_until) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19)")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(&observation.id).bind(&observation.route_id).bind(sequence).bind(&observation.printer_state).bind(observation.accepting_jobs).bind(&observation.state_reasons).bind(i32::try_from(observation.total_jobs).map_err(|_| StorageError::InvalidData("total job count exceeds integer".into()))?).bind(i32::try_from(observation.connector_jobs).map_err(|_| StorageError::InvalidData("connector job count exceeds integer".into()))?).bind(i32::try_from(observation.other_piqae_or_external_jobs).map_err(|_| StorageError::InvalidData("other job count exceeds integer".into()))?).bind(i32::try_from(observation.unknown_jobs).map_err(|_| StorageError::InvalidData("unknown job count exceeds integer".into()))?).bind(i32::try_from(observation.active_jobs).map_err(|_| StorageError::InvalidData("active job count exceeds integer".into()))?).bind(i32::try_from(observation.held_jobs).map_err(|_| StorageError::InvalidData("held job count exceeds integer".into()))?).bind(observation.estimated_busy_seconds.map(i32::try_from).transpose().map_err(|_| StorageError::InvalidData("estimated busy seconds exceeds integer".into()))?).bind(&observation.privacy_level).bind(&observation.stock_state).bind(observation.observed_at).bind(observation.fresh_until).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    async fn latest_route_observation(
        &self,
        scope: TenantScope,
        route_id: &str,
    ) -> Result<RouteObservation, StorageError> {
        let row = sqlx::query("SELECT id,route_id,sequence,printer_state,accepting_jobs,state_reasons,total_jobs,connector_jobs,other_piqae_or_external_jobs,unknown_jobs,active_jobs,held_jobs,estimated_busy_seconds,privacy_level,stock_state,observed_at,fresh_until FROM route_observations WHERE workspace_id=$1 AND environment_id=$2 AND route_id=$3 ORDER BY sequence DESC LIMIT 1")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(route_id).fetch_optional(self.pool()).await?.ok_or(StorageError::NotFound)?;
        map_observation(&row)
    }

    async fn list_route_observations(
        &self,
        scope: TenantScope,
        route_id: &str,
        limit: u32,
    ) -> Result<Vec<RouteObservation>, StorageError> {
        let limit = i64::from(limit.clamp(1, 1_000));
        let rows = sqlx::query("SELECT id,route_id,sequence,printer_state,accepting_jobs,state_reasons,total_jobs,connector_jobs,other_piqae_or_external_jobs,unknown_jobs,active_jobs,held_jobs,estimated_busy_seconds,privacy_level,stock_state,observed_at,fresh_until FROM route_observations WHERE workspace_id=$1 AND environment_id=$2 AND route_id=$3 ORDER BY sequence DESC LIMIT $4")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(route_id).bind(limit).fetch_all(self.pool()).await?;
        rows.iter().map(map_observation).collect()
    }

    async fn acknowledge_projection(
        &self,
        scope: TenantScope,
        acknowledgement: &ProjectionAcknowledgement,
    ) -> Result<(), StorageError> {
        sqlx::query("INSERT INTO projection_acknowledgements (workspace_id,environment_id,agent_id,route_id,inventory_revision,capability_revision,status,error_code,observed_at,acknowledged_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) ON CONFLICT (workspace_id,environment_id,agent_id,route_id) DO UPDATE SET inventory_revision=EXCLUDED.inventory_revision,capability_revision=EXCLUDED.capability_revision,status=EXCLUDED.status,error_code=EXCLUDED.error_code,observed_at=EXCLUDED.observed_at,acknowledged_at=EXCLUDED.acknowledged_at,updated_at=now() WHERE projection_acknowledgements.inventory_revision <= EXCLUDED.inventory_revision")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(&acknowledgement.agent_id).bind(&acknowledgement.route_id).bind(i64::try_from(acknowledgement.inventory_revision).map_err(|_| StorageError::InvalidData("inventory revision exceeds bigint".into()))?).bind(i64::try_from(acknowledgement.capability_revision).map_err(|_| StorageError::InvalidData("capability revision exceeds bigint".into()))?).bind(&acknowledgement.status).bind(&acknowledgement.error_code).bind(acknowledgement.observed_at).bind(acknowledgement.acknowledged_at).execute(self.pool()).await?;
        Ok(())
    }

    async fn get_projection_acknowledgement(
        &self,
        scope: TenantScope,
        agent_id: &str,
        route_id: &str,
    ) -> Result<ProjectionAcknowledgement, StorageError> {
        let row = sqlx::query("SELECT agent_id,route_id,inventory_revision,capability_revision,status,error_code,observed_at,acknowledged_at FROM projection_acknowledgements WHERE workspace_id=$1 AND environment_id=$2 AND agent_id=$3 AND route_id=$4")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(agent_id).bind(route_id).fetch_optional(self.pool()).await?.ok_or(StorageError::NotFound)?;
        map_projection_acknowledgement(&row)
    }

    async fn upsert_site_membership(
        &self,
        scope: TenantScope,
        membership: &SiteCoordinatorMembership,
    ) -> Result<(), StorageError> {
        let revoked_at = (membership.state == "revoked").then(Utc::now);
        sqlx::query("INSERT INTO site_coordinator_memberships (workspace_id,environment_id,authority_id,agent_id,site_id,state,last_seen_at,revoked_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT (workspace_id,environment_id,authority_id,agent_id) DO UPDATE SET site_id=EXCLUDED.site_id,state=EXCLUDED.state,last_seen_at=EXCLUDED.last_seen_at,revoked_at=EXCLUDED.revoked_at")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(&membership.authority_id).bind(&membership.agent_id).bind(&membership.site_id).bind(&membership.state).bind(membership.last_seen_at).bind(revoked_at).execute(self.pool()).await?;
        Ok(())
    }

    async fn begin_delivery_attempt(
        &self,
        scope: TenantScope,
        request: NewDeliveryAttempt<'_>,
    ) -> Result<StartedDeliveryAttempt, StorageError> {
        let mut tx = self.pool().begin().await?;
        let job = sqlx::query(
            "SELECT id,destination_id FROM jobs WHERE workspace_id=$1 AND environment_id=$2 AND id=$3 FOR UPDATE",
        )
        .bind(scope.workspace_id.to_string())
        .bind(scope.environment_id.to_string())
        .bind(request.job_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(StorageError::NotFound)?;
        let job_destination: Option<String> = job.try_get("destination_id")?;
        if job_destination
            .as_deref()
            .is_some_and(|id| id != request.destination_id)
        {
            return Err(StorageError::ConcurrentStateChange);
        }
        let route_matches_destination: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM printer_routes WHERE workspace_id=$1 AND environment_id=$2 AND id=$3 AND destination_id=$4 AND enabled AND retired_at IS NULL)")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(request.route_id).bind(request.destination_id).fetch_one(&mut *tx).await?;
        if !route_matches_destination {
            return Err(StorageError::NotFound);
        }
        if job_destination.is_none() {
            sqlx::query("UPDATE jobs SET destination_id=$4,route_id=$5,updated_at=now() WHERE workspace_id=$1 AND environment_id=$2 AND id=$3 AND destination_id IS NULL")
                .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(request.job_id).bind(request.destination_id).bind(request.route_id).execute(&mut *tx).await?;
        }
        // Serialize schedulers at the physical destination boundary, including
        // schedulers choosing different node routes for the same printer.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(format!(
                "{}:{}:{}",
                scope.workspace_id, scope.environment_id, request.destination_id
            ))
            .execute(&mut *tx)
            .await?;
        let unresolved: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM delivery_attempts attempt WHERE attempt.workspace_id=$1 AND attempt.environment_id=$2 AND attempt.destination_id=$3 AND attempt.state='delivery_uncertain' AND NOT EXISTS (SELECT 1 FROM delivery_uncertainty_resolutions resolution WHERE resolution.workspace_id=attempt.workspace_id AND resolution.environment_id=attempt.environment_id AND resolution.attempt_id=attempt.id))")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(request.destination_id).fetch_one(&mut *tx).await?;
        if unresolved {
            return Err(StorageError::ConcurrentStateChange);
        }
        // A scheduler may disappear after reserving but before the node accepts.
        // Retire only expired route_leased handoffs; every later state is
        // ambiguity-sensitive and must remain fenced until explicit evidence.
        let stale_attempt_ids: Vec<String> = sqlx::query_scalar(
            "UPDATE delivery_attempts
             SET state='superseded',final_at=now(),updated_at=now()
             WHERE workspace_id=$1 AND environment_id=$2 AND destination_id=$3
               AND state='route_leased' AND final_at IS NULL AND lease_until<=now()
             RETURNING id",
        )
        .bind(scope.workspace_id.to_string())
        .bind(scope.environment_id.to_string())
        .bind(request.destination_id)
        .fetch_all(&mut *tx)
        .await?;
        if !stale_attempt_ids.is_empty() {
            sqlx::query(
                "UPDATE route_reservations
                 SET state='superseded',released_at=now(),updated_at=now()
                 WHERE workspace_id=$1 AND environment_id=$2
                   AND attempt_id=ANY($3) AND state='active'",
            )
            .bind(scope.workspace_id.to_string())
            .bind(scope.environment_id.to_string())
            .bind(&stale_attempt_ids)
            .execute(&mut *tx)
            .await?;
        }
        let older_eligible_job: bool = sqlx::query_scalar(
            "SELECT EXISTS(
                SELECT 1 FROM jobs older
                WHERE older.workspace_id=$1 AND older.environment_id=$2
                  AND older.destination_id=$3 AND older.id<>$4
                  AND (older.created_at,older.id) < (
                      SELECT created_at,id FROM jobs
                      WHERE workspace_id=$1 AND environment_id=$2 AND id=$4
                  )
                  AND older.state IN ('waiting_for_agent','failed_retryable')
                  AND older.expires_at > now()
                  AND COALESCE((
                      SELECT attempt.state FROM delivery_attempts attempt
                      WHERE attempt.workspace_id=older.workspace_id
                        AND attempt.environment_id=older.environment_id
                        AND attempt.job_id=older.id
                      ORDER BY attempt.generation DESC LIMIT 1
                  ),'route_leased') NOT IN (
                      'accepted_by_spooler','printing_reported','completed_reported',
                      'cancelled','failed','delivery_uncertain','superseded'
                  )
            )",
        )
        .bind(scope.workspace_id.to_string())
        .bind(scope.environment_id.to_string())
        .bind(request.destination_id)
        .bind(request.job_id)
        .fetch_one(&mut *tx)
        .await?;
        if older_eligible_job {
            return Err(StorageError::ConcurrentStateChange);
        }
        let active = sqlx::query("SELECT id,state,lease_until FROM delivery_attempts WHERE workspace_id=$1 AND environment_id=$2 AND job_id=$3 AND final_at IS NULL FOR UPDATE")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(request.job_id).fetch_optional(&mut *tx).await?;
        if let Some(active) = active {
            let state: String = active.try_get("state")?;
            let lease_until: DateTime<Utc> = active.try_get("lease_until")?;
            if state != "route_leased" || lease_until > Utc::now() {
                return Err(StorageError::ConcurrentStateChange);
            }
            let active_id: String = active.try_get("id")?;
            let now = Utc::now();
            sqlx::query("UPDATE delivery_attempts SET state='superseded',final_at=$4,updated_at=$4 WHERE workspace_id=$1 AND environment_id=$2 AND id=$3 AND state='route_leased' AND final_at IS NULL")
                .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(&active_id).bind(now).execute(&mut *tx).await?;
            sqlx::query("UPDATE route_reservations SET state='superseded',released_at=$4,updated_at=$4 WHERE workspace_id=$1 AND environment_id=$2 AND attempt_id=$3 AND state='active'")
                .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(&active_id).bind(now).execute(&mut *tx).await?;
        } else {
            let latest_state: Option<String> = sqlx::query_scalar("SELECT state FROM delivery_attempts WHERE workspace_id=$1 AND environment_id=$2 AND job_id=$3 ORDER BY generation DESC LIMIT 1")
                .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(request.job_id).fetch_optional(&mut *tx).await?;
            if latest_state.as_deref() == Some("delivery_uncertain") {
                return Err(StorageError::ConcurrentStateChange);
            }
        }
        let generation: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(generation),0)+1 FROM delivery_attempts WHERE workspace_id=$1 AND environment_id=$2 AND job_id=$3")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(request.job_id).fetch_one(&mut *tx).await?;
        let token = new_fencing_token();
        let digest = token_digest(&token);
        let attempt_row = sqlx::query("INSERT INTO delivery_attempts (workspace_id,environment_id,id,job_id,destination_id,route_id,generation,fencing_token_hash,state,lease_until) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,'route_leased',$9) RETURNING id,job_id,destination_id,route_id,generation,state,lease_until,accepted_at,handoff_started_at,spooler_accepted_at,final_at,created_at,updated_at")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(request.attempt_id).bind(request.job_id).bind(request.destination_id).bind(request.route_id).bind(generation).bind(&digest).bind(request.lease_until).fetch_one(&mut *tx).await.map_err(reservation_write_error)?;
        sqlx::query("INSERT INTO route_reservations (workspace_id,environment_id,id,route_id,destination_id,job_id,attempt_id,generation,fencing_token_hash,state,lease_until) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,'active',$10)")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(request.reservation_id).bind(request.route_id).bind(request.destination_id).bind(request.job_id).bind(request.attempt_id).bind(generation).bind(&digest).bind(request.lease_until).execute(&mut *tx).await.map_err(reservation_write_error)?;
        tx.commit().await?;
        let attempt = map_attempt(&attempt_row)?;
        Ok(StartedDeliveryAttempt {
            reservation: RouteReservation {
                id: request.reservation_id.to_owned(),
                route_id: request.route_id.to_owned(),
                destination_id: request.destination_id.to_owned(),
                job_id: request.job_id.to_owned(),
                attempt_id: request.attempt_id.to_owned(),
                generation: attempt.generation,
                state: "active".into(),
                lease_until: request.lease_until,
                released_at: None,
                acquired_at: attempt.created_at,
            },
            attempt,
            fencing_token: token,
        })
    }

    async fn transition_delivery_attempt(
        &self,
        scope: TenantScope,
        attempt_id: &str,
        generation: u64,
        fencing_token: &str,
        next: DeliveryAttemptState,
    ) -> Result<DeliveryAttempt, StorageError> {
        let generation =
            i64::try_from(generation).map_err(|_| StorageError::ConcurrentStateChange)?;
        let digest = token_digest(fencing_token);
        let mut tx = self.pool().begin().await?;
        let row = sqlx::query("SELECT id,job_id,destination_id,route_id,generation,state,lease_until,accepted_at,handoff_started_at,spooler_accepted_at,final_at,created_at,updated_at FROM delivery_attempts WHERE workspace_id=$1 AND environment_id=$2 AND id=$3 FOR UPDATE")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(attempt_id).fetch_optional(&mut *tx).await?.ok_or(StorageError::NotFound)?;
        let current = map_attempt(&row)?;
        let stored_digest: String = sqlx::query_scalar("SELECT fencing_token_hash FROM delivery_attempts WHERE workspace_id=$1 AND environment_id=$2 AND id=$3")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(attempt_id).fetch_one(&mut *tx).await?;
        if i64::try_from(current.generation).ok() != Some(generation)
            || stored_digest != digest
            || !valid_attempt_transition(current.state, next)
        {
            return Err(StorageError::ConcurrentStateChange);
        }
        let now = Utc::now();
        let final_at = next.is_final().then_some(now);
        let updated = sqlx::query("UPDATE delivery_attempts SET state=$4,accepted_at=CASE WHEN $4='accepted_by_node' THEN $5 ELSE accepted_at END,handoff_started_at=CASE WHEN $4='handing_to_spooler' THEN $5 ELSE handoff_started_at END,spooler_accepted_at=CASE WHEN $4='accepted_by_spooler' THEN $5 ELSE spooler_accepted_at END,final_at=$6,updated_at=$5 WHERE workspace_id=$1 AND environment_id=$2 AND id=$3 RETURNING id,job_id,destination_id,route_id,generation,state,lease_until,accepted_at,handoff_started_at,spooler_accepted_at,final_at,created_at,updated_at")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(attempt_id).bind(next.as_str()).bind(now).bind(final_at).fetch_one(&mut *tx).await?;
        if next.is_final() || next == DeliveryAttemptState::AcceptedBySpooler {
            sqlx::query("UPDATE route_reservations SET state=CASE WHEN $4='superseded' THEN 'superseded' WHEN $4='cancelled' THEN 'cancelled' ELSE 'released' END,released_at=$5,updated_at=$5 WHERE workspace_id=$1 AND environment_id=$2 AND attempt_id=$3 AND state='active'")
                .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(attempt_id).bind(next.as_str()).bind(now).execute(&mut *tx).await?;
        }
        if next == DeliveryAttemptState::DeliveryUncertain {
            sqlx::query("UPDATE physical_destinations SET state='attention',updated_at=$4 WHERE workspace_id=$1 AND environment_id=$2 AND id=$3")
                .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(&current.destination_id).bind(now).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        map_attempt(&updated)
    }

    async fn transition_post_spooler_attempt(
        &self,
        scope: TenantScope,
        job_id: &str,
        agent_id: &str,
        route_id: &str,
        next: DeliveryAttemptState,
    ) -> Result<DeliveryAttempt, StorageError> {
        if !matches!(
            next,
            DeliveryAttemptState::PrintingReported
                | DeliveryAttemptState::CompletedReported
                | DeliveryAttemptState::Failed
                | DeliveryAttemptState::DeliveryUncertain
        ) {
            return Err(StorageError::InvalidTransition);
        }
        let mut tx = self.pool().begin().await?;
        let row = sqlx::query(
            "SELECT attempt.id,attempt.job_id,attempt.destination_id,attempt.route_id,
                    attempt.generation,attempt.state,attempt.lease_until,attempt.accepted_at,
                    attempt.handoff_started_at,attempt.spooler_accepted_at,attempt.final_at,
                    attempt.created_at,attempt.updated_at
             FROM delivery_attempts attempt
             JOIN printer_routes route
               ON route.workspace_id=attempt.workspace_id
              AND route.environment_id=attempt.environment_id
              AND route.id=attempt.route_id
             WHERE attempt.workspace_id=$1 AND attempt.environment_id=$2
               AND attempt.job_id=$3 AND attempt.route_id=$4 AND route.agent_id=$5
             ORDER BY attempt.generation DESC LIMIT 1 FOR UPDATE OF attempt",
        )
        .bind(scope.workspace_id.to_string())
        .bind(scope.environment_id.to_string())
        .bind(job_id)
        .bind(route_id)
        .bind(agent_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(StorageError::NotFound)?;
        let current = map_attempt(&row)?;
        if !valid_attempt_transition(current.state, next)
            || !matches!(
                current.state,
                DeliveryAttemptState::AcceptedBySpooler | DeliveryAttemptState::PrintingReported
            )
        {
            return Err(StorageError::ConcurrentStateChange);
        }
        let now = Utc::now();
        let updated = sqlx::query(
            "UPDATE delivery_attempts
             SET state=$4,final_at=$5,updated_at=$6
             WHERE workspace_id=$1 AND environment_id=$2 AND id=$3
             RETURNING id,job_id,destination_id,route_id,generation,state,lease_until,
                       accepted_at,handoff_started_at,spooler_accepted_at,final_at,
                       created_at,updated_at",
        )
        .bind(scope.workspace_id.to_string())
        .bind(scope.environment_id.to_string())
        .bind(&current.id)
        .bind(next.as_str())
        .bind(next.is_final().then_some(now))
        .bind(now)
        .fetch_one(&mut *tx)
        .await?;
        // Defensive cleanup for data created by an older server that retained
        // the handoff reservation beyond spooler acceptance.
        sqlx::query(
            "UPDATE route_reservations SET state='released',released_at=$4,updated_at=$4
             WHERE workspace_id=$1 AND environment_id=$2 AND attempt_id=$3 AND state='active'",
        )
        .bind(scope.workspace_id.to_string())
        .bind(scope.environment_id.to_string())
        .bind(&current.id)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        if next == DeliveryAttemptState::DeliveryUncertain {
            sqlx::query(
                "UPDATE physical_destinations SET state='attention',updated_at=$4
                 WHERE workspace_id=$1 AND environment_id=$2 AND id=$3",
            )
            .bind(scope.workspace_id.to_string())
            .bind(scope.environment_id.to_string())
            .bind(&current.destination_id)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        map_attempt(&updated)
    }

    async fn mark_post_spooler_attempt_uncertain(
        &self,
        scope: TenantScope,
        job_id: &str,
    ) -> Result<DeliveryAttempt, StorageError> {
        let mut tx = self.pool().begin().await?;
        let row = sqlx::query(
            "SELECT id,job_id,destination_id,route_id,generation,state,lease_until,
                    accepted_at,handoff_started_at,spooler_accepted_at,final_at,
                    created_at,updated_at
             FROM delivery_attempts
             WHERE workspace_id=$1 AND environment_id=$2 AND job_id=$3
             ORDER BY generation DESC LIMIT 1 FOR UPDATE",
        )
        .bind(scope.workspace_id.to_string())
        .bind(scope.environment_id.to_string())
        .bind(job_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(StorageError::NotFound)?;
        let current = map_attempt(&row)?;
        if !matches!(
            current.state,
            DeliveryAttemptState::AcceptedBySpooler | DeliveryAttemptState::PrintingReported
        ) {
            return Err(StorageError::ConcurrentStateChange);
        }
        let now = Utc::now();
        let updated = sqlx::query(
            "UPDATE delivery_attempts SET state='delivery_uncertain',final_at=$4,updated_at=$4
             WHERE workspace_id=$1 AND environment_id=$2 AND id=$3
             RETURNING id,job_id,destination_id,route_id,generation,state,lease_until,
                       accepted_at,handoff_started_at,spooler_accepted_at,final_at,
                       created_at,updated_at",
        )
        .bind(scope.workspace_id.to_string())
        .bind(scope.environment_id.to_string())
        .bind(&current.id)
        .bind(now)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE route_reservations SET state='released',released_at=$4,updated_at=$4
             WHERE workspace_id=$1 AND environment_id=$2 AND attempt_id=$3 AND state='active'",
        )
        .bind(scope.workspace_id.to_string())
        .bind(scope.environment_id.to_string())
        .bind(&current.id)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE physical_destinations SET state='attention',updated_at=$4
             WHERE workspace_id=$1 AND environment_id=$2 AND id=$3",
        )
        .bind(scope.workspace_id.to_string())
        .bind(scope.environment_id.to_string())
        .bind(&current.destination_id)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        map_attempt(&updated)
    }

    async fn renew_delivery_attempt(
        &self,
        scope: TenantScope,
        reservation_id: &str,
        generation: u64,
        fencing_token: &str,
        lease_until: DateTime<Utc>,
    ) -> Result<StartedDeliveryAttempt, StorageError> {
        if lease_until <= Utc::now() {
            return Err(StorageError::InvalidData(
                "renewed lease must be in the future".into(),
            ));
        }
        let generation =
            i64::try_from(generation).map_err(|_| StorageError::ConcurrentStateChange)?;
        let digest = token_digest(fencing_token);
        let mut tx = self.pool().begin().await?;
        let row = sqlx::query("SELECT attempt.id,attempt.job_id,attempt.destination_id,attempt.route_id,attempt.generation,attempt.state,attempt.lease_until,attempt.accepted_at,attempt.handoff_started_at,attempt.spooler_accepted_at,attempt.final_at,attempt.created_at,attempt.updated_at,reservation.id AS reservation_id,reservation.state AS reservation_state,reservation.released_at,reservation.created_at AS reservation_created_at FROM route_reservations reservation JOIN delivery_attempts attempt ON attempt.workspace_id=reservation.workspace_id AND attempt.environment_id=reservation.environment_id AND attempt.id=reservation.attempt_id WHERE reservation.workspace_id=$1 AND reservation.environment_id=$2 AND reservation.id=$3 FOR UPDATE OF reservation,attempt")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(reservation_id).fetch_optional(&mut *tx).await?.ok_or(StorageError::NotFound)?;
        let attempt = map_attempt(&row)?;
        let stored_digest: String = sqlx::query_scalar("SELECT fencing_token_hash FROM delivery_attempts WHERE workspace_id=$1 AND environment_id=$2 AND id=$3")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(&attempt.id).fetch_one(&mut *tx).await?;
        let renewable = matches!(
            attempt.state,
            DeliveryAttemptState::RouteLeased
                | DeliveryAttemptState::AcceptedByNode
                | DeliveryAttemptState::QueuedLocal
                | DeliveryAttemptState::HandingToSpooler
        );
        if i64::try_from(attempt.generation).ok() != Some(generation)
            || stored_digest != digest
            || row.try_get::<String, _>("reservation_state")? != "active"
            || !renewable
        {
            return Err(StorageError::ConcurrentStateChange);
        }
        sqlx::query("UPDATE delivery_attempts SET lease_until=$4,updated_at=now() WHERE workspace_id=$1 AND environment_id=$2 AND id=$3")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(&attempt.id).bind(lease_until).execute(&mut *tx).await?;
        sqlx::query("UPDATE route_reservations SET lease_until=$4,updated_at=now() WHERE workspace_id=$1 AND environment_id=$2 AND id=$3 AND state='active'")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(reservation_id).bind(lease_until).execute(&mut *tx).await?;
        tx.commit().await?;
        let mut renewed_attempt = attempt;
        renewed_attempt.lease_until = lease_until;
        renewed_attempt.updated_at = Utc::now();
        Ok(StartedDeliveryAttempt {
            reservation: RouteReservation {
                id: reservation_id.to_owned(),
                route_id: renewed_attempt.route_id.clone(),
                destination_id: renewed_attempt.destination_id.clone(),
                job_id: renewed_attempt.job_id.clone(),
                attempt_id: renewed_attempt.id.clone(),
                generation: renewed_attempt.generation,
                state: "active".into(),
                lease_until,
                released_at: None,
                acquired_at: row.try_get("reservation_created_at")?,
            },
            attempt: renewed_attempt,
            fencing_token: fencing_token.to_owned(),
        })
    }

    async fn enqueue_delivery_uncertainty_resolution(
        &self,
        scope: TenantScope,
        job_id: &str,
        resolution: &str,
        note: Option<&str>,
        actor_id: &str,
        request_id: &str,
    ) -> Result<PendingDeliveryUncertaintyResolution, StorageError> {
        let mut tx = self.pool().begin().await?;
        if let Some(existing) = sqlx::query("SELECT pending.*,command.command FROM delivery_uncertainty_resolution_commands pending JOIN agent_commands command ON command.cursor=pending.agent_command_cursor WHERE pending.workspace_id=$1 AND pending.environment_id=$2 AND pending.request_id=$3")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(request_id).fetch_optional(&mut *tx).await? {
            let existing = pending_uncertainty_resolution_from_row(&existing)?;
            if existing.job_id == job_id && existing.resolution == resolution && existing.note.as_deref() == note && existing.actor_id == actor_id {
                tx.commit().await?;
                return Ok(existing);
            }
            return Err(StorageError::IdempotencyConflict);
        }
        let attempt = sqlx::query("SELECT attempt.id,attempt.destination_id,attempt.route_id,attempt.generation,route.agent_id,route.local_route_key,reservation.id AS reservation_id FROM delivery_attempts attempt JOIN printer_routes route ON route.workspace_id=attempt.workspace_id AND route.environment_id=attempt.environment_id AND route.id=attempt.route_id JOIN route_reservations reservation ON reservation.workspace_id=attempt.workspace_id AND reservation.environment_id=attempt.environment_id AND reservation.attempt_id=attempt.id WHERE attempt.workspace_id=$1 AND attempt.environment_id=$2 AND attempt.job_id=$3 AND attempt.state='delivery_uncertain' ORDER BY attempt.generation DESC LIMIT 1 FOR UPDATE OF attempt")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(job_id).fetch_optional(&mut *tx).await?.ok_or(StorageError::NotFound)?;
        let attempt_id: String = attempt.try_get("id")?;
        let destination_id: String = attempt.try_get("destination_id")?;
        let route_id: String = attempt.try_get("route_id")?;
        let agent_id: String = attempt.try_get("agent_id")?;
        let local_route_key: String = attempt
            .try_get::<Option<String>, _>("local_route_key")?
            .ok_or_else(|| {
                StorageError::InvalidData("uncertain route has no node-local route key".into())
            })?;
        let reservation_id: String = attempt.try_get("reservation_id")?;
        let generation: i64 = attempt.try_get("generation")?;
        let command = serde_json::to_value(AgentCommand::ResolveAmbiguousHandoff {
            job_id: job_id.parse::<JobId>().map_err(|_| {
                StorageError::InvalidData("uncertain resolution has an invalid job id".into())
            })?,
            local_route_key,
            reservation_id: reservation_id.parse().map_err(|_| {
                StorageError::InvalidData(
                    "uncertain resolution has an invalid reservation id".into(),
                )
            })?,
            generation: u64::try_from(generation).map_err(|_| {
                StorageError::InvalidData("uncertain resolution has a negative generation".into())
            })?,
            resolution: AmbiguousHandoffResolution::ConfirmAccepted,
        })?;
        let cursor: i64 = sqlx::query_scalar("INSERT INTO agent_commands (workspace_id,environment_id,agent_id,command) VALUES ($1,$2,$3,$4) RETURNING cursor")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(&agent_id).bind(&command).fetch_one(&mut *tx).await?;
        let row = sqlx::query("INSERT INTO delivery_uncertainty_resolution_commands (workspace_id,environment_id,request_id,job_id,attempt_id,destination_id,route_id,agent_id,reservation_id,generation,resolution,note,actor_id,agent_command_cursor) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14) RETURNING *, $15::jsonb AS command")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(request_id).bind(job_id).bind(&attempt_id).bind(&destination_id).bind(&route_id).bind(&agent_id).bind(&reservation_id).bind(generation).bind(resolution).bind(note).bind(actor_id).bind(cursor).bind(&command).fetch_one(&mut *tx).await.map_err(reservation_write_error)?;
        tx.commit().await?;
        pending_uncertainty_resolution_from_row(&row)
    }

    async fn finalize_delivery_uncertainty_resolution(
        &self,
        scope: TenantScope,
        request_id: &str,
    ) -> Result<Option<DeliveryUncertaintyResolution>, StorageError> {
        let mut tx = self.pool().begin().await?;
        let pending = sqlx::query("SELECT pending.*,command.acknowledged_at FROM delivery_uncertainty_resolution_commands pending JOIN agent_commands command ON command.workspace_id=pending.workspace_id AND command.environment_id=pending.environment_id AND command.agent_id=pending.agent_id AND command.cursor=pending.agent_command_cursor WHERE pending.workspace_id=$1 AND pending.environment_id=$2 AND pending.request_id=$3 FOR UPDATE OF pending")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(request_id).fetch_optional(&mut *tx).await?.ok_or(StorageError::NotFound)?;
        if pending
            .try_get::<Option<DateTime<Utc>>, _>("acknowledged_at")?
            .is_none()
        {
            tx.commit().await?;
            return Ok(None);
        }
        if pending
            .try_get::<Option<DateTime<Utc>>, _>("finalized_at")?
            .is_some()
        {
            let row = sqlx::query("SELECT id,job_id,attempt_id,destination_id,resolution,note,actor_id,request_id,created_at FROM delivery_uncertainty_resolutions WHERE workspace_id=$1 AND environment_id=$2 AND request_id=$3")
                .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(request_id).fetch_one(&mut *tx).await?;
            tx.commit().await?;
            return map_uncertainty_resolution(&row).map(Some);
        }
        let id = format!("dur_{}", ulid::Ulid::new());
        let row = sqlx::query("INSERT INTO delivery_uncertainty_resolutions (workspace_id,environment_id,id,job_id,attempt_id,destination_id,resolution,note,actor_id,request_id) SELECT workspace_id,environment_id,$4,job_id,attempt_id,destination_id,resolution,note,actor_id,request_id FROM delivery_uncertainty_resolution_commands WHERE workspace_id=$1 AND environment_id=$2 AND request_id=$3 RETURNING id,job_id,attempt_id,destination_id,resolution,note,actor_id,request_id,created_at")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(request_id).bind(id).fetch_one(&mut *tx).await.map_err(reservation_write_error)?;
        let destination_id: String = row.try_get("destination_id")?;
        sqlx::query("UPDATE delivery_uncertainty_resolution_commands SET finalized_at=now() WHERE workspace_id=$1 AND environment_id=$2 AND request_id=$3")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(request_id).execute(&mut *tx).await?;
        let unresolved: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM delivery_attempts uncertain WHERE uncertain.workspace_id=$1 AND uncertain.environment_id=$2 AND uncertain.destination_id=$3 AND uncertain.state='delivery_uncertain' AND NOT EXISTS (SELECT 1 FROM delivery_uncertainty_resolutions resolved WHERE resolved.workspace_id=uncertain.workspace_id AND resolved.environment_id=uncertain.environment_id AND resolved.attempt_id=uncertain.id))")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(&destination_id).fetch_one(&mut *tx).await?;
        if !unresolved {
            sqlx::query("UPDATE physical_destinations SET state='available',updated_at=now() WHERE workspace_id=$1 AND environment_id=$2 AND id=$3 AND state='attention'")
                .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(&destination_id).execute(&mut *tx).await?;
        }
        let resolution_name: String = row.try_get("resolution")?;
        let resolved_at = Utc::now();
        sqlx::query("UPDATE jobs SET payload=jsonb_set(jsonb_set(jsonb_set(payload,'{metadata,piqae.delivery_resolution}',to_jsonb($4::text),true),'{metadata,piqae.delivery_resolution_request_id}',to_jsonb($5::text),true),'{metadata,piqae.delivery_resolved_at}',to_jsonb($6::text),true),updated_at=now() WHERE workspace_id=$1 AND environment_id=$2 AND id=$3")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(row.try_get::<String,_>("job_id")?).bind(&resolution_name).bind(request_id).bind(resolved_at.to_rfc3339()).execute(&mut *tx).await?;
        tx.commit().await?;
        map_uncertainty_resolution(&row).map(Some)
    }

    async fn finalize_acknowledged_uncertainty_resolutions(
        &self,
        scope: TenantScope,
        agent_id: &str,
        limit: u32,
    ) -> Result<Vec<DeliveryUncertaintyResolution>, StorageError> {
        let request_ids: Vec<String> = sqlx::query_scalar("SELECT pending.request_id FROM delivery_uncertainty_resolution_commands pending JOIN agent_commands command ON command.workspace_id=pending.workspace_id AND command.environment_id=pending.environment_id AND command.agent_id=pending.agent_id AND command.cursor=pending.agent_command_cursor WHERE pending.workspace_id=$1 AND pending.environment_id=$2 AND pending.agent_id=$3 AND pending.finalized_at IS NULL AND command.acknowledged_at IS NOT NULL ORDER BY pending.created_at,pending.request_id LIMIT $4")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(agent_id).bind(i64::from(limit.clamp(1,100))).fetch_all(self.pool()).await?;
        let mut resolutions = Vec::with_capacity(request_ids.len());
        for request_id in request_ids {
            if let Some(resolution) = self
                .finalize_delivery_uncertainty_resolution(scope, &request_id)
                .await?
            {
                resolutions.push(resolution);
            }
        }
        // Acknowledgement finalization and replacement-job creation cross the
        // topology/job repository boundary. Keep finalized reprint intents
        // visible until an idempotently linked replacement is durable so a
        // process crash cannot lose an authorized reprint.
        let remaining = i64::from(limit.clamp(1, 100))
            .saturating_sub(i64::try_from(resolutions.len()).unwrap_or(i64::MAX));
        if remaining > 0 {
            let rows = sqlx::query(
                "SELECT resolution.id,resolution.job_id,resolution.attempt_id,
                        resolution.destination_id,resolution.resolution,resolution.note,
                        resolution.actor_id,resolution.request_id,resolution.created_at
                 FROM delivery_uncertainty_resolutions resolution
                 JOIN delivery_uncertainty_resolution_commands pending
                   ON pending.workspace_id=resolution.workspace_id
                  AND pending.environment_id=resolution.environment_id
                  AND pending.request_id=resolution.request_id
                 WHERE resolution.workspace_id=$1 AND resolution.environment_id=$2
                   AND pending.agent_id=$3 AND resolution.resolution='reprint_authorized'
                   AND NOT EXISTS (
                     SELECT 1 FROM jobs replacement
                     WHERE replacement.workspace_id=resolution.workspace_id
                       AND replacement.environment_id=resolution.environment_id
                       AND replacement.payload #>> '{metadata,piqae.uncertainty_resolution_id}' = resolution.id
                       AND replacement.state <> 'registered'
                   )
                 ORDER BY resolution.created_at,resolution.request_id LIMIT $4",
            )
            .bind(scope.workspace_id.to_string())
            .bind(scope.environment_id.to_string())
            .bind(agent_id)
            .bind(remaining)
            .fetch_all(self.pool())
            .await?;
            let already_returned: std::collections::HashSet<_> = resolutions
                .iter()
                .map(|resolution| resolution.request_id.clone())
                .collect();
            for row in rows {
                let resolution = map_uncertainty_resolution(&row)?;
                if !already_returned.contains(&resolution.request_id) {
                    resolutions.push(resolution);
                }
            }
        }
        Ok(resolutions)
    }

    async fn acknowledge_uncertainty_resolution_command(
        &self,
        _scope: TenantScope,
        _agent_command_cursor: u64,
    ) -> Result<(), StorageError> {
        Err(StorageError::InvalidData(
            "PostgreSQL uncertainty commands are acknowledged only by agent command sync".into(),
        ))
    }

    async fn has_unresolved_destination_uncertainty(
        &self,
        scope: TenantScope,
        destination_id: &str,
    ) -> Result<bool, StorageError> {
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM delivery_attempts attempt WHERE attempt.workspace_id=$1 AND attempt.environment_id=$2 AND attempt.destination_id=$3 AND attempt.state='delivery_uncertain' AND NOT EXISTS (SELECT 1 FROM delivery_uncertainty_resolutions resolution WHERE resolution.workspace_id=attempt.workspace_id AND resolution.environment_id=attempt.environment_id AND resolution.attempt_id=attempt.id))")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(destination_id).fetch_one(self.pool()).await?)
    }

    async fn recompute_destination_attention(
        &self,
        scope: TenantScope,
        destination_id: &str,
    ) -> Result<StoredPhysicalDestination, StorageError> {
        let mut tx = self.pool().begin().await?;
        let unresolved: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM delivery_attempts attempt WHERE attempt.workspace_id=$1 AND attempt.environment_id=$2 AND attempt.destination_id=$3 AND attempt.state='delivery_uncertain' AND NOT EXISTS (SELECT 1 FROM delivery_uncertainty_resolutions resolution WHERE resolution.workspace_id=attempt.workspace_id AND resolution.environment_id=attempt.environment_id AND resolution.attempt_id=attempt.id))")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(destination_id).fetch_one(&mut *tx).await?;
        let row = sqlx::query("UPDATE physical_destinations SET state=CASE WHEN $4 THEN 'attention' WHEN state='attention' THEN 'available' ELSE state END,updated_at=now() WHERE workspace_id=$1 AND environment_id=$2 AND id=$3 RETURNING id,name,identity_confidence,state,scheduling_authority_id,identity_revision,updated_at")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(destination_id).bind(unresolved).fetch_optional(&mut *tx).await?.ok_or(StorageError::NotFound)?;
        tx.commit().await?;
        map_destination(&row)
    }

    async fn list_delivery_attempts(
        &self,
        scope: TenantScope,
        job_id: &str,
    ) -> Result<Vec<DeliveryAttempt>, StorageError> {
        let rows = sqlx::query("SELECT id,job_id,destination_id,route_id,generation,state,lease_until,accepted_at,handoff_started_at,spooler_accepted_at,final_at,created_at,updated_at FROM delivery_attempts WHERE workspace_id=$1 AND environment_id=$2 AND job_id=$3 ORDER BY generation,id")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(job_id).fetch_all(self.pool()).await?;
        rows.iter().map(map_attempt).collect()
    }

    async fn get_latest_delivery_attempt(
        &self,
        scope: TenantScope,
        job_id: &str,
    ) -> Result<DeliveryAttempt, StorageError> {
        let row = sqlx::query("SELECT id,job_id,destination_id,route_id,generation,state,lease_until,accepted_at,handoff_started_at,spooler_accepted_at,final_at,created_at,updated_at FROM delivery_attempts WHERE workspace_id=$1 AND environment_id=$2 AND job_id=$3 ORDER BY generation DESC,id DESC LIMIT 1")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(job_id).fetch_optional(self.pool()).await?.ok_or(StorageError::NotFound)?;
        map_attempt(&row)
    }

    async fn get_delivery_attempt_by_reservation(
        &self,
        scope: TenantScope,
        reservation_id: &str,
    ) -> Result<DeliveryAttempt, StorageError> {
        let row = sqlx::query("SELECT attempt.id,attempt.job_id,attempt.destination_id,attempt.route_id,attempt.generation,attempt.state,attempt.lease_until,attempt.accepted_at,attempt.handoff_started_at,attempt.spooler_accepted_at,attempt.final_at,attempt.created_at,attempt.updated_at FROM route_reservations reservation JOIN delivery_attempts attempt ON attempt.workspace_id=reservation.workspace_id AND attempt.environment_id=reservation.environment_id AND attempt.id=reservation.attempt_id WHERE reservation.workspace_id=$1 AND reservation.environment_id=$2 AND reservation.id=$3")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(reservation_id).fetch_optional(self.pool()).await?.ok_or(StorageError::NotFound)?;
        map_attempt(&row)
    }

    async fn list_route_reservations(
        &self,
        scope: TenantScope,
        limit: u32,
    ) -> Result<Vec<RouteReservation>, StorageError> {
        let rows = sqlx::query("SELECT id,route_id,destination_id,job_id,attempt_id,generation,state,lease_until,released_at,created_at FROM route_reservations WHERE workspace_id=$1 AND environment_id=$2 ORDER BY created_at DESC,id DESC LIMIT $3")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(i64::from(limit.clamp(1,1_000))).fetch_all(self.pool()).await?;
        rows.iter().map(map_reservation).collect()
    }
}

async fn apply_identity_decision(
    store: &PostgresStore,
    scope: TenantScope,
    decision: &IdentityDecision,
) -> Result<(), StorageError> {
    let mut transaction = store.pool().begin().await?;
    let mut destination_ids = vec![decision.destination_id.clone()];
    for destination_id in &decision.related_destination_ids {
        if !destination_ids.contains(destination_id) {
            destination_ids.push(destination_id.clone());
        }
    }
    let mut destination_snapshot = Vec::with_capacity(destination_ids.len());
    for destination_id in &destination_ids {
        let row = sqlx::query("SELECT id,state,identity_confidence,retired_at FROM physical_destinations WHERE workspace_id=$1 AND environment_id=$2 AND id=$3 FOR UPDATE")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(destination_id).fetch_optional(&mut *transaction).await?.ok_or(StorageError::NotFound)?;
        destination_snapshot.push(serde_json::json!({
            "id": row.try_get::<String,_>("id")?,
            "state": row.try_get::<String,_>("state")?,
            "identity_confidence": row.try_get::<String,_>("identity_confidence")?,
            "retired_at": row.try_get::<Option<DateTime<Utc>>,_>("retired_at")?,
        }));
    }
    let mut route_snapshot = Vec::with_capacity(decision.route_ids.len());
    for route_id in &decision.route_ids {
        let row = sqlx::query("SELECT id,destination_id,role FROM printer_routes WHERE workspace_id=$1 AND environment_id=$2 AND id=$3 AND retired_at IS NULL FOR UPDATE")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(route_id).fetch_optional(&mut *transaction).await?.ok_or(StorageError::NotFound)?;
        route_snapshot.push(serde_json::json!({
            "id": row.try_get::<String,_>("id")?,
            "destination_id": row.try_get::<String,_>("destination_id")?,
            "role": row.try_get::<String,_>("role")?,
        }));
    }
    let effect_snapshot = serde_json::json!({
        "destinations": destination_snapshot,
        "routes": route_snapshot,
    });
    if !decision.evidence_ids.is_empty() {
        let evidence_count: i64 = sqlx::query_scalar("SELECT count(*) FROM destination_identity_evidence WHERE workspace_id=$1 AND environment_id=$2 AND id=ANY($3)")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(&decision.evidence_ids).fetch_one(&mut *transaction).await?;
        if usize::try_from(evidence_count).ok() != Some(decision.evidence_ids.len()) {
            return Err(StorageError::NotFound);
        }
    }

    match decision.kind {
        IdentityDecisionKind::Merge | IdentityDecisionKind::Split => {
            if decision.route_ids.is_empty() || decision.related_destination_ids.is_empty() {
                return Err(StorageError::InvalidData(
                    "merge and split decisions require source destinations and routes".into(),
                ));
            }
            for route in effect_snapshot["routes"].as_array().into_iter().flatten() {
                let source = route["destination_id"].as_str().ok_or_else(|| {
                    StorageError::InvalidData("identity route snapshot is malformed".into())
                })?;
                if !decision
                    .related_destination_ids
                    .iter()
                    .any(|id| id == source)
                {
                    return Err(StorageError::InvalidData(
                        "identity route does not belong to a declared source destination".into(),
                    ));
                }
            }
            let route_busy: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM route_reservations WHERE workspace_id=$1 AND environment_id=$2 AND route_id=ANY($3) AND state='active') OR EXISTS(SELECT 1 FROM delivery_attempts WHERE workspace_id=$1 AND environment_id=$2 AND route_id=ANY($3) AND final_at IS NULL)")
                .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(&decision.route_ids).fetch_one(&mut *transaction).await?;
            if route_busy {
                return Err(StorageError::ConcurrentStateChange);
            }
            sqlx::query("UPDATE printer_routes SET role='standby',updated_at=now() WHERE workspace_id=$1 AND environment_id=$2 AND id=ANY($3)")
                .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(&decision.route_ids).execute(&mut *transaction).await?;
            sqlx::query("UPDATE printer_routes SET destination_id=$3,updated_at=now() WHERE workspace_id=$1 AND environment_id=$2 AND id=ANY($4)")
                .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(&decision.destination_id).bind(&decision.route_ids).execute(&mut *transaction).await?;
            sqlx::query("UPDATE destination_identity_evidence SET destination_id=$3 WHERE workspace_id=$1 AND environment_id=$2 AND route_id=ANY($4)")
                .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(&decision.destination_id).bind(&decision.route_ids).execute(&mut *transaction).await?;
            let has_primary: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM printer_routes WHERE workspace_id=$1 AND environment_id=$2 AND destination_id=$3 AND role='primary' AND enabled AND retired_at IS NULL)")
                .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(&decision.destination_id).fetch_one(&mut *transaction).await?;
            if !has_primary {
                sqlx::query("UPDATE printer_routes SET role='primary',updated_at=now() WHERE workspace_id=$1 AND environment_id=$2 AND id=$3")
                    .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(&decision.route_ids[0]).execute(&mut *transaction).await?;
            }
            sqlx::query("UPDATE target_bindings binding SET destination_id=route.destination_id,updated_at=now() FROM printer_routes route WHERE binding.workspace_id=$1 AND binding.environment_id=$2 AND route.workspace_id=binding.workspace_id AND route.environment_id=binding.environment_id AND route.id=binding.route_id AND route.id=ANY($3)")
                .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(&decision.route_ids).execute(&mut *transaction).await?;
            sqlx::query("UPDATE jobs job SET destination_id=route.destination_id,updated_at=now() FROM printer_routes route WHERE job.workspace_id=$1 AND job.environment_id=$2 AND route.workspace_id=job.workspace_id AND route.environment_id=job.environment_id AND route.id=job.route_id AND route.id=ANY($3)")
                .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(&decision.route_ids).execute(&mut *transaction).await?;
            if decision.kind == IdentityDecisionKind::Merge {
                sqlx::query("UPDATE physical_destinations destination SET state='retired',retired_at=now(),updated_at=now() WHERE destination.workspace_id=$1 AND destination.environment_id=$2 AND destination.id=ANY($3) AND NOT EXISTS (SELECT 1 FROM printer_routes route WHERE route.workspace_id=destination.workspace_id AND route.environment_id=destination.environment_id AND route.destination_id=destination.id AND route.retired_at IS NULL)")
                    .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(&decision.related_destination_ids).execute(&mut *transaction).await?;
            }
        }
        IdentityDecisionKind::Confirm => {
            sqlx::query("UPDATE physical_destinations SET identity_confidence='verified',identity_revision=identity_revision+1,updated_at=now() WHERE workspace_id=$1 AND environment_id=$2 AND id=$3")
                .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(&decision.destination_id).execute(&mut *transaction).await?;
        }
        IdentityDecisionKind::RejectMatch => {
            sqlx::query("UPDATE physical_destinations SET identity_confidence='conflict',identity_revision=identity_revision+1,updated_at=now() WHERE workspace_id=$1 AND environment_id=$2 AND id=ANY($3)")
                .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(&destination_ids).execute(&mut *transaction).await?;
        }
        IdentityDecisionKind::Reverse => {
            return Err(StorageError::InvalidData(
                "reverse decisions require reverse_identity_decision".into(),
            ));
        }
    }
    insert_decision_row(
        &mut transaction,
        scope,
        decision,
        &effect_snapshot,
        &decision.route_ids,
    )
    .await?;
    transaction.commit().await?;
    Ok(())
}

async fn reverse_applied_identity_decision(
    store: &PostgresStore,
    scope: TenantScope,
    reversal: &IdentityDecision,
) -> Result<(), StorageError> {
    let original_id = reversal
        .reverses_decision_id
        .as_deref()
        .ok_or_else(|| StorageError::InvalidData("missing reversed decision".into()))?;
    let mut transaction = store.pool().begin().await?;
    let snapshot: serde_json::Value = sqlx::query_scalar("SELECT effect_snapshot FROM destination_identity_decisions WHERE workspace_id=$1 AND environment_id=$2 AND id=$3 AND kind <> 'reverse' FOR UPDATE")
        .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(original_id).fetch_optional(&mut *transaction).await?.ok_or(StorageError::NotFound)?;
    let routes = snapshot["routes"].as_array().ok_or_else(|| {
        StorageError::InvalidData("stored identity route snapshot is malformed".into())
    })?;
    let route_ids: Vec<String> = routes
        .iter()
        .map(|route| {
            route["id"].as_str().map(str::to_owned).ok_or_else(|| {
                StorageError::InvalidData("stored identity route ID is malformed".into())
            })
        })
        .collect::<Result<_, _>>()?;
    let route_busy: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM route_reservations WHERE workspace_id=$1 AND environment_id=$2 AND route_id=ANY($3) AND state='active') OR EXISTS(SELECT 1 FROM delivery_attempts WHERE workspace_id=$1 AND environment_id=$2 AND route_id=ANY($3) AND final_at IS NULL)")
        .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(&route_ids).fetch_one(&mut *transaction).await?;
    if route_busy {
        return Err(StorageError::ConcurrentStateChange);
    }
    sqlx::query("UPDATE printer_routes SET role='standby',updated_at=now() WHERE workspace_id=$1 AND environment_id=$2 AND id=ANY($3)")
        .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(&route_ids).execute(&mut *transaction).await?;
    for destination in snapshot["destinations"].as_array().into_iter().flatten() {
        sqlx::query("UPDATE physical_destinations SET state=$4,identity_confidence=$5,retired_at=$6,identity_revision=identity_revision+1,updated_at=now() WHERE workspace_id=$1 AND environment_id=$2 AND id=$3")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(destination["id"].as_str()).bind(destination["state"].as_str()).bind(destination["identity_confidence"].as_str()).bind(serde_json::from_value::<Option<DateTime<Utc>>>(destination["retired_at"].clone())?).execute(&mut *transaction).await?;
    }
    for route in routes {
        sqlx::query("UPDATE printer_routes SET destination_id=$4,role=$5,updated_at=now() WHERE workspace_id=$1 AND environment_id=$2 AND id=$3")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(route["id"].as_str()).bind(route["destination_id"].as_str()).bind(route["role"].as_str()).execute(&mut *transaction).await.map_err(reservation_write_error)?;
    }
    sqlx::query("UPDATE destination_identity_evidence evidence SET destination_id=route.destination_id FROM printer_routes route WHERE evidence.workspace_id=$1 AND evidence.environment_id=$2 AND route.workspace_id=evidence.workspace_id AND route.environment_id=evidence.environment_id AND route.id=evidence.route_id AND route.id=ANY($3)")
        .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(&route_ids).execute(&mut *transaction).await?;
    sqlx::query("UPDATE target_bindings binding SET destination_id=route.destination_id,updated_at=now() FROM printer_routes route WHERE binding.workspace_id=$1 AND binding.environment_id=$2 AND route.workspace_id=binding.workspace_id AND route.environment_id=binding.environment_id AND route.id=binding.route_id AND route.id=ANY($3)")
        .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(&route_ids).execute(&mut *transaction).await?;
    sqlx::query("UPDATE jobs job SET destination_id=route.destination_id,updated_at=now() FROM printer_routes route WHERE job.workspace_id=$1 AND job.environment_id=$2 AND route.workspace_id=job.workspace_id AND route.environment_id=job.environment_id AND route.id=job.route_id AND route.id=ANY($3)")
        .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(&route_ids).execute(&mut *transaction).await?;
    insert_decision_row(
        &mut transaction,
        scope,
        reversal,
        &serde_json::json!({"reversed":original_id}),
        &route_ids,
    )
    .await?;
    transaction.commit().await?;
    Ok(())
}

async fn insert_decision_row(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    scope: TenantScope,
    decision: &IdentityDecision,
    effect_snapshot: &serde_json::Value,
    route_ids: &[String],
) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO destination_identity_decisions (workspace_id,environment_id,id,kind,destination_id,related_destination_ids,evidence_ids,effect_snapshot,actor_kind,actor_id,reason,reverses_decision_id,request_id,created_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)")
        .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(&decision.id).bind(decision.kind.as_str()).bind(&decision.destination_id).bind(serde_json::to_value(&decision.related_destination_ids)?).bind(serde_json::to_value(&decision.evidence_ids)?).bind(effect_snapshot).bind(&decision.actor_kind).bind(&decision.actor_id).bind(&decision.reason).bind(&decision.reverses_decision_id).bind(&decision.request_id).bind(decision.created_at).execute(&mut **transaction).await.map_err(reservation_write_error)?;
    for route_id in route_ids {
        sqlx::query("INSERT INTO destination_identity_decision_routes (workspace_id,environment_id,decision_id,route_id) VALUES ($1,$2,$3,$4)")
            .bind(scope.workspace_id.to_string()).bind(scope.environment_id.to_string()).bind(&decision.id).bind(route_id).execute(&mut **transaction).await?;
    }
    Ok(())
}

#[derive(Clone, Debug, Default)]
pub struct MemoryDestinationTopologyRepository {
    state: Arc<RwLock<MemoryState>>,
}

#[derive(Debug, Default)]
struct MemoryState {
    authorities: HashMap<(TenantScope, String), SchedulingAuthority>,
    destinations: HashMap<(TenantScope, String), StoredPhysicalDestination>,
    routes: HashMap<(TenantScope, String), StoredPrinterRoute>,
    evidence: HashMap<(TenantScope, String), IdentityEvidence>,
    decisions: HashMap<(TenantScope, String), IdentityDecision>,
    decision_snapshots: HashMap<(TenantScope, String), MemoryDecisionSnapshot>,
    observations: HashMap<(TenantScope, String), Vec<RouteObservation>>,
    acknowledgements: HashMap<(TenantScope, String, String), ProjectionAcknowledgement>,
    memberships: HashMap<(TenantScope, String, String), SiteCoordinatorMembership>,
    attempts: HashMap<(TenantScope, String), (DeliveryAttempt, String)>,
    reservations: HashMap<(TenantScope, String), RouteReservation>,
    uncertainty_resolutions: HashMap<(TenantScope, String), DeliveryUncertaintyResolution>,
    pending_uncertainty_resolutions:
        HashMap<(TenantScope, String), PendingDeliveryUncertaintyResolution>,
    acknowledged_uncertainty_commands: HashSet<(TenantScope, u64)>,
    next_uncertainty_command_cursor: u64,
}

#[derive(Clone, Debug, Default)]
struct MemoryDecisionSnapshot {
    destinations: Vec<StoredPhysicalDestination>,
    routes: Vec<StoredPrinterRoute>,
}

fn read_state(
    repository: &MemoryDestinationTopologyRepository,
) -> Result<std::sync::RwLockReadGuard<'_, MemoryState>, StorageError> {
    repository
        .state
        .read()
        .map_err(|_| StorageError::InvalidData("memory topology lock poisoned".into()))
}

fn write_state(
    repository: &MemoryDestinationTopologyRepository,
) -> Result<std::sync::RwLockWriteGuard<'_, MemoryState>, StorageError> {
    repository
        .state
        .write()
        .map_err(|_| StorageError::InvalidData("memory topology lock poisoned".into()))
}

#[async_trait]
impl DestinationTopologyRepository for MemoryDestinationTopologyRepository {
    async fn upsert_scheduling_authority(
        &self,
        scope: TenantScope,
        authority: &SchedulingAuthority,
    ) -> Result<(), StorageError> {
        write_state(self)?
            .authorities
            .insert((scope, authority.id.clone()), authority.clone());
        Ok(())
    }
    async fn upsert_destination(
        &self,
        scope: TenantScope,
        destination: &StoredPhysicalDestination,
    ) -> Result<(), StorageError> {
        write_state(self)?
            .destinations
            .insert((scope, destination.id.clone()), destination.clone());
        Ok(())
    }
    async fn get_destination(
        &self,
        scope: TenantScope,
        id: &str,
    ) -> Result<StoredPhysicalDestination, StorageError> {
        read_state(self)?
            .destinations
            .get(&(scope, id.to_owned()))
            .cloned()
            .ok_or(StorageError::NotFound)
    }
    async fn list_destinations(
        &self,
        scope: TenantScope,
    ) -> Result<Vec<StoredPhysicalDestination>, StorageError> {
        Ok(read_state(self)?
            .destinations
            .iter()
            .filter(|((tenant, _), _)| *tenant == scope)
            .map(|(_, value)| value.clone())
            .collect())
    }
    async fn upsert_route(
        &self,
        scope: TenantScope,
        route: &StoredPrinterRoute,
    ) -> Result<(), StorageError> {
        let state = read_state(self)?;
        if !state
            .destinations
            .contains_key(&(scope, route.destination_id.clone()))
        {
            return Err(StorageError::NotFound);
        }
        let existing = state
            .routes
            .iter()
            .find(|((tenant, _), stored)| {
                *tenant == scope
                    && stored.printer_id == route.printer_id
                    && stored.agent_id == route.agent_id
            })
            .map(|((_, id), stored)| (id.clone(), stored.local_route_key.clone()));
        drop(state);
        let mut stored = route.clone();
        if let Some((id, local_route_key)) = existing {
            stored.id = id;
            if stored.local_route_key.is_none() {
                stored.local_route_key = local_route_key;
            }
        }
        write_state(self)?
            .routes
            .insert((scope, stored.id.clone()), stored);
        Ok(())
    }
    async fn get_route(
        &self,
        scope: TenantScope,
        route_id: &str,
    ) -> Result<StoredPrinterRoute, StorageError> {
        read_state(self)?
            .routes
            .get(&(scope, route_id.to_owned()))
            .cloned()
            .ok_or(StorageError::NotFound)
    }
    async fn get_route_by_local_key(
        &self,
        scope: TenantScope,
        agent_id: &str,
        local_route_key: &str,
    ) -> Result<StoredPrinterRoute, StorageError> {
        read_state(self)?
            .routes
            .iter()
            .find(|((tenant, _), route)| {
                *tenant == scope
                    && route.agent_id == agent_id
                    && route.local_route_key.as_deref() == Some(local_route_key)
            })
            .map(|(_, route)| route.clone())
            .ok_or(StorageError::NotFound)
    }
    async fn list_all_routes(
        &self,
        scope: TenantScope,
    ) -> Result<Vec<StoredPrinterRoute>, StorageError> {
        Ok(read_state(self)?
            .routes
            .iter()
            .filter(|((tenant, _), _)| *tenant == scope)
            .map(|(_, value)| value.clone())
            .collect())
    }
    async fn list_routes(
        &self,
        scope: TenantScope,
        destination_id: &str,
    ) -> Result<Vec<StoredPrinterRoute>, StorageError> {
        Ok(read_state(self)?
            .routes
            .iter()
            .filter(|((tenant, _), route)| {
                *tenant == scope && route.destination_id == destination_id
            })
            .map(|(_, value)| value.clone())
            .collect())
    }
    async fn record_identity_evidence(
        &self,
        scope: TenantScope,
        evidence: &IdentityEvidence,
    ) -> Result<(), StorageError> {
        let mut state = write_state(self)?;
        if !state
            .routes
            .contains_key(&(scope, evidence.route_id.clone()))
        {
            return Err(StorageError::NotFound);
        }
        if state
            .routes
            .get(&(scope, evidence.route_id.clone()))
            .map(|route| route.destination_id.as_str())
            != Some(evidence.destination_id.as_str())
        {
            return Err(StorageError::NotFound);
        }
        state
            .evidence
            .insert((scope, evidence.id.clone()), evidence.clone());
        if evidence.conflicts {
            if let Some(destination) = state
                .destinations
                .get_mut(&(scope, evidence.destination_id.clone()))
            {
                destination.identity_confidence = IdentityConfidence::Conflict;
                destination.identity_revision = destination.identity_revision.saturating_add(1);
            }
        }
        Ok(())
    }
    async fn list_identity_evidence(
        &self,
        scope: TenantScope,
        destination_id: &str,
    ) -> Result<Vec<IdentityEvidence>, StorageError> {
        Ok(read_state(self)?
            .evidence
            .iter()
            .filter(|((tenant, _), item)| *tenant == scope && item.destination_id == destination_id)
            .map(|(_, value)| value.clone())
            .collect())
    }
    async fn record_identity_decision(
        &self,
        scope: TenantScope,
        decision: &IdentityDecision,
    ) -> Result<(), StorageError> {
        if decision.kind == IdentityDecisionKind::Reverse {
            return Err(StorageError::InvalidData(
                "reverse decisions require reverse_identity_decision".into(),
            ));
        }
        let mut state = write_state(self)?;
        if decision
            .route_ids
            .iter()
            .any(|route_id| !state.routes.contains_key(&(scope, route_id.clone())))
        {
            return Err(StorageError::NotFound);
        }
        if decision
            .evidence_ids
            .iter()
            .any(|evidence_id| !state.evidence.contains_key(&(scope, evidence_id.clone())))
        {
            return Err(StorageError::NotFound);
        }
        if decision.route_ids.iter().any(|route_id| {
            state.reservations.iter().any(|((tenant, _), reservation)| {
                *tenant == scope
                    && reservation.route_id == *route_id
                    && reservation.state == "active"
            }) || state.attempts.iter().any(|((tenant, _), (attempt, _))| {
                *tenant == scope && attempt.route_id == *route_id && !attempt.state.is_final()
            })
        }) {
            return Err(StorageError::ConcurrentStateChange);
        }
        let destination_ids = std::iter::once(&decision.destination_id)
            .chain(decision.related_destination_ids.iter());
        let snapshot = MemoryDecisionSnapshot {
            destinations: destination_ids
                .filter_map(|id| state.destinations.get(&(scope, id.clone())).cloned())
                .collect(),
            routes: decision
                .route_ids
                .iter()
                .filter_map(|id| state.routes.get(&(scope, id.clone())).cloned())
                .collect(),
        };
        match decision.kind {
            IdentityDecisionKind::Merge | IdentityDecisionKind::Split => {
                if decision.route_ids.is_empty() || decision.related_destination_ids.is_empty() {
                    return Err(StorageError::InvalidData(
                        "merge and split decisions require source destinations and routes".into(),
                    ));
                }
                for route_id in &decision.route_ids {
                    let route = state
                        .routes
                        .get(&(scope, route_id.clone()))
                        .ok_or(StorageError::NotFound)?;
                    if !decision
                        .related_destination_ids
                        .contains(&route.destination_id)
                    {
                        return Err(StorageError::InvalidData(
                            "identity route does not belong to a declared source destination"
                                .into(),
                        ));
                    }
                }
                for route_id in &decision.route_ids {
                    if let Some(route) = state.routes.get_mut(&(scope, route_id.clone())) {
                        route.destination_id.clone_from(&decision.destination_id);
                        route.role = "standby".into();
                        route.updated_at = Utc::now();
                    }
                }
                for ((tenant, _), evidence) in &mut state.evidence {
                    if *tenant != scope || !decision.route_ids.contains(&evidence.route_id) {
                        continue;
                    }
                    evidence.destination_id.clone_from(&decision.destination_id);
                }
                let has_primary = state.routes.iter().any(|((tenant, _), route)| {
                    *tenant == scope
                        && route.destination_id == decision.destination_id
                        && route.role == "primary"
                        && route.enabled
                });
                if !has_primary {
                    if let Some(route) = state
                        .routes
                        .get_mut(&(scope, decision.route_ids[0].clone()))
                    {
                        route.role = "primary".into();
                    }
                }
                if decision.kind == IdentityDecisionKind::Merge {
                    for source in &decision.related_destination_ids {
                        let has_route = state.routes.iter().any(|((tenant, _), route)| {
                            *tenant == scope && route.destination_id == *source
                        });
                        if !has_route {
                            if let Some(destination) =
                                state.destinations.get_mut(&(scope, source.clone()))
                            {
                                destination.state = "retired".into();
                            }
                        }
                    }
                }
            }
            IdentityDecisionKind::Confirm => {
                let destination = state
                    .destinations
                    .get_mut(&(scope, decision.destination_id.clone()))
                    .ok_or(StorageError::NotFound)?;
                destination.identity_confidence = IdentityConfidence::Verified;
                destination.identity_revision = destination.identity_revision.saturating_add(1);
            }
            IdentityDecisionKind::RejectMatch => {
                for id in std::iter::once(&decision.destination_id)
                    .chain(decision.related_destination_ids.iter())
                {
                    let destination = state
                        .destinations
                        .get_mut(&(scope, id.clone()))
                        .ok_or(StorageError::NotFound)?;
                    destination.identity_confidence = IdentityConfidence::Conflict;
                    destination.identity_revision = destination.identity_revision.saturating_add(1);
                }
            }
            IdentityDecisionKind::Reverse => unreachable!("validated above"),
        }
        state
            .decision_snapshots
            .insert((scope, decision.id.clone()), snapshot);
        state
            .decisions
            .insert((scope, decision.id.clone()), decision.clone());
        Ok(())
    }
    async fn reverse_identity_decision(
        &self,
        scope: TenantScope,
        reversal: &IdentityDecision,
    ) -> Result<(), StorageError> {
        let mut state = write_state(self)?;
        let Some(original) = reversal.reverses_decision_id.as_ref() else {
            return Err(StorageError::InvalidData(
                "missing reversed decision".into(),
            ));
        };
        if reversal.kind != IdentityDecisionKind::Reverse
            || !state.decisions.contains_key(&(scope, original.clone()))
        {
            return Err(StorageError::NotFound);
        }
        if state
            .decisions
            .values()
            .any(|decision| decision.reverses_decision_id.as_deref() == Some(original))
        {
            return Err(StorageError::ConcurrentStateChange);
        }
        let snapshot = state
            .decision_snapshots
            .get(&(scope, original.clone()))
            .cloned()
            .ok_or(StorageError::NotFound)?;
        if snapshot.routes.iter().any(|route| {
            state.reservations.iter().any(|((tenant, _), reservation)| {
                *tenant == scope
                    && reservation.route_id == route.id
                    && reservation.state == "active"
            }) || state.attempts.iter().any(|((tenant, _), (attempt, _))| {
                *tenant == scope && attempt.route_id.eq(&route.id) && !attempt.state.is_final()
            })
        }) {
            return Err(StorageError::ConcurrentStateChange);
        }
        for destination in snapshot.destinations {
            state
                .destinations
                .insert((scope, destination.id.clone()), destination);
        }
        for route in snapshot.routes {
            state.routes.insert((scope, route.id.clone()), route);
        }
        let restored_destinations: HashMap<_, _> = state
            .routes
            .iter()
            .filter(|((tenant, _), _)| *tenant == scope)
            .map(|((_, route_id), route)| (route_id.clone(), route.destination_id.clone()))
            .collect();
        for ((tenant, _), evidence) in &mut state.evidence {
            if *tenant != scope {
                continue;
            }
            if let Some(destination_id) = restored_destinations.get(&evidence.route_id) {
                evidence.destination_id.clone_from(destination_id);
            }
        }
        state
            .decisions
            .insert((scope, reversal.id.clone()), reversal.clone());
        Ok(())
    }
    async fn list_identity_decisions(
        &self,
        scope: TenantScope,
        destination_id: &str,
    ) -> Result<Vec<IdentityDecision>, StorageError> {
        let mut decisions: Vec<_> = read_state(self)?
            .decisions
            .iter()
            .filter(|((tenant, _), item)| *tenant == scope && item.destination_id == destination_id)
            .map(|(_, value)| value.clone())
            .collect();
        decisions.sort_by_key(|decision| (decision.created_at, decision.id.clone()));
        Ok(decisions)
    }
    async fn record_route_observation(
        &self,
        scope: TenantScope,
        observation: &RouteObservation,
    ) -> Result<(), StorageError> {
        let mut state = write_state(self)?;
        if !state
            .routes
            .contains_key(&(scope, observation.route_id.clone()))
        {
            return Err(StorageError::NotFound);
        }
        let existing_items = state
            .observations
            .get(&(scope, observation.route_id.clone()));
        if let Some(existing) = existing_items.and_then(|items| {
            items
                .iter()
                .find(|item| item.sequence == observation.sequence)
        }) {
            let mut existing = existing.clone();
            existing.id.clone_from(&observation.id);
            return if existing == *observation {
                Ok(())
            } else {
                Err(StorageError::IdempotencyConflict)
            };
        }
        if existing_items
            .and_then(|items| items.iter().map(|item| item.sequence).max())
            .is_some_and(|latest| observation.sequence <= latest)
        {
            return Err(StorageError::ConcurrentStateChange);
        }
        state
            .observations
            .entry((scope, observation.route_id.clone()))
            .or_default()
            .push(observation.clone());
        Ok(())
    }
    async fn latest_route_observation(
        &self,
        scope: TenantScope,
        route_id: &str,
    ) -> Result<RouteObservation, StorageError> {
        read_state(self)?
            .observations
            .get(&(scope, route_id.to_owned()))
            .and_then(|items| items.iter().max_by_key(|item| item.sequence))
            .cloned()
            .ok_or(StorageError::NotFound)
    }
    async fn list_route_observations(
        &self,
        scope: TenantScope,
        route_id: &str,
        limit: u32,
    ) -> Result<Vec<RouteObservation>, StorageError> {
        let mut items = read_state(self)?
            .observations
            .get(&(scope, route_id.to_owned()))
            .cloned()
            .unwrap_or_default();
        items.sort_by_key(|item| std::cmp::Reverse(item.sequence));
        items.truncate(usize::try_from(limit.clamp(1, 1_000)).unwrap_or(1_000));
        Ok(items)
    }
    async fn acknowledge_projection(
        &self,
        scope: TenantScope,
        acknowledgement: &ProjectionAcknowledgement,
    ) -> Result<(), StorageError> {
        let mut state = write_state(self)?;
        let key = (
            scope,
            acknowledgement.agent_id.clone(),
            acknowledgement.route_id.clone(),
        );
        if state.acknowledgements.get(&key).is_none_or(|existing| {
            existing.inventory_revision <= acknowledgement.inventory_revision
        }) {
            state.acknowledgements.insert(key, acknowledgement.clone());
        }
        Ok(())
    }
    async fn get_projection_acknowledgement(
        &self,
        scope: TenantScope,
        agent_id: &str,
        route_id: &str,
    ) -> Result<ProjectionAcknowledgement, StorageError> {
        read_state(self)?
            .acknowledgements
            .get(&(scope, agent_id.to_owned(), route_id.to_owned()))
            .cloned()
            .ok_or(StorageError::NotFound)
    }
    async fn upsert_site_membership(
        &self,
        scope: TenantScope,
        membership: &SiteCoordinatorMembership,
    ) -> Result<(), StorageError> {
        write_state(self)?.memberships.insert(
            (
                scope,
                membership.authority_id.clone(),
                membership.agent_id.clone(),
            ),
            membership.clone(),
        );
        Ok(())
    }
    async fn begin_delivery_attempt(
        &self,
        scope: TenantScope,
        request: NewDeliveryAttempt<'_>,
    ) -> Result<StartedDeliveryAttempt, StorageError> {
        let mut state = write_state(self)?;
        let now = Utc::now();
        if !state
            .routes
            .get(&(scope, request.route_id.to_owned()))
            .is_some_and(|route| route.destination_id == request.destination_id && route.enabled)
        {
            return Err(StorageError::NotFound);
        }
        let stale_attempt_keys: Vec<_> = state
            .attempts
            .iter()
            .filter_map(|(key @ (tenant, _), (attempt, _))| {
                (*tenant == scope
                    && attempt.destination_id == request.destination_id
                    && attempt.state == DeliveryAttemptState::RouteLeased
                    && attempt.lease_until <= now)
                    .then_some(key.clone())
            })
            .collect();
        let mut stale_attempt_ids = Vec::with_capacity(stale_attempt_keys.len());
        for key in stale_attempt_keys {
            if let Some((attempt, _)) = state.attempts.get_mut(&key) {
                attempt.state = DeliveryAttemptState::Superseded;
                attempt.final_at = Some(now);
                attempt.updated_at = now;
                stale_attempt_ids.push(attempt.id.clone());
            }
        }
        for ((tenant, _), reservation) in &mut state.reservations {
            if *tenant == scope
                && reservation.state == "active"
                && stale_attempt_ids.contains(&reservation.attempt_id)
            {
                reservation.state = "superseded".into();
                reservation.released_at = Some(now);
            }
        }
        let active_key = state
            .attempts
            .iter()
            .find(|((tenant, _), (attempt, _))| {
                *tenant == scope && attempt.job_id == request.job_id && !attempt.state.is_final()
            })
            .map(|(key, _)| key.clone());
        if let Some(active_key) = active_key {
            let active_id = {
                let (active, _) = state
                    .attempts
                    .get_mut(&active_key)
                    .ok_or(StorageError::ConcurrentStateChange)?;
                if active.state != DeliveryAttemptState::RouteLeased || active.lease_until > now {
                    return Err(StorageError::ConcurrentStateChange);
                }
                active.state = DeliveryAttemptState::Superseded;
                active.final_at = Some(now);
                active.updated_at = now;
                active.id.clone()
            };
            if let Some(reservation) =
                state
                    .reservations
                    .iter_mut()
                    .find_map(|((tenant, _), reservation)| {
                        (*tenant == scope
                            && reservation.attempt_id == active_id
                            && reservation.state == "active")
                            .then_some(reservation)
                    })
            {
                reservation.state = "superseded".into();
                reservation.released_at = Some(now);
            }
        } else if state.attempts.iter().any(|((tenant, _), (attempt, _))| {
            *tenant == scope
                && attempt.job_id == request.job_id
                && attempt.state == DeliveryAttemptState::DeliveryUncertain
        }) {
            return Err(StorageError::ConcurrentStateChange);
        }
        if state.reservations.iter().any(|((tenant, _), reservation)| {
            *tenant == scope
                && (reservation.route_id == request.route_id
                    || reservation.destination_id == request.destination_id)
                && reservation.state == "active"
        }) {
            return Err(StorageError::ConcurrentStateChange);
        }
        if state.attempts.iter().any(|((tenant, _), (attempt, _))| {
            *tenant == scope
                && attempt.destination_id == request.destination_id
                && attempt.state == DeliveryAttemptState::DeliveryUncertain
                && !state
                    .uncertainty_resolutions
                    .values()
                    .any(|resolution| resolution.attempt_id == attempt.id)
        }) {
            return Err(StorageError::ConcurrentStateChange);
        }
        let generation = state
            .attempts
            .iter()
            .filter(|((tenant, _), (attempt, _))| {
                *tenant == scope && attempt.job_id == request.job_id
            })
            .map(|(_, (attempt, _))| attempt.generation)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let token = new_fencing_token();
        let digest = token_digest(&token);
        let attempt = DeliveryAttempt {
            id: request.attempt_id.into(),
            job_id: request.job_id.into(),
            destination_id: request.destination_id.into(),
            route_id: request.route_id.into(),
            generation,
            state: DeliveryAttemptState::RouteLeased,
            lease_until: request.lease_until,
            accepted_at: None,
            handoff_started_at: None,
            spooler_accepted_at: None,
            final_at: None,
            created_at: now,
            updated_at: now,
        };
        let reservation = RouteReservation {
            id: request.reservation_id.into(),
            route_id: request.route_id.into(),
            destination_id: request.destination_id.into(),
            job_id: request.job_id.into(),
            attempt_id: request.attempt_id.into(),
            generation,
            state: "active".into(),
            lease_until: request.lease_until,
            released_at: None,
            acquired_at: now,
        };
        state
            .attempts
            .insert((scope, attempt.id.clone()), (attempt.clone(), digest));
        state
            .reservations
            .insert((scope, reservation.id.clone()), reservation.clone());
        Ok(StartedDeliveryAttempt {
            attempt,
            reservation,
            fencing_token: token,
        })
    }
    async fn transition_delivery_attempt(
        &self,
        scope: TenantScope,
        attempt_id: &str,
        generation: u64,
        fencing_token: &str,
        next: DeliveryAttemptState,
    ) -> Result<DeliveryAttempt, StorageError> {
        let mut state = write_state(self)?;
        let key = (scope, attempt_id.to_owned());
        let (attempt, digest) = state.attempts.get_mut(&key).ok_or(StorageError::NotFound)?;
        if attempt.generation != generation
            || *digest != token_digest(fencing_token)
            || !valid_attempt_transition(attempt.state, next)
        {
            return Err(StorageError::ConcurrentStateChange);
        }
        let now = Utc::now();
        attempt.state = next;
        attempt.updated_at = now;
        if next == DeliveryAttemptState::AcceptedByNode {
            attempt.accepted_at = Some(now);
        } else if next == DeliveryAttemptState::HandingToSpooler {
            attempt.handoff_started_at = Some(now);
        } else if next == DeliveryAttemptState::AcceptedBySpooler {
            attempt.spooler_accepted_at = Some(now);
        }
        let result = attempt.clone();
        if next.is_final() || next == DeliveryAttemptState::AcceptedBySpooler {
            attempt.final_at = Some(now);
            if !next.is_final() {
                attempt.final_at = None;
            }
            if let Some(reservation) =
                state
                    .reservations
                    .iter_mut()
                    .find_map(|((tenant, _), reservation)| {
                        (*tenant == scope
                            && reservation.attempt_id == attempt_id
                            && reservation.state == "active")
                            .then_some(reservation)
                    })
            {
                reservation.state = if next == DeliveryAttemptState::Superseded {
                    "superseded"
                } else if next == DeliveryAttemptState::Cancelled {
                    "cancelled"
                } else {
                    "released"
                }
                .into();
                reservation.released_at = Some(now);
            }
        }
        if next == DeliveryAttemptState::DeliveryUncertain {
            if let Some(destination) = state
                .destinations
                .get_mut(&(scope, result.destination_id.clone()))
            {
                destination.state = "attention".into();
                destination.updated_at = now;
            }
        }
        Ok(if next.is_final() {
            let mut result = result;
            result.final_at = Some(now);
            result
        } else {
            result
        })
    }
    async fn transition_post_spooler_attempt(
        &self,
        scope: TenantScope,
        job_id: &str,
        agent_id: &str,
        route_id: &str,
        next: DeliveryAttemptState,
    ) -> Result<DeliveryAttempt, StorageError> {
        if !matches!(
            next,
            DeliveryAttemptState::PrintingReported
                | DeliveryAttemptState::CompletedReported
                | DeliveryAttemptState::Failed
                | DeliveryAttemptState::DeliveryUncertain
        ) {
            return Err(StorageError::InvalidTransition);
        }
        let mut state = write_state(self)?;
        if state
            .routes
            .get(&(scope, route_id.to_owned()))
            .is_none_or(|route| route.agent_id != agent_id)
        {
            return Err(StorageError::NotFound);
        }
        let key = state
            .attempts
            .iter()
            .filter_map(|(key @ (tenant, _), (attempt, _))| {
                (*tenant == scope && attempt.job_id == job_id && attempt.route_id == route_id)
                    .then_some((key.clone(), attempt.generation))
            })
            .max_by_key(|(_, generation)| *generation)
            .map(|(key, _)| key)
            .ok_or(StorageError::NotFound)?;
        let now = Utc::now();
        let (attempt, _) = state.attempts.get_mut(&key).ok_or(StorageError::NotFound)?;
        if !matches!(
            attempt.state,
            DeliveryAttemptState::AcceptedBySpooler | DeliveryAttemptState::PrintingReported
        ) || !valid_attempt_transition(attempt.state, next)
        {
            return Err(StorageError::ConcurrentStateChange);
        }
        attempt.state = next;
        attempt.updated_at = now;
        attempt.final_at = next.is_final().then_some(now);
        let result = attempt.clone();
        if let Some(reservation) =
            state
                .reservations
                .iter_mut()
                .find_map(|((tenant, _), reservation)| {
                    (*tenant == scope
                        && reservation.attempt_id == result.id
                        && reservation.state == "active")
                        .then_some(reservation)
                })
        {
            reservation.state = "released".into();
            reservation.released_at = Some(now);
        }
        if next == DeliveryAttemptState::DeliveryUncertain {
            if let Some(destination) = state
                .destinations
                .get_mut(&(scope, result.destination_id.clone()))
            {
                destination.state = "attention".into();
                destination.updated_at = now;
            }
        }
        Ok(result)
    }
    async fn mark_post_spooler_attempt_uncertain(
        &self,
        scope: TenantScope,
        job_id: &str,
    ) -> Result<DeliveryAttempt, StorageError> {
        let mut state = write_state(self)?;
        let key = state
            .attempts
            .iter()
            .filter_map(|(key @ (tenant, _), (attempt, _))| {
                (*tenant == scope && attempt.job_id == job_id)
                    .then_some((key.clone(), attempt.generation))
            })
            .max_by_key(|(_, generation)| *generation)
            .map(|(key, _)| key)
            .ok_or(StorageError::NotFound)?;
        let now = Utc::now();
        let (attempt, _) = state.attempts.get_mut(&key).ok_or(StorageError::NotFound)?;
        if !matches!(
            attempt.state,
            DeliveryAttemptState::AcceptedBySpooler | DeliveryAttemptState::PrintingReported
        ) {
            return Err(StorageError::ConcurrentStateChange);
        }
        attempt.state = DeliveryAttemptState::DeliveryUncertain;
        attempt.updated_at = now;
        attempt.final_at = Some(now);
        let result = attempt.clone();
        if let Some(reservation) =
            state
                .reservations
                .iter_mut()
                .find_map(|((tenant, _), reservation)| {
                    (*tenant == scope
                        && reservation.attempt_id == result.id
                        && reservation.state == "active")
                        .then_some(reservation)
                })
        {
            reservation.state = "released".into();
            reservation.released_at = Some(now);
        }
        if let Some(destination) = state
            .destinations
            .get_mut(&(scope, result.destination_id.clone()))
        {
            destination.state = "attention".into();
            destination.updated_at = now;
        }
        Ok(result)
    }
    async fn renew_delivery_attempt(
        &self,
        scope: TenantScope,
        reservation_id: &str,
        generation: u64,
        fencing_token: &str,
        lease_until: DateTime<Utc>,
    ) -> Result<StartedDeliveryAttempt, StorageError> {
        if lease_until <= Utc::now() {
            return Err(StorageError::InvalidData(
                "renewed lease must be in the future".into(),
            ));
        }
        let mut state = write_state(self)?;
        let reservation_key = (scope, reservation_id.to_owned());
        let attempt_id = state
            .reservations
            .get(&reservation_key)
            .ok_or(StorageError::NotFound)?
            .attempt_id
            .clone();
        let attempt_key = (scope, attempt_id);
        let (attempt, digest) = state
            .attempts
            .get_mut(&attempt_key)
            .ok_or(StorageError::NotFound)?;
        let renewable = matches!(
            attempt.state,
            DeliveryAttemptState::RouteLeased
                | DeliveryAttemptState::AcceptedByNode
                | DeliveryAttemptState::QueuedLocal
                | DeliveryAttemptState::HandingToSpooler
        );
        if attempt.generation != generation || *digest != token_digest(fencing_token) || !renewable
        {
            return Err(StorageError::ConcurrentStateChange);
        }
        attempt.lease_until = lease_until;
        attempt.updated_at = Utc::now();
        let renewed_attempt = attempt.clone();
        let reservation = state
            .reservations
            .get_mut(&reservation_key)
            .ok_or(StorageError::NotFound)?;
        if reservation.state != "active" {
            return Err(StorageError::ConcurrentStateChange);
        }
        reservation.lease_until = lease_until;
        Ok(StartedDeliveryAttempt {
            attempt: renewed_attempt,
            reservation: reservation.clone(),
            fencing_token: fencing_token.to_owned(),
        })
    }
    async fn enqueue_delivery_uncertainty_resolution(
        &self,
        scope: TenantScope,
        job_id: &str,
        resolution: &str,
        note: Option<&str>,
        actor_id: &str,
        request_id: &str,
    ) -> Result<PendingDeliveryUncertaintyResolution, StorageError> {
        let mut state = write_state(self)?;
        if let Some(existing) = state
            .pending_uncertainty_resolutions
            .get(&(scope, request_id.to_owned()))
        {
            if existing.job_id == job_id
                && existing.resolution == resolution
                && existing.note.as_deref() == note
                && existing.actor_id == actor_id
            {
                return Ok(existing.clone());
            }
            return Err(StorageError::IdempotencyConflict);
        }
        let attempt = state
            .attempts
            .iter()
            .filter_map(|((tenant, _), (attempt, _))| {
                (*tenant == scope
                    && attempt.job_id == job_id
                    && attempt.state == DeliveryAttemptState::DeliveryUncertain)
                    .then_some(attempt)
            })
            .max_by_key(|attempt| attempt.generation)
            .cloned()
            .ok_or(StorageError::NotFound)?;
        if state
            .pending_uncertainty_resolutions
            .iter()
            .any(|((tenant, _), item)| *tenant == scope && item.attempt_id == attempt.id)
        {
            return Err(StorageError::ConcurrentStateChange);
        }
        let route = state
            .routes
            .get(&(scope, attempt.route_id.clone()))
            .cloned()
            .ok_or(StorageError::NotFound)?;
        let local_route_key = route.local_route_key.ok_or_else(|| {
            StorageError::InvalidData("uncertain route has no node-local route key".into())
        })?;
        let reservation = state
            .reservations
            .iter()
            .find_map(|((tenant, _), reservation)| {
                (*tenant == scope && reservation.attempt_id == attempt.id).then_some(reservation)
            })
            .cloned()
            .ok_or(StorageError::NotFound)?;
        state.next_uncertainty_command_cursor = state
            .next_uncertainty_command_cursor
            .checked_add(1)
            .ok_or_else(|| StorageError::InvalidData("agent command cursor overflow".into()))?;
        let cursor = state.next_uncertainty_command_cursor;
        let command = serde_json::json!({
            "type": "resolve_ambiguous_handoff",
            "job_id": job_id,
            "local_route_key": local_route_key,
            "reservation_id": reservation.id,
            "generation": attempt.generation,
            "resolution": "confirm_accepted"
        });
        let result = PendingDeliveryUncertaintyResolution {
            request_id: request_id.into(),
            job_id: job_id.into(),
            attempt_id: attempt.id.clone(),
            destination_id: attempt.destination_id.clone(),
            route_id: attempt.route_id.clone(),
            agent_id: route.agent_id,
            reservation_id: reservation.id,
            generation: attempt.generation,
            resolution: resolution.into(),
            note: note.map(str::to_owned),
            actor_id: actor_id.into(),
            agent_command_cursor: cursor,
            command,
            created_at: Utc::now(),
            finalized_at: None,
        };
        state
            .pending_uncertainty_resolutions
            .insert((scope, request_id.into()), result.clone());
        Ok(result)
    }
    async fn finalize_delivery_uncertainty_resolution(
        &self,
        scope: TenantScope,
        request_id: &str,
    ) -> Result<Option<DeliveryUncertaintyResolution>, StorageError> {
        let mut state = write_state(self)?;
        if let Some(existing) = state
            .uncertainty_resolutions
            .get(&(scope, request_id.to_owned()))
        {
            return Ok(Some(existing.clone()));
        }
        let pending = state
            .pending_uncertainty_resolutions
            .get(&(scope, request_id.to_owned()))
            .cloned()
            .ok_or(StorageError::NotFound)?;
        if !state
            .acknowledged_uncertainty_commands
            .contains(&(scope, pending.agent_command_cursor))
        {
            return Ok(None);
        }
        let result = DeliveryUncertaintyResolution {
            id: format!("dur_{}", ulid::Ulid::new()),
            job_id: pending.job_id,
            attempt_id: pending.attempt_id,
            destination_id: pending.destination_id.clone(),
            resolution: pending.resolution,
            note: pending.note,
            actor_id: pending.actor_id,
            request_id: pending.request_id,
            created_at: Utc::now(),
        };
        state
            .uncertainty_resolutions
            .insert((scope, request_id.into()), result.clone());
        let still_unresolved = state.attempts.iter().any(|((tenant, _), (candidate, _))| {
            *tenant == scope
                && candidate.destination_id == result.destination_id
                && candidate.state == DeliveryAttemptState::DeliveryUncertain
                && !state
                    .uncertainty_resolutions
                    .iter()
                    .any(|((resolution_tenant, _), item)| {
                        *resolution_tenant == scope && item.attempt_id == candidate.id
                    })
        });
        if !still_unresolved {
            if let Some(destination) = state
                .destinations
                .get_mut(&(scope, result.destination_id.clone()))
            {
                if destination.state == "attention" {
                    destination.state = "available".into();
                }
            }
        }
        if let Some(pending) = state
            .pending_uncertainty_resolutions
            .get_mut(&(scope, request_id.to_owned()))
        {
            pending.finalized_at = Some(Utc::now());
        }
        Ok(Some(result))
    }
    async fn finalize_acknowledged_uncertainty_resolutions(
        &self,
        scope: TenantScope,
        agent_id: &str,
        limit: u32,
    ) -> Result<Vec<DeliveryUncertaintyResolution>, StorageError> {
        let request_ids: Vec<_> = {
            let state = read_state(self)?;
            let mut items: Vec<_> = state
                .pending_uncertainty_resolutions
                .iter()
                .filter_map(|((tenant, request_id), pending)| {
                    (*tenant == scope
                        && pending.agent_id == agent_id
                        && pending.finalized_at.is_none()
                        && state
                            .acknowledged_uncertainty_commands
                            .contains(&(scope, pending.agent_command_cursor)))
                    .then_some((pending.created_at, request_id.clone()))
                })
                .collect();
            items.sort();
            items
                .into_iter()
                .take(usize::try_from(limit.clamp(1, 100)).unwrap_or(100))
                .map(|(_, request_id)| request_id)
                .collect()
        };
        let mut resolutions = Vec::with_capacity(request_ids.len());
        for request_id in request_ids {
            if let Some(resolution) = self
                .finalize_delivery_uncertainty_resolution(scope, &request_id)
                .await?
            {
                resolutions.push(resolution);
            }
        }
        if resolutions.len() < usize::try_from(limit.clamp(1, 100)).unwrap_or(100) {
            let state = read_state(self)?;
            let mut recoverable: Vec<_> = state
                .uncertainty_resolutions
                .iter()
                .filter_map(|((tenant, _), resolution)| {
                    let pending = state
                        .pending_uncertainty_resolutions
                        .get(&(*tenant, resolution.request_id.clone()))?;
                    (*tenant == scope
                        && pending.agent_id == agent_id
                        && resolution.resolution == "reprint_authorized")
                        .then_some(resolution.clone())
                })
                .collect();
            recoverable.sort_by_key(|resolution| (resolution.created_at, resolution.id.clone()));
            for resolution in recoverable {
                if resolutions.len() >= usize::try_from(limit.clamp(1, 100)).unwrap_or(100) {
                    break;
                }
                if !resolutions
                    .iter()
                    .any(|item| item.request_id == resolution.request_id)
                {
                    resolutions.push(resolution);
                }
            }
        }
        Ok(resolutions)
    }
    async fn acknowledge_uncertainty_resolution_command(
        &self,
        scope: TenantScope,
        agent_command_cursor: u64,
    ) -> Result<(), StorageError> {
        let mut state = write_state(self)?;
        if !state
            .pending_uncertainty_resolutions
            .iter()
            .any(|((tenant, _), pending)| {
                *tenant == scope && pending.agent_command_cursor == agent_command_cursor
            })
        {
            return Err(StorageError::NotFound);
        }
        state
            .acknowledged_uncertainty_commands
            .insert((scope, agent_command_cursor));
        Ok(())
    }
    async fn has_unresolved_destination_uncertainty(
        &self,
        scope: TenantScope,
        destination_id: &str,
    ) -> Result<bool, StorageError> {
        let state = read_state(self)?;
        Ok(state.attempts.iter().any(|((tenant, _), (attempt, _))| {
            *tenant == scope
                && attempt.destination_id == destination_id
                && attempt.state == DeliveryAttemptState::DeliveryUncertain
                && !state
                    .uncertainty_resolutions
                    .values()
                    .any(|item| item.attempt_id == attempt.id)
        }))
    }
    async fn recompute_destination_attention(
        &self,
        scope: TenantScope,
        destination_id: &str,
    ) -> Result<StoredPhysicalDestination, StorageError> {
        let mut state = write_state(self)?;
        let unresolved = state.attempts.iter().any(|((tenant, _), (attempt, _))| {
            *tenant == scope
                && attempt.destination_id == destination_id
                && attempt.state == DeliveryAttemptState::DeliveryUncertain
                && !state
                    .uncertainty_resolutions
                    .values()
                    .any(|resolution| resolution.attempt_id == attempt.id)
        });
        let destination = state
            .destinations
            .get_mut(&(scope, destination_id.to_owned()))
            .ok_or(StorageError::NotFound)?;
        if unresolved {
            destination.state = "attention".into();
        } else if destination.state == "attention" {
            destination.state = "available".into();
        }
        destination.updated_at = Utc::now();
        Ok(destination.clone())
    }
    async fn list_delivery_attempts(
        &self,
        scope: TenantScope,
        job_id: &str,
    ) -> Result<Vec<DeliveryAttempt>, StorageError> {
        let mut attempts: Vec<_> = read_state(self)?
            .attempts
            .iter()
            .filter(|((tenant, _), (attempt, _))| *tenant == scope && attempt.job_id == job_id)
            .map(|(_, (attempt, _))| attempt.clone())
            .collect();
        attempts.sort_by_key(|attempt| attempt.generation);
        Ok(attempts)
    }
    async fn get_latest_delivery_attempt(
        &self,
        scope: TenantScope,
        job_id: &str,
    ) -> Result<DeliveryAttempt, StorageError> {
        read_state(self)?
            .attempts
            .iter()
            .filter_map(|((tenant, _), (attempt, _))| {
                (*tenant == scope && attempt.job_id == job_id).then_some(attempt)
            })
            .max_by_key(|attempt| attempt.generation)
            .cloned()
            .ok_or(StorageError::NotFound)
    }
    async fn get_delivery_attempt_by_reservation(
        &self,
        scope: TenantScope,
        reservation_id: &str,
    ) -> Result<DeliveryAttempt, StorageError> {
        let state = read_state(self)?;
        let reservation = state
            .reservations
            .get(&(scope, reservation_id.to_owned()))
            .ok_or(StorageError::NotFound)?;
        state
            .attempts
            .get(&(scope, reservation.attempt_id.clone()))
            .map(|(attempt, _)| attempt.clone())
            .ok_or(StorageError::NotFound)
    }
    async fn list_route_reservations(
        &self,
        scope: TenantScope,
        limit: u32,
    ) -> Result<Vec<RouteReservation>, StorageError> {
        let mut reservations: Vec<_> = read_state(self)?
            .reservations
            .iter()
            .filter(|((tenant, _), _)| *tenant == scope)
            .map(|(_, reservation)| reservation.clone())
            .collect();
        reservations.sort_by_key(|reservation| {
            std::cmp::Reverse((reservation.acquired_at, reservation.id.clone()))
        });
        reservations.truncate(usize::try_from(limit.clamp(1, 1_000)).unwrap_or(1_000));
        Ok(reservations)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn scope() -> TenantScope {
        TenantScope {
            workspace_id: WorkspaceId::new(),
            environment_id: EnvironmentId::new(),
        }
    }
    fn destination(id: &str) -> StoredPhysicalDestination {
        StoredPhysicalDestination {
            id: id.into(),
            name: "Printer".into(),
            identity_confidence: IdentityConfidence::Unknown,
            state: "available".into(),
            scheduling_authority_id: None,
            identity_revision: 0,
            updated_at: Utc::now(),
        }
    }
    fn route(id: &str, destination_id: &str) -> StoredPrinterRoute {
        StoredPrinterRoute {
            id: id.into(),
            destination_id: destination_id.into(),
            printer_id: "ptr_test".into(),
            agent_id: "agt_test".into(),
            native_queue_id: "queue".into(),
            local_route_key: Some("rte_local_test".into()),
            state: "available".into(),
            role: "primary".into(),
            priority: 0,
            enabled: true,
            capability_revision: 1,
            profile_revision: 1,
            last_seen_at: Some(Utc::now()),
            updated_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn memory_repository_is_tenant_isolated_and_fences_stale_attempts() {
        let repository = MemoryDestinationTopologyRepository::default();
        let first = scope();
        let second = scope();
        repository
            .upsert_destination(first, &destination("dst_shared"))
            .await
            .unwrap();
        repository
            .upsert_route(first, &route("route_a", "dst_shared"))
            .await
            .unwrap();
        assert!(matches!(
            repository.get_destination(second, "dst_shared").await,
            Err(StorageError::NotFound)
        ));
        let started = repository
            .begin_delivery_attempt(
                first,
                NewDeliveryAttempt {
                    attempt_id: "attempt_1",
                    reservation_id: "reservation_1",
                    job_id: "job_1",
                    destination_id: "dst_shared",
                    route_id: "route_a",
                    lease_until: Utc::now() + chrono::Duration::minutes(1),
                },
            )
            .await
            .unwrap();
        assert!(matches!(
            repository
                .transition_delivery_attempt(
                    first,
                    "attempt_1",
                    started.attempt.generation,
                    "wrong",
                    DeliveryAttemptState::AcceptedByNode
                )
                .await,
            Err(StorageError::ConcurrentStateChange)
        ));
        repository
            .transition_delivery_attempt(
                first,
                "attempt_1",
                started.attempt.generation,
                &started.fencing_token,
                DeliveryAttemptState::AcceptedByNode,
            )
            .await
            .unwrap();
        assert!(matches!(
            repository
                .transition_delivery_attempt(
                    second,
                    "attempt_1",
                    started.attempt.generation,
                    &started.fencing_token,
                    DeliveryAttemptState::QueuedLocal
                )
                .await,
            Err(StorageError::NotFound)
        ));
        repository
            .transition_delivery_attempt(
                first,
                "attempt_1",
                started.attempt.generation,
                &started.fencing_token,
                DeliveryAttemptState::QueuedLocal,
            )
            .await
            .unwrap();
        repository
            .transition_delivery_attempt(
                first,
                "attempt_1",
                started.attempt.generation,
                &started.fencing_token,
                DeliveryAttemptState::HandingToSpooler,
            )
            .await
            .unwrap();
        repository
            .transition_delivery_attempt(
                first,
                "attempt_1",
                started.attempt.generation,
                &started.fencing_token,
                DeliveryAttemptState::DeliveryUncertain,
            )
            .await
            .unwrap();
        let pending = repository
            .enqueue_delivery_uncertainty_resolution(
                first,
                "job_1",
                "confirmed_delivered",
                None,
                "operator",
                "request_1",
            )
            .await
            .unwrap();
        assert!(
            repository
                .finalize_delivery_uncertainty_resolution(first, "request_1")
                .await
                .unwrap()
                .is_none()
        );
        repository
            .acknowledge_uncertainty_resolution_command(first, pending.agent_command_cursor)
            .await
            .unwrap();
        assert!(
            repository
                .finalize_delivery_uncertainty_resolution(first, "request_1")
                .await
                .unwrap()
                .is_some()
        );
        repository
            .begin_delivery_attempt(
                first,
                NewDeliveryAttempt {
                    attempt_id: "attempt_stale",
                    reservation_id: "reservation_stale",
                    job_id: "job_stale",
                    destination_id: "dst_shared",
                    route_id: "route_a",
                    lease_until: Utc::now() - chrono::Duration::seconds(1),
                },
            )
            .await
            .unwrap();
        repository
            .begin_delivery_attempt(
                first,
                NewDeliveryAttempt {
                    attempt_id: "attempt_after_stale",
                    reservation_id: "reservation_after_stale",
                    job_id: "job_after_stale",
                    destination_id: "dst_shared",
                    route_id: "route_a",
                    lease_until: Utc::now() + chrono::Duration::minutes(1),
                },
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn conflicting_evidence_marks_only_its_destination() {
        let repository = MemoryDestinationTopologyRepository::default();
        let tenant = scope();
        repository
            .upsert_destination(tenant, &destination("dst_a"))
            .await
            .unwrap();
        repository
            .upsert_route(tenant, &route("route_a", "dst_a"))
            .await
            .unwrap();
        repository
            .record_identity_evidence(
                tenant,
                &IdentityEvidence {
                    id: "evidence_a".into(),
                    destination_id: "dst_a".into(),
                    route_id: "route_a".into(),
                    kind: "device_serial".into(),
                    value_digest: format!("hmac-sha256:{}", "a".repeat(64)),
                    strength: "strong".into(),
                    conflicts: true,
                    observed_at: Utc::now(),
                    expires_at: None,
                    metadata: serde_json::json!({}),
                },
            )
            .await
            .unwrap();
        assert_eq!(
            repository
                .get_destination(tenant, "dst_a")
                .await
                .unwrap()
                .identity_confidence,
            IdentityConfidence::Conflict
        );
    }

    #[tokio::test]
    async fn reversing_identity_topology_never_mutates_colliding_tenant_evidence() {
        let repository = MemoryDestinationTopologyRepository::default();
        let first = scope();
        let second = scope();
        for (tenant, destination_id) in [
            (first, "dst_source"),
            (first, "dst_target"),
            (second, "dst_second"),
        ] {
            repository
                .upsert_destination(tenant, &destination(destination_id))
                .await
                .unwrap();
        }
        repository
            .upsert_route(first, &route("route_collision", "dst_source"))
            .await
            .unwrap();
        repository
            .upsert_route(second, &route("route_collision", "dst_second"))
            .await
            .unwrap();
        for (tenant, evidence_id, destination_id, digest) in [
            (first, "evidence_first", "dst_source", "a"),
            (second, "evidence_second", "dst_second", "b"),
        ] {
            repository
                .record_identity_evidence(
                    tenant,
                    &IdentityEvidence {
                        id: evidence_id.into(),
                        destination_id: destination_id.into(),
                        route_id: "route_collision".into(),
                        kind: "device_serial".into(),
                        value_digest: format!("hmac-sha256:{}", digest.repeat(64)),
                        strength: "strong".into(),
                        conflicts: false,
                        observed_at: Utc::now(),
                        expires_at: None,
                        metadata: serde_json::json!({}),
                    },
                )
                .await
                .unwrap();
        }
        let merge = IdentityDecision {
            id: "decision_merge".into(),
            kind: IdentityDecisionKind::Merge,
            destination_id: "dst_target".into(),
            related_destination_ids: vec!["dst_source".into()],
            route_ids: vec!["route_collision".into()],
            evidence_ids: vec!["evidence_first".into()],
            actor_kind: "operator".into(),
            actor_id: Some("operator".into()),
            reason: "verified merge".into(),
            reverses_decision_id: None,
            request_id: Some("merge_request".into()),
            created_at: Utc::now(),
        };
        repository
            .record_identity_decision(first, &merge)
            .await
            .unwrap();
        repository
            .reverse_identity_decision(
                first,
                &IdentityDecision {
                    id: "decision_reverse".into(),
                    kind: IdentityDecisionKind::Reverse,
                    destination_id: "dst_target".into(),
                    related_destination_ids: vec!["dst_source".into()],
                    route_ids: vec!["route_collision".into()],
                    evidence_ids: vec!["evidence_first".into()],
                    actor_kind: "operator".into(),
                    actor_id: Some("operator".into()),
                    reason: "undo verified merge".into(),
                    reverses_decision_id: Some("decision_merge".into()),
                    request_id: Some("reverse_request".into()),
                    created_at: Utc::now(),
                },
            )
            .await
            .unwrap();
        assert_eq!(
            repository
                .list_identity_evidence(second, "dst_second")
                .await
                .unwrap()
                .len(),
            1
        );
    }
}
