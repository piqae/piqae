//! Durable multi-connector runtime configuration and fair scheduling.
//!
//! A connector is an independent cloud security principal. Its signing key,
//! `SQLite` database, content directory, sync cursor, leases, and event outbox
//! must never be shared with another connector. The physical installation may
//! share printer discovery and the native executor, but those resources are
//! reached only after the connector-specific store has admitted a job.

#![allow(
    clippy::missing_errors_doc,
    reason = "durable compatibility API was moved intact and preserves existing anyhow error context"
)]

use anyhow::{Context, Result, bail};
use piqae_node_host_api::SecureKeyHandle;
use piqae_protocol::agent::PrinterGrant;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
use url::Url;

const REGISTRY_VERSION: u16 = 1;
const MAX_CONNECTORS: usize = 128;

/// Identifies connectors that share a cross-authority physical group.
///
/// Connectors whose allowed local queues share a physical
/// serialization group across independent control-plane origins. This is a
/// diagnostic only: the node serializes their native handoffs locally, but it
/// cannot promise global exactly-once scheduling or fail over work between
/// authorities which do not share a reservation ledger.
pub fn cross_authority_connectors(
    records: &[ConnectorRecord],
    printer_groups: &[(String, String)],
) -> BTreeSet<String> {
    let mut groups = BTreeMap::<String, BTreeMap<String, BTreeSet<String>>>::new();
    for record in records.iter().filter(|record| record.enabled) {
        let origin = record.control_plane_url.origin().ascii_serialization();
        let allowed = match record.printer_grant {
            PrinterGrant::AllLocalPrinters => None,
            PrinterGrant::SelectedPrinters => Some(
                record
                    .allowed_printer_ids
                    .iter()
                    .map(String::as_str)
                    .collect::<BTreeSet<_>>(),
            ),
        };
        for (printer_id, coordination_key) in printer_groups {
            if allowed
                .as_ref()
                .is_some_and(|selected| !selected.contains(printer_id.as_str()))
            {
                continue;
            }
            groups
                .entry(coordination_key.clone())
                .or_default()
                .entry(origin.clone())
                .or_default()
                .insert(record.connector_id.clone());
        }
    }
    groups
        .into_values()
        .filter(|origins| origins.len() > 1)
        .flat_map(|origins| origins.into_values().flatten())
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConnectorRecord {
    pub connector_id: String,
    pub agent_id: String,
    pub control_plane_url: Url,
    /// Operator-facing identity captured from the authenticated invitation.
    /// It is metadata only and is never used for authorization.
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub workspace_name: Option<String>,
    #[serde(default)]
    pub authorization_type: Option<String>,
    /// Stable, non-secret tenant identity captured from the signed preview.
    #[serde(default)]
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub environment_id: Option<String>,
    /// Platform/service account that requested a managed-customer connection.
    #[serde(default)]
    pub requesting_service_account_id: Option<String>,
    /// Explicit HTTPS destination supplied by the connection owner. This is
    /// the only URL the local UI may offer for management or reauthorization.
    #[serde(default)]
    pub manage_url: Option<Url>,
    /// Relative to the installation data directory. Never accept an absolute
    /// or parent-traversing path from a downloaded enrolment response.
    #[serde(default)]
    pub device_key_file: Option<PathBuf>,
    /// Opaque non-exporting secure-store reference used by embedded hosts.
    /// Exactly one signing credential representation must be present.
    #[serde(default)]
    pub secure_key_handle: Option<SecureKeyHandle>,
    pub enabled: bool,
    /// Durable authorization policy. Older registry documents safely decode as
    /// selected-printer grants rather than widening access.
    #[serde(default)]
    pub printer_grant: PrinterGrant,
    /// Explicit local printer grants. Empty is valid only for all-printer access.
    #[serde(default)]
    pub allowed_printer_ids: Vec<String>,
    /// Independent authority revision for display-only node metadata.
    #[serde(default)]
    pub node_identity_revision: Option<u64>,
    /// Local host-configuration revision acknowledged by this authority.
    #[serde(default)]
    pub node_identity_applied_local_revision: Option<u64>,
    /// Newer authority revision observed during optimistic reconciliation.
    #[serde(default)]
    pub node_identity_conflict_revision: Option<u64>,
    /// Local revision which produced the conflict. The exact edit is not
    /// retried until the operator explicitly saves a newer local revision.
    #[serde(default)]
    pub node_identity_conflict_local_revision: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ConnectorRegistryDocument {
    version: u16,
    connectors: Vec<ConnectorRecord>,
    #[serde(default)]
    prepared_keys: Vec<PreparedConnectorKey>,
    #[serde(default)]
    key_cleanup: Vec<SecureKeyHandle>,
    /// Connector ids whose authority grant has been durably revoked. Disabled
    /// ids absent here retain their credential for authenticated retry.
    #[serde(default)]
    remote_revoked: Vec<String>,
    #[serde(default)]
    installation_identity: Option<InstallationSigningIdentity>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InstallationSigningIdentity {
    pub installation_id: String,
    pub handle: SecureKeyHandle,
    pub public_key: [u8; 32],
}

/// Opaque key generated for an invitation which has not yet become active.
///
/// It is durable so abandoned exchange flows are reclaimed after a restart
/// instead of leaking platform credentials forever.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PreparedConnectorKey {
    pub handle: SecureKeyHandle,
    pub public_key: [u8; 32],
    pub expires_unix_ms: i64,
}

#[derive(Debug, Clone)]
#[allow(dead_code, reason = "consumed by the staged connector supervisor")]
pub struct ConnectorRuntimePaths {
    pub database: PathBuf,
    pub content: PathBuf,
    pub device_key: Option<PathBuf>,
}

#[derive(Debug)]
pub struct ConnectorRegistry {
    root: PathBuf,
    records: BTreeMap<String, ConnectorRecord>,
    prepared_keys: Vec<PreparedConnectorKey>,
    key_cleanup: Vec<SecureKeyHandle>,
    remote_revoked: BTreeSet<String>,
    installation_identity: Option<InstallationSigningIdentity>,
    #[cfg(test)]
    fail_next_persist: bool,
}

impl ConnectorRegistry {
    pub fn load(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        let path = root.join("connectors.json");
        let document = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice::<ConnectorRegistryDocument>(&bytes)
                .with_context(|| format!("parse {}", path.display()))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                ConnectorRegistryDocument {
                    version: REGISTRY_VERSION,
                    connectors: Vec::new(),
                    prepared_keys: Vec::new(),
                    key_cleanup: Vec::new(),
                    remote_revoked: Vec::new(),
                    installation_identity: None,
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
        validate_key_state(&records, &document.prepared_keys, &document.key_cleanup)?;
        let remote_revoked = document.remote_revoked.into_iter().collect::<BTreeSet<_>>();
        if remote_revoked.iter().any(|connector_id| {
            records
                .get(connector_id)
                .is_none_or(|record| record.enabled)
        }) {
            bail!("remote revocation state does not match a disabled connector");
        }
        Ok(Self {
            root,
            records,
            prepared_keys: document.prepared_keys,
            key_cleanup: document.key_cleanup,
            remote_revoked,
            installation_identity: document.installation_identity,
            #[cfg(test)]
            fail_next_persist: false,
        })
    }

    pub fn enabled(&self) -> impl Iterator<Item = &ConnectorRecord> {
        self.records.values().filter(|record| record.enabled)
    }

    pub fn records(&self) -> impl Iterator<Item = &ConnectorRecord> {
        self.records.values()
    }

    /// Atomically updates only connector-scoped node identity reconciliation
    /// metadata. Credentials, installation identity, grants, routes and queue
    /// paths are retained byte-for-byte.
    pub fn update_identity_reconciliation(
        &mut self,
        connector_id: &str,
        server_revision: Option<u64>,
        applied_local_revision: Option<u64>,
        conflict_revision: Option<u64>,
        conflict_local_revision: Option<u64>,
    ) -> Result<()> {
        let mut candidate = self.records.clone();
        let record = candidate
            .get_mut(connector_id)
            .context("connector was not found")?;
        record.node_identity_revision = server_revision;
        record.node_identity_applied_local_revision = applied_local_revision;
        record.node_identity_conflict_revision = conflict_revision;
        record.node_identity_conflict_local_revision = conflict_local_revision;
        self.persist_records(&candidate)?;
        self.records = candidate;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) const fn inject_next_persist_failure(&mut self) {
        self.fail_next_persist = true;
    }

    #[must_use]
    pub const fn installation_identity(&self) -> Option<&InstallationSigningIdentity> {
        self.installation_identity.as_ref()
    }

    /// Stores the installation signing principal once. It is intentionally
    /// outside connector revocation and cleanup lifecycles.
    pub fn set_installation_identity_once(
        &mut self,
        identity: InstallationSigningIdentity,
    ) -> Result<()> {
        if self.installation_identity.is_some() || self.handle_is_known(&identity.handle) {
            bail!("installation signing identity already exists or reuses a connector key");
        }
        if !identity.installation_id.starts_with("ins_") || identity.installation_id.len() > 128 {
            bail!("invalid installation identity");
        }
        self.persist_all(
            &self.records.clone(),
            &self.prepared_keys.clone(),
            &self.key_cleanup.clone(),
            &self.remote_revoked.clone(),
            Some(identity.clone()),
        )?;
        self.installation_identity = Some(identity);
        Ok(())
    }

    /// Durably records an invitation key before its handle is returned to the
    /// host UI. Active and cleanup-pending handles may never be reused.
    pub fn register_prepared_key(
        &mut self,
        handle: SecureKeyHandle,
        public_key: [u8; 32],
        expires_unix_ms: i64,
    ) -> Result<()> {
        if expires_unix_ms <= chrono::Utc::now().timestamp_millis() || self.handle_is_known(&handle)
        {
            bail!("prepared connector key is invalid or already known");
        }
        let mut prepared = self.prepared_keys.clone();
        prepared.push(PreparedConnectorKey {
            handle,
            public_key,
            expires_unix_ms,
        });
        self.persist_state(&self.records.clone(), &prepared, &self.key_cleanup.clone())?;
        self.prepared_keys = prepared;
        Ok(())
    }

    /// Atomically activates a prepared key and removes its expiry record.
    pub fn complete_prepared(&mut self, mut record: ConnectorRecord) -> Result<()> {
        validate_record(&record)?;
        record.allowed_printer_ids.sort();
        let handle = record
            .secure_key_handle
            .as_ref()
            .context("embedded connector has no secure key handle")?;
        let Some(prepared_index) = self
            .prepared_keys
            .iter()
            .position(|pending| pending.handle == *handle)
        else {
            bail!("connector key was not prepared by this runtime");
        };
        if self.prepared_keys[prepared_index].expires_unix_ms
            <= chrono::Utc::now().timestamp_millis()
        {
            bail!("connector key preparation expired");
        }
        if self.records.contains_key(&record.connector_id) {
            bail!("connector already exists");
        }
        let mut records = self.records.clone();
        records.insert(record.connector_id.clone(), record);
        let mut prepared = self.prepared_keys.clone();
        prepared.remove(prepared_index);
        self.persist_state(&records, &prepared, &self.key_cleanup.clone())?;
        self.records = records;
        self.prepared_keys = prepared;
        Ok(())
    }

    /// Schedules an abandoned prepared key for idempotent provider deletion.
    /// The durable cleanup intent is committed before the caller invokes the
    /// provider.
    pub fn cancel_prepared_key(&mut self, handle: &SecureKeyHandle) -> Result<bool> {
        let Some(index) = self
            .prepared_keys
            .iter()
            .position(|pending| pending.handle == *handle)
        else {
            return Ok(false);
        };
        let mut prepared = self.prepared_keys.clone();
        let removed = prepared.remove(index);
        let mut cleanup = self.key_cleanup.clone();
        if !cleanup.contains(&removed.handle) {
            cleanup.push(removed.handle);
        }
        self.persist_state(&self.records.clone(), &prepared, &cleanup)?;
        self.prepared_keys = prepared;
        self.key_cleanup = cleanup;
        Ok(true)
    }

    /// Moves every expired preparation into the durable cleanup queue.
    pub fn expire_prepared_keys(&mut self, now_unix_ms: i64) -> Result<usize> {
        let (expired, retained): (Vec<_>, Vec<_>) = self
            .prepared_keys
            .iter()
            .cloned()
            .partition(|pending| pending.expires_unix_ms <= now_unix_ms);
        if expired.is_empty() {
            return Ok(0);
        }
        let mut cleanup = self.key_cleanup.clone();
        for pending in &expired {
            if !cleanup.contains(&pending.handle) {
                cleanup.push(pending.handle.clone());
            }
        }
        self.persist_state(&self.records.clone(), &retained, &cleanup)?;
        self.prepared_keys = retained;
        self.key_cleanup = cleanup;
        Ok(expired.len())
    }

    #[must_use]
    pub fn key_cleanup(&self) -> &[SecureKeyHandle] {
        &self.key_cleanup
    }

    pub fn pending_remote_revocations(&self) -> impl Iterator<Item = &ConnectorRecord> {
        self.records
            .values()
            .filter(|record| !record.enabled && !self.remote_revoked.contains(&record.connector_id))
    }

    pub fn remotely_revoked(&self) -> impl Iterator<Item = &ConnectorRecord> {
        self.records
            .values()
            .filter(|record| self.remote_revoked.contains(&record.connector_id))
    }

    /// Commits authority-side denial before making the credential eligible
    /// for provider deletion.
    pub fn confirm_remote_revocation(&mut self, connector_id: &str) -> Result<bool> {
        let record = self
            .records
            .get(connector_id)
            .context("connector was not found")?;
        if record.enabled {
            bail!("active connector cannot confirm remote revocation");
        }
        if self.remote_revoked.contains(connector_id) {
            return Ok(false);
        }
        let mut revoked = self.remote_revoked.clone();
        revoked.insert(connector_id.to_owned());
        let mut cleanup = self.key_cleanup.clone();
        if let Some(handle) = &record.secure_key_handle
            && !cleanup.contains(handle)
        {
            cleanup.push(handle.clone());
        }
        self.persist_all(
            &self.records.clone(),
            &self.prepared_keys.clone(),
            &cleanup,
            &revoked,
            self.installation_identity.clone(),
        )?;
        self.remote_revoked = revoked;
        self.key_cleanup = cleanup;
        Ok(true)
    }

    #[must_use]
    pub fn prepared_key(&self, handle: &SecureKeyHandle) -> Option<&PreparedConnectorKey> {
        self.prepared_keys
            .iter()
            .find(|pending| pending.handle == *handle)
    }

    /// Removes a cleanup intent only after the secure provider confirms its
    /// idempotent delete. Active connector handles are never eligible.
    pub fn confirm_key_cleanup(&mut self, handle: &SecureKeyHandle) -> Result<bool> {
        if self.active_handle(handle) {
            bail!("refusing to clean up an active connector key");
        }
        let Some(index) = self.key_cleanup.iter().position(|value| value == handle) else {
            return Ok(false);
        };
        let mut cleanup = self.key_cleanup.clone();
        cleanup.remove(index);
        self.persist_state(&self.records.clone(), &self.prepared_keys.clone(), &cleanup)?;
        self.key_cleanup = cleanup;
        Ok(true)
    }

    #[must_use]
    pub fn contains(&self, connector_id: &str) -> bool {
        self.records.contains_key(connector_id)
    }

    #[allow(dead_code, reason = "consumed by the staged connector supervisor")]
    pub fn paths(&self, connector_id: &str) -> Result<ConnectorRuntimePaths> {
        let record = self
            .records
            .get(connector_id)
            .context("connector was not found")?;
        let connector_root = self.root.join("connectors").join(&record.connector_id);
        Ok(ConnectorRuntimePaths {
            database: connector_root.join("agent.sqlite3"),
            content: connector_root.join("content"),
            device_key: record
                .device_key_file
                .as_ref()
                .map(|path| self.root.join(path)),
        })
    }

    /// Adds a connector without replacing an existing security principal.
    #[allow(
        dead_code,
        reason = "called by the native consent IPC in the next integration slice"
    )]
    pub fn add(&mut self, mut record: ConnectorRecord) -> Result<()> {
        validate_record(&record)?;
        record.allowed_printer_ids.sort();
        if self.records.len() >= MAX_CONNECTORS {
            bail!("connector limit reached");
        }
        if self.records.contains_key(&record.connector_id) {
            bail!("connector already exists");
        }
        let mut candidate = self.records.clone();
        candidate.insert(record.connector_id.clone(), record);
        self.persist_records(&candidate)?;
        self.records = candidate;
        Ok(())
    }

    /// Revocation is fail-closed and durable before the caller stops its task.
    #[allow(
        dead_code,
        reason = "called by the native consent IPC in the next integration slice"
    )]
    pub fn revoke(&mut self, connector_id: &str) -> Result<bool> {
        let Some(record) = self.records.get(connector_id) else {
            return Ok(false);
        };
        if !record.enabled {
            return Ok(false);
        }
        let mut candidate = self.records.clone();
        if let Some(record) = candidate.get_mut(connector_id) {
            record.enabled = false;
        }
        self.persist_state(
            &candidate,
            &self.prepared_keys.clone(),
            &self.key_cleanup.clone(),
        )?;
        self.records = candidate;
        Ok(true)
    }

    /// Atomically replaces the durable credentials and metadata for one
    /// server-side connector. Re-enrolment may rotate the agent signing key
    /// while deliberately returning the same connector id; retaining the old
    /// local key in that case strands the connector with
    /// `invalid_agent_signature`.
    pub fn replace(&mut self, mut record: ConnectorRecord) -> Result<ConnectorRecord> {
        validate_record(&record)?;
        record.allowed_printer_ids.sort();
        let previous = self
            .records
            .get(&record.connector_id)
            .cloned()
            .context("connector was not found")?;
        let mut candidate = self.records.clone();
        candidate.insert(record.connector_id.clone(), record);
        let mut remote_revoked = self.remote_revoked.clone();
        remote_revoked.remove(&previous.connector_id);
        self.persist_all(
            &candidate,
            &self.prepared_keys.clone(),
            &self.key_cleanup.clone(),
            &remote_revoked,
            self.installation_identity.clone(),
        )?;
        self.records = candidate;
        self.remote_revoked = remote_revoked;
        Ok(previous)
    }

    #[allow(
        clippy::needless_pass_by_ref_mut,
        reason = "test fault injection is consumed atomically before the replacement"
    )]
    fn persist_records(&mut self, records: &BTreeMap<String, ConnectorRecord>) -> Result<()> {
        self.persist_state(
            records,
            &self.prepared_keys.clone(),
            &self.key_cleanup.clone(),
        )
    }

    #[allow(
        clippy::needless_pass_by_ref_mut,
        reason = "test fault injection is consumed by the delegated atomic replacement"
    )]
    fn persist_state(
        &mut self,
        records: &BTreeMap<String, ConnectorRecord>,
        prepared_keys: &[PreparedConnectorKey],
        key_cleanup: &[SecureKeyHandle],
    ) -> Result<()> {
        self.persist_all(
            records,
            prepared_keys,
            key_cleanup,
            &self.remote_revoked.clone(),
            self.installation_identity.clone(),
        )
    }

    #[allow(
        clippy::needless_pass_by_ref_mut,
        reason = "test builds consume the injected atomic replacement failure"
    )]
    fn persist_all(
        &mut self,
        records: &BTreeMap<String, ConnectorRecord>,
        prepared_keys: &[PreparedConnectorKey],
        key_cleanup: &[SecureKeyHandle],
        remote_revoked: &BTreeSet<String>,
        installation_identity: Option<InstallationSigningIdentity>,
    ) -> Result<()> {
        if records.len() > MAX_CONNECTORS {
            bail!("connector registry exceeds the {MAX_CONNECTORS} connector limit");
        }
        validate_key_state(records, prepared_keys, key_cleanup)?;
        if remote_revoked.iter().any(|connector_id| {
            records
                .get(connector_id)
                .is_none_or(|record| record.enabled)
        }) {
            bail!("remote revocation state does not match a disabled connector");
        }
        #[cfg(test)]
        if std::mem::take(&mut self.fail_next_persist) {
            bail!("injected connector registry replacement failure");
        }
        let path = self.root.join("connectors.json");
        let document = ConnectorRegistryDocument {
            version: REGISTRY_VERSION,
            connectors: records.values().cloned().collect(),
            prepared_keys: prepared_keys.to_vec(),
            key_cleanup: key_cleanup.to_vec(),
            remote_revoked: remote_revoked.iter().cloned().collect(),
            installation_identity,
        };
        let bytes = serde_json::to_vec_pretty(&document)?;
        crate::durable_file::replace_json(&path, &bytes)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }

    fn active_handle(&self, handle: &SecureKeyHandle) -> bool {
        self.records
            .values()
            .any(|record| record.enabled && record.secure_key_handle.as_ref() == Some(handle))
    }

    fn handle_is_known(&self, handle: &SecureKeyHandle) -> bool {
        self.records
            .values()
            .any(|record| record.secure_key_handle.as_ref() == Some(handle))
            || self
                .prepared_keys
                .iter()
                .any(|value| value.handle == *handle)
            || self.key_cleanup.contains(handle)
            || self
                .installation_identity
                .as_ref()
                .is_some_and(|identity| identity.handle == *handle)
    }
}

