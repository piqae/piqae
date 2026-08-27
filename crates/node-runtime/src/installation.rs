//! Single-installation ownership and attach-versus-embed selection.

use fs2::FileExt as _;
use piqae_local_ipc::{BROKER_PROTOCOL_MIN_VERSION, BROKER_PROTOCOL_VERSION};
use serde::{Deserialize, Serialize};
use std::{fs::OpenOptions, path::PathBuf};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachPolicy {
    /// Attach when a compatible broker is present, otherwise embed.
    Automatic,
    /// A broker is required; never create app-scoped state.
    Attach,
    /// Use app-scoped state even when an installed broker is present.
    Embedded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerEndpoint {
    pub address: String,
    pub protocol_min: u16,
    pub protocol_max: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeDisposition {
    Attached(BrokerEndpoint),
    Embedded,
}

#[derive(Debug, Error)]
pub enum RuntimeSelectionError {
    #[error("an installed Piqae node is required but no compatible local broker was found")]
    BrokerRequired,
    #[error("embedded mode requires an application-scoped data directory")]
    EmbeddedDataDirectoryRequired,
    #[error("local broker protocol range {minimum}..={maximum} is incompatible")]
    IncompatibleBrokerProtocol { minimum: u16, maximum: u16 },
}

/// Makes mode selection explicit before any database or private key is opened.
/// A presence probe is non-sensitive: it may reveal supported protocol ranges,
/// but no tenant, connector or printer data.
///
/// # Errors
///
/// Returns a typed error when the selected policy cannot be satisfied safely.
pub fn select_runtime(
    policy: AttachPolicy,
    broker: Option<BrokerEndpoint>,
    embedded_data_directory: Option<&std::path::Path>,
) -> Result<RuntimeDisposition, RuntimeSelectionError> {
    match (policy, broker) {
        (AttachPolicy::Automatic | AttachPolicy::Attach, Some(endpoint))
            if broker_is_compatible(&endpoint) =>
        {
            Ok(RuntimeDisposition::Attached(endpoint))
        }
        (AttachPolicy::Attach, Some(endpoint)) => {
            Err(RuntimeSelectionError::IncompatibleBrokerProtocol {
                minimum: endpoint.protocol_min,
                maximum: endpoint.protocol_max,
            })
        }
        (AttachPolicy::Automatic, Some(_)) => embedded_data_directory
            .map(|_| RuntimeDisposition::Embedded)
            .ok_or(RuntimeSelectionError::EmbeddedDataDirectoryRequired),
        (AttachPolicy::Attach, None) => Err(RuntimeSelectionError::BrokerRequired),
        (AttachPolicy::Automatic | AttachPolicy::Embedded, _) => embedded_data_directory
            .map(|_| RuntimeDisposition::Embedded)
            .ok_or(RuntimeSelectionError::EmbeddedDataDirectoryRequired),
    }
}

const fn broker_is_compatible(endpoint: &BrokerEndpoint) -> bool {
    endpoint.protocol_min <= endpoint.protocol_max
        && endpoint.protocol_max >= BROKER_PROTOCOL_MIN_VERSION
        && endpoint.protocol_min <= BROKER_PROTOCOL_VERSION
}

#[derive(Debug, Error)]
pub enum InstallationLockError {
    #[error("create installation state directory failed: {0}")]
    CreateDirectory(#[source] std::io::Error),
    #[error("open installation lock failed: {0}")]
    Open(#[source] std::io::Error),
    #[error("nodeAlreadyRunning: another runtime owns this installation state root")]
    AlreadyRunning,
}

/// Process-lifetime exclusive ownership of one state root. Dropping the guard
/// releases the OS lock, including after a crash. The lock file itself remains
/// and contains no secret material.
pub struct InstallationGuard {
    file: std::fs::File,
    path: PathBuf,
}

impl std::fmt::Debug for InstallationGuard {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("InstallationGuard")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl InstallationGuard {
    /// Acquires process-lifetime ownership of a state root.
    ///
    /// # Errors
    ///
    /// Returns `AlreadyRunning` for an occupied root and preserves all state.
    pub fn acquire(root: impl AsRef<std::path::Path>) -> Result<Self, InstallationLockError> {
        let root = root.as_ref();
        std::fs::create_dir_all(root).map_err(InstallationLockError::CreateDirectory)?;
        let path = root.join("node-runtime.lock");
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt as _;
            // Windows byte-range locks are process scoped, so a second handle
            // in this process can otherwise appear to acquire the same lock.
            // Denying handle sharing makes ownership exclusive across handles
            // and processes; Windows releases it automatically after a crash.
            options.share_mode(0);
        }
        let file = options.open(&path).map_err(|error| {
            #[cfg(windows)]
            if error
                .raw_os_error()
                .and_then(|code| u32::try_from(code).ok())
                == Some(windows_sys::Win32::Foundation::ERROR_SHARING_VIOLATION)
            {
                return InstallationLockError::AlreadyRunning;
            }
            InstallationLockError::Open(error)
        })?;
        file.try_lock_exclusive().map_err(|error| {
            if error.kind() == std::io::ErrorKind::WouldBlock {
                InstallationLockError::AlreadyRunning
            } else {
                InstallationLockError::Open(error)
            }
        })?;
        Ok(Self { file, path })
    }

    #[must_use]
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl Drop for InstallationGuard {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.file);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn automatic_prefers_a_compatible_broker() {
        let endpoint = BrokerEndpoint {
            address: "/tmp/piqae.sock".into(),
            protocol_min: 1,
            protocol_max: 1,
        };
        assert_eq!(
            select_runtime(AttachPolicy::Automatic, Some(endpoint.clone()), None).unwrap(),
            RuntimeDisposition::Attached(endpoint)
        );
    }

    #[test]
    fn embedded_requires_an_explicit_app_scoped_root() {
        assert!(matches!(
            select_runtime(AttachPolicy::Embedded, None, None),
            Err(RuntimeSelectionError::EmbeddedDataDirectoryRequired)
        ));
    }

    #[test]
    fn incompatible_broker_falls_back_only_for_automatic_policy() {
        let endpoint = BrokerEndpoint {
            address: "test".into(),
            protocol_min: 99,
            protocol_max: 100,
        };
        assert!(matches!(
            select_runtime(
                AttachPolicy::Automatic,
                Some(endpoint.clone()),
                Some(std::path::Path::new("app"))
            ),
            Ok(RuntimeDisposition::Embedded)
        ));
        assert!(matches!(
            select_runtime(AttachPolicy::Attach, Some(endpoint), None),
            Err(RuntimeSelectionError::IncompatibleBrokerProtocol { .. })
        ));
    }

    #[test]
    fn one_state_root_has_one_runtime_owner() {
        let directory = tempfile::tempdir().unwrap();
        let first = InstallationGuard::acquire(directory.path()).unwrap();
        assert!(matches!(
            InstallationGuard::acquire(directory.path()),
            Err(InstallationLockError::AlreadyRunning)
        ));
        drop(first);
        InstallationGuard::acquire(directory.path()).unwrap();
    }
}
