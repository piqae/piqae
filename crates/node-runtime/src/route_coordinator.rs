//! Installation-wide printer topology and native handoff fencing.
//!
//! Connector runtimes deliberately keep credentials, queues, cursors and
//! documents isolated. This coordinator owns only shared physical facts and
//! the final OS-route reservation boundary. Its durable handoff journal closes
//! the crash window where a connector could otherwise replay a job after the
//! operating system had already accepted it.

#![allow(
    clippy::missing_errors_doc,
    clippy::must_use_candidate,
    clippy::too_long_first_doc_paragraph,
    reason = "durable compatibility API was moved intact; method contracts document fail-closed semantics inline"
)]

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use piqae_protocol::{
    agent::{
        IdentityConfidence, IdentityEvidenceStrength, NativeHandoffEvidence, NativeHandoffOutcome,
        PhysicalIdentityEvidence, PhysicalIdentityEvidenceKind, PrinterRouteSnapshot,
        RouteTopologyChange, TopologyChange,
    },
    executor::DiscoveredPrinter,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
#[cfg(test)]
use std::cell::Cell;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
use uuid::Uuid;

const DOCUMENT_VERSION: u16 = 1;
const MAX_ROUTES: usize = 512;
const MAX_HANDOFFS: usize = 512;
const MAX_CONSUMED_AUTHORITATIVE_ROUTES: usize = 4_096;
const MAX_ACKNOWLEDGED_CONNECTORS: usize = 512;
const MAX_TOPOLOGY_CHANGES: usize = 512;
const OBSERVATION_SEQUENCE_RESERVATION: u64 = 512;
const RESERVATION_LIFETIME_MS: i64 = 2 * 60 * 1_000;

#[derive(Clone, Deserialize, Serialize)]
struct CoordinatorDocument {
    version: u16,
    installation_namespace: Uuid,
    topology_revision: u64,
    handoff_sequence: u64,
    #[serde(default)]
    observation_sequence: u64,
    routes: BTreeMap<String, DurableRoute>,
    reservations: BTreeMap<String, DurableReservation>,
    /// High-water marks for terminal cloud-authorized attempts. Handoff
    /// evidence can be acknowledged and compacted, but an old server offer
    /// must never regain authority after that compaction.
    #[serde(default)]
    consumed_authoritative_routes: BTreeMap<String, ConsumedAuthoritativeRoute>,
    /// Connector-scoped server acknowledgement cursor. Ambiguous records are
    /// retained beyond this cursor until an explicit resolution arrives.
    #[serde(default)]
    acknowledged_handoff_sequences: BTreeMap<String, u64>,
    handoffs: Vec<DurableHandoff>,
    topology_changes: Vec<RouteTopologyChange>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct DurableRoute {
    route_id: String,
    coordination_key: String,
    /// Set when independently occupied exclusion domains are later observed
    /// as one physical group. New reservations fail closed until the active
    /// fences resolve and a later reconciliation can safely regroup routes.
    #[serde(default)]
    coordination_conflict: bool,
    native_fingerprint: String,
    generation: u64,
    present: bool,
    identity_evidence: Vec<PhysicalIdentityEvidence>,
    identity_confidence: IdentityConfidence,
}

#[derive(Clone, Deserialize, Serialize)]
struct DurableReservation {
    server_route_id: Option<String>,
    local_route_key: String,
    reservation_id: Uuid,
    connector_id: String,
    job_id: String,
    generation: u64,
    fencing_token: String,
    expires_unix_ms: i64,
}

#[derive(Clone, Deserialize, Serialize)]
struct DurableHandoff {
    sequence: u64,
    connector_id: String,
    server_route_id: Option<String>,
    local_route_key: String,
    job_id: String,
    reservation_id: Uuid,
    fencing_generation: u64,
    fencing_token: String,
    observed_at: DateTime<Utc>,
    outcome: NativeHandoffOutcome,
    native_job_id: Option<String>,
}

#[derive(Clone, Deserialize, Serialize)]
struct ConsumedAuthoritativeRoute {
    connector_id: String,
    server_route_id: String,
    local_route_key: String,
    job_id: String,
    generation: u64,
    reservation_id: Uuid,
    fencing_token: String,
    observed_at: DateTime<Utc>,
    outcome: NativeHandoffOutcome,
    native_job_id: Option<String>,
}

#[derive(Clone, Eq, PartialEq)]
pub struct RouteReservation {
    pub local_route_key: String,
    pub server_route_id: Option<String>,
    coordination_key: String,
    pub reservation_id: Uuid,
    pub generation: u64,
    fencing_token: String,
}

#[derive(Debug)]
pub struct AuthoritativeRouteRegistration {
    pub reservation: RouteReservation,
    pub newly_registered: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthoritativeHandoffResolution {
    pub outcome: NativeHandoffOutcome,
    pub native_job_id: Option<String>,
    pub observed_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalHandoffCommit {
    pub job_id: String,
    pub state: String,
    pub native_job_id: Option<String>,
    pub ambiguity_confirmed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalHandoffResolutionTarget {
    pub ambiguity_id: String,
    pub local_route_key: String,
    pub reservation_id: Uuid,
    pub generation: u64,
    pub outcome: NativeHandoffOutcome,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalHandoffEvidence {
    pub sequence: u64,
    pub job_id: String,
    pub ambiguity_id: String,
    pub observed_at: DateTime<Utc>,
    pub outcome: NativeHandoffOutcome,
    pub native_job_id: Option<String>,
}

pub struct RouteCoordinator {
    root: PathBuf,
    document: CoordinatorDocument,
    next_observation_sequence: u64,
    reserved_observation_sequence: u64,
    poisoned: bool,
    #[cfg(test)]
    persist_fault: Cell<Option<PersistFault>>,
}

#[cfg(test)]
#[derive(Clone, Copy, Eq, PartialEq)]
enum PersistFault {
    BeforeReplace,
    BeforeReplaceTwice,
    AfterReplace,
}

impl std::fmt::Debug for RouteReservation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RouteReservation")
            .field("local_route_key", &self.local_route_key)
            .field("server_route_id", &self.server_route_id)
            .field("coordination_key", &self.coordination_key)
            .field("reservation_id", &self.reservation_id)
            .field("generation", &self.generation)
            .field("fencing_token", &"[REDACTED]")
            .finish()
    }
}

impl RouteReservation {
    pub fn matches_cloud_proof(
        &self,
        reservation_id: &str,
        generation: u64,
        fencing_token: &str,
    ) -> bool {
        self.server_route_id.is_some()
            && self.reservation_id.to_string() == reservation_id
            && self.generation == generation
            && self.fencing_token == fencing_token
    }
}

impl std::fmt::Debug for RouteCoordinator {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RouteCoordinator")
            .field("routes", &self.document.routes.len())
            .field("reservations", &self.document.reservations.len())
            .field("handoffs", &self.document.handoffs.len())
            .field("topology_changes", &self.document.topology_changes.len())
            .finish_non_exhaustive()
    }
}

impl RouteCoordinator {
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        let path = root.join("route-coordinator.json");
        let document = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice::<CoordinatorDocument>(&bytes)
                .with_context(|| format!("parse {}", path.display()))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => CoordinatorDocument {
                version: DOCUMENT_VERSION,
                installation_namespace: Uuid::new_v4(),
                topology_revision: 0,
                handoff_sequence: 0,
                observation_sequence: 0,
                routes: BTreeMap::new(),
                reservations: BTreeMap::new(),
                consumed_authoritative_routes: BTreeMap::new(),
                acknowledged_handoff_sequences: BTreeMap::new(),
                handoffs: Vec::new(),
                topology_changes: Vec::new(),
            },
            Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
        };
        if document.version != DOCUMENT_VERSION {
            bail!("unsupported route coordinator version {}", document.version);
        }
        if document.handoffs.len() > MAX_HANDOFFS
            || document.topology_changes.len() > MAX_TOPOLOGY_CHANGES
            || document.consumed_authoritative_routes.len() > MAX_CONSUMED_AUTHORITATIVE_ROUTES
            || document.acknowledged_handoff_sequences.len() > MAX_ACKNOWLEDGED_CONNECTORS
        {
            bail!("route coordinator state exceeds supported bounds");
        }
        if document
            .consumed_authoritative_routes
            .iter()
            .any(|(key, route)| {
                route.generation == 0
                    || route.server_route_id.is_empty()
                    || route.local_route_key.is_empty()
                    || route.job_id.is_empty()
                    || route.connector_id.is_empty()
                    || route.fencing_token.is_empty()
                    || *key
                        != authoritative_route_key(
                            &route.connector_id,
                            &route.server_route_id,
                            &route.local_route_key,
                            &route.job_id,
                        )
            })
        {
            bail!("route coordinator contains an invalid authoritative replay barrier");
        }
        let next_observation_sequence = document
            .observation_sequence
            .checked_add(1)
            .context("route observation sequence space is exhausted")?;
        let reserved_observation_sequence = document.observation_sequence;
        let mut coordinator = Self {
            root,
            document,
            next_observation_sequence,
            reserved_observation_sequence,
            poisoned: false,
            #[cfg(test)]
            persist_fault: Cell::new(None),
        };
        // Repair an intermediate journal written by an older coordinator
        // which re-keyed route evidence without moving the unresolved fence.
        // This also restores the persisted exclusion domain before any worker
        // can reserve after process restart.
        coordinator.reconcile_coordination_keys();
        // Repair must precede pruning because an older journal can still
        // protect an absent route through its previous coordination key.
        coordinator.prune_absent_routes();
        if coordinator.document.routes.len() > MAX_ROUTES {
            bail!("route coordinator state exceeds supported bounds");
        }
        coordinator.persist()?;
        Ok(coordinator)
    }

    pub fn reconcile(
        &mut self,
        printers: &[DiscoveredPrinter],
        inventory_revision: u64,
        observed_at: DateTime<Utc>,
    ) -> Result<BTreeMap<String, PrinterRouteSnapshot>> {
        self.ensure_operational()?;
        self.prune_absent_routes();
        let protected_native_ids = self
            .document
            .routes
            .iter()
            .filter(|(_, route)| self.route_requires_retention(route))
            .map(|(native_id, _)| native_id.clone())
            .collect::<BTreeSet<_>>();
        let available_unprotected_routes = MAX_ROUTES.saturating_sub(protected_native_ids.len());
        let mut accepted_unprotected_routes = 0usize;
        let mut present = BTreeSet::new();
        let mut snapshots = BTreeMap::new();
        for printer in printers {
            if !present.insert(printer.native_id.clone()) {
                continue;
            }
            if !protected_native_ids.contains(&printer.native_id) {
                if accepted_unprotected_routes >= available_unprotected_routes {
                    present.remove(&printer.native_id);
                    continue;
                }
                accepted_unprotected_routes = accepted_unprotected_routes.saturating_add(1);
            }
            let route_id = self.route_id(&printer.native_id);
            let evidence = canonical_evidence(printer);
            let confidence = identity_confidence(&evidence);
            let fingerprint = topology_fingerprint(&evidence);
            let desired_coordination_key = physical_coordination_key(&route_id, &evidence);
            let change = match self.document.routes.get(&printer.native_id) {
                None => Some(TopologyChange::Added),
                Some(route) if !route.present || route.native_fingerprint != fingerprint => {
                    Some(TopologyChange::Changed)
                }
                Some(_) => None,
            };
            if change.is_some() {
                self.document.topology_revision = self.document.topology_revision.saturating_add(1);
            }
            let previous = self.document.routes.get(&printer.native_id);
            let generation = previous.map_or(0, |route| route.generation);
            // Preserve the prior exclusion domain until the batch planner has
            // considered every active reservation and every alias. Writing
            // the newly observed key directly here would create a window in
            // which a reservation remained under the old map key while this
            // route escaped to a new one.
            let coordination_key = previous.map_or_else(
                || desired_coordination_key.clone(),
                |route| route.coordination_key.clone(),
            );
            self.document.routes.insert(
                printer.native_id.clone(),
                DurableRoute {
                    route_id: route_id.clone(),
                    coordination_key,
                    coordination_conflict: false,
                    native_fingerprint: fingerprint,
                    generation,
                    present: true,
                    identity_evidence: evidence.clone(),
                    identity_confidence: confidence,
                },
            );
            if let Some(change) = change {
                self.record_topology_change(&route_id, change, observed_at);
            }
            snapshots.insert(
                printer.native_id.clone(),
                PrinterRouteSnapshot {
                    local_route_key: route_id,
                    inventory_revision,
                    topology_revision: self.document.topology_revision,
                    observed_at,
                    identity_evidence: evidence,
                    identity_confidence: confidence,
                    topology_change: change,
                    profile_observed_at: Some(observed_at),
                    stock_observed_at: None,
                },
            );
        }
        let removed = self
            .document
            .routes
            .iter()
            .filter(|(native_id, route)| route.present && !present.contains(*native_id))
            .map(|(native_id, route)| (native_id.clone(), route.route_id.clone()))
            .collect::<Vec<_>>();
        for (native_id, route_id) in removed {
            self.document.topology_revision = self.document.topology_revision.saturating_add(1);
            if let Some(route) = self.document.routes.get_mut(&native_id) {
                route.present = false;
            }
            self.record_topology_change(&route_id, TopologyChange::Removed, observed_at);
        }
        self.prune_absent_routes();
        self.reconcile_coordination_keys();
        self.persist()?;
        Ok(snapshots)
    }

    pub fn route_id(&self, native_id: &str) -> String {
        let mut digest = Sha256::new();
        digest.update(b"piqae-route-v1\0");
        digest.update(self.document.installation_namespace.as_bytes());
        digest.update(native_id.as_bytes());
        format!("rte_{}", &hex::encode(digest.finalize())[..32])
    }

    /// Returns the installation-local serialization group for a present OS
    /// route. Separate queues share this only when discovery supplied enough
    /// independent identity evidence to coordinate them safely.
    pub fn coordination_key(&self, native_id: &str) -> Option<&str> {
        self.document
            .routes
            .get(native_id)
            .filter(|route| route.present)
            .map(|route| route.coordination_key.as_str())
    }

    pub fn topology_changes(&self) -> Vec<RouteTopologyChange> {
        self.document.topology_changes.clone()
    }

    pub fn allocate_observation_sequences(&mut self, count: usize) -> Result<Vec<u64>> {
        self.ensure_operational()?;
        let bounded = count.min(MAX_ROUTES);
        if bounded == 0 {
            return Ok(Vec::new());
        }
        let bounded = u64::try_from(bounded).context("route observation batch is too large")?;
        let allocated_through = self
            .next_observation_sequence
            .checked_add(bounded - 1)
            .context("route observation sequence space is exhausted")?;
        if allocated_through > self.reserved_observation_sequence {
            let reservation = OBSERVATION_SEQUENCE_RESERVATION.max(bounded);
            let reserved_through = self
                .document
                .observation_sequence
                .checked_add(reservation)
                .context("route observation sequence space is exhausted")?;
            self.document.observation_sequence = reserved_through;
            // Persist the high-water mark before returning any number from the
            // range. A crash can skip unused values but can never reuse one.
            self.persist()?;
            self.reserved_observation_sequence = reserved_through;
        }
        let start = self.next_observation_sequence;
        self.next_observation_sequence = allocated_through
            .checked_add(1)
            .context("route observation sequence space is exhausted")?;
        Ok((start..=allocated_through).collect())
    }

    pub fn reserve(
        &mut self,
        connector_id: &str,
        native_id: &str,
        job_id: &str,
        now_unix_ms: i64,
    ) -> Result<RouteReservation> {
        self.ensure_operational()?;
        let previous = self.document.clone();
        match self.reserve_inner(connector_id, native_id, job_id, now_unix_ms) {
            Ok(reservation) => {
                self.commit_document_change(previous)?;
                Ok(reservation)
            }
            Err(error) => {
                self.document = previous;
                Err(error)
            }
        }
    }

    fn reserve_inner(
        &mut self,
        connector_id: &str,
        native_id: &str,
        job_id: &str,
        now_unix_ms: i64,
    ) -> Result<RouteReservation> {
        let route_id = self.route_id(native_id);
        let route = self.document.routes.get(native_id);
        let coordination_key =
            route.map_or_else(|| route_id.clone(), |route| route.coordination_key.clone());
        if self.document.handoffs.iter().rev().any(|handoff| {
            handoff.connector_id == connector_id
                && handoff.job_id == job_id
                && matches!(
                    handoff.outcome,
                    NativeHandoffOutcome::Accepted | NativeHandoffOutcome::Ambiguous
                )
        }) {
            bail!("job already crossed or may have crossed native handoff");
        }
        if let Some(existing) = self.document.reservations.get(&coordination_key) {
            if existing.job_id == job_id && existing.connector_id == connector_id {
                return Ok(RouteReservation {
                    local_route_key: route_id,
                    server_route_id: existing.server_route_id.clone(),
                    coordination_key,
                    reservation_id: existing.reservation_id,
                    generation: existing.generation,
                    fencing_token: existing.fencing_token.clone(),
                });
            }
            // Time passing cannot prove that a blocked, crashed, or cancelled
            // executor did not cross native handoff. Never steal a route from
            // an unresolved reservation solely because its advisory deadline
            // elapsed; explicit terminal evidence is required first.
            bail!("printer route is reserved by another connector");
        }
        if self
            .document
            .handoffs
            .len()
            .saturating_add(self.document.reservations.len())
            >= MAX_HANDOFFS
        {
            bail!("native handoff journal is full pending control-plane acknowledgement");
        }
        if route.is_some_and(|route| route.coordination_conflict) {
            bail!("printer route has conflicting unresolved physical reservations");
        }
        let generation = self
            .document
            .routes
            .values_mut()
            .find(|route| route.route_id == route_id)
            .map_or(1, |route| {
                route.generation = route.generation.saturating_add(1);
                route.generation
            });
        let reservation = DurableReservation {
            server_route_id: None,
            local_route_key: route_id.clone(),
            reservation_id: Uuid::new_v4(),
            connector_id: connector_id.to_owned(),
            job_id: job_id.to_owned(),
            generation,
            fencing_token: Uuid::new_v4().to_string(),
            expires_unix_ms: now_unix_ms.saturating_add(RESERVATION_LIFETIME_MS),
        };
        let result = RouteReservation {
            local_route_key: route_id,
            server_route_id: None,
            coordination_key: coordination_key.clone(),
            reservation_id: reservation.reservation_id,
            generation,
            fencing_token: reservation.fencing_token.clone(),
        };
        self.document
            .reservations
            .insert(coordination_key, reservation);
        Ok(result)
    }

    #[allow(
        clippy::too_many_lines,
        reason = "validation, replay fencing, capacity reservation, and idempotent registration form one atomic decision"
    )]
    pub fn register_authoritative(
        &mut self,
        connector_id: &str,
        native_id: &str,
        job_id: &str,
        reservation: &piqae_protocol::agent::CloudRouteReservation,
        now: DateTime<Utc>,
    ) -> Result<AuthoritativeRouteRegistration> {
        self.ensure_operational()?;
        let previous = self.document.clone();
        match self.register_authoritative_inner(connector_id, native_id, job_id, reservation, now) {
            Ok(registration) => {
                self.commit_document_change(previous)?;
                Ok(registration)
            }
            Err(error) => {
                self.document = previous;
                Err(error)
            }
        }
    }

    #[allow(
        clippy::too_many_lines,
        reason = "validation, replay fencing, capacity reservation, and idempotent registration form one atomic decision"
    )]
    fn register_authoritative_inner(
        &mut self,
        connector_id: &str,
        native_id: &str,
        job_id: &str,
        reservation: &piqae_protocol::agent::CloudRouteReservation,
        now: DateTime<Utc>,
    ) -> Result<AuthoritativeRouteRegistration> {
        let expected_route = self.route_id(native_id);
        if reservation.local_route_key != expected_route
            || reservation.route_id.is_empty()
            || reservation.generation == 0
            || reservation.fencing_token.is_empty()
            || reservation.fencing_token.len() > 512
            || reservation.lease_expires_at <= now
        {
            bail!("cloud route reservation is invalid for this installation route");
        }
        if self
            .document
            .consumed_authoritative_routes
            .values()
            .any(|consumed| {
                consumed.connector_id == connector_id
                    && consumed.job_id == job_id
                    && consumed.outcome == NativeHandoffOutcome::Accepted
            })
        {
            bail!("job already crossed native handoff for this connector authority");
        }
        let consumed_key = authoritative_route_key(
            connector_id,
            &reservation.route_id,
            &reservation.local_route_key,
            job_id,
        );
        if self
            .document
            .consumed_authoritative_routes
            .get(&consumed_key)
            .is_some_and(|consumed| {
                consumed.outcome == NativeHandoffOutcome::Accepted
                    || reservation.generation <= consumed.generation
            })
        {
            bail!("cloud route reservation generation was already consumed for this job route");
        }
        if self.document.handoffs.iter().any(|handoff| {
            handoff.connector_id == connector_id
                && handoff.job_id == job_id
                && matches!(
                    handoff.outcome,
                    NativeHandoffOutcome::Accepted | NativeHandoffOutcome::Ambiguous
                )
        }) {
            bail!("job already crossed or may have crossed native handoff");
        }
        if self
            .document
            .handoffs
            .iter()
            .filter(|handoff| handoff.connector_id == connector_id && handoff.job_id == job_id)
            .map(|handoff| handoff.fencing_generation)
            .max()
            .is_some_and(|generation| reservation.generation <= generation)
        {
            bail!("cloud route reservation generation is stale for this job");
        }
        let route = self
            .document
            .routes
            .values()
            .find(|route| route.route_id == expected_route)
            .context("cloud reservation references an unknown local route")?;
        let coordination_key = route.coordination_key.clone();
        if let Some(existing) = self.document.reservations.get(&coordination_key) {
            if existing.connector_id == connector_id
                && existing.job_id == job_id
                && existing.reservation_id == reservation.reservation_id
                && existing.generation == reservation.generation
                && existing.fencing_token == reservation.fencing_token
                && existing.server_route_id.as_deref() == Some(reservation.route_id.as_str())
            {
                return Ok(AuthoritativeRouteRegistration {
                    reservation: RouteReservation {
                        local_route_key: expected_route,
                        server_route_id: existing.server_route_id.clone(),
                        coordination_key,
                        reservation_id: existing.reservation_id,
                        generation: existing.generation,
                        fencing_token: existing.fencing_token.clone(),
                    },
                    newly_registered: false,
                });
            }
            bail!("printer route already has an unresolved reservation");
        }
        if route.coordination_conflict {
            bail!("printer route has conflicting unresolved physical reservations");
        }
        let active_unconsumed = self
            .document
            .reservations
            .values()
            .filter_map(|active| {
                let server_route_id = active.server_route_id.as_deref()?;
                let key = authoritative_route_key(
                    &active.connector_id,
                    server_route_id,
                    &active.local_route_key,
                    &active.job_id,
                );
                (!self
                    .document
                    .consumed_authoritative_routes
                    .contains_key(&key))
                .then_some(key)
            })
            .collect::<BTreeSet<_>>();
        if !self
            .document
            .consumed_authoritative_routes
            .contains_key(&consumed_key)
            && !active_unconsumed.contains(&consumed_key)
            && self
                .document
                .consumed_authoritative_routes
                .len()
                .saturating_add(active_unconsumed.len())
                >= MAX_CONSUMED_AUTHORITATIVE_ROUTES
        {
            bail!("authoritative route replay barrier journal is full");
        }
        let durable = DurableReservation {
            server_route_id: Some(reservation.route_id.clone()),
            local_route_key: expected_route.clone(),
            reservation_id: reservation.reservation_id,
            connector_id: connector_id.to_owned(),
            job_id: job_id.to_owned(),
            generation: reservation.generation,
            fencing_token: reservation.fencing_token.clone(),
            expires_unix_ms: reservation.lease_expires_at.timestamp_millis(),
        };
        let result = RouteReservation {
            local_route_key: expected_route,
            server_route_id: durable.server_route_id.clone(),
            coordination_key: coordination_key.clone(),
            reservation_id: durable.reservation_id,
            generation: durable.generation,
            fencing_token: durable.fencing_token.clone(),
        };
        self.document.reservations.insert(coordination_key, durable);
        Ok(AuthoritativeRouteRegistration {
            reservation: result,
            newly_registered: true,
        })
    }

    /// Returns the exact current authoritative reservations for one connector.
    /// These summaries contain the fencing proof needed to compensate a crash
    /// between the coordinator journal and the connector's `SQLite` acceptance.
    pub fn authoritative_reservations_for_connector(
        &self,
        connector_id: &str,
    ) -> Vec<(String, RouteReservation)> {
        self.document
            .reservations
            .iter()
            .filter(|(_, reservation)| {
                reservation.connector_id == connector_id && reservation.server_route_id.is_some()
            })
            .map(|(coordination_key, reservation)| {
                (
                    reservation.job_id.clone(),
                    RouteReservation {
                        local_route_key: reservation.local_route_key.clone(),
                        server_route_id: reservation.server_route_id.clone(),
                        coordination_key: coordination_key.clone(),
                        reservation_id: reservation.reservation_id,
                        generation: reservation.generation,
                        fencing_token: reservation.fencing_token.clone(),
                    },
                )
            })
            .collect()
    }

    /// Returns active local-only reservations so startup can conservatively
    /// repair a crash/fail-stop before terminal handoff evidence was journaled.
    pub fn local_reservations_for_connector(
        &self,
        connector_id: &str,
    ) -> Vec<(String, RouteReservation)> {
        self.document
            .reservations
            .iter()
            .filter(|(_, reservation)| {
                reservation.connector_id == connector_id && reservation.server_route_id.is_none()
            })
            .map(|(coordination_key, reservation)| {
                (
                    reservation.job_id.clone(),
                    RouteReservation {
                        local_route_key: reservation.local_route_key.clone(),
                        server_route_id: None,
                        coordination_key: coordination_key.clone(),
                        reservation_id: reservation.reservation_id,
                        generation: reservation.generation,
                        fencing_token: reservation.fencing_token.clone(),
                    },
                )
            })
            .collect()
    }

    /// Reports whether exact terminal/ambiguous evidence already exists for
    /// an active reservation, without exposing its fencing token.
    #[must_use]
    pub fn has_handoff_for_reservation(
        &self,
        connector_id: &str,
        job_id: &str,
        reservation: &RouteReservation,
    ) -> bool {
        self.document.handoffs.iter().any(|handoff| {
            handoff.connector_id == connector_id
                && handoff.job_id == job_id
                && handoff.reservation_id == reservation.reservation_id
                && handoff.fencing_generation == reservation.generation
                && handoff.local_route_key == reservation.local_route_key
        })
    }

    /// Returns the newest local-only handoff proof for one connector job.
    /// This omits the fencing token while retaining enough identity for an
    /// authenticated local operator to resolve or idempotently replay an
    /// uncertainty decision across the coordinator/queue crash boundary.
    pub fn local_handoff_resolution_target(
        &self,
        connector_id: &str,
        job_id: &str,
    ) -> Option<LocalHandoffResolutionTarget> {
        self.document
            .handoffs
            .iter()
            .rev()
            .find(|handoff| {
                handoff.connector_id == connector_id
                    && handoff.job_id == job_id
                    && handoff.server_route_id.is_none()
            })
            .map(|handoff| LocalHandoffResolutionTarget {
                ambiguity_id: local_ambiguity_id(
                    &handoff.connector_id,
                    &handoff.job_id,
                    &handoff.local_route_key,
                    handoff.reservation_id,
                    handoff.fencing_generation,
                ),
                local_route_key: handoff.local_route_key.clone(),
                reservation_id: handoff.reservation_id,
                generation: handoff.fencing_generation,
                outcome: handoff.outcome,
            })
    }

    /// Resolves an upgrade owner only from one live authoritative reservation.
    /// Connector topology and scheduler defaults are deliberately irrelevant.
    pub fn authoritative_owner_for_job(&self, job_id: &str) -> Result<Option<String>> {
        let owners = self
            .document
            .reservations
            .values()
            .filter(|reservation| {
                reservation.job_id == job_id && reservation.server_route_id.is_some()
            })
            .map(|reservation| reservation.connector_id.clone())
            .collect::<BTreeSet<_>>();
        if owners.len() > 1 {
            bail!("job has conflicting authoritative connector reservations");
        }
        Ok(owners.into_iter().next())
    }

    /// Returns terminal/ambiguous handoff evidence only when it carries the
    /// exact cloud fencing proof retained beside the local job. Consumed
    /// terminal evidence remains available after server acknowledgement has
    /// compacted the outbound handoff journal.
    pub fn authoritative_handoff_for_proof(
        &self,
        connector_id: &str,
        job_id: &str,
        reservation_id: &str,
        generation: u64,
        fencing_token: &str,
    ) -> Option<AuthoritativeHandoffResolution> {
        if let Some(handoff) = self.document.handoffs.iter().rev().find(|handoff| {
            handoff.connector_id == connector_id
                && handoff.job_id == job_id
                && handoff.reservation_id.to_string() == reservation_id
                && handoff.fencing_generation == generation
                && handoff.fencing_token == fencing_token
        }) {
            return Some(AuthoritativeHandoffResolution {
                outcome: handoff.outcome,
                native_job_id: handoff.native_job_id.clone(),
                observed_at: handoff.observed_at,
            });
        }
        self.document
            .consumed_authoritative_routes
            .values()
            .find(|consumed| {
                consumed.connector_id == connector_id
                    && consumed.job_id == job_id
                    && consumed.reservation_id.to_string() == reservation_id
                    && consumed.generation == generation
                    && consumed.fencing_token == fencing_token
            })
            .map(|consumed| AuthoritativeHandoffResolution {
                outcome: consumed.outcome,
                native_job_id: consumed.native_job_id.clone(),
                observed_at: consumed.observed_at,
            })
    }

    pub fn validate(&self, reservation: &RouteReservation) -> Result<()> {
        self.ensure_operational()?;
        let Some(current) = self
            .document
            .reservations
            .get(&reservation.coordination_key)
        else {
            bail!("route reservation is no longer active");
        };
        if current.reservation_id != reservation.reservation_id
            || current.generation != reservation.generation
            || current.fencing_token != reservation.fencing_token
            || current.server_route_id != reservation.server_route_id
        {
            bail!("stale route fencing token");
        }
        Ok(())
    }

    pub fn finish(
        &mut self,
        connector_id: &str,
        job_id: &str,
        reservation: &RouteReservation,
        outcome: NativeHandoffOutcome,
        native_job_id: Option<String>,
        observed_at: DateTime<Utc>,
    ) -> Result<()> {
        self.ensure_operational()?;
        let previous = self.document.clone();
        if let Err(error) = self.finish_inner(
            connector_id,
            job_id,
            reservation,
            outcome,
            native_job_id,
            observed_at,
        ) {
            self.document = previous;
            return Err(error);
        }
        self.commit_document_change(previous)
    }

    /// Commits terminal native evidence with one exact retry when a failed
    /// write is proven not to have replaced the journal. An ambiguous or
    /// repeated write failure fail-stops the coordinator so its process owner
    /// can restart and adopt the durable file before doing more route work.
    pub fn finish_runtime(
        &mut self,
        connector_id: &str,
        job_id: &str,
        reservation: &RouteReservation,
        outcome: NativeHandoffOutcome,
        native_job_id: Option<String>,
        observed_at: DateTime<Utc>,
    ) -> Result<()> {
        let first = self.finish(
            connector_id,
            job_id,
            reservation,
            outcome,
            native_job_id.clone(),
            observed_at,
        );
        let Err(first_error) = first else {
            return Ok(());
        };
        if !self.poisoned
            && self
                .finish(
                    connector_id,
                    job_id,
                    reservation,
                    outcome,
                    native_job_id,
                    observed_at,
                )
                .is_ok()
        {
            return Ok(());
        }
        self.poisoned = true;
        Err(first_error).context(
            "route coordinator could not durably resolve native handoff; coordinator fail-stopped",
        )
    }

    /// Reports whether journal persistence became ambiguous and the owning
    /// process must restart before any further route operation.
    #[must_use]
    pub const fn is_poisoned(&self) -> bool {
        self.poisoned
    }

    fn finish_inner(
        &mut self,
        connector_id: &str,
        job_id: &str,
        reservation: &RouteReservation,
        outcome: NativeHandoffOutcome,
        native_job_id: Option<String>,
        observed_at: DateTime<Utc>,
    ) -> Result<()> {
        self.validate(reservation)?;
        let active = self
            .document
            .reservations
            .get(&reservation.coordination_key)
            .cloned()
            .context("route reservation disappeared before terminal handoff evidence")?;
        if active.connector_id != connector_id || active.job_id != job_id {
            bail!("terminal handoff evidence does not match the reserved connector job");
        }
        self.document.handoff_sequence = self.document.handoff_sequence.saturating_add(1);
        self.document.handoffs.push(DurableHandoff {
            sequence: self.document.handoff_sequence,
            connector_id: connector_id.to_owned(),
            server_route_id: reservation.server_route_id.clone(),
            local_route_key: reservation.local_route_key.clone(),
            job_id: job_id.to_owned(),
            reservation_id: reservation.reservation_id,
            fencing_generation: reservation.generation,
            fencing_token: self
                .document
                .reservations
                .get(&reservation.coordination_key)
                .map(|value| value.fencing_token.clone())
                .context("route fencing proof disappeared before handoff journal commit")?,
            observed_at,
            outcome,
            native_job_id: native_job_id.clone(),
        });
        // An ambiguous timeout/crash is not a release signal: the native call
        // may still have crossed the spooler boundary. Keep the route fenced
        // until reconciliation or explicit operator resolution instead of
        // allowing a different connector to create a duplicate.
        if outcome != NativeHandoffOutcome::Ambiguous {
            self.record_consumed_authoritative_route(&active, outcome, native_job_id, observed_at)?;
            self.document
                .reservations
                .remove(&reservation.coordination_key);
        }
        Ok(())
    }

    /// Removes only handoff evidence explicitly acknowledged by its connector.
    /// Capacity pressure never discards replay barriers implicitly.
    pub fn acknowledge_handoffs(
        &mut self,
        connector_id: &str,
        through_sequence: u64,
    ) -> Result<()> {
        self.ensure_operational()?;
        let previous = self.document.clone();
        if !self
            .document
            .acknowledged_handoff_sequences
            .contains_key(connector_id)
            && self.document.acknowledged_handoff_sequences.len() >= MAX_ACKNOWLEDGED_CONNECTORS
        {
            bail!("handoff acknowledgement journal is full");
        }
        self.document
            .acknowledged_handoff_sequences
            .entry(connector_id.to_owned())
            .and_modify(|current| *current = (*current).max(through_sequence))
            .or_insert(through_sequence);
        self.document.handoffs.retain(|handoff| {
            handoff.connector_id != connector_id
                || handoff.sequence > through_sequence
                || handoff.outcome == NativeHandoffOutcome::Ambiguous
        });
        self.commit_document_change(previous)
    }

    pub fn handoffs_for_connector(
        &self,
        connector_id: &str,
        after_sequence: u64,
    ) -> Vec<NativeHandoffEvidence> {
        self.document
            .handoffs
            .iter()
            .filter(|handoff| {
                handoff.connector_id == connector_id && handoff.sequence > after_sequence
            })
            .filter_map(|handoff| {
                Some(NativeHandoffEvidence {
                    sequence: handoff.sequence,
                    route_id: handoff.server_route_id.clone(),
                    local_route_key: handoff.local_route_key.clone(),
                    job_id: handoff.job_id.parse().ok()?,
                    reservation_id: handoff.reservation_id,
                    fencing_generation: handoff.fencing_generation,
                    fencing_token: handoff.fencing_token.clone(),
                    observed_at: handoff.observed_at,
                    outcome: handoff.outcome,
                    native_job_id: handoff.native_job_id.clone(),
                })
            })
            .take(100)
            .collect()
    }

    /// Returns the bounded local-only handoff journal without requiring local
    /// deterministic job identifiers to parse as cloud `JobId` values.
    pub fn local_handoffs_for_connector(&self, connector_id: &str) -> Vec<LocalHandoffEvidence> {
        self.document
            .handoffs
            .iter()
            .filter(|handoff| {
                handoff.connector_id == connector_id && handoff.server_route_id.is_none()
            })
            .map(|handoff| LocalHandoffEvidence {
                sequence: handoff.sequence,
                job_id: handoff.job_id.clone(),
                ambiguity_id: local_ambiguity_id(
                    &handoff.connector_id,
                    &handoff.job_id,
                    &handoff.local_route_key,
                    handoff.reservation_id,
                    handoff.fencing_generation,
                ),
                observed_at: handoff.observed_at,
                outcome: handoff.outcome,
                native_job_id: handoff.native_job_id.clone(),
            })
            .collect()
    }

    /// Compacts only local, non-ambiguous handoffs whose exact `SQLite` outcome
    /// has already committed. Cloud evidence remains server-acknowledged.
    pub fn compact_local_terminal_handoffs(
        &mut self,
        connector_id: &str,
        commits: &[LocalHandoffCommit],
    ) -> Result<usize> {
        self.ensure_operational()?;
        if commits.len() > MAX_HANDOFFS {
            bail!("local handoff commit batch exceeds supported bounds");
        }
        let by_job = commits
            .iter()
            .map(|commit| (commit.job_id.as_str(), commit))
            .collect::<BTreeMap<_, _>>();
        let previous = self.document.clone();
        let before = self.document.handoffs.len();
        self.document.handoffs.retain(|handoff| {
            if handoff.connector_id != connector_id
                || handoff.server_route_id.is_some()
                || handoff.outcome == NativeHandoffOutcome::Ambiguous
            {
                return true;
            }
            let Some(commit) = by_job.get(handoff.job_id.as_str()) else {
                return true;
            };
            let durably_projected = match handoff.outcome {
                NativeHandoffOutcome::Accepted => {
                    let accepted_state = matches!(
                        commit.state.as_str(),
                        "accepted_by_spooler"
                            | "spooling"
                            | "printing"
                            | "completed_reported"
                            | "blocked"
                            | "cancel_requested"
                            | "delivery_uncertain"
                            | "cancelled"
                            | "failed_terminal"
                    );
                    let confirmed_without_native_id = commit.state == "delivery_uncertain"
                        && handoff.native_job_id.is_none()
                        && commit.ambiguity_confirmed;
                    accepted_state
                        && commit.native_job_id == handoff.native_job_id
                        && (handoff.native_job_id.is_some()
                            || commit.state != "delivery_uncertain"
                            || confirmed_without_native_id)
                }
                NativeHandoffOutcome::RejectedBeforeHandoff => {
                    matches!(
                        commit.state.as_str(),
                        "failed_retryable" | "failed_terminal" | "cancelled" | "expired"
                    ) || (commit.native_job_id.is_some()
                        && matches!(
                            commit.state.as_str(),
                            "accepted_by_spooler"
                                | "spooling"
                                | "printing"
                                | "completed_reported"
                                | "blocked"
                                | "cancel_requested"
                                | "delivery_uncertain"
                        ))
                }
                NativeHandoffOutcome::Ambiguous => false,
            };
            !durably_projected
        });
        let removed = before.saturating_sub(self.document.handoffs.len());
        if removed == 0 {
            return Ok(0);
        }
        self.commit_document_change(previous)?;
        Ok(removed)
    }

    pub fn cloud_proof_for_job(
        &self,
        connector_id: &str,
        job_id: &str,
    ) -> Option<piqae_protocol::agent::CloudRouteReservation> {
        self.document
            .reservations
            .values()
            .find(|reservation| {
                reservation.connector_id == connector_id && reservation.job_id == job_id
            })
            .and_then(|reservation| {
                Some(piqae_protocol::agent::CloudRouteReservation {
                    route_id: reservation.server_route_id.clone()?,
                    local_route_key: reservation.local_route_key.clone(),
                    reservation_id: reservation.reservation_id,
                    generation: reservation.generation,
                    fencing_token: reservation.fencing_token.clone(),
                    lease_expires_at: DateTime::from_timestamp_millis(reservation.expires_unix_ms)?,
                })
            })
    }

    pub fn resolve_ambiguous_handoff(
        &mut self,
        connector_id: &str,
        job_id: &str,
        local_route_key: &str,
        reservation_id: Uuid,
        generation: u64,
        resolution: piqae_protocol::agent::AmbiguousHandoffResolution,
    ) -> Result<()> {
        self.ensure_operational()?;
        let previous = self.document.clone();
        if let Err(error) = self.resolve_ambiguous_handoff_inner(
            connector_id,
            job_id,
            local_route_key,
            reservation_id,
            generation,
            resolution,
        ) {
            self.document = previous;
            return Err(error);
        }
        self.commit_document_change(previous)
    }

    fn resolve_ambiguous_handoff_inner(
        &mut self,
        connector_id: &str,
        job_id: &str,
        local_route_key: &str,
        reservation_id: Uuid,
        generation: u64,
        resolution: piqae_protocol::agent::AmbiguousHandoffResolution,
    ) -> Result<()> {
        let resolved_outcome = match resolution {
            piqae_protocol::agent::AmbiguousHandoffResolution::ReleaseForRetry => {
                NativeHandoffOutcome::RejectedBeforeHandoff
            }
            piqae_protocol::agent::AmbiguousHandoffResolution::ConfirmAccepted => {
                NativeHandoffOutcome::Accepted
            }
        };
        if let Some(consumed) =
            self.document
                .consumed_authoritative_routes
                .values()
                .find(|consumed| {
                    consumed.connector_id == connector_id
                        && consumed.job_id == job_id
                        && consumed.local_route_key == local_route_key
                        && consumed.reservation_id == reservation_id
                        && consumed.generation == generation
                })
        {
            if consumed.outcome == resolved_outcome {
                return Ok(());
            }
            bail!("ambiguous handoff was already resolved differently");
        }
        if let Some(existing) = self.document.handoffs.iter().rev().find(|handoff| {
            handoff.connector_id == connector_id
                && handoff.job_id == job_id
                && handoff.local_route_key == local_route_key
                && handoff.reservation_id == reservation_id
                && handoff.fencing_generation == generation
        }) && existing.outcome != NativeHandoffOutcome::Ambiguous
        {
            if existing.outcome == resolved_outcome {
                // Coordinator persistence and connector command-cursor
                // persistence are separate durable stores. A crash between
                // them must make an exact command replay succeed.
                return Ok(());
            }
            bail!("ambiguous handoff was already resolved differently");
        }
        let coordination_key = self
            .document
            .reservations
            .iter()
            .find_map(|(coordination_key, reservation)| {
                (reservation.connector_id == connector_id
                    && reservation.job_id == job_id
                    && reservation.local_route_key == local_route_key
                    && reservation.reservation_id == reservation_id
                    && reservation.generation == generation)
                    .then(|| coordination_key.clone())
            })
            .context("ambiguous route reservation does not match active fence")?;
        let handoff = self
            .document
            .handoffs
            .iter_mut()
            .rev()
            .find(|handoff| {
                handoff.connector_id == connector_id
                    && handoff.job_id == job_id
                    && handoff.local_route_key == local_route_key
                    && handoff.reservation_id == reservation_id
                    && handoff.fencing_generation == generation
                    && handoff.outcome == NativeHandoffOutcome::Ambiguous
            })
            .context("matching native handoff is not ambiguous")?;
        handoff.outcome = resolved_outcome;
        let resolved_sequence = handoff.sequence;
        let resolved_native_job_id = handoff.native_job_id.clone();
        let resolved_at = handoff.observed_at;
        let active = self
            .document
            .reservations
            .get(&coordination_key)
            .cloned()
            .context("ambiguous route reservation disappeared before resolution")?;
        self.record_consumed_authoritative_route(
            &active,
            resolved_outcome,
            resolved_native_job_id,
            resolved_at,
        )?;
        self.document.reservations.remove(&coordination_key);
        let acknowledged = self
            .document
            .acknowledged_handoff_sequences
            .get(connector_id)
            .copied()
            .unwrap_or(0);
        if resolved_sequence <= acknowledged {
            self.document
                .handoffs
                .retain(|handoff| handoff.sequence != resolved_sequence);
        }
        Ok(())
    }

    fn record_consumed_authoritative_route(
        &mut self,
        reservation: &DurableReservation,
        outcome: NativeHandoffOutcome,
        native_job_id: Option<String>,
        observed_at: DateTime<Utc>,
    ) -> Result<()> {
        let Some(server_route_id) = reservation.server_route_id.as_ref() else {
            return Ok(());
        };
        let key = authoritative_route_key(
            &reservation.connector_id,
            server_route_id,
            &reservation.local_route_key,
            &reservation.job_id,
        );
        if !self
            .document
            .consumed_authoritative_routes
            .contains_key(&key)
            && self.document.consumed_authoritative_routes.len()
                >= MAX_CONSUMED_AUTHORITATIVE_ROUTES
        {
            bail!("authoritative route replay barrier journal is full");
        }
        let consumed = ConsumedAuthoritativeRoute {
            connector_id: reservation.connector_id.clone(),
            server_route_id: server_route_id.clone(),
            local_route_key: reservation.local_route_key.clone(),
            job_id: reservation.job_id.clone(),
            generation: reservation.generation,
            reservation_id: reservation.reservation_id,
            fencing_token: reservation.fencing_token.clone(),
            observed_at,
            outcome,
            native_job_id,
        };
        match self.document.consumed_authoritative_routes.get(&key) {
            Some(existing) if existing.generation >= consumed.generation => {}
            _ => {
                self.document
                    .consumed_authoritative_routes
                    .insert(key, consumed);
            }
        }
        Ok(())
    }

    fn record_topology_change(
        &mut self,
        route_id: &str,
        change: TopologyChange,
        observed_at: DateTime<Utc>,
    ) {
        self.document.topology_changes.push(RouteTopologyChange {
            local_route_key: route_id.to_owned(),
            topology_revision: self.document.topology_revision,
            observed_at,
            change,
        });
        trim_front(&mut self.document.topology_changes, MAX_TOPOLOGY_CHANGES);
    }

    fn route_requires_retention(&self, route: &DurableRoute) -> bool {
        self.document
            .reservations
            .values()
            .any(|reservation| reservation.local_route_key == route.route_id)
            || self.document.handoffs.iter().any(|handoff| {
                handoff.local_route_key == route.route_id
                    && handoff.outcome == NativeHandoffOutcome::Ambiguous
            })
    }

    fn prune_absent_routes(&mut self) {
        let retained_route_ids = self
            .document
            .routes
            .values()
            .filter(|route| !route.present && self.route_requires_retention(route))
            .map(|route| route.route_id.clone())
            .collect::<BTreeSet<_>>();
        self.document
            .routes
            .retain(|_, route| route.present || retained_route_ids.contains(&route.route_id));
    }

    /// Reconciles newly observed physical identity with the exclusion domains
    /// already occupied by unresolved reservations. An occupied domain wins
    /// over a newly derived physical key. If a proposed physical group would
    /// combine two independently occupied domains, every involved route is
    /// marked conflicted and refuses new reservations until later evidence is
    /// reconciled after those fences resolve.
    fn reconcile_coordination_keys(&mut self) {
        let active_keys = self
            .document
            .reservations
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        let mut desired_by_native = BTreeMap::new();
        let mut occupied_by_desired = BTreeMap::<String, BTreeSet<String>>::new();
        for (native_id, route) in &self.document.routes {
            let desired = physical_coordination_key(&route.route_id, &route.identity_evidence);
            if active_keys.contains(&route.coordination_key) {
                occupied_by_desired
                    .entry(desired.clone())
                    .or_default()
                    .insert(route.coordination_key.clone());
            }
            desired_by_native.insert(native_id.clone(), desired);
        }
        // The reservation's local route is authoritative even when an older
        // buggy journal already wrote a different effective key onto it.
        for (active_key, reservation) in &self.document.reservations {
            if let Some((_, desired)) = desired_by_native.iter().find(|(native_id, _)| {
                self.document
                    .routes
                    .get(*native_id)
                    .is_some_and(|route| route.route_id == reservation.local_route_key)
            }) {
                occupied_by_desired
                    .entry(desired.clone())
                    .or_default()
                    .insert(active_key.clone());
            }
        }
        for (native_id, route) in &mut self.document.routes {
            let Some(desired) = desired_by_native.get(native_id) else {
                continue;
            };
            match occupied_by_desired.get(desired) {
                Some(occupied) if occupied.len() > 1 => {
                    route.coordination_conflict = true;
                    // Preserve an occupied prior key so existing reservation
                    // handles still validate and can reach terminal evidence.
                    // Unoccupied aliases retain the observed key but cannot
                    // reserve while the conflict flag is set.
                    if !active_keys.contains(&route.coordination_key) {
                        route.coordination_key.clone_from(desired);
                    }
                }
                Some(occupied) if occupied.len() == 1 => {
                    if let Some(active_key) = occupied.first() {
                        route.coordination_key.clone_from(active_key);
                    }
                    route.coordination_conflict = false;
                }
                _ => {
                    route.coordination_key.clone_from(desired);
                    route.coordination_conflict = false;
                }
            }
        }
    }

    fn ensure_operational(&self) -> Result<()> {
        if self.poisoned {
            bail!("route coordinator is fail-stopped after an ambiguous journal write");
        }
        Ok(())
    }

    fn commit_document_change(&mut self, previous: CoordinatorDocument) -> Result<()> {
        let attempted = self.document.clone();
        if let Err(error) = self.persist() {
            let path = self.root.join("route-coordinator.json");
            match std::fs::read(&path)
                .with_context(|| format!("read {} after failed persistence", path.display()))
                .and_then(|bytes| {
                    serde_json::from_slice::<CoordinatorDocument>(&bytes).with_context(|| {
                        format!("parse {} after failed persistence", path.display())
                    })
                }) {
                Ok(disk) if documents_match(&disk, &previous) => {
                    self.document = previous;
                }
                Ok(disk) if documents_match(&disk, &attempted) => {
                    self.document = disk;
                    self.poisoned = true;
                }
                Ok(disk) => {
                    self.document = disk;
                    self.poisoned = true;
                }
                Err(_) => {
                    self.document = previous;
                    self.poisoned = true;
                }
            }
            return Err(error).context(if self.poisoned {
                "route coordinator journal outcome is ambiguous; coordinator fail-stopped"
            } else {
                "route coordinator journal change was not committed"
            });
        }
        Ok(())
    }

    fn persist(&self) -> Result<()> {
        #[cfg(test)]
        let persist_fault = self.persist_fault.replace(None);
        #[cfg(test)]
        if persist_fault == Some(PersistFault::BeforeReplaceTwice) {
            self.persist_fault.set(Some(PersistFault::BeforeReplace));
            bail!("injected repeated route coordinator failure before replacement");
        }
        #[cfg(test)]
        if persist_fault == Some(PersistFault::BeforeReplace) {
            bail!("injected route coordinator failure before replacement");
        }
        let path = self.root.join("route-coordinator.json");
        let bytes = serde_json::to_vec_pretty(&self.document)?;
        crate::durable_file::replace_json(&path, &bytes)?;
        #[cfg(test)]
        if persist_fault == Some(PersistFault::AfterReplace) {
            bail!("injected route coordinator failure after replacement");
        }
        Ok(())
    }
}