fn validate_key_state(
    records: &BTreeMap<String, ConnectorRecord>,
    prepared: &[PreparedConnectorKey],
    cleanup: &[SecureKeyHandle],
) -> Result<()> {
    if prepared.len() > MAX_CONNECTORS || cleanup.len() > MAX_CONNECTORS.saturating_mul(2) {
        bail!("connector key state exceeds supported bounds");
    }
    for pending in prepared {
        if records
            .values()
            .any(|record| record.secure_key_handle.as_ref() == Some(&pending.handle))
            || cleanup.contains(&pending.handle)
        {
            bail!("connector key state overlaps active or cleanup state");
        }
    }
    Ok(())
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
    if record.manage_url.as_ref().is_some_and(|url| {
        let local_http = url.scheme() == "http"
            && url
                .host_str()
                .is_some_and(|host| matches!(host, "localhost" | "127.0.0.1" | "::1"));
        (url.scheme() != "https" && !local_http)
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
    }) {
        bail!("connector management URL is not operator-safe");
    }
    if record.control_plane_url.scheme() == "http"
        && !record
            .control_plane_url
            .host_str()
            .is_some_and(|host| host == "localhost" || host == "127.0.0.1" || host == "::1")
    {
        bail!("plaintext control-plane URLs are allowed only for loopback development");
    }
    if record.device_key_file.is_some() == record.secure_key_handle.is_some() {
        bail!("connector must have exactly one secure signing credential");
    }
    if record.device_key_file.as_ref().is_some_and(|path| {
        path.is_absolute()
            || path.components().any(|part| {
                matches!(
                    part,
                    std::path::Component::ParentDir
                        | std::path::Component::RootDir
                        | std::path::Component::Prefix(_)
                )
            })
    }) {
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
    if [
        record.node_identity_revision,
        record.node_identity_applied_local_revision,
        record.node_identity_conflict_revision,
        record.node_identity_conflict_local_revision,
    ]
    .into_iter()
    .flatten()
    .any(|revision| revision == 0)
        || record.node_identity_conflict_revision.is_some()
            != record.node_identity_conflict_local_revision.is_some()
    {
        bail!("invalid connector node identity reconciliation state");
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use chrono::Utc;
    use piqae_agent_storage::AgentStore;

    fn record(id: &str) -> ConnectorRecord {
        ConnectorRecord {
            connector_id: id.into(),
            agent_id: format!("agt_{id}"),
            control_plane_url: Url::parse("https://api.piqae.example/").unwrap(),
            display_name: Some("Example service".into()),
            workspace_name: Some("Example customer".into()),
            authorization_type: Some("platform_customer".into()),
            workspace_id: Some("wsp_test".into()),
            environment_id: Some("env_live".into()),
            requesting_service_account_id: Some("svc_example".into()),
            manage_url: Some(Url::parse("https://app.example/manage").unwrap()),
            device_key_file: Some(format!("connectors/{id}/device.key").into()),
            secure_key_handle: None,
            enabled: true,
            printer_grant: PrinterGrant::SelectedPrinters,
            allowed_printer_ids: vec!["prn_allowed".into()],
            node_identity_revision: None,
            node_identity_applied_local_revision: None,
            node_identity_conflict_revision: None,
            node_identity_conflict_local_revision: None,
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
        let surviving = restarted.enabled().next().unwrap();
        assert_eq!(surviving.workspace_id.as_deref(), Some("wsp_test"));
        assert_eq!(
            surviving.manage_url.as_ref().map(Url::as_str),
            Some("https://app.example/manage")
        );
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
    fn identity_reconciliation_is_connector_scoped_and_preserves_authority_state() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = ConnectorRegistry::load(dir.path()).unwrap();
        let first = record("ncon_a");
        let second = record("ncon_b");
        registry.add(first.clone()).unwrap();
        registry.add(second.clone()).unwrap();
        registry
            .update_identity_reconciliation("ncon_a", Some(8), Some(3), None, None)
            .unwrap();

        let restarted = ConnectorRegistry::load(dir.path()).unwrap();
        let updated = restarted
            .records()
            .find(|record| record.connector_id == "ncon_a")
            .unwrap();
        assert_eq!(updated.node_identity_revision, Some(8));
        assert_eq!(updated.node_identity_applied_local_revision, Some(3));
        assert_eq!(updated.agent_id, first.agent_id);
        assert_eq!(updated.device_key_file, first.device_key_file);
        assert_eq!(updated.printer_grant, first.printer_grant);
        assert_eq!(updated.allowed_printer_ids, first.allowed_printer_ids);

        let untouched = restarted
            .records()
            .find(|record| record.connector_id == "ncon_b")
            .unwrap();
        assert_eq!(untouched, &second);
    }

    #[test]
    fn cross_authority_warning_requires_a_shared_allowed_physical_group() {
        let mut hosted = record("ncon_hosted");
        hosted.allowed_printer_ids = vec!["ptr_shared".into()];
        let mut same_origin = record("ncon_same_origin");
        same_origin.allowed_printer_ids = vec!["ptr_shared".into()];
        let mut self_hosted = record("ncon_self_hosted");
        self_hosted.control_plane_url = Url::parse("https://print.internal.example/").unwrap();
        self_hosted.allowed_printer_ids = vec!["ptr_shared".into()];
        let mut unrelated = record("ncon_unrelated");
        unrelated.control_plane_url = Url::parse("https://other.example/").unwrap();
        unrelated.allowed_printer_ids = vec!["ptr_other".into()];

        let warnings = cross_authority_connectors(
            &[hosted, same_origin, self_hosted, unrelated],
            &[
                ("ptr_shared".into(), "physical-a".into()),
                ("ptr_other".into(), "physical-b".into()),
            ],
        );
        assert_eq!(
            warnings,
            BTreeSet::from([
                "ncon_hosted".to_owned(),
                "ncon_same_origin".to_owned(),
                "ncon_self_hosted".to_owned(),
            ])
        );
        assert!(!warnings.contains("ncon_unrelated"));
    }

    #[test]
    fn management_destinations_fail_closed() {
        let mut connector = record("ncon_unsafe");
        connector.manage_url = Some(Url::parse("http://owner.example/manage").unwrap());
        assert!(validate_record(&connector).is_err());

        connector.manage_url = Some(Url::parse("http://localhost:5173/manage").unwrap());
        assert!(validate_record(&connector).is_ok());
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
        invalid.device_key_file = Some("../device.key".into());
        assert!(registry.add(invalid).is_err());
        let mut insecure = record("ncon_http");
        insecure.control_plane_url = Url::parse("http://example.com").unwrap();
        assert!(registry.add(insecure).is_err());
        registry.add(record("ncon_a")).unwrap();
        assert!(registry.add(record("ncon_a")).is_err());
    }

    #[test]
    fn mutation_bounds_never_persist_an_unopenable_key_registry() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = ConnectorRegistry::load(dir.path()).unwrap();
        let now = Utc::now().timestamp_millis();
        for index in 0..MAX_CONNECTORS {
            registry
                .register_prepared_key(
                    SecureKeyHandle::new(format!("prepared-{index}")).unwrap(),
                    [u8::try_from(index).unwrap_or_default(); 32],
                    now.saturating_add(60_000),
                )
                .unwrap();
        }
        assert!(
            registry
                .register_prepared_key(
                    SecureKeyHandle::new("prepared-overflow".into()).unwrap(),
                    [255; 32],
                    now.saturating_add(60_000),
                )
                .is_err()
        );
        drop(registry);
        let mut registry = ConnectorRegistry::load(dir.path()).unwrap();
        assert_eq!(registry.prepared_keys.len(), MAX_CONNECTORS);

        for index in 0..MAX_CONNECTORS {
            registry
                .cancel_prepared_key(&SecureKeyHandle::new(format!("prepared-{index}")).unwrap())
                .unwrap();
        }
        for index in 0..MAX_CONNECTORS {
            let handle = SecureKeyHandle::new(format!("second-{index}")).unwrap();
            registry
                .register_prepared_key(
                    handle.clone(),
                    [u8::try_from(index).unwrap_or_default(); 32],
                    now.saturating_add(60_000),
                )
                .unwrap();
            registry.cancel_prepared_key(&handle).unwrap();
        }
        let overflow = SecureKeyHandle::new("cleanup-overflow".into()).unwrap();
        registry
            .register_prepared_key(overflow.clone(), [254; 32], now.saturating_add(60_000))
            .unwrap();
        assert!(registry.cancel_prepared_key(&overflow).is_err());
        assert!(registry.prepared_key(&overflow).is_some());
        drop(registry);
        let restarted = ConnectorRegistry::load(dir.path()).unwrap();
        assert_eq!(restarted.key_cleanup.len(), MAX_CONNECTORS * 2);
        assert!(restarted.prepared_key(&overflow).is_some());
    }

    #[test]
    fn embedded_connector_persists_only_an_opaque_secure_key_handle() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = ConnectorRegistry::load(dir.path()).unwrap();
        let mut connector = record("ncon_embedded");
        connector.device_key_file = None;
        connector.secure_key_handle =
            Some(SecureKeyHandle::new("app/connector-key".into()).unwrap());
        registry.add(connector).unwrap();
        drop(registry);

        let restarted = ConnectorRegistry::load(dir.path()).unwrap();
        let connector = restarted.records.get("ncon_embedded").unwrap();
        assert!(connector.device_key_file.is_none());
        assert_eq!(
            connector.secure_key_handle.as_ref().unwrap().as_str(),
            "app/connector-key"
        );
        assert!(
            restarted
                .paths("ncon_embedded")
                .unwrap()
                .device_key
                .is_none()
        );
        let serialized = std::fs::read_to_string(dir.path().join("connectors.json")).unwrap();
        assert!(!serialized.contains("PRIVATE KEY"));
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
    fn reauthentication_rotates_only_the_matching_durable_connector() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = ConnectorRegistry::load(dir.path()).unwrap();
        let healthy = record("ncon_healthy");
        let mut stale = record("ncon_child");
        stale.device_key_file = Some("connectors/keys/stale.key".into());
        stale.enabled = false;
        registry.add(healthy.clone()).unwrap();
        registry.add(stale.clone()).unwrap();

        let mut replacement = stale;
        replacement.enabled = true;
        replacement.agent_id = "agt_child_reauthenticated".into();
        replacement.device_key_file = Some("connectors/keys/current.key".into());
        replacement.printer_grant = PrinterGrant::AllLocalPrinters;
        replacement.allowed_printer_ids.clear();
        let previous = registry.replace(replacement.clone()).unwrap();
        assert_eq!(
            previous.device_key_file,
            Some(PathBuf::from("connectors/keys/stale.key"))
        );

        let restarted = ConnectorRegistry::load(dir.path()).unwrap();
        assert_eq!(restarted.records["ncon_healthy"], healthy);
        assert_eq!(restarted.records["ncon_child"], replacement);
        assert_eq!(restarted.enabled().count(), 2);
    }

    #[test]
    fn failed_replacements_never_change_live_or_restarted_registry_state() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = ConnectorRegistry::load(dir.path()).unwrap();

        registry.fail_next_persist = true;
        assert!(registry.add(record("ncon_add")).is_err());
        assert!(!registry.contains("ncon_add"));
        assert!(
            !ConnectorRegistry::load(dir.path())
                .unwrap()
                .contains("ncon_add")
        );

        registry.add(record("ncon_existing")).unwrap();
        registry.fail_next_persist = true;
        assert!(registry.revoke("ncon_existing").is_err());
        assert!(registry.records["ncon_existing"].enabled);
        assert!(ConnectorRegistry::load(dir.path()).unwrap().records["ncon_existing"].enabled);

        let mut replacement = record("ncon_existing");
        replacement.agent_id = "agt_rotated".into();
        registry.fail_next_persist = true;
        assert!(registry.replace(replacement).is_err());
        assert_eq!(
            registry.records["ncon_existing"].agent_id,
            "agt_ncon_existing"
        );
        assert_eq!(
            ConnectorRegistry::load(dir.path()).unwrap().records["ncon_existing"].agent_id,
            "agt_ncon_existing"
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

    #[test]
    fn prepared_keys_expire_and_cleanup_retries_across_restart_without_touching_active_keys() {
        let dir = tempfile::tempdir().unwrap();
        let pending = SecureKeyHandle::new("keychain/pending".into()).unwrap();
        let active = SecureKeyHandle::new("keychain/active".into()).unwrap();
        let mut registry = ConnectorRegistry::load(dir.path()).unwrap();
        registry
            .register_prepared_key(pending.clone(), [3; 32], Utc::now().timestamp_millis() + 1)
            .unwrap();
        registry
            .register_prepared_key(
                active.clone(),
                [5; 32],
                Utc::now().timestamp_millis() + 60_000,
            )
            .unwrap();
        let mut connector = record("ncon_embedded_active");
        connector.device_key_file = None;
        connector.secure_key_handle = Some(active.clone());
        registry.complete_prepared(connector).unwrap();
        assert_eq!(
            registry
                .expire_prepared_keys(Utc::now().timestamp_millis() + 5)
                .unwrap(),
            1
        );
        drop(registry);

        let mut restarted = ConnectorRegistry::load(dir.path()).unwrap();
        assert_eq!(restarted.key_cleanup(), &[pending.clone()]);
        assert!(restarted.confirm_key_cleanup(&active).is_err());
        // A provider failure leaves this list unchanged; only a confirmed
        // idempotent provider delete calls confirm_key_cleanup.
        drop(restarted);
        let mut retried = ConnectorRegistry::load(dir.path()).unwrap();
        assert_eq!(retried.key_cleanup(), &[pending.clone()]);
        assert!(retried.confirm_key_cleanup(&pending).unwrap());
        assert!(
            ConnectorRegistry::load(dir.path())
                .unwrap()
                .key_cleanup()
                .is_empty()
        );
    }

    #[test]
    fn revocation_is_durable_before_secure_key_cleanup_and_never_reenables_on_failure() {
        let dir = tempfile::tempdir().unwrap();
        let handle = SecureKeyHandle::new("credential/child".into()).unwrap();
        let mut registry = ConnectorRegistry::load(dir.path()).unwrap();
        registry
            .register_prepared_key(
                handle.clone(),
                [7; 32],
                Utc::now().timestamp_millis() + 60_000,
            )
            .unwrap();
        let mut connector = record("ncon_child_secure");
        connector.device_key_file = None;
        connector.secure_key_handle = Some(handle.clone());
        registry.complete_prepared(connector).unwrap();
        assert!(registry.revoke("ncon_child_secure").unwrap());
        assert!(registry.key_cleanup().is_empty());
        assert_eq!(
            registry
                .pending_remote_revocations()
                .map(|record| record.connector_id.as_str())
                .collect::<Vec<_>>(),
            ["ncon_child_secure"]
        );
        registry
            .confirm_remote_revocation("ncon_child_secure")
            .unwrap();
        assert_eq!(registry.key_cleanup(), &[handle.clone()]);
        drop(registry);

        let restarted = ConnectorRegistry::load(dir.path()).unwrap();
        assert!(!restarted.records["ncon_child_secure"].enabled);
        assert_eq!(restarted.key_cleanup(), &[handle]);
    }

    #[test]
    fn installation_identity_survives_first_connector_revocation_and_second_enrollment() {
        let dir = tempfile::tempdir().unwrap();
        let installation = InstallationSigningIdentity {
            installation_id: "ins_stable_fixture".into(),
            handle: SecureKeyHandle::new("keychain/installation".into()).unwrap(),
            public_key: [29; 32],
        };
        let first = SecureKeyHandle::new("keychain/first".into()).unwrap();
        let mut registry = ConnectorRegistry::load(dir.path()).unwrap();
        registry
            .set_installation_identity_once(installation.clone())
            .unwrap();
        registry
            .register_prepared_key(
                first.clone(),
                [31; 32],
                Utc::now().timestamp_millis() + 60_000,
            )
            .unwrap();
        let mut first_record = record("ncon_first");
        first_record.device_key_file = None;
        first_record.secure_key_handle = Some(first);
        registry.complete_prepared(first_record).unwrap();
        registry.revoke("ncon_first").unwrap();
        drop(registry);

        let second = SecureKeyHandle::new("keychain/second".into()).unwrap();
        let mut restarted = ConnectorRegistry::load(dir.path()).unwrap();
        assert_eq!(restarted.installation_identity(), Some(&installation));
        assert!(!restarted.key_cleanup().contains(&installation.handle));
        restarted
            .register_prepared_key(
                second.clone(),
                [37; 32],
                Utc::now().timestamp_millis() + 60_000,
            )
            .unwrap();
        let mut second_record = record("ncon_second");
        second_record.device_key_file = None;
        second_record.secure_key_handle = Some(second);
        restarted.complete_prepared(second_record).unwrap();
        assert_eq!(
            ConnectorRegistry::load(dir.path())
                .unwrap()
                .installation_identity(),
            Some(&installation)
        );
    }
}
