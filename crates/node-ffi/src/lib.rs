//! Allocator-neutral, panic-contained native ABI for generated Swift/.NET SDKs.
//!
//! Rust layouts and allocators never cross the boundary. Every operation
//! returns a bounded UTF-8 JSON envelope in a `PiqaeBuffer`; callers release it
//! with `piqae_node_free`. Opaque integer handles index process-local runtime
//! instances and contain no pointer or identity material.

#![allow(
    unsafe_code,
    reason = "isolated C ABI pointer validation and paired buffer deallocation"
)]

use base64::Engine as _;
use piqae_node_runtime::connector_registry::{ConnectorRecord, ConnectorRegistry};
use piqae_node_runtime::{
    AdapterOperationOutcome, AvailabilityClass, ConnectorKeyError, EmbeddedAdapterRegistration,
    EmbeddedJobRequest, EmbeddedPrinterObservation, EmbeddedQueue, GeneratedConnectorKey,
    HostCapabilities, HostKeyError, HostKeyProvider, HostKind, LifecycleEvent, NodeRuntime,
    NodeRuntimeMode, PrinterTransport, RuntimeConfiguration, SecureConnectorSigner,
    SecureKeyHandle,
};
use piqae_support_packs::SupportPackRegistry;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path},
    sync::{
        Condvar, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};
use thiserror::Error;

pub const NODE_ABI_VERSION: u16 = 1;
pub const NODE_CONTRACT_VERSION: u16 = 1;
const MAX_ABI_INPUT_BYTES: usize = 24 * 1024 * 1024;
const MAX_APPLICATION_ID_BYTES: usize = 255;
const MAX_DATA_DIRECTORY_BYTES: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct PiqaeNodeAbiDescriptor {
    pub abi_version: u16,
    pub contract_min: u16,
    pub contract_max: u16,
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct PiqaeBuffer {
    pub data: *mut u8,
    pub length: usize,
}

pub type PiqaeHmacSha256Callback = unsafe extern "C" fn(
    context: *mut core::ffi::c_void,
    key_scope: *const u8,
    key_scope_length: usize,
    message: *const u8,
    message_length: usize,
    output: *mut u8,
    output_length: usize,
) -> i32;

#[derive(Clone, Copy)]
#[repr(C)]
pub struct PiqaeHostKeyProvider {
    pub context: *mut core::ffi::c_void,
    pub hmac_sha256: Option<PiqaeHmacSha256Callback>,
}

pub type PiqaeGenerateConnectorKeyCallback = unsafe extern "C" fn(
    *mut core::ffi::c_void,
    *const u8,
    usize,
    *mut u8,
    usize,
    *mut usize,
    *mut u8,
    usize,
) -> i32;
pub type PiqaeSignConnectorCallback = unsafe extern "C" fn(
    *mut core::ffi::c_void,
    *const u8,
    usize,
    *const u8,
    usize,
    *mut u8,
    usize,
) -> i32;
pub type PiqaeDeleteConnectorKeyCallback =
    unsafe extern "C" fn(*mut core::ffi::c_void, *const u8, usize) -> i32;

#[derive(Clone, Copy)]
#[repr(C)]
pub struct PiqaeConnectorKeyProvider {
    pub context: *mut core::ffi::c_void,
    pub generate: Option<PiqaeGenerateConnectorKeyCallback>,
    pub sign: Option<PiqaeSignConnectorCallback>,
    pub delete: Option<PiqaeDeleteConnectorKeyCallback>,
}

impl std::fmt::Debug for PiqaeConnectorKeyProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PiqaeConnectorKeyProvider(<opaque host context>)")
    }
}
unsafe impl Send for PiqaeConnectorKeyProvider {}
unsafe impl Sync for PiqaeConnectorKeyProvider {}

impl SecureConnectorSigner for PiqaeConnectorKeyProvider {
    fn generate(&self, scope: &str) -> Result<GeneratedConnectorKey, ConnectorKeyError> {
        let callback = self.generate.ok_or(ConnectorKeyError::Unavailable)?;
        let mut handle = [0_u8; 256];
        let mut handle_len = 0_usize;
        let mut public = [0_u8; 32];
        let status = unsafe {
            callback(
                self.context,
                scope.as_ptr(),
                scope.len(),
                handle.as_mut_ptr(),
                handle.len(),
                &raw mut handle_len,
                public.as_mut_ptr(),
                public.len(),
            )
        };
        if status != 0 || handle_len > handle.len() {
            return Err(ConnectorKeyError::Unavailable);
        }
        let value = String::from_utf8(handle[..handle_len].to_vec())
            .map_err(|_| ConnectorKeyError::InvalidKeyMaterial)?;
        Ok(GeneratedConnectorKey {
            handle: SecureKeyHandle::new(value)?,
            public_key: public,
        })
    }
    fn sign(
        &self,
        handle: &SecureKeyHandle,
        message: &[u8],
    ) -> Result<[u8; 64], ConnectorKeyError> {
        let callback = self.sign.ok_or(ConnectorKeyError::Unavailable)?;
        let mut output = [0_u8; 64];
        let status = unsafe {
            callback(
                self.context,
                handle.as_str().as_ptr(),
                handle.as_str().len(),
                message.as_ptr(),
                message.len(),
                output.as_mut_ptr(),
                output.len(),
            )
        };
        (status == 0)
            .then_some(output)
            .ok_or(ConnectorKeyError::Rejected)
    }
    fn delete(&self, handle: &SecureKeyHandle) -> Result<(), ConnectorKeyError> {
        let callback = self.delete.ok_or(ConnectorKeyError::Unavailable)?;
        let status = unsafe {
            callback(
                self.context,
                handle.as_str().as_ptr(),
                handle.as_str().len(),
            )
        };
        (status == 0)
            .then_some(())
            .ok_or(ConnectorKeyError::Rejected)
    }
}

impl std::fmt::Debug for PiqaeHostKeyProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("PiqaeHostKeyProvider(<opaque host context>)")
    }
}

