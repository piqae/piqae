//! Host capability and lifecycle policy shared by service and embedded nodes.

use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, path::PathBuf};
use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum HostKeyError {
    #[error("the host secure key store is unavailable")]
    Unavailable,
    #[error("the host secure key store returned invalid key material")]
    InvalidKeyMaterial,
}

/// Non-exporting secure-store adapter. Implementations generate and retain the
/// key in Keychain/Credential Manager and return only keyed digest output.
pub trait HostKeyProvider: std::fmt::Debug + Send + Sync {
    /// Produces a keyed digest without exporting the installation key.
    ///
    /// # Errors
    ///
    /// Returns `Unavailable` when the platform secure store cannot safely
    /// load or create the scoped key, and `InvalidKeyMaterial` for a provider
    /// contract violation.
    fn hmac_sha256(&self, key_scope: &str, message: &[u8]) -> Result<[u8; 32], HostKeyError>;
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ConnectorKeyError {
    #[error("the host connector key store is unavailable")]
    Unavailable,
    #[error("the host connector key handle or signature is invalid")]
    InvalidKeyMaterial,
    #[error("the host connector key operation was rejected")]
    Rejected,
}

/// Opaque reference to a non-exportable Ed25519 key held by the host secure
/// store. The identifier is persisted; key material is never returned to Rust.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SecureKeyHandle(String);

impl SecureKeyHandle {
    /// Constructs a bounded provider-owned handle.
    ///
    /// # Errors
    ///
    /// Returns `InvalidKeyMaterial` for an empty, oversized, or unsafe handle.
    pub fn new(value: String) -> Result<Self, ConnectorKeyError> {
        if value.is_empty()
            || value.len() > 256
            || !value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b'/')
            })
        {
            return Err(ConnectorKeyError::InvalidKeyMaterial);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for SecureKeyHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SecureKeyHandle([REDACTED])")
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct GeneratedConnectorKey {
    pub handle: SecureKeyHandle,
    pub public_key: [u8; 32],
}

impl std::fmt::Debug for GeneratedConnectorKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GeneratedConnectorKey")
            .field("handle", &self.handle)
            .field("public_key", &"[PUBLIC KEY]")
            .finish()
    }
}

/// Non-exporting connector identity provider.
///
/// Apple hosts implement this with Keychain/Secure Enclave where supported;
/// Windows hosts use Credential Manager/DPAPI-backed material. Calls may be
/// concurrent and remain valid until all connector workers have stopped.
pub trait SecureConnectorSigner: std::fmt::Debug + Send + Sync {
    /// Generates a new key under an application-scoped label and returns only
    /// an opaque durable handle plus its public verification key.
    ///
    /// # Errors
    ///
    /// Returns a provider error if secure generation or persistence fails.
    fn generate(&self, application_scope: &str)
    -> Result<GeneratedConnectorKey, ConnectorKeyError>;

    /// Signs one bounded canonical request without exporting private bytes.
    ///
    /// # Errors
    ///
    /// Returns a provider error for an absent key or rejected signing request.
    fn sign(&self, handle: &SecureKeyHandle, message: &[u8])
    -> Result<[u8; 64], ConnectorKeyError>;

    /// Deletes a key after connector revocation. Failure must leave the
    /// connector revoked; callers retry secure-store cleanup separately.
    ///
    /// # Errors
    ///
    /// Returns a provider error when secure-store cleanup could not complete.
    fn delete(&self, handle: &SecureKeyHandle) -> Result<(), ConnectorKeyError>;
}

