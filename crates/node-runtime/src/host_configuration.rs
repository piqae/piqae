//! Durable, portable host identity and connection policy.
//!
//! The document is display/configuration metadata only. Runtime keys,
//! connector credentials, printer observations and user account details are
//! deliberately excluded.

use crate::durable_file::replace_json;
use anyhow::{Context as _, Result, bail};
use piqae_node_host_api::{HostConfiguration, NodeIdentity};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const DOCUMENT_VERSION: u16 = 1;
const MAX_DOCUMENT_BYTES: u64 = 32 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HostConfigurationDocument {
    version: u16,
    revision: u64,
    configuration: HostConfiguration,
}

#[derive(Debug)]
pub struct HostConfigurationStore {
    path: PathBuf,
    document: HostConfigurationDocument,
}

impl HostConfigurationStore {
    /// Opens an existing durable host configuration without creating a new
    /// identity or policy.
    ///
    /// # Errors
    ///
    /// Fails closed when the document is absent, malformed, oversized,
    /// unsupported, or invalid.
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let path = root.as_ref().join("node-host.json");
        let bytes = read_bounded(&path).with_context(|| format!("read {}", path.display()))?;
        let document: HostConfigurationDocument =
            serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))?;
        validate_document(&document)?;
        Ok(Self { path, document })
    }

    /// Opens a durable host configuration or creates it exactly once from the
    /// caller's privacy-safe platform default.
    ///
    /// # Errors
    ///
    /// Fails closed for malformed, oversized, unsupported, or invalid state.
    pub fn open_or_create(root: impl AsRef<Path>, initial: HostConfiguration) -> Result<Self> {
        initial
            .validate()
            .context("validate initial node host configuration")?;
        let path = root.as_ref().join("node-host.json");
        match read_bounded(&path) {
            Ok(_) => Self::open(root),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let store = Self {
                    path,
                    document: HostConfigurationDocument {
                        version: DOCUMENT_VERSION,
                        revision: 1,
                        configuration: initial,
                    },
                };
                store.persist()?;
                Ok(store)
            }
            Err(error) => Err(error).with_context(|| format!("read {}", path.display())),
        }
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.document.revision
    }

    #[must_use]
    pub const fn configuration(&self) -> &HostConfiguration {
        &self.document.configuration
    }

    /// Returns one internally consistent revision/configuration snapshot for
    /// SDK and connector reconciliation tasks.
    #[must_use]
    pub fn snapshot(&self) -> (u64, HostConfiguration) {
        (self.document.revision, self.document.configuration.clone())
    }

    /// Replaces only operator-visible identity metadata with optimistic
    /// concurrency. Connection and installed-host policies cannot be widened
    /// through this method.
    ///
    /// # Errors
    ///
    /// Returns an error for stale revisions, invalid metadata, or a failed
    /// durable replacement. The in-memory value changes only after persistence.
    pub fn update_identity(
        &mut self,
        expected_revision: u64,
        identity: NodeIdentity,
    ) -> Result<u64> {
        if expected_revision != self.document.revision {
            bail!("node host configuration revision conflict");
        }
        identity.validate().context("validate node identity")?;
        let revision = self
            .document
            .revision
            .checked_add(1)
            .context("node host configuration revision exhausted")?;
        let mut next = self.document.clone();
        next.revision = revision;
        next.configuration.identity = identity;
        persist_document(&self.path, &next)?;
        self.document = next;
        Ok(revision)
    }

    fn persist(&self) -> Result<()> {
        persist_document(&self.path, &self.document)
    }
}

fn validate_document(document: &HostConfigurationDocument) -> Result<()> {
    if document.version != DOCUMENT_VERSION || document.revision == 0 {
        bail!("unsupported node host configuration document");
    }
    document
        .configuration
        .validate()
        .context("validate node host configuration")
}

fn persist_document(path: &Path, document: &HostConfigurationDocument) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(document)?;
    if bytes.len() as u64 > MAX_DOCUMENT_BYTES {
        bail!("node host configuration exceeds supported bounds");
    }
    replace_json(path, &bytes).with_context(|| format!("persist {}", path.display()))
}

fn read_bounded(path: &Path) -> std::io::Result<Vec<u8>> {
    use std::io::Read as _;
    let file = std::fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.take(MAX_DOCUMENT_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_DOCUMENT_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "node host configuration exceeds supported bounds",
        ));
    }
    Ok(bytes)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use piqae_node_host_api::NodeIdentity;

    fn initial() -> HostConfiguration {
        HostConfiguration::standalone(
            NodeIdentity::new("Warehouse Mac", None, None, Vec::new()).unwrap(),
        )
    }

    #[test]
    fn identity_update_survives_restart_without_changing_policy() {
        let root = tempfile::tempdir().unwrap();
        let mut store = HostConfigurationStore::open_or_create(root.path(), initial()).unwrap();
        let revision = store
            .update_identity(
                1,
                NodeIdentity::new(
                    "Dispatch Mac mini",
                    Some("Main warehouse".into()),
                    Some("Dispatch desk".into()),
                    vec!["shipping".into()],
                )
                .unwrap(),
            )
            .unwrap();
        assert_eq!(revision, 2);
        drop(store);

        let reopened = HostConfigurationStore::open_or_create(root.path(), initial()).unwrap();
        assert_eq!(reopened.revision(), 2);
        assert_eq!(
            reopened.configuration().identity.display_name,
            "Dispatch Mac mini"
        );
        assert!(reopened.configuration().connection_policy.allows_multiple);
    }

    #[test]
    fn stale_identity_edit_cannot_overwrite_a_newer_one() {
        let root = tempfile::tempdir().unwrap();
        let mut store = HostConfigurationStore::open_or_create(root.path(), initial()).unwrap();
        store
            .update_identity(
                1,
                NodeIdentity::new("First", None, None, Vec::new()).unwrap(),
            )
            .unwrap();
        assert!(
            store
                .update_identity(
                    1,
                    NodeIdentity::new("Stale", None, None, Vec::new()).unwrap(),
                )
                .is_err()
        );
        assert_eq!(store.configuration().identity.display_name, "First");
    }

    #[test]
    fn malformed_existing_state_fails_closed() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("node-host.json"), b"not json").unwrap();
        assert!(HostConfigurationStore::open_or_create(root.path(), initial()).is_err());
    }
}
