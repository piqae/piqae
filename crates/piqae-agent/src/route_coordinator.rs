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
const RESERVATION_LIFETIME_MS: i64 = 2 * 60 * 1_000;

#[derive(Clone, Debug, Deserialize, Serialize)]
struct CoordinatorDocument {
    version: u16,
    installation_namespace: Uuid,
    topology_revision: u64,
    handoff_sequence: u64,
    routes: BTreeMap<String, DurableRoute>,
    reservations: BTreeMap<String, DurableReservation>,
    handoffs: Vec<DurableHandoff>,
    topology_changes: Vec<RouteTopologyChange>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct DurableRoute {
    route_id: String,
    coordination_key: String,
    native_fingerprint: String,
    generation: u64,
    present: bool,
    identity_evidence: Vec<PhysicalIdentityEvidence>,
    identity_confidence: IdentityConfidence,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
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

#[derive(Clone, Debug, Deserialize, Serialize)]
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RouteReservation {
    pub local_route_key: String,
    pub server_route_id: Option<String>,
    coordination_key: String,
    pub reservation_id: Uuid,
    pub generation: u64,
    fencing_token: String,
}

#[derive(Debug)]
pub struct RouteCoordinator {
    root: PathBuf,
    document: CoordinatorDocument,
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
        if document.routes.len() > MAX_ROUTES
            || document.handoffs.len() > MAX_HANDOFFS
            || document.topology_changes.len() > MAX_TOPOLOGY_CHANGES
        {
            bail!("route coordinator state exceeds supported bounds");
        }
        let coordinator = Self { root, document };
        coordinator.persist()?;
        Ok(coordinator)
    }

    pub fn reconcile(
        &mut self,
        printers: &[DiscoveredPrinter],
        inventory_revision: u64,
        observed_at: DateTime<Utc>,
    ) -> Result<BTreeMap<String, PrinterRouteSnapshot>> {
        let mut present = BTreeSet::new();
        let mut snapshots = BTreeMap::new();
        for printer in printers.iter().take(MAX_ROUTES) {
            present.insert(printer.native_id.clone());
            let route_id = self.route_id(&printer.native_id);
            let evidence = canonical_evidence(printer);
            let confidence = identity_confidence(&evidence);
            let fingerprint = topology_fingerprint(&evidence);
            let coordination_key = physical_coordination_key(&route_id, &evidence);
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
            let generation = self
                .document
                .routes
                .get(&printer.native_id)
                .map_or(0, |route| route.generation);
            self.document.routes.insert(
                printer.native_id.clone(),
                DurableRoute {
                    route_id: route_id.clone(),
                    coordination_key,
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

    pub fn topology_changes(&self) -> Vec<RouteTopologyChange> {
        self.document.topology_changes.clone()
    }

    pub fn reserve(
        &mut self,
        connector_id: &str,
        native_id: &str,
        job_id: &str,
        now_unix_ms: i64,
    ) -> Result<RouteReservation> {
        let route_id = self.route_id(native_id);
        let coordination_key = self
            .document
            .routes
            .get(native_id)
            .map_or_else(|| route_id.clone(), |route| route.coordination_key.clone());
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
            local_route_key: route_id.clone(),
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
        let coordination_key = self
            .document
            .routes
            .values()
            .find(|route| route.route_id == expected_route)
            .map(|route| route.coordination_key.clone())
            .context("cloud reservation references an unknown local route")?;
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
        let route = self
            .document
            .routes
            .values_mut()
            .find(|route| route.route_id == expected_route)
            .context("cloud reservation references an unknown local route")?;
        if reservation.generation <= route.generation {
            bail!("cloud route reservation generation is stale");
        }
        route.generation = reservation.generation;
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
        std::fs::rename(&staged, &path)?;
        Ok(())
    }
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
    let strong = evidence
        .iter()
        .filter(|item| item.strength == IdentityEvidenceStrength::Strong)
        .count();
    let supporting = evidence
        .iter()
        .filter(|item| item.strength == IdentityEvidenceStrength::Medium)
        .count();
    if strong > 0 || supporting >= 2 {
        IdentityConfidence::High
    } else if supporting == 1 {
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
        format!("pgrp_{}", &hex::encode(digest.finalize())[..32])
    } else {
        local_route_key.to_owned()
    }
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