// SAFETY: the ABI contract requires hosts to keep context alive and make the
// callback thread-safe until the instance is destroyed.
unsafe impl Send for PiqaeHostKeyProvider {}
// SAFETY: same contract as `Send`; calls are synchronous and may be concurrent.
unsafe impl Sync for PiqaeHostKeyProvider {}

impl HostKeyProvider for PiqaeHostKeyProvider {
    fn hmac_sha256(&self, key_scope: &str, message: &[u8]) -> Result<[u8; 32], HostKeyError> {
        let callback = self.hmac_sha256.ok_or(HostKeyError::Unavailable)?;
        let mut output = [0_u8; 32];
        // SAFETY: inputs and output remain valid for the synchronous call. The
        // host must not retain them and writes exactly `output_length` bytes.
        let status = unsafe {
            callback(
                self.context,
                key_scope.as_ptr(),
                key_scope.len(),
                message.as_ptr(),
                message.len(),
                output.as_mut_ptr(),
                output.len(),
            )
        };
        if status == 0 {
            Ok(output)
        } else {
            Err(HostKeyError::Unavailable)
        }
    }
}

#[must_use]
pub const fn abi_descriptor() -> PiqaeNodeAbiDescriptor {
    PiqaeNodeAbiDescriptor {
        abi_version: NODE_ABI_VERSION,
        contract_min: NODE_CONTRACT_VERSION,
        contract_max: NODE_CONTRACT_VERSION,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeRuntimeConfiguration {
    pub contract: u16,
    pub host_mode: HostKind,
    pub availability: AvailabilityClass,
    pub local_only: bool,
    pub application_id: String,
    /// Relative, application-scoped path. SDK facades resolve it below their
    /// private container before calling the ABI.
    pub data_directory: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
#[allow(
    clippy::large_enum_variant,
    reason = "bounded JSON ABI commands intentionally retain a direct generated schema"
)]
pub enum NativeCommand {
    Snapshot,
    ApplyLifecycle {
        event: LifecycleEvent,
    },
    DeriveOpaqueEvidence {
        namespace: String,
        canonical_identity: String,
    },
    PrepareConnectorKey {
        application_scope: String,
    },
    RegisterAdapter {
        registration: EmbeddedAdapterRegistration,
    },
    ObservePrinterInventory {
        adapter_id: String,
        printers: Vec<EmbeddedPrinterObservation>,
    },
    PrinterInventory,
    EnqueueLocalJob {
        adapter_id: String,
        idempotency_key: String,
        printer_id: String,
        title: String,
        content_kind: String,
        content_base64: String,
        #[serde(default = "empty_json_object")]
        options_json: String,
        expires_unix_ms: Option<i64>,
    },
    NextAdapterOperation {
        adapter_id: String,
    },
    BeginAdapterHandoff {
        adapter_id: String,
        operation_id: String,
        fence: String,
    },
    CompleteAdapterOperation {
        adapter_id: String,
        operation_id: String,
        fence: String,
        result: AdapterOperationOutcome,
    },
    JobSnapshot {
        job_id: String,
    },
    ProfileSnapshots {
        printer_id: String,
    },
    CreateProfile {
        printer_id: String,
        name: String,
        is_default: bool,
        options_json: String,
    },
    UpdateProfile {
        printer_id: String,
        profile_id: String,
        expected_revision: u64,
        name: String,
        is_default: bool,
        options_json: String,
    },
    DeleteProfile {
        printer_id: String,
        profile_id: String,
        expected_revision: u64,
    },
    CaptureNativeProfile {
        adapter_id: String,
        printer_id: String,
    },
    CompleteConnectorInvitationExchange {
        invitation_nonce: String,
        expires_unix_ms: i64,
        connector: ConnectorRecord,
    },
    ConnectorSnapshots,
    RevokeConnector {
        connector_id: String,
    },
}

fn empty_json_object() -> String {
    "{}".into()
}

#[derive(Debug, Error)]
pub enum ContractError {
    #[error("SDK contract serialization failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("unsupported SDK contract version {0}")]
    UnsupportedVersion(u16),
    #[error("application id or app-scoped data directory is invalid")]
    InvalidApplicationScope,
}

/// Decodes and strictly validates the versioned native runtime configuration.
///
/// # Errors
///
/// Returns an error for malformed JSON, an unsupported contract version, or
/// an application/data-directory scope that could escape the host container.
pub fn decode_configuration(bytes: &[u8]) -> Result<NativeRuntimeConfiguration, ContractError> {
    let configuration = serde_json::from_slice::<NativeRuntimeConfiguration>(bytes)?;
    if configuration.contract != NODE_CONTRACT_VERSION {
        return Err(ContractError::UnsupportedVersion(configuration.contract));
    }
    if !valid_application_id(&configuration.application_id)
        || !valid_relative_data_directory(&configuration.data_directory)
    {
        return Err(ContractError::InvalidApplicationScope);
    }
    Ok(configuration)
}

fn valid_application_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_APPLICATION_ID_BYTES
        && value.contains('.')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        && !value.starts_with('.')
        && !value.ends_with('.')
        && !value.contains("..")
}

fn valid_relative_data_directory(value: &str) -> bool {
    if value.is_empty()
        || value.len() > MAX_DATA_DIRECTORY_BYTES
        || value.chars().any(char::is_control)
    {
        return false;
    }
    let path = Path::new(value);
    !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(value) if !value.is_empty()))
}

#[derive(Debug)]
struct Instance {
    configuration: NativeRuntimeConfiguration,
    runtime: Option<std::sync::Arc<NodeRuntime>>,
    host_key_provider: Option<PiqaeHostKeyProvider>,
    connector_key_provider: Option<PiqaeConnectorKeyProvider>,
    embedded_queue: Option<std::sync::Arc<Mutex<EmbeddedQueue>>>,
    connector_registry: Option<std::sync::Arc<Mutex<ConnectorRegistry>>>,
    in_flight: std::sync::Arc<InFlight>,
}

