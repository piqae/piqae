//! Minimal reusable runtime composition and lifecycle entry point.

use crate::{
    HostKeyError, HostKeyProvider, HostLifecycle, InstallationGuard, LifecycleEvent,
    LifecycleSnapshot, RuntimeConfiguration,
};
use anyhow::{Context as _, Result};
use std::sync::{Arc, Mutex};

/// Owns exclusive installation access and the host lifecycle state. Queue,
/// connector and driver services are composed around this foundation by both
/// the packaged agent and embedded SDK hosts.
pub struct NodeRuntime {
    configuration: RuntimeConfiguration,
    lifecycle: Arc<Mutex<HostLifecycle>>,
    _installation: InstallationGuard,
}

impl std::fmt::Debug for NodeRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NodeRuntime")
            .field("configuration", &self.configuration)
            .field("lifecycle", &self.snapshot())
            .finish_non_exhaustive()
    }
}

impl NodeRuntime {
    /// Starts an installation-scoped runtime foundation.
    ///
    /// # Errors
    ///
    /// Returns an error if another runtime owns the state root.
    pub fn start(configuration: RuntimeConfiguration) -> Result<Self> {
        let installation = InstallationGuard::acquire(&configuration.data_directory)
            .context("acquire exclusive node installation ownership")?;
        let lifecycle = HostLifecycle::new(configuration.mode, &configuration.host);
        Ok(Self {
            configuration,
            lifecycle: Arc::new(Mutex::new(lifecycle)),
            _installation: installation,
        })
    }

    #[must_use]
    pub const fn configuration(&self) -> &RuntimeConfiguration {
        &self.configuration
    }

    #[must_use]
    pub fn apply_lifecycle(&self, event: LifecycleEvent) -> LifecycleSnapshot {
        match self.lifecycle.lock() {
            Ok(mut lifecycle) => lifecycle.apply(event),
            Err(poisoned) => poisoned.into_inner().apply(event),
        }
    }

    #[must_use]
    pub fn snapshot(&self) -> LifecycleSnapshot {
        match self.lifecycle.lock() {
            Ok(lifecycle) => lifecycle.snapshot(),
            Err(poisoned) => poisoned.into_inner().snapshot(),
        }
    }

    /// Derives non-reversible physical-destination evidence without exporting
    /// the installation key or canonical printer endpoint.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid bounded namespace/canonical identity or
    /// when the host secure key provider is unavailable.
    pub fn opaque_evidence(
        &self,
        provider: &dyn HostKeyProvider,
        namespace: &str,
        canonical_identity: &[u8],
    ) -> Result<String, HostKeyError> {
        if namespace.is_empty()
            || namespace.len() > 64
            || !namespace
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
            || canonical_identity.is_empty()
            || canonical_identity.len() > 4096
        {
            return Err(HostKeyError::InvalidKeyMaterial);
        }
        let mut message = Vec::with_capacity(namespace.len() + canonical_identity.len() + 32);
        message.extend_from_slice(b"piqae-opaque-evidence-v1\0");
        message.extend_from_slice(namespace.as_bytes());
        message.push(0);
        message.extend_from_slice(canonical_identity);
        let digest = provider.hmac_sha256("physical-destination-v1", &message)?;
        Ok(format!("pid_{}", hex::encode(digest)))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::{
        AvailabilityClass, HostCapabilities, HostKind, NetworkAvailability, NodeRuntimeMode,
        PrinterTransport,
    };
    use sha2::Digest as _;
    use std::sync::Mutex;

    #[derive(Debug, Default)]
    struct RecordingKeyProvider {
        calls: Mutex<Vec<(String, Vec<u8>)>>,
        unavailable: bool,
    }

    impl HostKeyProvider for RecordingKeyProvider {
        fn hmac_sha256(&self, key_scope: &str, message: &[u8]) -> Result<[u8; 32], HostKeyError> {
            if self.unavailable {
                return Err(HostKeyError::Unavailable);
            }
            self.calls
                .lock()
                .unwrap()
                .push((key_scope.to_owned(), message.to_vec()));
            let digest = sha2::Sha256::digest(message);
            Ok(digest.into())
        }
    }

    #[test]
    fn runtime_owns_one_state_root_and_tracks_host_lifecycle() {
        let directory = tempfile::tempdir().unwrap();
        let configuration = RuntimeConfiguration {
            data_directory: directory.path().to_path_buf(),
            mode: NodeRuntimeMode::LocalOnly,
            host: HostCapabilities {
                host_kind: HostKind::EmbeddedApplication,
                availability: AvailabilityClass::ForegroundOnly,
                secure_storage: true,
                local_ipc_broker: false,
                can_prevent_idle_sleep_during_handoff: false,
                can_receive_remote_wake_hint: false,
                printer_transports: std::iter::once(PrinterTransport::Fake).collect(),
            },
        };
        let runtime = NodeRuntime::start(configuration.clone()).unwrap();
        assert!(NodeRuntime::start(configuration).is_err());
        let _ = runtime.apply_lifecycle(LifecycleEvent::NetworkUnavailable);
        assert_eq!(runtime.snapshot().network, NetworkAvailability::Unavailable);
    }

    #[test]
    fn opaque_evidence_is_domain_separated_and_fails_closed_without_a_host_key() {
        let directory = tempfile::tempdir().unwrap();
        let runtime = NodeRuntime::start(RuntimeConfiguration {
            data_directory: directory.path().to_path_buf(),
            mode: NodeRuntimeMode::LocalOnly,
            host: HostCapabilities {
                host_kind: HostKind::EmbeddedApplication,
                availability: AvailabilityClass::ForegroundOnly,
                secure_storage: true,
                local_ipc_broker: false,
                can_prevent_idle_sleep_during_handoff: false,
                can_receive_remote_wake_hint: false,
                printer_transports: std::collections::BTreeSet::default(),
            },
        })
        .unwrap();
        let provider = RecordingKeyProvider::default();
        let first = runtime
            .opaque_evidence(&provider, "airprint", b"ipps://printer/ipp/print")
            .unwrap();
        let second = runtime
            .opaque_evidence(&provider, "ble", b"ipps://printer/ipp/print")
            .unwrap();
        assert_ne!(first, second);
        let calls = provider.calls.lock().unwrap();
        assert!(
            calls
                .iter()
                .all(|(scope, _)| scope == "physical-destination-v1")
        );
        assert!(
            calls[0]
                .1
                .starts_with(b"piqae-opaque-evidence-v1\0airprint\0")
        );
        drop(calls);
        let unavailable = RecordingKeyProvider {
            unavailable: true,
            ..Default::default()
        };
        assert!(matches!(
            runtime.opaque_evidence(&unavailable, "airprint", b"canonical"),
            Err(HostKeyError::Unavailable)
        ));
    }
}
