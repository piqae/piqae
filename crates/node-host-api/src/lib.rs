//! Host capability and lifecycle policy shared by service and embedded nodes.

use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, path::PathBuf};
use thiserror::Error;
use url::Url;

pub const HOST_CONFIGURATION_CONTRACT: u16 = 1;
pub const STANDALONE_APPLICATION_ID: &str = "com.piqae.node.desktop";

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum HostConfigurationError {
    #[error("the host configuration contract is unsupported")]
    UnsupportedContract,
    #[error("the host application identifier is invalid")]
    InvalidApplicationId,
    #[error("the node display name is invalid")]
    InvalidDisplayName,
    #[error("the node site is invalid")]
    InvalidSite,
    #[error("the node location is invalid")]
    InvalidLocation,
    #[error("the node labels are invalid")]
    InvalidLabels,
    #[error("standalone nodes require user-managed connections")]
    StandaloneConnectionsMustBeUserManaged,
    #[error("all Piqae hosts must retain multi-connection support")]
    MultipleConnectionsRequired,
    #[error("the allowed authority origin is invalid")]
    InvalidAuthorityOrigin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostProduct {
    Standalone,
    Embedded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstalledHostPolicy {
    /// Attach to an approved, compatible installed node; otherwise use the
    /// application's isolated runtime.
    PreferInstalled,
    /// Refuse to create another runtime when no approved compatible node exists.
    RequireInstalled,
    /// Always use the application's own sandboxed runtime.
    IsolatedApplication,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionManagement {
    UserManaged,
    HostManaged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeIdentity {
    pub display_name: String,
    pub site: Option<String>,
    pub location: Option<String>,
    pub labels: Vec<String>,
}

impl NodeIdentity {
    /// Creates display-only metadata. It must never contain a username,
    /// account address, connector credential, or authorization identity.
    ///
    /// # Errors
    ///
    /// Returns a typed error when a field is empty, unbounded, duplicated, or
    /// contains control characters.
    pub fn new(
        display_name: impl Into<String>,
        site: Option<String>,
        location: Option<String>,
        labels: Vec<String>,
    ) -> Result<Self, HostConfigurationError> {
        let identity = Self {
            display_name: display_name.into().trim().to_owned(),
            site: site.map(|value| value.trim().to_owned()),
            location: location.map(|value| value.trim().to_owned()),
            labels: labels
                .into_iter()
                .map(|label| label.trim().to_owned())
                .collect(),
        };
        identity.validate()?;
        Ok(identity)
    }

    /// Validates the canonical bounded identity representation.
    ///
    /// # Errors
    ///
    /// Returns a field-specific error for non-canonical or unsafe metadata.
    pub fn validate(&self) -> Result<(), HostConfigurationError> {
        validate_display_field(&self.display_name, 120)
            .map_err(|()| HostConfigurationError::InvalidDisplayName)?;
        validate_optional_display_field(self.site.as_deref(), 120)
            .map_err(|()| HostConfigurationError::InvalidSite)?;
        validate_optional_display_field(self.location.as_deref(), 120)
            .map_err(|()| HostConfigurationError::InvalidLocation)?;
        if self.labels.len() > 16
            || self
                .labels
                .iter()
                .any(|label| validate_display_field(label, 64).is_err())
            || self.labels.iter().collect::<BTreeSet<_>>().len() != self.labels.len()
        {
            return Err(HostConfigurationError::InvalidLabels);
        }
        Ok(())
    }
}

/// Returns a privacy-safe default derived only from an operating-system device
/// name supplied by the host. It never reads account/user/address fields.
#[must_use]
pub fn default_device_display_name(device_name: Option<&str>, platform: &str) -> String {
    device_name
        .map(str::trim)
        .filter(|name| validate_display_field(name, 120).is_ok())
        .map_or_else(
            || {
                let platform = platform.trim();
                if validate_display_field(platform, 96).is_ok() {
                    format!("Piqae on {platform}")
                } else {
                    "Piqae node".to_owned()
                }
            },
            ToOwned::to_owned,
        )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionPolicy {
    pub management: ConnectionManagement,
    pub allows_multiple: bool,
    pub allowed_authority_origins: Vec<String>,
}

impl ConnectionPolicy {
    #[must_use]
    pub const fn user_managed() -> Self {
        Self {
            management: ConnectionManagement::UserManaged,
            allows_multiple: true,
            allowed_authority_origins: Vec::new(),
        }
    }

    /// Creates a host-managed policy pinned to one or more exact HTTPS
    /// authority origins.
    ///
    /// # Errors
    ///
    /// Returns an error when the allowlist is empty, duplicated, too large, or
    /// contains anything other than an exact HTTPS origin.
    pub fn host_managed(
        allowed_authority_origins: Vec<String>,
    ) -> Result<Self, HostConfigurationError> {
        let policy = Self {
            management: ConnectionManagement::HostManaged,
            allows_multiple: true,
            allowed_authority_origins,
        };
        policy.validate()?;
        Ok(policy)
    }

    fn validate(&self) -> Result<(), HostConfigurationError> {
        if !self.allows_multiple {
            return Err(HostConfigurationError::MultipleConnectionsRequired);
        }
        if (self.management == ConnectionManagement::HostManaged
            && self.allowed_authority_origins.is_empty())
            || self.allowed_authority_origins.len() > 32
            || self
                .allowed_authority_origins
                .iter()
                .collect::<BTreeSet<_>>()
                .len()
                != self.allowed_authority_origins.len()
            || self
                .allowed_authority_origins
                .iter()
                .any(|origin| !valid_https_origin(origin))
        {
            return Err(HostConfigurationError::InvalidAuthorityOrigin);
        }
        Ok(())
    }

    /// Whether the authority may be used without presenting another host UI.
    /// Empty allowlists intentionally require an explicit user choice.
    #[must_use]
    pub fn allows_authority(&self, authority: &Url) -> bool {
        let origin = authority.origin().ascii_serialization();
        self.allowed_authority_origins
            .iter()
            .any(|allowed| allowed == &origin)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostConfiguration {
    pub contract: u16,
    pub product: HostProduct,
    pub application_id: String,
    pub identity: NodeIdentity,
    pub installed_host_policy: InstalledHostPolicy,
    pub connection_policy: ConnectionPolicy,
}

impl HostConfiguration {
    #[must_use]
    pub fn standalone(identity: NodeIdentity) -> Self {
        Self {
            contract: HOST_CONFIGURATION_CONTRACT,
            product: HostProduct::Standalone,
            application_id: STANDALONE_APPLICATION_ID.to_owned(),
            identity,
            installed_host_policy: InstalledHostPolicy::IsolatedApplication,
            connection_policy: ConnectionPolicy::user_managed(),
        }
    }

    /// Validates the host contract before it can select or open a runtime.
    ///
    /// # Errors
    ///
    /// Returns a typed contract, identity, application, or policy error.
    pub fn validate(&self) -> Result<(), HostConfigurationError> {
        if self.contract != HOST_CONFIGURATION_CONTRACT {
            return Err(HostConfigurationError::UnsupportedContract);
        }
        if !valid_application_id(&self.application_id) {
            return Err(HostConfigurationError::InvalidApplicationId);
        }
        self.identity.validate()?;
        self.connection_policy.validate()?;
        if self.product == HostProduct::Standalone
            && self.connection_policy.management != ConnectionManagement::UserManaged
        {
            return Err(HostConfigurationError::StandaloneConnectionsMustBeUserManaged);
        }
        Ok(())
    }
}

fn validate_display_field(value: &str, maximum: usize) -> Result<(), ()> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.len() > maximum
        || trimmed.chars().any(char::is_control)
        || trimmed != value
    {
        return Err(());
    }
    Ok(())
}

fn validate_optional_display_field(value: Option<&str>, maximum: usize) -> Result<(), ()> {
    value.map_or(Ok(()), |value| validate_display_field(value, maximum))
}

fn valid_application_id(value: &str) -> bool {
    (3..=255).contains(&value.len())
        && value.split('.').all(|segment| {
            !segment.is_empty()
                && segment.len() <= 63
                && segment
                    .bytes()
                    .next()
                    .is_some_and(|byte| byte.is_ascii_alphanumeric())
                && segment
                    .bytes()
                    .next_back()
                    .is_some_and(|byte| byte.is_ascii_alphanumeric())
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

fn valid_https_origin(value: &str) -> bool {
    Url::parse(value).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && matches!(url.path(), "" | "/")
            && url.query().is_none()
            && url.fragment().is_none()
            && url.origin().ascii_serialization() == value.trim_end_matches('/')
    })
}

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
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    fn identity() -> NodeIdentity {
        NodeIdentity::new(
            "Dispatch Mac mini",
            Some("Christchurch".into()),
            Some("Dispatch desk".into()),
            vec!["shipping".into()],
        )
        .expect("valid fixture")
    }

    #[test]
    fn standalone_configuration_is_user_managed_and_multi_connection() {
        let configuration = HostConfiguration::standalone(identity());
        assert_eq!(configuration.product, HostProduct::Standalone);
        assert_eq!(
            configuration.connection_policy.management,
            ConnectionManagement::UserManaged
        );
        assert!(configuration.connection_policy.allows_multiple);
        configuration.validate().expect("valid standalone host");
    }

    #[test]
    fn embedded_configuration_allows_many_host_managed_connections() {
        let configuration = HostConfiguration {
            contract: HOST_CONFIGURATION_CONTRACT,
            product: HostProduct::Embedded,
            application_id: "com.example.production-labels".into(),
            identity: identity(),
            installed_host_policy: InstalledHostPolicy::PreferInstalled,
            connection_policy: ConnectionPolicy {
                management: ConnectionManagement::HostManaged,
                allows_multiple: true,
                allowed_authority_origins: vec!["https://api.piqae.com".into()],
            },
        };
        configuration.validate().expect("valid embedded host");
        assert!(
            configuration
                .connection_policy
                .allows_authority(&Url::parse("https://api.piqae.com/v1").expect("url"))
        );
    }

    #[test]
    fn host_managed_connections_require_a_pinned_https_authority() {
        assert_eq!(
            ConnectionPolicy::host_managed(Vec::new()),
            Err(HostConfigurationError::InvalidAuthorityOrigin)
        );
        let policy = ConnectionPolicy::host_managed(vec!["https://api.piqae.com".into()])
            .expect("valid pinned authority");
        assert_eq!(policy.management, ConnectionManagement::HostManaged);
        assert!(policy.allows_multiple);
    }

    #[test]
    fn no_host_can_disable_multiple_connections() {
        let mut configuration = HostConfiguration::standalone(identity());
        configuration.connection_policy.allows_multiple = false;
        assert_eq!(
            configuration.validate(),
            Err(HostConfigurationError::MultipleConnectionsRequired)
        );
    }

    #[test]
    fn authority_allowlist_is_exact_https_origin_only() {
        for origin in [
            "http://api.piqae.com",
            "https://user@api.piqae.com",
            "https://api.piqae.com/connect",
            "https://api.piqae.com?tenant=1",
        ] {
            let mut configuration = HostConfiguration::standalone(identity());
            configuration.connection_policy.allowed_authority_origins = vec![origin.into()];
            assert_eq!(
                configuration.validate(),
                Err(HostConfigurationError::InvalidAuthorityOrigin),
                "{origin}"
            );
        }
    }

    #[test]
    fn application_ids_use_bounded_dns_label_syntax() {
        for valid in ["app", "com.piqae.pos", "com.piqae.pos-2"] {
            assert!(valid_application_id(valid), "expected valid: {valid}");
        }
        for invalid in [
            "ab",
            ".prefix",
            "suffix.",
            "com..piqae",
            "com.-piqae",
            "com.piqae-",
            "com.piqae_app",
            "com.piqaé.app",
        ] {
            assert!(
                !valid_application_id(invalid),
                "expected invalid: {invalid}"
            );
        }
    }

    #[test]
    fn display_name_default_does_not_consume_user_or_address_data() {
        assert_eq!(
            default_device_display_name(Some("Warehouse Mac mini"), "macOS"),
            "Warehouse Mac mini"
        );
        assert_eq!(
            default_device_display_name(Some("\n"), "macOS"),
            "Piqae on macOS"
        );
    }

    #[test]
    fn typed_identity_constructor_normalizes_then_validates() {
        let identity = NodeIdentity::new(
            " Dispatch Mac ",
            Some(" Warehouse ".into()),
            Some(" Desk 2 ".into()),
            vec![" shipping ".into()],
        )
        .expect("normalized identity");
        assert_eq!(identity.display_name, "Dispatch Mac");
        assert_eq!(identity.site.as_deref(), Some("Warehouse"));
        assert_eq!(identity.location.as_deref(), Some("Desk 2"));
        assert_eq!(identity.labels, ["shipping"]);
    }

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
