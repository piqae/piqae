//! Installation-wide printer topology and native handoff fencing.
//!
//! Connector runtimes deliberately keep credentials, queues, cursors and
//! documents isolated. This coordinator owns only shared physical facts and
//! the final OS-route reservation boundary. Its durable handoff journal closes
//! the crash window where a connector could otherwise replay a job after the
//! operating system had already accepted it.

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
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::OpenOptions,
    io::Write as _,
    path::{Path, PathBuf},
};
use uuid::Uuid;

const DOCUMENT_VERSION: u16 = 1;
const MAX_ROUTES: usize = 512;
const MAX_HANDOFFS: usize = 512;
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

#[derive(Clone, Eq, PartialEq)]
pub struct RouteReservation {
    pub local_route_key: String,
    pub server_route_id: Option<String>,
    coordination_key: String,
    pub reservation_id: Uuid,
    pub generation: u64,
    fencing_token: String,
}

pub struct RouteCoordinator {
    root: PathBuf,
    document: CoordinatorDocument,
    next_observation_sequence: u64,
    reserved_observation_sequence: u64,
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
        {
            bail!("route coordinator state exceeds supported bounds");
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
        let route_id = self.route_id(native_id);
        let route = self.document.routes.get(native_id);
        let coordination_key =
            route.map_or_else(|| route_id.clone(), |route| route.coordination_key.clone());
        if self.document.handoffs.iter().rev().any(|handoff| {
            handoff.job_id == job_id
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
        self.persist()?;
        Ok(result)
    }

    pub fn register_authoritative(
        &mut self,
        connector_id: &str,
        native_id: &str,
        job_id: &str,
        reservation: &piqae_protocol::agent::CloudRouteReservation,
        now: DateTime<Utc>,
    ) -> Result<()> {
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
        if self.document.handoffs.iter().any(|handoff| {
            handoff.job_id == job_id
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
            .filter(|handoff| handoff.job_id == job_id)
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
                return Ok(());
            }
            bail!("printer route already has an unresolved reservation");
        }
        if route.coordination_conflict {
            bail!("printer route has conflicting unresolved physical reservations");
        }
        self.document.reservations.insert(
            coordination_key,
            DurableReservation {
                server_route_id: Some(reservation.route_id.clone()),
                local_route_key: expected_route,
                reservation_id: reservation.reservation_id,
                connector_id: connector_id.to_owned(),
                job_id: job_id.to_owned(),
                generation: reservation.generation,
                fencing_token: reservation.fencing_token.clone(),
                expires_unix_ms: reservation.lease_expires_at.timestamp_millis(),
            },
        );
        self.persist()
    }

    pub fn validate(&self, reservation: &RouteReservation) -> Result<()> {
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
        self.validate(reservation)?;
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
            native_job_id,
        });
        // An ambiguous timeout/crash is not a release signal: the native call
        // may still have crossed the spooler boundary. Keep the route fenced
        // until reconciliation or explicit operator resolution instead of
        // allowing a different connector to create a duplicate.
        if outcome != NativeHandoffOutcome::Ambiguous {
            self.document
                .reservations
                .remove(&reservation.coordination_key);
        }
        trim_front(&mut self.document.handoffs, MAX_HANDOFFS);
        self.persist()
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
        let resolved_outcome = match resolution {
            piqae_protocol::agent::AmbiguousHandoffResolution::ReleaseForRetry => {
                NativeHandoffOutcome::RejectedBeforeHandoff
            }
            piqae_protocol::agent::AmbiguousHandoffResolution::ConfirmAccepted => {
                NativeHandoffOutcome::Accepted
            }
        };
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
        self.document.reservations.remove(&coordination_key);
        self.persist()
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

    fn persist(&self) -> Result<()> {
        std::fs::create_dir_all(&self.root)?;
        let path = self.root.join("route-coordinator.json");
        let staged = self.root.join("route-coordinator.json.replacing");
        let _ = std::fs::remove_file(&staged);
        let bytes = serde_json::to_vec_pretty(&self.document)?;
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options.open(&staged)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        replace_file_atomically(&staged, &path)?;
        #[cfg(unix)]
        std::fs::File::open(&self.root)
            .and_then(|directory| directory.sync_all())
            .with_context(|| format!("sync {}", self.root.display()))?;
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_file_atomically(staged: &Path, destination: &Path) -> Result<()> {
    std::fs::rename(staged, destination)
        .with_context(|| format!("replace {}", destination.display()))
}

#[cfg(windows)]
#[allow(
    unsafe_code,
    reason = "isolated, documented MoveFileExW call required for atomic Windows journal replacement"
)]
fn replace_file_atomically(staged: &Path, destination: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt as _;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let staged = staged
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination_wide = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // SAFETY: both paths are live NUL-terminated UTF-16 buffers. Replace and
    // write-through preserve an existing journal without a delete gap.
    if unsafe {
        MoveFileExW(
            staged.as_ptr(),
            destination_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("replace {}", destination.display()));
    }
    Ok(())
}

fn trim_front<T>(values: &mut Vec<T>, limit: usize) {
    if values.len() > limit {
        values.drain(..values.len() - limit);
    }
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
        coordinator
            .register_authoritative(
                "hosted",
                "native-a",
                "job_01ARZ3NDEKTSV4RRFFQ69G5FAV",
                &cloud,
                Utc::now(),
            )
            .unwrap();
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
}