/// Whether this runtime has no remote authority or may hold isolated cloud
/// connectors. `CloudCapable` does not imply that a connector is configured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeRuntimeMode {
    LocalOnly,
    CloudCapable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostKind {
    MachineService,
    UserAgent,
    EmbeddedApplication,
    AttachedClient,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrinterTransport {
    OperatingSystemDriver,
    Ipp,
    AirPrint,
    Usb,
    Bluetooth,
    ExternalAccessory,
    VendorAdapter,
    Fake,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AvailabilityClass {
    /// Process remains eligible whenever the host is awake.
    ContinuousWhileAwake,
    /// Host application controls lifetime and may be terminated at any time.
    ForegroundOnly,
    /// OS may grant bounded execution, but wake and duration are not promised.
    BackgroundOpportunistic,
    /// Supervised, powered foreground deployment validated by the operator.
    ManagedKiosk,
    /// A separately verified LAN relay may request wake. The route still must
    /// publish a fresh authenticated observation before it receives a lease.
    WakeRelayCapable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PowerAvailability {
    Awake,
    Suspending,
    Sleeping,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkAvailability {
    Available,
    Constrained,
    Unavailable,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "orthogonal host capabilities are serialized feature facts, not state"
)]
pub struct HostCapabilities {
    pub host_kind: HostKind,
    pub availability: AvailabilityClass,
    pub secure_storage: bool,
    pub local_ipc_broker: bool,
    pub can_prevent_idle_sleep_during_handoff: bool,
    pub can_receive_remote_wake_hint: bool,
    pub printer_transports: BTreeSet<PrinterTransport>,
}

impl HostCapabilities {
    #[must_use]
    pub const fn unattended_ready(&self) -> bool {
        matches!(
            self.availability,
            AvailabilityClass::ContinuousWhileAwake
                | AvailabilityClass::ManagedKiosk
                | AvailabilityClass::WakeRelayCapable
        ) && self.secure_storage
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeConfiguration {
    pub data_directory: PathBuf,
    pub mode: NodeRuntimeMode,
    pub host: HostCapabilities,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleEvent {
    Started,
    EnteredForeground,
    EnteredBackground,
    SuspendImminent,
    Sleeping,
    Woke,
    NetworkAvailable,
    NetworkConstrained,
    NetworkUnavailable,
    ShutdownRequested,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifecycleSnapshot {
    pub foreground: bool,
    pub power: PowerAvailability,
    pub network: NetworkAvailability,
    pub accepting_cloud_leases: bool,
    pub shutdown_requested: bool,
    pub generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaseAdmission {
    Allowed,
    LocalOnly,
    HostSuspended,
    NetworkUnavailable,
    BackgroundBudgetNotGuaranteed,
    ShutdownRequested,
}

/// Deterministic lifecycle state machine.
///
/// A mobile host must explicitly mark a bounded background execution window as
/// foreground-equivalent before a caller may admit new cloud work; receiving a
/// push hint alone never does so.
#[derive(Debug)]
pub struct HostLifecycle {
    mode: NodeRuntimeMode,
    availability: AvailabilityClass,
    snapshot: LifecycleSnapshot,
}

impl HostLifecycle {
    #[must_use]
    pub const fn new(mode: NodeRuntimeMode, capabilities: &HostCapabilities) -> Self {
        Self {
            mode,
            availability: capabilities.availability,
            snapshot: LifecycleSnapshot {
                foreground: !matches!(
                    capabilities.availability,
                    AvailabilityClass::ForegroundOnly | AvailabilityClass::BackgroundOpportunistic
                ),
                power: PowerAvailability::Awake,
                network: NetworkAvailability::Unknown,
                accepting_cloud_leases: false,
                shutdown_requested: false,
                generation: 0,
            },
        }
    }

    pub fn apply(&mut self, event: LifecycleEvent) -> LifecycleSnapshot {
        match event {
            LifecycleEvent::Started | LifecycleEvent::Woke => {
                self.snapshot.power = PowerAvailability::Awake;
            }
            LifecycleEvent::EnteredForeground => self.snapshot.foreground = true,
            LifecycleEvent::EnteredBackground => self.snapshot.foreground = false,
            LifecycleEvent::SuspendImminent => {
                self.snapshot.power = PowerAvailability::Suspending;
            }
            LifecycleEvent::Sleeping => self.snapshot.power = PowerAvailability::Sleeping,
            LifecycleEvent::NetworkAvailable => {
                self.snapshot.network = NetworkAvailability::Available;
            }
            LifecycleEvent::NetworkConstrained => {
                self.snapshot.network = NetworkAvailability::Constrained;
            }
            LifecycleEvent::NetworkUnavailable => {
                self.snapshot.network = NetworkAvailability::Unavailable;
            }
            LifecycleEvent::ShutdownRequested => self.snapshot.shutdown_requested = true,
        }
        self.snapshot.generation = self.snapshot.generation.saturating_add(1);
        self.snapshot.accepting_cloud_leases = self.lease_admission() == LeaseAdmission::Allowed;
        self.snapshot
    }

    #[must_use]
    pub const fn snapshot(&self) -> LifecycleSnapshot {
        self.snapshot
    }

    #[must_use]
    pub fn lease_admission(&self) -> LeaseAdmission {
        if self.mode == NodeRuntimeMode::LocalOnly {
            return LeaseAdmission::LocalOnly;
        }
        if self.snapshot.shutdown_requested {
            return LeaseAdmission::ShutdownRequested;
        }
        if self.snapshot.power != PowerAvailability::Awake {
            return LeaseAdmission::HostSuspended;
        }
        if self.snapshot.network == NetworkAvailability::Unavailable {
            return LeaseAdmission::NetworkUnavailable;
        }
        if matches!(
            self.availability,
            AvailabilityClass::ForegroundOnly | AvailabilityClass::BackgroundOpportunistic
        ) && !self.snapshot.foreground
        {
            return LeaseAdmission::BackgroundBudgetNotGuaranteed;
        }
        LeaseAdmission::Allowed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mobile() -> HostCapabilities {
        HostCapabilities {
            host_kind: HostKind::EmbeddedApplication,
            availability: AvailabilityClass::BackgroundOpportunistic,
            secure_storage: true,
            local_ipc_broker: false,
            can_prevent_idle_sleep_during_handoff: false,
            can_receive_remote_wake_hint: true,
            printer_transports: std::iter::once(PrinterTransport::AirPrint).collect(),
        }
    }

    #[test]
    fn mobile_push_or_network_does_not_admit_work_while_backgrounded() {
        let mut lifecycle = HostLifecycle::new(NodeRuntimeMode::CloudCapable, &mobile());
        lifecycle.apply(LifecycleEvent::NetworkAvailable);
        assert_eq!(
            lifecycle.lease_admission(),
            LeaseAdmission::BackgroundBudgetNotGuaranteed
        );
        lifecycle.apply(LifecycleEvent::EnteredForeground);
        assert_eq!(lifecycle.lease_admission(), LeaseAdmission::Allowed);
    }

    #[test]
    fn suspend_closes_admission_before_sleep() {
        let mut capabilities = mobile();
        capabilities.host_kind = HostKind::MachineService;
        capabilities.availability = AvailabilityClass::ContinuousWhileAwake;
        let mut lifecycle = HostLifecycle::new(NodeRuntimeMode::CloudCapable, &capabilities);
        lifecycle.apply(LifecycleEvent::NetworkAvailable);
        assert_eq!(lifecycle.lease_admission(), LeaseAdmission::Allowed);
        lifecycle.apply(LifecycleEvent::SuspendImminent);
        assert_eq!(lifecycle.lease_admission(), LeaseAdmission::HostSuspended);
    }

    #[test]
    fn local_only_runtime_never_accepts_cloud_lease() {
        let mut lifecycle = HostLifecycle::new(NodeRuntimeMode::LocalOnly, &mobile());
        lifecycle.apply(LifecycleEvent::EnteredForeground);
        lifecycle.apply(LifecycleEvent::NetworkAvailable);
        assert_eq!(lifecycle.lease_admission(), LeaseAdmission::LocalOnly);
    }
}