#[derive(Debug, Default)]
struct InFlight {
    count: Mutex<usize>,
    idle: Condvar,
}

impl InFlight {
    fn begin(self: &std::sync::Arc<Self>) -> Result<InFlightGuard, FfiError> {
        let mut count = self.count.lock().map_err(|_| FfiError::Internal)?;
        *count = count.checked_add(1).ok_or(FfiError::Internal)?;
        drop(count);
        Ok(InFlightGuard(std::sync::Arc::clone(self)))
    }

    fn wait_until_idle(&self) -> Result<(), FfiError> {
        let mut count = self.count.lock().map_err(|_| FfiError::Internal)?;
        while *count != 0 {
            count = self.idle.wait(count).map_err(|_| FfiError::Internal)?;
        }
        drop(count);
        Ok(())
    }
}

struct InFlightGuard(std::sync::Arc<InFlight>);

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        if let Ok(mut count) = self.0.count.lock() {
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.0.idle.notify_all();
            }
        }
    }
}

fn instances() -> &'static Mutex<BTreeMap<u64, Instance>> {
    static INSTANCES: OnceLock<Mutex<BTreeMap<u64, Instance>>> = OnceLock::new();
    INSTANCES.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn next_handle() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

#[unsafe(no_mangle)]
pub const extern "C" fn piqae_node_abi_descriptor() -> PiqaeNodeAbiDescriptor {
    abi_descriptor()
}

