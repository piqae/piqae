//! Narrow, versioned native ABI surface for generated Swift and .NET facades.
//!
//! Complex commands and snapshots use the versioned contract envelope rather
//! than exposing Rust layout, strings or allocator ownership across the ABI.

use piqae_node_runtime::{AvailabilityClass, HostKind};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const NODE_ABI_VERSION: u16 = 1;
pub const NODE_CONTRACT_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct PiqaeNodeAbiDescriptor {
    pub abi_version: u16,
    pub contract_min: u16,
    pub contract_max: u16,
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
    pub data_directory: String,
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

/// Decodes and validates the allocator-neutral JSON envelope used by generated
/// native facades before a host creates runtime-owned state.
///
/// # Errors
///
/// Returns a typed error for invalid JSON, versions or application scope.
pub fn decode_configuration(bytes: &[u8]) -> Result<NativeRuntimeConfiguration, ContractError> {
    let configuration = serde_json::from_slice::<NativeRuntimeConfiguration>(bytes)?;
    if configuration.contract != NODE_CONTRACT_VERSION {
        return Err(ContractError::UnsupportedVersion(configuration.contract));
    }
    if configuration.application_id.is_empty()
        || configuration.application_id.len() > 255
        || configuration.data_directory.is_empty()
    {
        return Err(ContractError::InvalidApplicationScope);
    }
    Ok(configuration)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn abi_descriptor_is_fixed_width_and_versioned() {
        assert_eq!(std::mem::size_of::<PiqaeNodeAbiDescriptor>(), 6);
        assert_eq!(abi_descriptor().abi_version, 1);
    }

    #[test]
    fn contract_rejects_unknown_versions_before_state_is_opened() {
        let bytes = br#"{
          "contract": 2,
          "host_mode": "embedded_application",
          "availability": "foreground_only",
          "local_only": true,
          "application_id": "com.example.pos",
          "data_directory": "app-state"
        }"#;
        assert!(matches!(
            decode_configuration(bytes),
            Err(ContractError::UnsupportedVersion(2))
        ));
    }

    #[test]
    fn checked_in_v1_configuration_is_a_conformance_fixture() {
        let fixture = include_bytes!("../../../contracts/node-sdk/v1/runtime-configuration.json");
        let configuration = decode_configuration(fixture).unwrap();
        assert_eq!(configuration.contract, 1);
        assert_eq!(configuration.host_mode, HostKind::EmbeddedApplication);
        assert_eq!(
            configuration.availability,
            AvailabilityClass::ForegroundOnly
        );
    }
}
