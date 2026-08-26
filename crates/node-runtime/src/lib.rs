//! Reusable durable node runtime contracts.
//!
//! This crate is the single composition boundary shared by the packaged node,
//! desktop application SDKs and constrained mobile hosts. It deliberately does
//! not own application UI or operating-system driver behaviour. Hosts declare
//! capabilities and deliver lifecycle events; the runtime remains responsible
//! for durable identity, connector isolation, queue admission and handoff
//! fencing.

pub mod broker;
pub mod command;
pub mod connector_registry;
mod durable_file;
pub mod embedded;
pub mod installation;
pub mod route_coordinator;
pub mod runtime;
pub mod supervision;

pub use broker::{
    ApplicationAuthorization, ApplicationCapabilities, ApplicationIdentity, BrokerConsentHandle,
    BrokerRegistry, BrokerServerState, BrokerToken,
};
pub use command::*;
pub use embedded::*;
pub use installation::{
    AttachPolicy, BrokerEndpoint, InstallationGuard, InstallationLockError, RuntimeDisposition,
    RuntimeSelectionError, select_runtime,
};
pub use piqae_node_host_api::{
    AvailabilityClass, HostCapabilities, HostKeyError, HostKeyProvider, HostKind, HostLifecycle,
    LeaseAdmission, LifecycleEvent, LifecycleSnapshot, NetworkAvailability, NodeRuntimeMode,
    PowerAvailability, PrinterTransport, RuntimeConfiguration,
};
pub use runtime::NodeRuntime;
pub use supervision::{ConnectorReconciliation, WorkerObservation, plan_connector_reconciliation};