#[unsafe(no_mangle)]
pub extern "C" fn piqae_node_create(data: *const u8, length: usize) -> PiqaeBuffer {
    ffi_entry(|| {
        let bytes = input_bytes(data, length)?;
        let configuration =
            decode_configuration(bytes).map_err(|error| FfiError::contract(&error))?;
        let handle = next_handle();
        lock_instances()?.insert(
            handle,
            Instance {
                configuration,
                runtime: None,
                host_key_provider: None,
                connector_key_provider: None,
                embedded_queue: None,
                connector_registry: None,
                in_flight: std::sync::Arc::new(InFlight::default()),
            },
        );
        Ok(json!({ "handle": handle }))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn piqae_node_set_connector_key_provider(
    handle: u64,
    provider: PiqaeConnectorKeyProvider,
) -> PiqaeBuffer {
    ffi_entry(|| {
        if provider.generate.is_none() || provider.sign.is_none() || provider.delete.is_none() {
            return Err(FfiError::SecureConnectorProviderRequired);
        }
        let mut instances = lock_instances()?;
        let instance = instances.get_mut(&handle).ok_or(FfiError::InvalidHandle)?;
        if instance.runtime.is_some() || instance.connector_key_provider.is_some() {
            return Err(FfiError::ProviderLocked);
        }
        instance.connector_key_provider = Some(provider);
        drop(instances);
        Ok(json!({"handle":handle,"connector_key_provider":"configured"}))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn piqae_node_set_host_key_provider(
    handle: u64,
    provider: PiqaeHostKeyProvider,
) -> PiqaeBuffer {
    ffi_entry(|| {
        if provider.hmac_sha256.is_none() {
            return Err(FfiError::HostKeyUnavailable);
        }
        let mut instances = lock_instances()?;
        let instance = instances.get_mut(&handle).ok_or(FfiError::InvalidHandle)?;
        if instance.runtime.is_some() || instance.host_key_provider.is_some() {
            return Err(FfiError::ProviderLocked);
        }
        instance.host_key_provider = Some(provider);
        drop(instances);
        Ok(json!({ "handle": handle, "host_key_provider": "configured" }))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn piqae_node_start(handle: u64) -> PiqaeBuffer {
    ffi_entry(|| {
        let mut instances = lock_instances()?;
        let instance = instances.get_mut(&handle).ok_or(FfiError::InvalidHandle)?;
        if instance.runtime.is_none() {
            let root = app_scoped_root(&instance.configuration)?;
            let runtime = NodeRuntime::start(RuntimeConfiguration {
                data_directory: root.clone(),
                mode: if instance.configuration.local_only {
                    NodeRuntimeMode::LocalOnly
                } else {
                    NodeRuntimeMode::CloudCapable
                },
                host: HostCapabilities {
                    host_kind: instance.configuration.host_mode,
                    availability: instance.configuration.availability,
                    secure_storage: true,
                    local_ipc_broker: false,
                    can_prevent_idle_sleep_during_handoff: false,
                    can_receive_remote_wake_hint: false,
                    printer_transports: BTreeSet::<PrinterTransport>::new(),
                },
            })
            .map_err(|_| FfiError::StartFailed)?;
            instance.runtime = Some(std::sync::Arc::new(runtime));
            let queue = EmbeddedQueue::open(root.join("embedded"), SupportPackRegistry::default())
                .map_err(|_| FfiError::StartFailed)?;
            instance.embedded_queue = Some(std::sync::Arc::new(Mutex::new(queue)));
            let connectors = ConnectorRegistry::load(root.join("embedded"))
                .map_err(|_| FfiError::StartFailed)?;
            instance.connector_registry = Some(std::sync::Arc::new(Mutex::new(connectors)));
        }
        let snapshot = instance_snapshot(handle, instance);
        drop(instances);
        Ok(snapshot)
    })
}

#[allow(
    clippy::unnecessary_wraps,
    reason = "production platform roots fail closed when their private-container environment is unavailable"
)]
fn app_scoped_root(
    configuration: &NativeRuntimeConfiguration,
) -> Result<std::path::PathBuf, FfiError> {
    #[cfg(test)]
    let base = std::env::temp_dir().join("piqae-node-ffi-tests");
    #[cfg(all(not(test), windows))]
    let base = std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .ok_or(FfiError::StartFailed)?
        .join("Piqae")
        .join("embedded");
    #[cfg(all(not(test), any(target_os = "macos", target_os = "ios")))]
    let base = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .ok_or(FfiError::StartFailed)?
        .join("Library")
        .join("Application Support")
        .join("Piqae")
        .join("embedded");
    #[cfg(all(not(test), unix, not(any(target_os = "macos", target_os = "ios"))))]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(std::path::PathBuf::from)
                .map(|home| home.join(".local").join("share"))
        })
        .ok_or(FfiError::StartFailed)?
        .join("piqae")
        .join("embedded");
    #[cfg(all(not(test), not(any(unix, windows))))]
    let base = return Err(FfiError::StartFailed);
    Ok(base
        .join(&configuration.application_id)
        .join(&configuration.data_directory))
}

#[unsafe(no_mangle)]
pub extern "C" fn piqae_node_stop(handle: u64) -> PiqaeBuffer {
    ffi_entry(|| {
        let mut instances = lock_instances()?;
        let instance = instances.get_mut(&handle).ok_or(FfiError::InvalidHandle)?;
        instance.runtime = None;
        instance.embedded_queue = None;
        instance.connector_registry = None;
        let snapshot = instance_snapshot(handle, instance);
        drop(instances);
        Ok(snapshot)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn piqae_node_snapshot(handle: u64) -> PiqaeBuffer {
    ffi_entry(|| {
        let instances = lock_instances()?;
        let instance = instances.get(&handle).ok_or(FfiError::InvalidHandle)?;
        let snapshot = instance_snapshot(handle, instance);
        drop(instances);
        Ok(snapshot)
    })
}

#[unsafe(no_mangle)]
#[allow(
    clippy::cognitive_complexity,
    clippy::too_many_lines,
    reason = "the exhaustive versioned ABI dispatch stays in one panic-containment boundary"
)]
pub extern "C" fn piqae_node_command(handle: u64, data: *const u8, length: usize) -> PiqaeBuffer {
    ffi_entry(|| {
        let bytes = input_bytes(data, length)?;
        let command =
            serde_json::from_slice::<NativeCommand>(bytes).map_err(|_| FfiError::InvalidCommand)?;
        let (runtime, provider, connector_provider, embedded_queue, connector_registry, _in_flight) = {
            let instances = lock_instances()?;
            let instance = instances.get(&handle).ok_or(FfiError::InvalidHandle)?;
            let runtime = instance.runtime.clone().ok_or(FfiError::NotStarted)?;
            let provider = instance.host_key_provider;
            let connector_provider = instance.connector_key_provider;
            let embedded_queue = instance
                .embedded_queue
                .clone()
                .ok_or(FfiError::NotStarted)?;
            let connector_registry = instance
                .connector_registry
                .clone()
                .ok_or(FfiError::NotStarted)?;
            let in_flight = instance.in_flight.begin()?;
            drop(instances);
            (
                runtime,
                provider,
                connector_provider,
                embedded_queue,
                connector_registry,
                in_flight,
            )
        };
        match command {
            NativeCommand::Snapshot => {}
            NativeCommand::ApplyLifecycle { event } => {
                let _ = runtime.apply_lifecycle(event);
            }
            NativeCommand::DeriveOpaqueEvidence {
                namespace,
                canonical_identity,
            } => {
                let provider = provider.as_ref().ok_or(FfiError::HostKeyUnavailable)?;
                let evidence = runtime
                    .opaque_evidence(provider, &namespace, canonical_identity.as_bytes())
                    .map_err(|_| FfiError::HostKeyUnavailable)?;
                return Ok(json!({ "handle": handle, "opaque_evidence": evidence }));
            }
            NativeCommand::PrepareConnectorKey { application_scope } => {
                let provider = connector_provider
                    .as_ref()
                    .ok_or(FfiError::SecureConnectorProviderRequired)?;
                let generated = provider
                    .generate(&application_scope)
                    .map_err(|_| FfiError::SecureConnectorProviderRequired)?;
                return Ok(
                    json!({"handle":handle,"key_handle":generated.handle,"public_key_base64":base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(generated.public_key)}),
                );
            }
            NativeCommand::RegisterAdapter { registration } => {
                lock_embedded(&embedded_queue)?
                    .register_adapter(registration)
                    .map_err(|_| FfiError::AdapterOperation)?;
                return Ok(json!({ "handle": handle, "registered": true }));
            }
            NativeCommand::ObservePrinterInventory {
                adapter_id,
                printers,
            } => {
                let printers = lock_embedded(&embedded_queue)?
                    .observe_inventory(&adapter_id, &printers)
                    .map_err(|_| FfiError::AdapterOperation)?;
                return Ok(json!({ "handle": handle, "printers": printers }));
            }
            NativeCommand::PrinterInventory => {
                let printers = lock_embedded(&embedded_queue)?
                    .printer_snapshots()
                    .map_err(|_| FfiError::AdapterOperation)?;
                return Ok(json!({ "handle": handle, "printers": printers }));
            }
            NativeCommand::EnqueueLocalJob {
                adapter_id,
                idempotency_key,
                printer_id,
                title,
                content_kind,
                content_base64,
                options_json,
                expires_unix_ms,
            } => {
                let content = base64::engine::general_purpose::STANDARD
                    .decode(content_base64)
                    .map_err(|_| FfiError::InvalidCommand)?;
                let accepted = lock_embedded(&embedded_queue)?
                    .enqueue(EmbeddedJobRequest {
                        adapter_id,
                        idempotency_key,
                        printer_id,
                        title,
                        content_kind,
                        content,
                        options_json,
                        expires_unix_ms,
                    })
                    .map_err(|_| FfiError::AdapterOperation)?;
                return Ok(json!({ "handle": handle, "job": accepted }));
            }
            NativeCommand::NextAdapterOperation { adapter_id } => {
                let operation = lock_embedded(&embedded_queue)?
                    .next_operation(&adapter_id)
                    .map_err(|_| FfiError::AdapterOperation)?;
                return Ok(json!({ "handle": handle, "operation": operation }));
            }
            NativeCommand::BeginAdapterHandoff {
                adapter_id,
                operation_id,
                fence,
            } => {
                let operation = lock_embedded(&embedded_queue)?
                    .begin_handoff(&adapter_id, &operation_id, &fence)
                    .map_err(|_| FfiError::AdapterOperation)?;
                return Ok(json!({ "handle": handle, "operation": operation }));
            }
            NativeCommand::CompleteAdapterOperation {
                adapter_id,
                operation_id,
                fence,
                result,
            } => {
                let acknowledgement = lock_embedded(&embedded_queue)?
                    .complete_operation(&adapter_id, &operation_id, &fence, &result)
                    .map_err(|_| FfiError::AdapterOperation)?;
                return Ok(json!({ "handle": handle, "acknowledgement": acknowledgement }));
            }
            NativeCommand::JobSnapshot { job_id } => {
                let job = lock_embedded(&embedded_queue)?
                    .job(&job_id)
                    .map_err(|_| FfiError::AdapterOperation)?;
                return Ok(json!({ "handle": handle, "job": job }));
            }
            NativeCommand::ProfileSnapshots { printer_id } => {
                let profiles = lock_embedded(&embedded_queue)?
                    .profiles(&printer_id)
                    .map_err(|_| FfiError::AdapterOperation)?;
                return Ok(json!({ "handle": handle, "profiles": profiles }));
            }
            NativeCommand::CreateProfile {
                printer_id,
                name,
                is_default,
                options_json,
            } => {
                let profile = lock_embedded(&embedded_queue)?
                    .create_profile(&printer_id, &name, is_default, &options_json)
                    .map_err(|_| FfiError::AdapterOperation)?;
                return Ok(json!({ "handle": handle, "profile": profile }));
            }
            NativeCommand::UpdateProfile {
                printer_id,
                profile_id,
                expected_revision,
                name,
                is_default,
                options_json,
            } => {
                let profile = lock_embedded(&embedded_queue)?
                    .update_profile(
                        &printer_id,
                        &profile_id,
                        expected_revision,
                        &name,
                        is_default,
                        &options_json,
                    )
                    .map_err(|_| FfiError::AdapterOperation)?;
                return Ok(json!({ "handle": handle, "profile": profile }));
            }
            NativeCommand::DeleteProfile {
                printer_id,
                profile_id,
                expected_revision,
            } => {
                lock_embedded(&embedded_queue)?
                    .delete_profile(&printer_id, &profile_id, expected_revision)
                    .map_err(|_| FfiError::AdapterOperation)?;
                return Ok(json!({ "handle": handle, "deleted": true }));
            }
            NativeCommand::CaptureNativeProfile { .. } => {
                return Err(FfiError::UnsupportedAdapterCapture);
            }
            NativeCommand::CompleteConnectorInvitationExchange {
                invitation_nonce,
                expires_unix_ms,
                connector,
            } => {
                if invitation_nonce.is_empty()
                    || invitation_nonce.len() > 256
                    || expires_unix_ms <= chrono::Utc::now().timestamp_millis()
                {
                    return Err(FfiError::InvalidCommand);
                }
                let provider = connector_provider
                    .as_ref()
                    .ok_or(FfiError::SecureConnectorProviderRequired)?;
                let key = connector
                    .secure_key_handle
                    .as_ref()
                    .ok_or(FfiError::SecureConnectorProviderRequired)?;
                provider
                    .sign(
                        key,
                        format!(
                            "piqae-connector-bind-v1\0{}\0{}",
                            connector.connector_id, invitation_nonce
                        )
                        .as_bytes(),
                    )
                    .map_err(|_| FfiError::SecureConnectorProviderRequired)?;
                lock_connectors(&connector_registry)?
                    .add(connector)
                    .map_err(|_| FfiError::ConnectorOperation)?;
                return Ok(json!({"handle":handle,"connected":true}));
            }
            NativeCommand::ConnectorSnapshots => {
                let connectors = lock_connectors(&connector_registry)?
                    .records()
                    .cloned()
                    .collect::<Vec<_>>();
                return Ok(json!({ "handle": handle, "connectors": connectors }));
            }
            NativeCommand::RevokeConnector { connector_id } => {
                let revoked = lock_connectors(&connector_registry)?
                    .revoke(&connector_id)
                    .map_err(|_| FfiError::ConnectorOperation)?;
                return Ok(json!({ "handle": handle, "revoked": revoked }));
            }
        }
        let instances = lock_instances()?;
        let instance = instances.get(&handle).ok_or(FfiError::InvalidHandle)?;
        let snapshot = instance_snapshot(handle, instance);
        drop(instances);
        Ok(snapshot)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn piqae_node_destroy(handle: u64) -> PiqaeBuffer {
    ffi_entry(|| {
        let removed = lock_instances()?
            .remove(&handle)
            .ok_or(FfiError::InvalidHandle)?;
        removed.in_flight.wait_until_idle()?;
        Ok(json!({ "handle": handle, "destroyed": true }))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn piqae_node_free(buffer: PiqaeBuffer) {
    let _ = std::panic::catch_unwind(|| {
        if buffer.data.is_null() || buffer.length == 0 {
            return;
        }
        // SAFETY: every non-empty buffer returned by this crate was allocated
        // as a boxed slice of exactly `length` bytes. Ownership is transferred
        // back exactly once through this function.
        unsafe {
            drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(
                buffer.data,
                buffer.length,
            )));
        }
    });
}

fn instance_snapshot(handle: u64, instance: &Instance) -> Value {
    json!({
        "handle": handle,
        "started": instance.runtime.is_some(),
        "local_only": instance.configuration.local_only,
        "host_mode": instance.configuration.host_mode,
        "availability": instance.configuration.availability,
        "lifecycle": instance.runtime.as_ref().map(|runtime| runtime.snapshot()),
    })
}

#[derive(Debug)]
enum FfiError {
    InvalidInput,
    InvalidHandle,
    InvalidCommand,
    NotStarted,
    StartFailed,
    HostKeyUnavailable,
    ProviderLocked,
    AdapterOperation,
    UnsupportedAdapterCapture,
    ConnectorOperation,
    SecureConnectorProviderRequired,
    Contract(String),
    Internal,
}

impl FfiError {
    fn contract(error: &ContractError) -> Self {
        match error {
            ContractError::UnsupportedVersion(version) => {
                Self::Contract(format!("unsupported contract version {version}"))
            }
            ContractError::Json(_) | ContractError::InvalidApplicationScope => {
                Self::Contract("invalid runtime configuration".into())
            }
        }
    }

    const fn code(&self) -> &'static str {
        match self {
            Self::InvalidInput => "invalid_input",
            Self::InvalidHandle => "invalid_handle",
            Self::InvalidCommand => "invalid_command",
            Self::NotStarted => "runtime_not_started",
            Self::StartFailed => "runtime_start_failed",
            Self::HostKeyUnavailable => "host_key_unavailable",
            Self::ProviderLocked => "host_key_provider_locked",
            Self::AdapterOperation => "adapter_operation_failed",
            Self::UnsupportedAdapterCapture => "adapter_capture_unsupported",
            Self::ConnectorOperation => "connector_operation_failed",
            Self::SecureConnectorProviderRequired => "secure_connector_provider_required",
            Self::Contract(_) => "invalid_configuration",
            Self::Internal => "internal_error",
        }
    }

    fn safe_message(&self) -> &str {
        match self {
            Self::Contract(message) => message,
            Self::InvalidInput => "the ABI input was null or exceeded its bound",
            Self::InvalidHandle => "the runtime handle is invalid",
            Self::InvalidCommand => "the runtime command is invalid",
            Self::NotStarted => "the runtime has not been started",
            Self::StartFailed => "the runtime could not acquire its application state root",
            Self::HostKeyUnavailable => {
                "the host secure key provider is unavailable or rejected the request"
            }
            Self::ProviderLocked => {
                "the host key provider cannot be replaced after configuration or start"
            }
            Self::AdapterOperation => "the durable adapter operation could not be completed",
            Self::UnsupportedAdapterCapture => {
                "this adapter does not expose a runtime-controlled native profile capture operation"
            }
            Self::ConnectorOperation => "the connector operation could not be completed",
            Self::SecureConnectorProviderRequired => {
                "cloud connector enrollment requires a non-exporting platform signing-key provider"
            }
            Self::Internal => "the runtime operation failed",
        }
    }
}

fn lock_embedded(
    queue: &std::sync::Arc<Mutex<EmbeddedQueue>>,
) -> Result<std::sync::MutexGuard<'_, EmbeddedQueue>, FfiError> {
    queue.lock().map_err(|_| FfiError::Internal)
}

fn lock_connectors(
    registry: &std::sync::Arc<Mutex<ConnectorRegistry>>,
) -> Result<std::sync::MutexGuard<'_, ConnectorRegistry>, FfiError> {
    registry.lock().map_err(|_| FfiError::Internal)
}

fn lock_instances() -> Result<std::sync::MutexGuard<'static, BTreeMap<u64, Instance>>, FfiError> {
    instances().lock().map_err(|_| FfiError::Internal)
}

const fn input_bytes<'a>(data: *const u8, length: usize) -> Result<&'a [u8], FfiError> {
    if data.is_null() || length == 0 || length > MAX_ABI_INPUT_BYTES {
        return Err(FfiError::InvalidInput);
    }
    // SAFETY: the caller promises `length` readable bytes; null and excessive
    // lengths were rejected before constructing the temporary borrowed slice.
    Ok(unsafe { std::slice::from_raw_parts(data, length) })
}

fn ffi_entry(operation: impl FnOnce() -> Result<Value, FfiError>) -> PiqaeBuffer {
    let envelope = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation)) {
        Ok(Ok(data)) => json!({ "ok": true, "data": data }),
        Ok(Err(error)) => json!({
            "ok": false,
            "error": { "code": error.code(), "message": error.safe_message() }
        }),
        Err(_) => json!({
            "ok": false,
            "error": { "code": "panic_contained", "message": "the native runtime operation failed" }
        }),
    };
    let bytes = serde_json::to_vec(&envelope).unwrap_or_else(|_| {
        b"{\"ok\":false,\"error\":{\"code\":\"internal_error\",\"message\":\"the native runtime operation failed\"}}".to_vec()
    });
    let bytes = bytes.into_boxed_slice();
    let length = bytes.len();
    let data = Box::into_raw(bytes).cast::<u8>();
    PiqaeBuffer { data, length }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn unique_fixture() -> Vec<u8> {
        let mut fixture: Value = serde_json::from_slice(include_bytes!(
            "../../../contracts/node-sdk/v1/runtime-configuration.json"
        ))
        .unwrap();
        fixture["data_directory"] = Value::String(format!("test-{}", uuid::Uuid::new_v4()));
        serde_json::to_vec(&fixture).unwrap()
    }

    fn read_and_free(buffer: PiqaeBuffer) -> Value {
        let bytes = unsafe { std::slice::from_raw_parts(buffer.data, buffer.length) };
        let value = serde_json::from_slice(bytes).unwrap();
        piqae_node_free(buffer);
        value
    }

    #[allow(
        clippy::needless_pass_by_value,
        reason = "test call sites construct one-shot JSON command values"
    )]
    fn command(handle: u64, value: Value) -> Value {
        let bytes = serde_json::to_vec(&value).unwrap();
        read_and_free(piqae_node_command(handle, bytes.as_ptr(), bytes.len()))
    }

    #[test]
    fn abi_descriptor_is_fixed_width_and_versioned() {
        assert_eq!(std::mem::size_of::<PiqaeNodeAbiDescriptor>(), 6);
        assert_eq!(piqae_node_abi_descriptor().abi_version, 1);
    }

    #[test]
    fn config_rejects_absolute_parent_traversal_control_and_invalid_app_ids() {
        for directory in ["/tmp/app", "../app", "app/../other", "app\nstate"] {
            let bytes = format!(
                r#"{{"contract":1,"host_mode":"embedded_application","availability":"foreground_only","local_only":true,"application_id":"com.example.pos","data_directory":{}}}"#,
                serde_json::to_string(directory).unwrap()
            );
            assert!(
                decode_configuration(bytes.as_bytes()).is_err(),
                "{directory}"
            );
        }
        let bytes = br#"{"contract":1,"host_mode":"embedded_application","availability":"foreground_only","local_only":true,"application_id":"not valid","data_directory":"app-state"}"#;
        assert!(decode_configuration(bytes).is_err());
    }

    #[test]
    fn ffi_lifecycle_is_instance_scoped_and_errors_are_redacted() {
        let fixture = unique_fixture();
        let created = read_and_free(piqae_node_create(fixture.as_ptr(), fixture.len()));
        let handle = created["data"]["handle"].as_u64().unwrap();
        let started = read_and_free(piqae_node_start(handle));
        assert_eq!(started["data"]["started"], true);
        let command = br#"{"type":"apply_lifecycle","event":"suspend_imminent"}"#;
        let snapshot = read_and_free(piqae_node_command(handle, command.as_ptr(), command.len()));
        assert_eq!(
            snapshot["data"]["lifecycle"]["accepting_cloud_leases"],
            false
        );
        let stopped = read_and_free(piqae_node_stop(handle));
        assert_eq!(stopped["data"]["started"], false);
        let _ = read_and_free(piqae_node_destroy(handle));
        let invalid = read_and_free(piqae_node_snapshot(handle));
        assert_eq!(invalid["error"]["code"], "invalid_handle");
        assert!(!invalid.to_string().contains("app-state"));
    }

    #[test]
    fn panic_is_contained_in_a_bounded_error_envelope() {
        let value = read_and_free(ffi_entry(|| -> Result<Value, FfiError> {
            panic!("secret")
        }));
        assert_eq!(value["error"]["code"], "panic_contained");
        assert!(!value.to_string().contains("secret"));
    }

    unsafe extern "C" fn test_hmac(
        _context: *mut core::ffi::c_void,
        _scope: *const u8,
        _scope_length: usize,
        _message: *const u8,
        _message_length: usize,
        output: *mut u8,
        output_length: usize,
    ) -> i32 {
        if output.is_null() || output_length != 32 {
            return 1;
        }
        // SAFETY: the test caller supplies a live 32-byte output buffer.
        unsafe { std::slice::from_raw_parts_mut(output, output_length).fill(7) };
        0
    }

    #[test]
    fn opaque_evidence_uses_host_callback_and_never_returns_canonical_identity() {
        let fixture = unique_fixture();
        let created = read_and_free(piqae_node_create(fixture.as_ptr(), fixture.len()));
        let handle = created["data"]["handle"].as_u64().unwrap();
        let configured = read_and_free(piqae_node_set_host_key_provider(
            handle,
            PiqaeHostKeyProvider {
                context: std::ptr::null_mut(),
                hmac_sha256: Some(test_hmac),
            },
        ));
        assert_eq!(configured["ok"], true);
        let _ = read_and_free(piqae_node_start(handle));
        let command = br#"{"type":"derive_opaque_evidence","namespace":"airprint","canonical_identity":"ipps://printer.local/ipp/print"}"#;
        let evidence = read_and_free(piqae_node_command(handle, command.as_ptr(), command.len()));
        assert!(
            evidence["data"]["opaque_evidence"]
                .as_str()
                .unwrap()
                .starts_with("pid_")
        );
        assert!(!evidence.to_string().contains("printer.local"));
        let _ = read_and_free(piqae_node_destroy(handle));
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "one end-to-end ABI regression proves the required handoff ordering"
    )]
    fn ffi_embedded_print_is_durable_and_requires_handoff_start() {
        let fixture = unique_fixture();
        let created = read_and_free(piqae_node_create(fixture.as_ptr(), fixture.len()));
        let handle = created["data"]["handle"].as_u64().unwrap();
        assert_eq!(read_and_free(piqae_node_start(handle))["ok"], true);
        assert_eq!(
            command(
                handle,
                json!({
                    "type": "register_adapter",
                    "registration": {
                        "fingerprint": {
                            "platform": "ios_air_print",
                            "adapter_id": "com.example.pos.airprint",
                            "adapter_version": "1.0.0",
                            "device_family": "ipad",
                            "firmware_version": null
                        },
                        "capability_contract": {"document_kinds": ["pdf"]}
                    }
                }),
            )["ok"],
            true
        );
        let inventory = command(
            handle,
            json!({
                "type": "observe_printer_inventory",
                "adapter_id": "com.example.pos.airprint",
                "printers": [{
                    "native_id": "ipps://printer/ipp/print",
                    "name": "Kitchen",
                    "state": "available",
                    "is_default": true,
                    "native_options": {}
                }]
            }),
        );
        let printer_id = inventory["data"]["printers"][0]["printer_id"]
            .as_str()
            .unwrap();
        let accepted = command(
            handle,
            json!({
                "type": "enqueue_local_job",
                "adapter_id": "com.example.pos.airprint",
                "idempotency_key": "order-42",
                "printer_id": printer_id,
                "title": "Order 42",
                "content_kind": "pdf",
                "content_base64": base64::engine::general_purpose::STANDARD.encode(b"%PDF fake"),
                "options_json": "{}",
                "expires_unix_ms": null
            }),
        );
        let job_id = accepted["data"]["job"]["job_id"].as_str().unwrap();
        let next = command(
            handle,
            json!({"type":"next_adapter_operation","adapter_id":"com.example.pos.airprint"}),
        );
        let operation = &next["data"]["operation"];
        let operation_id = operation["operation_id"].as_str().unwrap();
        let fence = operation["fence"].as_str().unwrap();
        let premature = command(
            handle,
            json!({
                "type":"complete_adapter_operation",
                "adapter_id":"com.example.pos.airprint",
                "operation_id":operation_id,
                "fence":fence,
                "result":{"outcome":"accepted","native_job_id":"native-42"}
            }),
        );
        assert_eq!(premature["error"]["code"], "adapter_operation_failed");
        assert_eq!(
            command(
                handle,
                json!({
                    "type":"begin_adapter_handoff",
                    "adapter_id":"com.example.pos.airprint",
                    "operation_id":operation_id,
                    "fence":fence
                }),
            )["data"]["operation"]["phase"],
            "handoff_started"
        );
        for result in [
            json!({"outcome":"accepted","native_job_id":"native-42"}),
            json!({"outcome":"completed_reported","native_job_id":"native-42"}),
        ] {
            assert_eq!(
                command(
                    handle,
                    json!({
                        "type":"complete_adapter_operation",
                        "adapter_id":"com.example.pos.airprint",
                        "operation_id":operation_id,
                        "fence":fence,
                        "result":result
                    }),
                )["ok"],
                true
            );
        }
        let _ = read_and_free(piqae_node_stop(handle));
        let _ = read_and_free(piqae_node_start(handle));
        assert_eq!(
            command(handle, json!({"type":"job_snapshot","job_id":job_id}))["data"]["job"]["state"],
            "completed_reported"
        );
        let _ = read_and_free(piqae_node_destroy(handle));
    }

    #[derive(Debug)]
    struct BlockingHmacContext {
        entered: std::sync::mpsc::Sender<()>,
        release: Mutex<std::sync::mpsc::Receiver<()>>,
    }

    unsafe extern "C" fn blocking_hmac(
        context: *mut core::ffi::c_void,
        _scope: *const u8,
        _scope_length: usize,
        _message: *const u8,
        _message_length: usize,
        output: *mut u8,
        output_length: usize,
    ) -> i32 {
        if context.is_null() || output.is_null() || output_length != 32 {
            return 1;
        }
        // SAFETY: the test retains this boxed context until destroy returns,
        // which is the lifetime rule under test.
        let context = unsafe { &*(context.cast::<BlockingHmacContext>()) };
        let _ = context.entered.send(());
        if context
            .release
            .lock()
            .ok()
            .and_then(|release| release.recv().ok())
            .is_none()
        {
            return 1;
        }
        // SAFETY: the ABI supplied a live exact-size output buffer.
        unsafe { std::slice::from_raw_parts_mut(output, output_length).fill(9) };
        0
    }

    #[test]
    fn destroy_waits_for_host_callback_and_provider_cannot_be_replaced() {
        let fixture = unique_fixture();
        let created = read_and_free(piqae_node_create(fixture.as_ptr(), fixture.len()));
        let handle = created["data"]["handle"].as_u64().unwrap();
        let (entered_send, entered_receive) = std::sync::mpsc::channel();
        let (release_send, release_receive) = std::sync::mpsc::channel();
        let context = Box::into_raw(Box::new(BlockingHmacContext {
            entered: entered_send,
            release: Mutex::new(release_receive),
        }));
        let provider = PiqaeHostKeyProvider {
            context: context.cast(),
            hmac_sha256: Some(blocking_hmac),
        };
        assert_eq!(
            read_and_free(piqae_node_set_host_key_provider(handle, provider))["ok"],
            true
        );
        assert_eq!(
            read_and_free(piqae_node_set_host_key_provider(handle, provider))["error"]["code"],
            "host_key_provider_locked"
        );
        let _ = read_and_free(piqae_node_start(handle));
        let command_thread = std::thread::spawn(move || {
            command(
                handle,
                json!({
                    "type":"derive_opaque_evidence",
                    "namespace":"airprint",
                    "canonical_identity":"ipps://printer/ipp/print"
                }),
            )
        });
        entered_receive
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        let (destroyed_send, destroyed_receive) = std::sync::mpsc::channel();
        let destroy_thread = std::thread::spawn(move || {
            let result = read_and_free(piqae_node_destroy(handle));
            let _ = destroyed_send.send(result);
        });
        assert!(
            destroyed_receive
                .recv_timeout(std::time::Duration::from_millis(50))
                .is_err(),
            "destroy returned while a host callback still held its context"
        );
        release_send.send(()).unwrap();
        assert_eq!(command_thread.join().unwrap()["ok"], true);
        assert_eq!(
            destroyed_receive
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap()["ok"],
            true
        );
        destroy_thread.join().unwrap();
        // SAFETY: destroy has waited for every in-flight callback.
        unsafe { drop(Box::from_raw(context)) };
    }

    #[test]
    fn apple_adapter_platform_names_are_stable_in_abi_json() {
        for platform in [
            "ios_air_print",
            "ios_network",
            "ios_bluetooth_le",
            "ios_external_accessory",
        ] {
            let command: NativeCommand = serde_json::from_value(json!({
                "type":"register_adapter",
                "registration": {
                    "fingerprint": {
                        "platform": platform,
                        "adapter_id":"com.example.adapter",
                        "adapter_version":"1.0.0",
                        "device_family":null,
                        "firmware_version":null
                    },
                    "capability_contract": {}
                }
            }))
            .unwrap();
            assert!(matches!(command, NativeCommand::RegisterAdapter { .. }));
        }
    }

    #[test]
    fn durable_adapter_command_fixtures_pin_the_v1_schema() {
        for fixture in [
            include_bytes!("../../../contracts/node-sdk/v1/adapter-register.json").as_slice(),
            include_bytes!("../../../contracts/node-sdk/v1/adapter-enqueue.json").as_slice(),
            include_bytes!("../../../contracts/node-sdk/v1/adapter-next.json").as_slice(),
            include_bytes!("../../../contracts/node-sdk/v1/adapter-begin-handoff.json").as_slice(),
            include_bytes!("../../../contracts/node-sdk/v1/adapter-complete.json").as_slice(),
        ] {
            serde_json::from_slice::<NativeCommand>(fixture).unwrap();
        }
    }
}
