//! Minimal reusable runtime composition and lifecycle entry point.

use crate::{
    HostLifecycle, InstallationGuard, LifecycleEvent, LifecycleSnapshot, RuntimeConfiguration,
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
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::{
        AvailabilityClass, HostCapabilities, HostKind, NetworkAvailability, NodeRuntimeMode,
        PrinterTransport,
    };

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
}
