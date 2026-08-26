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

use piqae_node_runtime::{
    AvailabilityClass, HostCapabilities, HostKeyError, HostKeyProvider, HostKind, LifecycleEvent,
    NodeRuntime, NodeRuntimeMode, PrinterTransport, RuntimeConfiguration,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};
use thiserror::Error;

pub const NODE_ABI_VERSION: u16 = 1;
pub const NODE_CONTRACT_VERSION: u16 = 1;
const MAX_ABI_INPUT_BYTES: usize = 1024 * 1024;
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
pub enum NativeCommand {
    Snapshot,
    ApplyLifecycle {
        event: LifecycleEvent,
    },
    DeriveOpaqueEvidence {
        namespace: String,
        canonical_identity: String,
    },
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
            },
        );
        Ok(json!({ "handle": handle }))
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
                data_directory: root,
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
pub extern "C" fn piqae_node_command(handle: u64, data: *const u8, length: usize) -> PiqaeBuffer {
    ffi_entry(|| {
        let bytes = input_bytes(data, length)?;
        let command =
            serde_json::from_slice::<NativeCommand>(bytes).map_err(|_| FfiError::InvalidCommand)?;
        let (runtime, provider) = {
            let instances = lock_instances()?;
            let instance = instances.get(&handle).ok_or(FfiError::InvalidHandle)?;
            let runtime = instance.runtime.clone().ok_or(FfiError::NotStarted)?;
            let provider = instance.host_key_provider;
            drop(instances);
            (runtime, provider)
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
        let removed = lock_instances()?.remove(&handle).is_some();
        if !removed {
            return Err(FfiError::InvalidHandle);
        }
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
            Self::Internal => "the runtime operation failed",
        }
    }
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
}