fn local_ambiguity_id(
    connector_id: &str,
    job_id: &str,
    local_route_key: &str,
    reservation_id: Uuid,
    generation: u64,
) -> String {
    let mut digest = Sha256::new();
    digest.update(b"piqae-local-ambiguity-v1\0");
    for value in [connector_id, job_id, local_route_key] {
        digest.update(u64::try_from(value.len()).unwrap_or(u64::MAX).to_be_bytes());
        digest.update(value.as_bytes());
    }
    digest.update(reservation_id.as_bytes());
    digest.update(generation.to_be_bytes());
    format!("amb_{}", &hex::encode(digest.finalize())[..32])
}

fn documents_match(left: &CoordinatorDocument, right: &CoordinatorDocument) -> bool {
    matches!(
        (serde_json::to_vec(left), serde_json::to_vec(right)),
        (Ok(left), Ok(right)) if left == right
    )
}

fn trim_front<T>(values: &mut Vec<T>, limit: usize) {
    if values.len() > limit {
        values.drain(..values.len() - limit);
    }
}

fn authoritative_route_key(
    connector_id: &str,
    server_route_id: &str,
    local_route_key: &str,
    job_id: &str,
) -> String {
    let mut digest = Sha256::new();
    for value in [connector_id, server_route_id, local_route_key, job_id] {
        digest.update(u64::try_from(value.len()).unwrap_or(u64::MAX).to_be_bytes());
        digest.update(value.as_bytes());
    }
    hex::encode(digest.finalize())
}

