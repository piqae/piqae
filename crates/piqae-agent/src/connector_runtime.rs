//! Durable multi-connector runtime configuration and fair scheduling.
//!
//! A connector is an independent cloud security principal. Its signing key,
//! `SQLite` database, content directory, sync cursor, leases, and event outbox
//! must never be shared with another connector. The physical installation may
//! share printer discovery and the native executor, but those resources are
//! reached only after the connector-specific store has admitted a job.

use anyhow::{Context, Result, bail};
use piqae_protocol::agent::PrinterGrant;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, VecDeque},
    fs::OpenOptions,
    io::Write as _,
    path::{Path, PathBuf},
};
use url::Url;

const REGISTRY_VERSION: u16 = 1;
const MAX_CONNECTORS: usize = 128;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConnectorRecord {
    pub connector_id: String,
    pub agent_id: String,
    pub control_plane_url: Url,
    /// Relative to the installation data directory. Never accept an absolute
    /// or parent-traversing path from a downloaded enrolment response.
    pub device_key_file: PathBuf,
    pub enabled: bool,
    /// Durable authorization policy. Older registry documents safely decode as
    /// selected-printer grants rather than widening access.
    #[serde(default)]
    pub printer_grant: PrinterGrant,
    /// Explicit local printer grants. Empty is valid only for all-printer access.
    #[serde(default)]
    pub allowed_printer_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ConnectorRegistryDocument {
    version: u16,
    connectors: Vec<ConnectorRecord>,
}

#[derive(Debug, Clone)]
#[allow(dead_code, reason = "consumed by the staged connector supervisor")]
pub struct ConnectorRuntimePaths {
    pub database: PathBuf,
    pub content: PathBuf,
    pub device_key: PathBuf,
}

#[derive(Debug)]
pub struct ConnectorRegistry {
    root: PathBuf,
    records: BTreeMap<String, ConnectorRecord>,
}

impl ConnectorRegistry {
    pub(crate) fn load(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        let path = root.join("connectors.json");
        let document = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice::<ConnectorRegistryDocument>(&bytes)
                .with_context(|| format!("parse {}", path.display()))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                ConnectorRegistryDocument {
                    version: REGISTRY_VERSION,
                    connectors: Vec::new(),
                }
            }
            Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
        };
        if document.version != REGISTRY_VERSION {
            bail!(
                "unsupported connector registry version {}",
                document.version
            );
        }
        if document.connectors.len() > MAX_CONNECTORS {
            bail!("connector registry exceeds the {MAX_CONNECTORS} connector limit");
        }
        let mut records = BTreeMap::new();
        for mut record in document.connectors {
            validate_record(&record)?;
            record.allowed_printer_ids.sort();
            if records
                .insert(record.connector_id.clone(), record)
                .is_some()
            {
                bail!("connector registry contains a duplicate connector id");
            }
        }
        Ok(Self { root, records })
    }

    pub(crate) fn enabled(&self) -> impl Iterator<Item = &ConnectorRecord> {
        self.records.values().filter(|record| record.enabled)
    }

    #[allow(dead_code, reason = "consumed by the staged connector supervisor")]
    pub(crate) fn paths(&self, connector_id: &str) -> Result<ConnectorRuntimePaths> {
        let record = self
            .records
            .get(connector_id)
            .context("connector was not found")?;
        let connector_root = self.root.join("connectors").join(&record.connector_id);
        Ok(ConnectorRuntimePaths {
            database: connector_root.join("agent.sqlite3"),
            content: connector_root.join("content"),
            device_key: self.root.join(&record.device_key_file),
        })
    }

    /// Adds a connector without replacing an existing security principal.
    #[allow(
        dead_code,
        reason = "called by the native consent IPC in the next integration slice"
    )]
    pub(crate) fn add(&mut self, mut record: ConnectorRecord) -> Result<()> {
        validate_record(&record)?;
        record.allowed_printer_ids.sort();
        if self.records.len() >= MAX_CONNECTORS {
            bail!("connector limit reached");
        }
        if self.records.contains_key(&record.connector_id) {
            bail!("connector already exists");
        }
        self.records.insert(record.connector_id.clone(), record);
        self.persist()
    }

    /// Revocation is fail-closed and durable before the caller stops its task.
    #[allow(
        dead_code,
        reason = "called by the native consent IPC in the next integration slice"
    )]
    pub(crate) fn revoke(&mut self, connector_id: &str) -> Result<bool> {
        let Some(record) = self.records.get_mut(connector_id) else {
            return Ok(false);
        };
        if !record.enabled {
            return Ok(false);
        }
        record.enabled = false;
        self.persist()?;
        Ok(true)
    }

    /// Replaces the explicit printer grant returned by a replay-safe
    /// enrolment before the running worker is asked to reload it.
    pub(crate) fn update_allowed_printers(
        &mut self,
        connector_id: &str,
        printer_grant: PrinterGrant,
        mut allowed_printer_ids: Vec<String>,
    ) -> Result<bool> {
        allowed_printer_ids.sort();
        let Some(record) = self.records.get_mut(connector_id) else {
            return Ok(false);
        };
        let mut candidate = record.clone();
        candidate.printer_grant = printer_grant;
        candidate.allowed_printer_ids = allowed_printer_ids;
        validate_record(&candidate)?;
        record.printer_grant = candidate.printer_grant;
        record.allowed_printer_ids = candidate.allowed_printer_ids;
        self.persist()?;
        Ok(true)
    }

    fn persist(&self) -> Result<()> {
        std::fs::create_dir_all(&self.root)?;
        let path = self.root.join("connectors.json");
        let staged = self.root.join("connectors.json.replacing");
        let _ = std::fs::remove_file(&staged);
        let document = ConnectorRegistryDocument {
            version: REGISTRY_VERSION,
            connectors: self.records.values().cloned().collect(),
        };
        let bytes = serde_json::to_vec_pretty(&document)?;
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
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }
}

