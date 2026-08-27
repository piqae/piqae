//! Reusable durable node runtime contracts.
//!
//! This crate is the single composition boundary shared by the packaged node,
//! desktop application SDKs and constrained mobile hosts. It deliberately does
//! not own application UI or operating-system driver behaviour. Hosts declare
//! capabilities and deliver lifecycle events; the runtime remains responsible
//! for durable identity, connector isolation, queue admission and handoff
//! fencing.

pub mod broker;
pub mod cloud_worker;
pub mod command;
pub mod command_recovery;
pub mod connector_enrollment;
pub mod connector_registry;
mod durable_file;
pub mod embedded;
pub mod embedded_cloud;
pub mod host_configuration;
pub mod installation;
pub mod route_coordinator;
pub mod runtime;
pub mod secure_connector;
pub mod supervision;

pub use broker::{
    ApplicationAuthorization, ApplicationCapabilities, ApplicationIdentity, BrokerConsentHandle,
    BrokerRegistry, BrokerServerState, BrokerToken,
};
pub use cloud_worker::*;
pub use command::*;
pub use command_recovery::*;
pub use connector_enrollment::*;
pub use embedded::*;
pub use embedded_cloud::*;
pub use host_configuration::HostConfigurationStore;
pub use installation::{
    AttachPolicy, BrokerEndpoint, InstallationGuard, InstallationLockError, InstalledBroker,
    RuntimeDisposition, RuntimeSelectionError, select_host_runtime, select_runtime,
};
pub use piqae_node_host_api::{
    AvailabilityClass, ConnectionManagement, ConnectionPolicy, ConnectorKeyError,
    GeneratedConnectorKey, HostCapabilities, HostConfiguration, HostConfigurationError,
    HostKeyError, HostKeyProvider, HostKind, HostLifecycle, HostProduct, InstalledHostPolicy,
    LeaseAdmission, LifecycleEvent, LifecycleSnapshot, NetworkAvailability, NodeIdentity,
    NodeRuntimeMode, PowerAvailability, PrinterTransport, RuntimeConfiguration,
    SecureConnectorSigner, SecureKeyHandle, default_device_display_name,
};
pub use runtime::NodeRuntime;
pub use secure_connector::{
    CONNECTOR_KEY_SCOPE_PREFIX, HostBackedDeviceIdentity, INSTALLATION_KEY_SCOPE_PREFIX,
    connector_key_scope, installation_key_scope, verify_generated_key,
};
pub use supervision::{ConnectorReconciliation, WorkerObservation, plan_connector_reconciliation};