fn canonical_evidence(printer: &DiscoveredPrinter) -> Vec<PhysicalIdentityEvidence> {
    let mut evidence = printer
        .identity_evidence
        .iter()
        .filter(|item| {
            item.value_sha256.len() == 64
                && item
                    .value_sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit())
        })
        .cloned()
        .collect::<Vec<_>>();
    if let Some(driver) = &printer.driver_fingerprint
        && let Ok(encoded) = serde_json::to_vec(driver)
    {
        evidence.push(PhysicalIdentityEvidence {
            kind: PhysicalIdentityEvidenceKind::DriverFingerprint,
            value_sha256: hex::encode(Sha256::digest(encoded)),
            strength: IdentityEvidenceStrength::Medium,
        });
    }
    // The installation/native queue hash is the route ID, not physical-device
    // evidence. Keeping it out of this list prevents similarly named queues
    // on different computers from being inferred as one printer.
    evidence.sort_by_key(|item| (item.kind as u8, item.value_sha256.clone()));
    evidence
        .dedup_by(|left, right| left.kind == right.kind && left.value_sha256 == right.value_sha256);
    evidence.truncate(16);
    evidence
}

fn identity_confidence(evidence: &[PhysicalIdentityEvidence]) -> IdentityConfidence {
    let has_strong = evidence
        .iter()
        .any(|item| item.strength == IdentityEvidenceStrength::Strong);
    let has_endpoint = evidence.iter().any(|item| {
        item.strength == IdentityEvidenceStrength::Medium
            && item.kind == PhysicalIdentityEvidenceKind::NetworkEndpoint
    });
    let has_device_description = evidence.iter().any(|item| {
        item.strength == IdentityEvidenceStrength::Medium
            && matches!(
                item.kind,
                PhysicalIdentityEvidenceKind::ManufacturerModel
                    | PhysicalIdentityEvidenceKind::CapabilityFingerprint
            )
    });
    let has_medium = evidence
        .iter()
        .any(|item| item.strength == IdentityEvidenceStrength::Medium);
    if has_strong || (has_endpoint && has_device_description) {
        IdentityConfidence::High
    } else if has_medium {
        IdentityConfidence::Possible
    } else {
        IdentityConfidence::Unknown
    }
}