fn validate_record(record: &ConnectorRecord) -> Result<()> {
    if record.connector_id.len() > 128
        || !record.connector_id.starts_with("ncon_")
        || !record
            .connector_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        bail!("invalid connector id");
    }
    if record.agent_id.len() > 128 || !record.agent_id.starts_with("agt_") {
        bail!("invalid agent id");
    }
    if !matches!(record.control_plane_url.scheme(), "https" | "http") {
        bail!("unsupported control-plane URL scheme");
    }
    if record.control_plane_url.scheme() == "http"
        && !record
            .control_plane_url
            .host_str()
            .is_some_and(|host| host == "localhost" || host == "127.0.0.1" || host == "::1")
    {
        bail!("plaintext control-plane URLs are allowed only for loopback development");
    }
    if record.device_key_file.is_absolute()
        || record.device_key_file.components().any(|part| {
            matches!(
                part,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        bail!("device key path must remain inside the installation data directory");
    }
    if (record.printer_grant == PrinterGrant::SelectedPrinters
        && record.allowed_printer_ids.is_empty())
        || (record.printer_grant == PrinterGrant::AllLocalPrinters
            && !record.allowed_printer_ids.is_empty())
        || record.allowed_printer_ids.len() > 128
        || record
            .allowed_printer_ids
            .iter()
            .any(|id| id.is_empty() || id.len() > 128)
        || record
            .allowed_printer_ids
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != record.allowed_printer_ids.len()
    {
        bail!("invalid connector printer grants");
    }
    Ok(())
}

/// Round-robin admission queue for a bounded shared native executor.
/// Duplicate readiness signals are coalesced, preventing a busy tenant from
/// occupying the queue and starving quiet connectors.
#[derive(Debug)]
#[allow(dead_code, reason = "consumed by the staged connector supervisor")]
pub struct FairConnectorQueue {
    ready: VecDeque<String>,
    queued: std::collections::BTreeSet<String>,
    capacity: usize,
}

impl FairConnectorQueue {
    #[allow(dead_code, reason = "consumed by the staged connector supervisor")]
    pub(crate) fn new(capacity: usize) -> Result<Self> {
        if capacity == 0 || capacity > MAX_CONNECTORS {
            bail!("invalid scheduler capacity");
        }
        Ok(Self {
            ready: VecDeque::new(),
            queued: std::collections::BTreeSet::default(),
            capacity,
        })
    }
    #[allow(dead_code, reason = "consumed by the staged connector supervisor")]
    pub(crate) fn notify_ready(&mut self, connector_id: &str) -> bool {
        if self.queued.contains(connector_id) || self.ready.len() >= self.capacity {
            return false;
        }
        self.queued.insert(connector_id.to_owned());
        self.ready.push_back(connector_id.to_owned());
        true
    }
    #[allow(dead_code, reason = "consumed by the staged connector supervisor")]
    pub(crate) fn next(&mut self) -> Option<String> {
        let id = self.ready.pop_front()?;
        self.queued.remove(&id);
        Some(id)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use piqae_agent_storage::AgentStore;

    fn record(id: &str) -> ConnectorRecord {
        ConnectorRecord {
            connector_id: id.into(),
            agent_id: format!("agt_{id}"),
            control_plane_url: Url::parse("https://api.piqae.example/").unwrap(),
            device_key_file: format!("connectors/{id}/device.key").into(),
            enabled: true,
            printer_grant: PrinterGrant::SelectedPrinters,
            allowed_printer_ids: vec!["prn_allowed".into()],
        }
    }

    #[test]
    fn registry_survives_restart_and_revocation_is_connector_scoped() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = ConnectorRegistry::load(dir.path()).unwrap();
        registry.add(record("ncon_a")).unwrap();
        registry.add(record("ncon_b")).unwrap();
        assert!(registry.revoke("ncon_a").unwrap());
        let restarted = ConnectorRegistry::load(dir.path()).unwrap();
        assert_eq!(
            restarted
                .enabled()
                .map(|r| r.connector_id.as_str())
                .collect::<Vec<_>>(),
            ["ncon_b"]
        );
        assert_ne!(
            restarted.paths("ncon_a").unwrap().database,
            restarted.paths("ncon_b").unwrap().database
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                std::fs::metadata(dir.path().join("connectors.json"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn legacy_registry_defaults_to_selected_without_widening_access() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("connectors.json"),
            r#"{
              "version": 1,
              "connectors": [{
                "connector_id": "ncon_legacy",
                "agent_id": "agt_legacy",
                "control_plane_url": "https://api.piqae.example/",
                "device_key_file": "connectors/ncon_legacy/device.key",
                "enabled": true,
                "allowed_printer_ids": ["prn_allowed"]
              }]
            }"#,
        )
        .unwrap();
        let registry = ConnectorRegistry::load(dir.path()).unwrap();
        let record = registry.enabled().next().unwrap();
        assert_eq!(record.printer_grant, PrinterGrant::SelectedPrinters);
        assert_eq!(record.allowed_printer_ids, ["prn_allowed"]);
    }

    #[test]
    fn registry_rejects_escape_duplicate_and_insecure_remote_origin() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = ConnectorRegistry::load(dir.path()).unwrap();
        let mut invalid = record("ncon_escape");
        invalid.device_key_file = "../device.key".into();
        assert!(registry.add(invalid).is_err());
        let mut insecure = record("ncon_http");
        insecure.control_plane_url = Url::parse("http://example.com").unwrap();
        assert!(registry.add(insecure).is_err());
        registry.add(record("ncon_a")).unwrap();
        assert!(registry.add(record("ncon_a")).is_err());
    }

    #[test]
    fn scheduler_is_bounded_coalesced_and_round_robin() {
        let mut queue = FairConnectorQueue::new(2).unwrap();
        assert!(queue.notify_ready("ncon_a"));
        assert!(!queue.notify_ready("ncon_a"));
        assert!(queue.notify_ready("ncon_b"));
        assert!(!queue.notify_ready("ncon_c"));
        assert_eq!(queue.next().as_deref(), Some("ncon_a"));
        assert!(queue.notify_ready("ncon_a"));
        assert_eq!(queue.next().as_deref(), Some("ncon_b"));
        assert_eq!(queue.next().as_deref(), Some("ncon_a"));
    }

    #[test]
    fn printer_grants_are_persisted_in_canonical_order() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = ConnectorRegistry::load(dir.path()).unwrap();
        let mut connector = record("ncon_ordered");
        connector.allowed_printer_ids = vec!["prn_z".into(), "prn_a".into()];
        registry.add(connector).unwrap();
        let restarted = ConnectorRegistry::load(dir.path()).unwrap();
        assert_eq!(
            restarted.records["ncon_ordered"].allowed_printer_ids,
            ["prn_a", "prn_z"]
        );
    }

    #[test]
    fn replayed_enrolment_replaces_the_durable_printer_grant() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = ConnectorRegistry::load(dir.path()).unwrap();
        registry.add(record("ncon_existing")).unwrap();
        assert!(
            registry
                .update_allowed_printers(
                    "ncon_existing",
                    PrinterGrant::SelectedPrinters,
                    vec!["prn_new_z".into(), "prn_new_a".into()],
                )
                .unwrap()
        );

        let restarted = ConnectorRegistry::load(dir.path()).unwrap();
        assert_eq!(
            restarted.records["ncon_existing"].allowed_printer_ids,
            ["prn_new_a", "prn_new_z"]
        );
    }

    #[test]
    fn connector_databases_keep_cursors_isolated_across_restart() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = ConnectorRegistry::load(dir.path()).unwrap();
        registry.add(record("ncon_a")).unwrap();
        registry.add(record("ncon_b")).unwrap();
        let a = registry.paths("ncon_a").unwrap();
        let b = registry.paths("ncon_b").unwrap();
        std::fs::create_dir_all(a.database.parent().unwrap()).unwrap();
        std::fs::create_dir_all(b.database.parent().unwrap()).unwrap();
        let mut a_store = AgentStore::open(&a.database).unwrap();
        let mut b_store = AgentStore::open(&b.database).unwrap();
        a_store.set_setting("cloud_cursor", "41").unwrap();
        b_store.set_setting("cloud_cursor", "9001").unwrap();
        drop((a_store, b_store));

        let restarted = ConnectorRegistry::load(dir.path()).unwrap();
        let a_store = AgentStore::open(restarted.paths("ncon_a").unwrap().database).unwrap();
        let b_store = AgentStore::open(restarted.paths("ncon_b").unwrap().database).unwrap();
        assert_eq!(
            a_store.setting("cloud_cursor").unwrap().as_deref(),
            Some("41")
        );
        assert_eq!(
            b_store.setting("cloud_cursor").unwrap().as_deref(),
            Some("9001")
        );
    }
}