fn topology_fingerprint(evidence: &[PhysicalIdentityEvidence]) -> String {
    let encoded = serde_json::to_vec(evidence).unwrap_or_default();
    hex::encode(Sha256::digest(encoded))
}

fn physical_coordination_key(
    local_route_key: &str,
    evidence: &[PhysicalIdentityEvidence],
) -> String {
    let mut strong = evidence
        .iter()
        .filter(|item| item.strength == IdentityEvidenceStrength::Strong)
        .map(|item| (item.kind as u8, item.value_sha256.as_str()))
        .collect::<Vec<_>>();
    strong.sort_unstable();
    strong.dedup();
    // A single strong, canonical device identity is safe for local grouping.
    // Multiple strong identities can represent a composite device or a
    // conflict and require server/operator reconciliation, so they fall back
    // to the exact OS route rather than risking false serialization/merging.
    if let [identity] = strong.as_slice() {
        let mut digest = Sha256::new();
        digest.update(b"piqae-physical-coordination-v1\0");
        digest.update([identity.0]);
        digest.update(identity.1.as_bytes());
        return format!("pgrp_{}", &hex::encode(digest.finalize())[..32]);
    }
    let endpoints = evidence
        .iter()
        .filter(|item| {
            item.strength == IdentityEvidenceStrength::Medium
                && item.kind == PhysicalIdentityEvidenceKind::NetworkEndpoint
        })
        .collect::<Vec<_>>();
    let device_descriptions = evidence
        .iter()
        .filter(|item| {
            item.strength == IdentityEvidenceStrength::Medium
                && matches!(
                    item.kind,
                    PhysicalIdentityEvidenceKind::ManufacturerModel
                        | PhysicalIdentityEvidenceKind::CapabilityFingerprint
                )
        })
        .collect::<Vec<_>>();
    if let [endpoint] = endpoints.as_slice()
        && !device_descriptions.is_empty()
    {
        let mut digest = Sha256::new();
        digest.update(b"piqae-physical-coordination-v1\0medium\0");
        digest.update(endpoint.value_sha256.as_bytes());
        for description in device_descriptions {
            digest.update([description.kind as u8]);
            digest.update(description.value_sha256.as_bytes());
        }
        return format!("pgrp_{}", &hex::encode(digest.finalize())[..32]);
    }
    local_route_key.to_owned()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use piqae_domain::{PrinterCapabilities, PrinterState};
    use tempfile::TempDir;

    fn printer(native_id: &str, evidence: Vec<PhysicalIdentityEvidence>) -> DiscoveredPrinter {
        DiscoveredPrinter {
            native_id: native_id.into(),
            name: "Test printer".into(),
            is_default: true,
            state: PrinterState::Online,
            capabilities: PrinterCapabilities::default(),
            native_options: BTreeMap::new(),
            driver_fingerprint: None,
            identity_evidence: evidence,
        }
    }

    fn strong_aliases(evidence: &PhysicalIdentityEvidence) -> Vec<DiscoveredPrinter> {
        ["native-a", "native-b", "native-new-alias"]
            .into_iter()
            .map(|native_id| printer(native_id, vec![evidence.clone()]))
            .collect()
    }

    fn reject_before_handoff(
        coordinator: &mut RouteCoordinator,
        connector_id: &str,
        job_id: &str,
        reservation: &RouteReservation,
    ) {
        coordinator
            .finish(
                connector_id,
                job_id,
                reservation,
                NativeHandoffOutcome::RejectedBeforeHandoff,
                None,
                Utc::now(),
            )
            .unwrap();
    }

    fn cloud_reservation(
        coordinator: &RouteCoordinator,
        native_id: &str,
        route_id: &str,
        generation: u64,
    ) -> piqae_protocol::agent::CloudRouteReservation {
        piqae_protocol::agent::CloudRouteReservation {
            route_id: route_id.into(),
            local_route_key: coordinator.route_id(native_id),
            reservation_id: Uuid::new_v4(),
            generation,
            fencing_token: format!("fence-{route_id}-{generation}"),
            lease_expires_at: Utc::now() + chrono::Duration::minutes(1),
        }
    }

    #[test]
    fn route_identity_is_shared_by_connectors_and_survives_restart() {
        let root = TempDir::new().unwrap();
        let first = RouteCoordinator::open(root.path()).unwrap();
        let route = first.route_id("native-a");
        drop(first);
        let restarted = RouteCoordinator::open(root.path()).unwrap();
        assert_eq!(route, restarted.route_id("native-a"));
        assert_ne!(route, restarted.route_id("native-b"));
    }

    #[test]
    fn live_observation_sequence_is_monotonic_across_restart() {
        let root = TempDir::new().unwrap();
        let mut first = RouteCoordinator::open(root.path()).unwrap();
        assert_eq!(first.allocate_observation_sequences(2).unwrap(), vec![1, 2]);
        let reserved_through = first.document.observation_sequence;
        assert_eq!(first.allocate_observation_sequences(2).unwrap(), vec![3, 4]);
        assert_eq!(
            first.document.observation_sequence, reserved_through,
            "hot-path allocations must use the crash-safe reserved range"
        );
        drop(first);
        let mut restarted = RouteCoordinator::open(root.path()).unwrap();
        let after_restart = restarted.allocate_observation_sequences(2).unwrap();
        assert!(
            after_restart[0] > reserved_through,
            "a restart must skip the unused durable reservation rather than reuse it"
        );
    }

    #[test]
    fn stale_absent_routes_are_pruned_without_releasing_unresolved_handoffs() {
        let root = TempDir::new().unwrap();
        let mut coordinator = RouteCoordinator::open(root.path()).unwrap();
        coordinator
            .reconcile(&[printer("protected", Vec::new())], 1, Utc::now())
            .unwrap();
        let protected = coordinator
            .reserve("hosted", "protected", "job_protected", 1)
            .unwrap();
        coordinator
            .finish(
                "hosted",
                "job_protected",
                &protected,
                NativeHandoffOutcome::Ambiguous,
                None,
                Utc::now(),
            )
            .unwrap();
        coordinator.reconcile(&[], 2, Utc::now()).unwrap();

        for revision in 3..=(u64::try_from(MAX_ROUTES).unwrap() + 12) {
            let native_id = format!("transient-{revision}");
            coordinator
                .reconcile(&[printer(&native_id, Vec::new())], revision, Utc::now())
                .unwrap();
        }
        assert!(coordinator.document.routes.len() <= MAX_ROUTES);
        assert!(coordinator.document.routes.contains_key("protected"));
        assert!(
            coordinator
                .reserve("self_hosted", "protected", "job_other", 999)
                .is_err(),
            "pruning must not release an unresolved physical handoff fence"
        );

        drop(coordinator);
        let mut restarted = RouteCoordinator::open(root.path()).unwrap();
        assert!(restarted.document.routes.len() <= MAX_ROUTES);
        assert!(
            restarted
                .reserve("self_hosted", "protected", "job_other", 1_000)
                .is_err(),
            "restart repair must preserve the unresolved exclusion domain"
        );
    }

    #[test]
    fn topology_tracks_add_change_remove_without_name_matching() {
        let root = TempDir::new().unwrap();
        let mut coordinator = RouteCoordinator::open(root.path()).unwrap();
        let observed = Utc::now();
        let first = printer("native-a", Vec::new());
        let snapshots = coordinator.reconcile(&[first], 1, observed).unwrap();
        assert_eq!(
            snapshots["native-a"].topology_change,
            Some(TopologyChange::Added)
        );
        let strong = PhysicalIdentityEvidence {
            kind: PhysicalIdentityEvidenceKind::IppPrinterUuid,
            value_sha256: "a".repeat(64),
            strength: IdentityEvidenceStrength::Strong,
        };
        let changed = printer("native-a", vec![strong]);
        let snapshots = coordinator.reconcile(&[changed], 2, observed).unwrap();
        assert_eq!(
            snapshots["native-a"].topology_change,
            Some(TopologyChange::Changed)
        );
        coordinator.reconcile(&[], 3, observed).unwrap();
        assert!(
            coordinator
                .topology_changes()
                .iter()
                .any(|change| change.change == TopologyChange::Removed)
        );
    }

    #[test]
    fn one_route_is_fenced_across_connectors_and_stale_tokens_fail() {
        let root = TempDir::new().unwrap();
        let mut coordinator = RouteCoordinator::open(root.path()).unwrap();
        coordinator
            .reconcile(&[printer("native-a", Vec::new())], 1, Utc::now())
            .unwrap();
        let first = coordinator
            .reserve("hosted", "native-a", "job_a", 1)
            .unwrap();
        assert!(
            coordinator
                .reserve("self_hosted", "native-a", "job_b", 2)
                .is_err()
        );
        coordinator
            .finish(
                "hosted",
                "job_a",
                &first,
                NativeHandoffOutcome::RejectedBeforeHandoff,
                None,
                Utc::now(),
            )
            .unwrap();
        let second = coordinator
            .reserve("self_hosted", "native-a", "job_b", 3)
            .unwrap();
        assert!(coordinator.validate(&first).is_err());
        assert!(coordinator.validate(&second).is_ok());
    }

    #[test]
    fn strong_physical_identity_serializes_two_native_queues_but_weak_evidence_does_not() {
        let strong = PhysicalIdentityEvidence {
            kind: PhysicalIdentityEvidenceKind::IppPrinterUuid,
            value_sha256: "a".repeat(64),
            strength: IdentityEvidenceStrength::Strong,
        };
        let weak = PhysicalIdentityEvidence {
            kind: PhysicalIdentityEvidenceKind::DriverFingerprint,
            value_sha256: "b".repeat(64),
            strength: IdentityEvidenceStrength::Weak,
        };
        let root = TempDir::new().unwrap();
        let mut coordinator = RouteCoordinator::open(root.path()).unwrap();
        coordinator
            .reconcile(
                &[
                    printer("native-primary", vec![strong.clone()]),
                    printer("native-alias", vec![strong]),
                    printer("native-similar", vec![weak]),
                ],
                1,
                Utc::now(),
            )
            .unwrap();
        let primary = coordinator
            .reserve("hosted", "native-primary", "job_a", 1)
            .unwrap();
        assert!(
            coordinator
                .reserve("self_hosted", "native-alias", "job_b", 2)
                .is_err(),
            "verified physical aliases must share one final handoff fence"
        );
        assert!(
            coordinator
                .reserve("local", "native-similar", "job_c", 3)
                .is_ok(),
            "weak similarity must not merge physical routes"
        );
        coordinator
            .finish(
                "hosted",
                "job_a",
                &primary,
                NativeHandoffOutcome::RejectedBeforeHandoff,
                None,
                Utc::now(),
            )
            .unwrap();
        assert!(
            coordinator
                .reserve("self_hosted", "native-alias", "job_b", 4)
                .is_ok()
        );
    }

    #[test]
    fn reservation_evidence_upgrade_and_new_alias_keep_the_old_fence_after_restart() {
        let root = TempDir::new().unwrap();
        let mut coordinator = RouteCoordinator::open(root.path()).unwrap();
        coordinator
            .reconcile(&[printer("native-primary", Vec::new())], 1, Utc::now())
            .unwrap();
        let held = coordinator
            .reserve("hosted", "native-primary", "job_a", 1)
            .unwrap();
        let old_exclusion_domain = held.coordination_key.clone();
        let strong = PhysicalIdentityEvidence {
            kind: PhysicalIdentityEvidenceKind::IppPrinterUuid,
            value_sha256: "a".repeat(64),
            strength: IdentityEvidenceStrength::Strong,
        };
        coordinator
            .reconcile(
                &[
                    printer("native-primary", vec![strong.clone()]),
                    printer("native-alias", vec![strong]),
                ],
                2,
                Utc::now(),
            )
            .unwrap();
        assert_eq!(
            coordinator.coordination_key("native-primary"),
            Some(old_exclusion_domain.as_str())
        );
        assert_eq!(
            coordinator.coordination_key("native-alias"),
            Some(old_exclusion_domain.as_str())
        );
        assert!(
            coordinator
                .reserve("self_hosted", "native-alias", "job_b", 2)
                .is_err(),
            "new physical aliases must join the unresolved old exclusion domain"
        );

        drop(coordinator);
        let mut restarted = RouteCoordinator::open(root.path()).unwrap();
        assert_eq!(
            restarted.coordination_key("native-alias"),
            Some(old_exclusion_domain.as_str())
        );
        assert!(
            restarted
                .reserve("self_hosted", "native-alias", "job_b", 3)
                .is_err(),
            "restart must not let evidence re-keying escape an active fence"
        );
        restarted
            .finish(
                "hosted",
                "job_a",
                &held,
                NativeHandoffOutcome::RejectedBeforeHandoff,
                None,
                Utc::now(),
            )
            .unwrap();
    }

    #[test]
    fn independently_reserved_groups_fail_closed_then_regroup_after_resolution() {
        let root = TempDir::new().unwrap();
        let mut coordinator = RouteCoordinator::open(root.path()).unwrap();
        coordinator
            .reconcile(
                &[
                    printer("native-a", Vec::new()),
                    printer("native-b", Vec::new()),
                ],
                1,
                Utc::now(),
            )
            .unwrap();
        let first = coordinator
            .reserve("hosted", "native-a", "job_a", 1)
            .unwrap();
        let second = coordinator
            .reserve("self_hosted", "native-b", "job_b", 1)
            .unwrap();
        let strong = PhysicalIdentityEvidence {
            kind: PhysicalIdentityEvidenceKind::IppPrinterUuid,
            value_sha256: "b".repeat(64),
            strength: IdentityEvidenceStrength::Strong,
        };
        coordinator
            .reconcile(&strong_aliases(&strong), 2, Utc::now())
            .unwrap();
        assert!(
            coordinator
                .document
                .routes
                .values()
                .all(|route| route.coordination_conflict)
        );
        assert!(
            coordinator
                .reserve("third", "native-new-alias", "job_c", 2)
                .is_err(),
            "an alias cannot enter a physical group with two occupied domains"
        );

        drop(coordinator);
        let mut restarted = RouteCoordinator::open(root.path()).unwrap();
        assert!(
            restarted
                .reserve("third", "native-new-alias", "job_c", 3)
                .is_err(),
            "the merge conflict must remain fail-closed across restart"
        );
        reject_before_handoff(&mut restarted, "hosted", "job_a", &first);
        reject_before_handoff(&mut restarted, "self_hosted", "job_b", &second);
        assert!(
            restarted
                .reserve("third", "native-new-alias", "job_c", 4)
                .is_err(),
            "terminal resolution is persisted before a later reconcile clears the conflict"
        );
        restarted
            .reconcile(&strong_aliases(&strong), 3, Utc::now())
            .unwrap();
        assert!(
            restarted
                .document
                .routes
                .values()
                .all(|route| !route.coordination_conflict)
        );
        assert_eq!(
            restarted.coordination_key("native-a"),
            restarted.coordination_key("native-b")
        );
        let regrouped = restarted
            .reserve("third", "native-new-alias", "job_c", 5)
            .unwrap();
        assert!(
            restarted.reserve("hosted", "native-a", "job_d", 6).is_err(),
            "safely regrouped aliases must share the next reservation"
        );
        restarted
            .finish(
                "third",
                "job_c",
                &regrouped,
                NativeHandoffOutcome::RejectedBeforeHandoff,
                None,
                Utc::now(),
            )
            .unwrap();
    }

    #[test]
    fn correlated_model_and_capability_are_not_high_confidence_or_auto_grouped() {
        let model = PhysicalIdentityEvidence {
            kind: PhysicalIdentityEvidenceKind::ManufacturerModel,
            value_sha256: "c".repeat(64),
            strength: IdentityEvidenceStrength::Medium,
        };
        let capabilities = PhysicalIdentityEvidence {
            kind: PhysicalIdentityEvidenceKind::CapabilityFingerprint,
            value_sha256: "d".repeat(64),
            strength: IdentityEvidenceStrength::Medium,
        };
        let endpoint_a = PhysicalIdentityEvidence {
            kind: PhysicalIdentityEvidenceKind::NetworkEndpoint,
            value_sha256: "e".repeat(64),
            strength: IdentityEvidenceStrength::Medium,
        };
        let endpoint_b = PhysicalIdentityEvidence {
            kind: PhysicalIdentityEvidenceKind::NetworkEndpoint,
            value_sha256: "f".repeat(64),
            strength: IdentityEvidenceStrength::Medium,
        };
        assert_eq!(
            identity_confidence(&[model.clone(), capabilities.clone()]),
            IdentityConfidence::Possible
        );
        assert_ne!(
            physical_coordination_key(
                "route-a",
                &[model.clone(), capabilities.clone(), endpoint_a]
            ),
            physical_coordination_key("route-b", &[model, capabilities, endpoint_b]),
            "identical-model printers at different endpoints must not merge"
        );
    }

    #[test]
    fn accepted_handoff_prevents_restart_replay() {
        let root = TempDir::new().unwrap();
        let mut coordinator = RouteCoordinator::open(root.path()).unwrap();
        let reservation = coordinator
            .reserve("hosted", "native-a", "job_01ARZ3NDEKTSV4RRFFQ69G5FAV", 1)
            .unwrap();
        coordinator
            .finish(
                "hosted",
                "job_01ARZ3NDEKTSV4RRFFQ69G5FAV",
                &reservation,
                NativeHandoffOutcome::Accepted,
                Some("native-1".into()),
                Utc::now(),
            )
            .unwrap();
        drop(coordinator);
        let mut restarted = RouteCoordinator::open(root.path()).unwrap();
        assert!(
            restarted
                .reserve("hosted", "native-a", "job_01ARZ3NDEKTSV4RRFFQ69G5FAV", 2)
                .is_err()
        );
        assert_eq!(restarted.handoffs_for_connector("hosted", 0).len(), 1);
        assert!(
            restarted
                .handoffs_for_connector("self_hosted", 0)
                .is_empty()
        );
    }

    #[test]
    fn replay_barriers_fail_closed_at_bound_and_compact_only_after_acknowledgement() {
        let root = TempDir::new().unwrap();
        let mut coordinator = RouteCoordinator::open(root.path()).unwrap();
        for sequence in 1..=MAX_HANDOFFS {
            coordinator.document.handoffs.push(DurableHandoff {
                sequence: u64::try_from(sequence).unwrap(),
                connector_id: "hosted".into(),
                server_route_id: None,
                local_route_key: format!("route-{sequence}"),
                job_id: piqae_domain::JobId::new().to_string(),
                reservation_id: Uuid::new_v4(),
                fencing_generation: 1,
                fencing_token: format!("fence-{sequence}"),
                observed_at: Utc::now(),
                outcome: NativeHandoffOutcome::Accepted,
                native_job_id: Some(format!("native-{sequence}")),
            });
        }
        coordinator.document.handoff_sequence = u64::try_from(MAX_HANDOFFS).unwrap();
        coordinator.persist().unwrap();
        assert!(
            coordinator
                .reserve("hosted", "native-new", "job-new", 1)
                .is_err(),
            "an unacknowledged replay barrier must never be evicted to admit new work"
        );

        drop(coordinator);
        let mut restarted = RouteCoordinator::open(root.path()).unwrap();
        assert_eq!(restarted.document.handoffs.len(), MAX_HANDOFFS);
        restarted.acknowledge_handoffs("hosted", 256).unwrap();
        assert_eq!(restarted.document.handoffs.len(), 256);
        assert!(
            restarted
                .reserve("hosted", "native-new", "job-new", 2)
                .is_ok(),
            "explicit durable acknowledgement is the only compaction gate"
        );
    }

    #[test]
    fn elapsed_deadline_never_steals_an_unknown_handoff() {
        let root = TempDir::new().unwrap();
        let mut coordinator = RouteCoordinator::open(root.path()).unwrap();
        coordinator
            .reconcile(&[printer("native-a", Vec::new())], 1, Utc::now())
            .unwrap();
        coordinator
            .reserve("hosted", "native-a", "job_a", 1)
            .unwrap();
        assert!(
            coordinator
                .reserve(
                    "self_hosted",
                    "native-a",
                    "job_b",
                    1 + RESERVATION_LIFETIME_MS + 1,
                )
                .is_err()
        );
    }

    #[test]
    fn ambiguous_native_handoff_keeps_route_fenced_after_restart() {
        let root = TempDir::new().unwrap();
        let mut coordinator = RouteCoordinator::open(root.path()).unwrap();
        coordinator
            .reconcile(&[printer("native-a", Vec::new())], 1, Utc::now())
            .unwrap();
        let reservation = coordinator
            .reserve("hosted", "native-a", "job_a", 1)
            .unwrap();
        coordinator
            .finish(
                "hosted",
                "job_a",
                &reservation,
                NativeHandoffOutcome::Ambiguous,
                None,
                Utc::now(),
            )
            .unwrap();
        drop(coordinator);
        let mut restarted = RouteCoordinator::open(root.path()).unwrap();
        assert!(
            restarted
                .reserve("self_hosted", "native-a", "job_b", 99)
                .is_err()
        );
        assert!(
            restarted
                .resolve_ambiguous_handoff(
                    "hosted",
                    "job_a",
                    &reservation.local_route_key,
                    Uuid::new_v4(),
                    reservation.generation,
                    piqae_protocol::agent::AmbiguousHandoffResolution::ReleaseForRetry,
                )
                .is_err(),
            "a stale or incorrect resolution must not unlock the route"
        );
        restarted
            .resolve_ambiguous_handoff(
                "hosted",
                "job_a",
                &reservation.local_route_key,
                reservation.reservation_id,
                reservation.generation,
                piqae_protocol::agent::AmbiguousHandoffResolution::ReleaseForRetry,
            )
            .unwrap();
        restarted
            .resolve_ambiguous_handoff(
                "hosted",
                "job_a",
                &reservation.local_route_key,
                reservation.reservation_id,
                reservation.generation,
                piqae_protocol::agent::AmbiguousHandoffResolution::ReleaseForRetry,
            )
            .unwrap();
        assert!(
            restarted
                .resolve_ambiguous_handoff(
                    "hosted",
                    "job_a",
                    &reservation.local_route_key,
                    reservation.reservation_id,
                    reservation.generation,
                    piqae_protocol::agent::AmbiguousHandoffResolution::ConfirmAccepted,
                )
                .is_err(),
            "a replay is idempotent but a conflicting resolution must fail"
        );
        assert!(
            restarted
                .reserve("hosted", "native-a", "job_a", 100)
                .is_ok(),
            "explicit durable release-for-retry permits the original job again"
        );
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "release and confirm resolution variants share the same acknowledgement/restart proof"
    )]
    fn acknowledged_ambiguous_handoff_survives_restart_until_explicit_resolution() {
        for (resolution, conflicting) in [
            (
                piqae_protocol::agent::AmbiguousHandoffResolution::ReleaseForRetry,
                piqae_protocol::agent::AmbiguousHandoffResolution::ConfirmAccepted,
            ),
            (
                piqae_protocol::agent::AmbiguousHandoffResolution::ConfirmAccepted,
                piqae_protocol::agent::AmbiguousHandoffResolution::ReleaseForRetry,
            ),
        ] {
            let root = TempDir::new().unwrap();
            let mut coordinator = RouteCoordinator::open(root.path()).unwrap();
            coordinator
                .reconcile(
                    &[
                        printer("native-a", Vec::new()),
                        printer("native-b", Vec::new()),
                    ],
                    1,
                    Utc::now(),
                )
                .unwrap();
            let job_id = "job_01ARZ3NDEKTSV4RRFFQ69G5FAV";
            let local_route_key = coordinator.route_id("native-a");
            let cloud = piqae_protocol::agent::CloudRouteReservation {
                route_id: "rte_01ARZ3NDEKTSV4RRFFQ69G5FAV".into(),
                local_route_key: local_route_key.clone(),
                reservation_id: Uuid::new_v4(),
                generation: 5,
                fencing_token: "ambiguous-resolution-fence".into(),
                lease_expires_at: Utc::now() + chrono::Duration::minutes(1),
            };
            let reservation = coordinator
                .register_authoritative("hosted", "native-a", job_id, &cloud, Utc::now())
                .unwrap()
                .reservation;
            coordinator
                .finish(
                    "hosted",
                    job_id,
                    &reservation,
                    NativeHandoffOutcome::Ambiguous,
                    None,
                    Utc::now(),
                )
                .unwrap();
            let sequence = coordinator.handoffs_for_connector("hosted", 0)[0].sequence;
            coordinator
                .acknowledge_handoffs("hosted", sequence)
                .unwrap();
            assert!(
                coordinator
                    .handoffs_for_connector("hosted", sequence)
                    .is_empty(),
                "acknowledged evidence is suppressed from outbound replay"
            );
            assert!(
                coordinator
                    .reserve("hosted", "native-a", "other-before-resolution", 1)
                    .is_err()
            );
            drop(coordinator);

            let mut coordinator = RouteCoordinator::open(root.path()).unwrap();
            assert!(
                coordinator
                    .reserve("hosted", "native-a", "other-after-restart", 2)
                    .is_err(),
                "acknowledgement alone must not release an ambiguous native fence"
            );
            coordinator
                .resolve_ambiguous_handoff(
                    "hosted",
                    job_id,
                    &local_route_key,
                    cloud.reservation_id,
                    cloud.generation,
                    resolution,
                )
                .unwrap();
            assert!(coordinator.document.handoffs.is_empty());
            coordinator
                .resolve_ambiguous_handoff(
                    "hosted",
                    job_id,
                    &local_route_key,
                    cloud.reservation_id,
                    cloud.generation,
                    resolution,
                )
                .unwrap();
            let reroute = cloud_reservation(&coordinator, "native-b", "rerouted-job", 99);
            let reroute_result = coordinator.register_authoritative(
                "hosted",
                "native-b",
                job_id,
                &reroute,
                Utc::now(),
            );
            if resolution == piqae_protocol::agent::AmbiguousHandoffResolution::ConfirmAccepted {
                assert!(
                    reroute_result.is_err(),
                    "confirmed native acceptance is connector/job terminal across route changes"
                );
            } else {
                assert!(
                    reroute_result.is_ok(),
                    "an explicit release permits fresh authority on a changed route"
                );
            }
            assert!(
                coordinator
                    .resolve_ambiguous_handoff(
                        "hosted",
                        job_id,
                        &local_route_key,
                        cloud.reservation_id,
                        cloud.generation,
                        conflicting,
                    )
                    .is_err(),
                "a conflicting resolution can never replace durable authority"
            );
            assert!(
                coordinator
                    .reserve("hosted", "native-a", "other-after-resolution", 3)
                    .is_ok(),
                "only explicit resolution releases the physical route"
            );
        }
    }

    #[test]
    fn cloud_fence_is_authoritative_and_evidence_echoes_its_generation() {
        let root = TempDir::new().unwrap();
        let mut coordinator = RouteCoordinator::open(root.path()).unwrap();
        coordinator
            .reconcile(&[printer("native-a", Vec::new())], 1, Utc::now())
            .unwrap();
        let local_route_key = coordinator.route_id("native-a");
        let cloud = piqae_protocol::agent::CloudRouteReservation {
            route_id: "rte_01ARZ3NDEKTSV4RRFFQ69G5FAV".into(),
            local_route_key: local_route_key.clone(),
            reservation_id: Uuid::new_v4(),
            generation: 9,
            fencing_token: "opaque-cloud-fence".into(),
            lease_expires_at: Utc::now() + chrono::Duration::minutes(1),
        };
        let registration = coordinator
            .register_authoritative(
                "hosted",
                "native-a",
                "job_01ARZ3NDEKTSV4RRFFQ69G5FAV",
                &cloud,
                Utc::now(),
            )
            .unwrap();
        assert!(registration.newly_registered);
        let duplicate = coordinator
            .register_authoritative(
                "hosted",
                "native-a",
                "job_01ARZ3NDEKTSV4RRFFQ69G5FAV",
                &cloud,
                Utc::now(),
            )
            .unwrap();
        assert!(!duplicate.newly_registered);
        assert_eq!(duplicate.reservation, registration.reservation);
        let reserved = coordinator
            .reserve(
                "hosted",
                "native-a",
                "job_01ARZ3NDEKTSV4RRFFQ69G5FAV",
                Utc::now().timestamp_millis(),
            )
            .unwrap();
        assert_eq!(reserved.reservation_id, cloud.reservation_id);
        assert_eq!(reserved.generation, 9);
        coordinator
            .finish(
                "hosted",
                "job_01ARZ3NDEKTSV4RRFFQ69G5FAV",
                &reserved,
                NativeHandoffOutcome::Accepted,
                Some("native-9".into()),
                Utc::now(),
            )
            .unwrap();
        let evidence = coordinator.handoffs_for_connector("hosted", 0);
        assert_eq!(evidence[0].reservation_id, cloud.reservation_id);
        assert_eq!(evidence[0].fencing_generation, 9);
        assert_eq!(evidence[0].fencing_token, "opaque-cloud-fence");
        assert_eq!(
            evidence[0].route_id.as_deref(),
            Some(cloud.route_id.as_str())
        );
        assert_eq!(evidence[0].local_route_key, local_route_key);
        let reservation_debug = format!("{reserved:?}");
        let coordinator_debug = format!("{coordinator:?}");
        assert!(!reservation_debug.contains("opaque-cloud-fence"));
        assert!(!coordinator_debug.contains("opaque-cloud-fence"));
        assert!(reservation_debug.contains("[REDACTED]"));
    }

    #[test]
    fn cloud_generation_is_per_job_and_stale_same_job_attempts_are_rejected() {
        let root = TempDir::new().unwrap();
        let mut coordinator = RouteCoordinator::open(root.path()).unwrap();
        coordinator
            .reconcile(&[printer("native-a", Vec::new())], 1, Utc::now())
            .unwrap();
        let local_route_key = coordinator.route_id("native-a");
        for job in ["job_a", "job_b"] {
            let cloud = piqae_protocol::agent::CloudRouteReservation {
                route_id: "rte_01ARZ3NDEKTSV4RRFFQ69G5FAV".into(),
                local_route_key: local_route_key.clone(),
                reservation_id: Uuid::new_v4(),
                generation: 1,
                fencing_token: format!("fence-{job}"),
                lease_expires_at: Utc::now() + chrono::Duration::minutes(1),
            };
            coordinator
                .register_authoritative("hosted", "native-a", job, &cloud, Utc::now())
                .unwrap();
            let reserved = coordinator
                .reserve("hosted", "native-a", job, Utc::now().timestamp_millis())
                .unwrap();
            coordinator
                .finish(
                    "hosted",
                    job,
                    &reserved,
                    NativeHandoffOutcome::RejectedBeforeHandoff,
                    None,
                    Utc::now(),
                )
                .unwrap();
        }
        coordinator.acknowledge_handoffs("hosted", 2).unwrap();
        assert!(coordinator.handoffs_for_connector("hosted", 0).is_empty());
        drop(coordinator);
        let mut coordinator = RouteCoordinator::open(root.path()).unwrap();
        let stale = piqae_protocol::agent::CloudRouteReservation {
            route_id: "rte_01ARZ3NDEKTSV4RRFFQ69G5FAV".into(),
            local_route_key,
            reservation_id: Uuid::new_v4(),
            generation: 1,
            fencing_token: "stale-fence".into(),
            lease_expires_at: Utc::now() + chrono::Duration::minutes(1),
        };
        assert!(
            coordinator
                .register_authoritative("hosted", "native-a", "job_a", &stale, Utc::now())
                .is_err()
        );
        let fresh = piqae_protocol::agent::CloudRouteReservation {
            generation: 2,
            reservation_id: Uuid::new_v4(),
            fencing_token: "fresh-fence".into(),
            ..stale
        };
        assert!(
            coordinator
                .register_authoritative("hosted", "native-a", "job_a", &fresh, Utc::now())
                .is_ok(),
            "only a strictly newer authoritative generation can reopen this job route"
        );
    }

    #[test]
    fn authoritative_replay_barrier_capacity_fails_closed_without_eviction() {
        let root = TempDir::new().unwrap();
        let mut coordinator = RouteCoordinator::open(root.path()).unwrap();
        coordinator
            .reconcile(&[printer("native-a", Vec::new())], 1, Utc::now())
            .unwrap();
        let local_route_key = coordinator.route_id("native-a");
        let server_route_id = "rte_01ARZ3NDEKTSV4RRFFQ69G5FAV";
        for index in 0..MAX_CONSUMED_AUTHORITATIVE_ROUTES {
            let job_id = if index == 0 {
                "job-retained".to_owned()
            } else {
                format!("bounded-job-{index}")
            };
            let route_id = if index == 0 {
                server_route_id.to_owned()
            } else {
                format!("bounded-route-{index}")
            };
            let route_key = if index == 0 {
                local_route_key.clone()
            } else {
                format!("bounded-local-{index}")
            };
            let consumed = ConsumedAuthoritativeRoute {
                connector_id: "hosted".into(),
                server_route_id: route_id.clone(),
                local_route_key: route_key.clone(),
                job_id: job_id.clone(),
                generation: 1,
                reservation_id: Uuid::new_v4(),
                fencing_token: format!("bounded-fence-{index}"),
                observed_at: Utc::now(),
                outcome: NativeHandoffOutcome::RejectedBeforeHandoff,
                native_job_id: None,
            };
            coordinator.document.consumed_authoritative_routes.insert(
                authoritative_route_key("hosted", &route_id, &route_key, &job_id),
                consumed,
            );
        }
        coordinator.persist().unwrap();

        let unknown = piqae_protocol::agent::CloudRouteReservation {
            route_id: "rte_unknown".into(),
            local_route_key: local_route_key.clone(),
            reservation_id: Uuid::new_v4(),
            generation: 1,
            fencing_token: "unknown-fence".into(),
            lease_expires_at: Utc::now() + chrono::Duration::minutes(1),
        };
        assert!(
            coordinator
                .register_authoritative("hosted", "native-a", "job-new", &unknown, Utc::now())
                .is_err(),
            "capacity pressure must not evict any replay barrier"
        );
        assert_eq!(
            coordinator.document.consumed_authoritative_routes.len(),
            MAX_CONSUMED_AUTHORITATIVE_ROUTES
        );
        let fresh = piqae_protocol::agent::CloudRouteReservation {
            route_id: server_route_id.into(),
            local_route_key,
            reservation_id: Uuid::new_v4(),
            generation: 2,
            fencing_token: "retained-fresh-fence".into(),
            lease_expires_at: Utc::now() + chrono::Duration::minutes(1),
        };
        assert!(
            coordinator
                .register_authoritative("hosted", "native-a", "job-retained", &fresh, Utc::now())
                .is_ok(),
            "a retained job can advance only with a newer authority generation"
        );
    }

    #[test]
    fn raw_identity_and_external_job_metadata_are_not_persisted() {
        let root = TempDir::new().unwrap();
        let mut coordinator = RouteCoordinator::open(root.path()).unwrap();
        coordinator
            .reconcile(
                &[printer(
                    "secret-native-queue",
                    vec![PhysicalIdentityEvidence {
                        kind: PhysicalIdentityEvidenceKind::DeviceSerial,
                        value_sha256: "b".repeat(64),
                        strength: IdentityEvidenceStrength::Strong,
                    }],
                )],
                1,
                Utc::now(),
            )
            .unwrap();
        let stored = std::fs::read_to_string(root.path().join("route-coordinator.json")).unwrap();
        assert!(!stored.contains("external title"));
        assert!(!stored.contains("external user"));
        assert!(!stored.contains("document.pdf"));
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "pre/post replacement failures exercise each cross-file mutation boundary"
    )]
    fn coordinator_persistence_faults_rollback_or_fail_stop_until_reopen() {
        let before = TempDir::new().unwrap();
        let mut coordinator = RouteCoordinator::open(before.path()).unwrap();
        coordinator
            .persist_fault
            .set(Some(PersistFault::BeforeReplace));
        assert!(
            coordinator
                .reserve("local", "native-a", "job-before", 1)
                .is_err()
        );
        assert!(coordinator.document.reservations.is_empty());
        let durable = coordinator
            .reserve("local", "native-a", "job-before", 2)
            .unwrap();
        coordinator.validate(&durable).unwrap();
        drop(coordinator);
        RouteCoordinator::open(before.path())
            .unwrap()
            .validate(&durable)
            .unwrap();

        let reserve_after = TempDir::new().unwrap();
        let mut coordinator = RouteCoordinator::open(reserve_after.path()).unwrap();
        coordinator
            .persist_fault
            .set(Some(PersistFault::AfterReplace));
        assert!(
            coordinator
                .reserve("local", "native-a", "job-reserve-after", 1)
                .is_err()
        );
        assert!(coordinator.poisoned);
        assert!(
            coordinator
                .reserve("local", "native-b", "blocked-while-poisoned", 2)
                .is_err()
        );
        drop(coordinator);
        let mut reopened = RouteCoordinator::open(reserve_after.path()).unwrap();
        assert!(
            reopened
                .reserve("local", "native-a", "job-reserve-after", 3)
                .is_ok(),
            "reopen must adopt the attempted state written before parent fsync failed"
        );

        let register_after = TempDir::new().unwrap();
        let mut coordinator = RouteCoordinator::open(register_after.path()).unwrap();
        coordinator
            .reconcile(&[printer("native-a", Vec::new())], 1, Utc::now())
            .unwrap();
        let cloud = cloud_reservation(&coordinator, "native-a", "register-route", 1);
        coordinator
            .persist_fault
            .set(Some(PersistFault::AfterReplace));
        assert!(
            coordinator
                .register_authoritative(
                    "legacy",
                    "native-a",
                    "job-register-after",
                    &cloud,
                    Utc::now(),
                )
                .is_err()
        );
        assert!(
            coordinator
                .validate(&RouteReservation {
                    local_route_key: cloud.local_route_key.clone(),
                    server_route_id: Some(cloud.route_id.clone()),
                    coordination_key: cloud.local_route_key.clone(),
                    reservation_id: cloud.reservation_id,
                    generation: cloud.generation,
                    fencing_token: cloud.fencing_token.clone(),
                })
                .is_err()
        );
        drop(coordinator);
        let mut reopened = RouteCoordinator::open(register_after.path()).unwrap();
        assert!(
            !reopened
                .register_authoritative(
                    "legacy",
                    "native-a",
                    "job-register-after",
                    &cloud,
                    Utc::now(),
                )
                .unwrap()
                .newly_registered,
            "reopen adopts the exact durable registration without duplicating it"
        );

        let finish_after = TempDir::new().unwrap();
        let mut coordinator = RouteCoordinator::open(finish_after.path()).unwrap();
        let reservation = coordinator
            .reserve("local", "native-a", "job-finish-after", 1)
            .unwrap();
        coordinator
            .persist_fault
            .set(Some(PersistFault::AfterReplace));
        assert!(
            coordinator
                .finish(
                    "local",
                    "job-finish-after",
                    &reservation,
                    NativeHandoffOutcome::Accepted,
                    Some("native-job".into()),
                    Utc::now(),
                )
                .is_err()
        );
        assert!(coordinator.validate(&reservation).is_err());
        assert!(coordinator.acknowledge_handoffs("local", u64::MAX).is_err());
        drop(coordinator);
        let mut reopened = RouteCoordinator::open(finish_after.path()).unwrap();
        assert_eq!(reopened.document.handoffs.len(), 1);
        assert!(
            reopened
                .reserve("local", "native-a", "job-finish-after", 2)
                .is_err()
        );

        let finish_before = TempDir::new().unwrap();
        let mut coordinator = RouteCoordinator::open(finish_before.path()).unwrap();
        let reservation = coordinator
            .reserve("local", "native-a", "job-finish-before", 1)
            .unwrap();
        coordinator
            .persist_fault
            .set(Some(PersistFault::BeforeReplace));
        coordinator
            .finish_runtime(
                "local",
                "job-finish-before",
                &reservation,
                NativeHandoffOutcome::Accepted,
                Some("native-before-retry".into()),
                Utc::now(),
            )
            .unwrap();
        assert!(!coordinator.is_poisoned());
        assert_eq!(coordinator.document.handoffs.len(), 1);
        drop(coordinator);
        assert_eq!(
            RouteCoordinator::open(finish_before.path())
                .unwrap()
                .document
                .handoffs
                .len(),
            1,
            "a proven pre-replace failure retries the exact terminal outcome durably"
        );

        let finish_repeated = TempDir::new().unwrap();
        let mut coordinator = RouteCoordinator::open(finish_repeated.path()).unwrap();
        let reservation = coordinator
            .reserve("local", "native-a", "job-finish-repeated", 1)
            .unwrap();
        coordinator
            .persist_fault
            .set(Some(PersistFault::BeforeReplaceTwice));
        assert!(
            coordinator
                .finish_runtime(
                    "local",
                    "job-finish-repeated",
                    &reservation,
                    NativeHandoffOutcome::Accepted,
                    Some("native-unknown-after-repeat".into()),
                    Utc::now(),
                )
                .is_err()
        );
        assert!(coordinator.is_poisoned());
        drop(coordinator);
        let mut reopened = RouteCoordinator::open(finish_repeated.path()).unwrap();
        let active = reopened.local_reservations_for_connector("local");
        assert_eq!(active.len(), 1);
        assert!(reopened.document.handoffs.is_empty());
        reopened
            .finish(
                "local",
                "job-finish-repeated",
                &active[0].1,
                NativeHandoffOutcome::Ambiguous,
                None,
                Utc::now(),
            )
            .unwrap();
        reopened
            .resolve_ambiguous_handoff(
                "local",
                "job-finish-repeated",
                &reservation.local_route_key,
                reservation.reservation_id,
                reservation.generation,
                piqae_protocol::agent::AmbiguousHandoffResolution::ReleaseForRetry,
            )
            .unwrap();
        assert!(
            reopened
                .reserve("local", "native-a", "job-after-recovery", 2)
                .is_ok()
        );

        let resolve_after = TempDir::new().unwrap();
        let mut coordinator = RouteCoordinator::open(resolve_after.path()).unwrap();
        let reservation = coordinator
            .reserve("local", "native-a", "job-resolve-after", 1)
            .unwrap();
        coordinator
            .finish(
                "local",
                "job-resolve-after",
                &reservation,
                NativeHandoffOutcome::Ambiguous,
                None,
                Utc::now(),
            )
            .unwrap();
        coordinator
            .persist_fault
            .set(Some(PersistFault::AfterReplace));
        let resolution = piqae_protocol::agent::AmbiguousHandoffResolution::ReleaseForRetry;
        assert!(
            coordinator
                .resolve_ambiguous_handoff(
                    "local",
                    "job-resolve-after",
                    &reservation.local_route_key,
                    reservation.reservation_id,
                    reservation.generation,
                    resolution,
                )
                .is_err()
        );
        assert!(
            coordinator
                .resolve_ambiguous_handoff(
                    "local",
                    "job-resolve-after",
                    &reservation.local_route_key,
                    reservation.reservation_id,
                    reservation.generation,
                    resolution,
                )
                .is_err()
        );
        drop(coordinator);
        let mut reopened = RouteCoordinator::open(resolve_after.path()).unwrap();
        reopened
            .resolve_ambiguous_handoff(
                "local",
                "job-resolve-after",
                &reservation.local_route_key,
                reservation.reservation_id,
                reservation.generation,
                resolution,
            )
            .unwrap();
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "accepted, rejected, and ambiguous connector isolation share one restart proof"
    )]
    fn authoritative_replay_state_is_connector_scoped_but_physical_fence_is_shared() {
        let root = TempDir::new().unwrap();
        let mut coordinator = RouteCoordinator::open(root.path()).unwrap();
        coordinator
            .reconcile(&[printer("native-a", Vec::new())], 1, Utc::now())
            .unwrap();
        let job_id = "job_01ARZ3NDEKTSV4RRFFQ69G5FAV";
        let a = cloud_reservation(&coordinator, "native-a", "route-a", 5);
        let a_local = coordinator
            .register_authoritative("connector-a", "native-a", job_id, &a, Utc::now())
            .unwrap()
            .reservation;
        coordinator
            .finish(
                "connector-a",
                job_id,
                &a_local,
                NativeHandoffOutcome::Accepted,
                Some("native-a-job".into()),
                Utc::now(),
            )
            .unwrap();
        let through = coordinator.handoffs_for_connector("connector-a", 0)[0].sequence;
        coordinator
            .acknowledge_handoffs("connector-a", through)
            .unwrap();
        drop(coordinator);

        let mut coordinator = RouteCoordinator::open(root.path()).unwrap();
        coordinator
            .reconcile(
                &[
                    printer("native-a", Vec::new()),
                    printer("native-b", Vec::new()),
                ],
                2,
                Utc::now(),
            )
            .unwrap();
        assert!(
            coordinator
                .register_authoritative(
                    "connector-a",
                    "native-b",
                    job_id,
                    &cloud_reservation(&coordinator, "native-b", "route-a-moved", 99),
                    Utc::now(),
                )
                .is_err(),
            "accepted connector/job truth remains terminal across topology and route changes"
        );
        let b = cloud_reservation(&coordinator, "native-a", "route-b", 1);
        let b_local = coordinator
            .register_authoritative("connector-b", "native-a", job_id, &b, Utc::now())
            .unwrap()
            .reservation;
        coordinator
            .finish(
                "connector-b",
                job_id,
                &b_local,
                NativeHandoffOutcome::RejectedBeforeHandoff,
                None,
                Utc::now(),
            )
            .unwrap();
        assert!(
            coordinator
                .register_authoritative(
                    "connector-a",
                    "native-a",
                    job_id,
                    &cloud_reservation(&coordinator, "native-a", "route-a", 6),
                    Utc::now(),
                )
                .is_err(),
            "accepted authority is terminal within connector A"
        );
        assert!(
            coordinator
                .register_authoritative(
                    "connector-b",
                    "native-a",
                    job_id,
                    &cloud_reservation(&coordinator, "native-a", "route-b", 2),
                    Utc::now(),
                )
                .is_ok(),
            "connector B can advance its independently rejected generation"
        );

        let rejected_root = TempDir::new().unwrap();
        let mut coordinator = RouteCoordinator::open(rejected_root.path()).unwrap();
        coordinator
            .reconcile(&[printer("native-a", Vec::new())], 1, Utc::now())
            .unwrap();
        let a = cloud_reservation(&coordinator, "native-a", "rejected-a", 1);
        let a_local = coordinator
            .register_authoritative("connector-a", "native-a", job_id, &a, Utc::now())
            .unwrap()
            .reservation;
        coordinator
            .finish(
                "connector-a",
                job_id,
                &a_local,
                NativeHandoffOutcome::RejectedBeforeHandoff,
                None,
                Utc::now(),
            )
            .unwrap();
        let through = coordinator.handoffs_for_connector("connector-a", 0)[0].sequence;
        coordinator
            .acknowledge_handoffs("connector-a", through)
            .unwrap();
        drop(coordinator);
        let mut coordinator = RouteCoordinator::open(rejected_root.path()).unwrap();
        let b = cloud_reservation(&coordinator, "native-a", "rejected-b", 1);
        assert!(
            coordinator
                .register_authoritative("connector-b", "native-a", job_id, &b, Utc::now())
                .is_ok(),
            "A's acknowledged rejected generation cannot poison B after restart"
        );

        let ambiguous_root = TempDir::new().unwrap();
        let mut coordinator = RouteCoordinator::open(ambiguous_root.path()).unwrap();
        coordinator
            .reconcile(&[printer("native-a", Vec::new())], 1, Utc::now())
            .unwrap();
        let a = cloud_reservation(&coordinator, "native-a", "route-a", 1);
        let a_local = coordinator
            .register_authoritative("connector-a", "native-a", job_id, &a, Utc::now())
            .unwrap()
            .reservation;
        coordinator
            .finish(
                "connector-a",
                job_id,
                &a_local,
                NativeHandoffOutcome::Ambiguous,
                None,
                Utc::now(),
            )
            .unwrap();
        let through = coordinator.handoffs_for_connector("connector-a", 0)[0].sequence;
        coordinator
            .acknowledge_handoffs("connector-a", through)
            .unwrap();
        drop(coordinator);
        let mut coordinator = RouteCoordinator::open(ambiguous_root.path()).unwrap();
        let b = cloud_reservation(&coordinator, "native-a", "route-b", 1);
        assert!(
            coordinator
                .register_authoritative("connector-b", "native-a", job_id, &b, Utc::now())
                .is_err(),
            "connector isolation never weakens physical handoff serialization"
        );
        coordinator
            .resolve_ambiguous_handoff(
                "connector-a",
                job_id,
                &a.local_route_key,
                a.reservation_id,
                a.generation,
                piqae_protocol::agent::AmbiguousHandoffResolution::ReleaseForRetry,
            )
            .unwrap();
        assert!(
            coordinator
                .register_authoritative("connector-b", "native-a", job_id, &b, Utc::now())
                .is_ok()
        );
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "the retained cloud record and 600 local restart attempts form one capacity proof"
    )]
    fn committed_local_handoffs_compact_beyond_capacity_without_touching_cloud() {
        let root = TempDir::new().unwrap();
        let mut coordinator = RouteCoordinator::open(root.path()).unwrap();
        coordinator
            .reconcile(&[printer("cloud-native", Vec::new())], 1, Utc::now())
            .unwrap();
        let cloud = cloud_reservation(&coordinator, "cloud-native", "cloud-route", 1);
        let cloud_local = coordinator
            .register_authoritative(
                "local",
                "cloud-native",
                "job_01ARZ3NDEKTSV4RRFFQ69G5FAV",
                &cloud,
                Utc::now(),
            )
            .unwrap()
            .reservation;
        coordinator
            .finish(
                "local",
                "job_01ARZ3NDEKTSV4RRFFQ69G5FAV",
                &cloud_local,
                NativeHandoffOutcome::RejectedBeforeHandoff,
                None,
                Utc::now(),
            )
            .unwrap();

        let retry_job = "job_01ARZ3NDEKTSV4RRFFQ69G5FAA";
        let first = coordinator
            .reserve("local", "retry-native", retry_job, 1)
            .unwrap();
        reject_before_handoff(&mut coordinator, "local", retry_job, &first);
        let second = coordinator
            .reserve("local", "retry-native", retry_job, 2)
            .unwrap();
        coordinator
            .finish(
                "local",
                retry_job,
                &second,
                NativeHandoffOutcome::Accepted,
                Some("retry-native-job".into()),
                Utc::now(),
            )
            .unwrap();
        assert_eq!(
            coordinator
                .compact_local_terminal_handoffs(
                    "local",
                    &[LocalHandoffCommit {
                        job_id: retry_job.into(),
                        state: "accepted_by_spooler".into(),
                        native_job_id: Some("retry-native-job".into()),
                        ambiguity_confirmed: false,
                    }],
                )
                .unwrap(),
            2,
            "a later durable acceptance compacts its earlier rejected attempt too"
        );

        for index in 0..600_u64 {
            let job_id = piqae_domain::JobId::new().to_string();
            let native_job_id = format!("native-job-{index}");
            let observed_unix_ms = i64::try_from(index).unwrap();
            let reservation = coordinator
                .reserve(
                    "local",
                    &format!("native-{index}"),
                    &job_id,
                    observed_unix_ms,
                )
                .unwrap();
            coordinator
                .finish(
                    "local",
                    &job_id,
                    &reservation,
                    NativeHandoffOutcome::Accepted,
                    Some(native_job_id.clone()),
                    Utc::now(),
                )
                .unwrap();
            assert_eq!(
                coordinator
                    .compact_local_terminal_handoffs(
                        "local",
                        &[LocalHandoffCommit {
                            job_id,
                            state: "accepted_by_spooler".into(),
                            native_job_id: Some(native_job_id),
                            ambiguity_confirmed: false,
                        }],
                    )
                    .unwrap(),
                1
            );
            if index % 100 == 99 {
                drop(coordinator);
                coordinator = RouteCoordinator::open(root.path()).unwrap();
            }
        }
        let retained = coordinator.handoffs_for_connector("local", 0);
        assert_eq!(retained.len(), 1);
        assert!(retained[0].route_id.is_some());
    }
}
