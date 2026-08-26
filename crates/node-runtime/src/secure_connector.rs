//! Host-backed connector identity with non-exportable private key material.

use piqae_agent_client::{ClientError, DeviceRequestSigner};
use piqae_domain::AgentId;
use piqae_node_host_api::{SecureConnectorSigner, SecureKeyHandle};
use std::sync::Arc;

/// Device request identity backed by an opaque host secure-store handle.
pub struct HostBackedDeviceIdentity {
    agent_id: AgentId,
    key_handle: SecureKeyHandle,
    provider: Arc<dyn SecureConnectorSigner>,
}

impl HostBackedDeviceIdentity {
    #[must_use]
    pub fn new(
        agent_id: AgentId,
        key_handle: SecureKeyHandle,
        provider: Arc<dyn SecureConnectorSigner>,
    ) -> Self {
        Self {
            agent_id,
            key_handle,
            provider,
        }
    }

    #[must_use]
    pub const fn key_handle(&self) -> &SecureKeyHandle {
        &self.key_handle
    }
}

impl std::fmt::Debug for HostBackedDeviceIdentity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HostBackedDeviceIdentity")
            .field("agent_id", &self.agent_id)
            .field("key_handle", &self.key_handle)
            .field("provider", &"[SECURE PROVIDER]")
            .finish()
    }
}

impl DeviceRequestSigner for HostBackedDeviceIdentity {
    fn agent_id(&self) -> &AgentId {
        &self.agent_id
    }

    fn sign(&self, message: &[u8]) -> Result<[u8; 64], ClientError> {
        if message.is_empty() || message.len() > 16 * 1024 {
            return Err(ClientError::Signing);
        }
        self.provider
            .sign(&self.key_handle, message)
            .map_err(|_| ClientError::Signing)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer as _, SigningKey};
    use piqae_node_host_api::{ConnectorKeyError, GeneratedConnectorKey, SecureConnectorSigner};

    struct TestProvider(SigningKey);

    impl std::fmt::Debug for TestProvider {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("TestProvider([REDACTED])")
        }
    }

    impl SecureConnectorSigner for TestProvider {
        fn generate(
            &self,
            _application_scope: &str,
        ) -> Result<GeneratedConnectorKey, ConnectorKeyError> {
            Ok(GeneratedConnectorKey {
                handle: SecureKeyHandle::new("test/key".into())?,
                public_key: self.0.verifying_key().to_bytes(),
            })
        }

        fn sign(
            &self,
            _handle: &SecureKeyHandle,
            message: &[u8],
        ) -> Result<[u8; 64], ConnectorKeyError> {
            Ok(self.0.sign(message).to_bytes())
        }

        fn delete(&self, _handle: &SecureKeyHandle) -> Result<(), ConnectorKeyError> {
            Ok(())
        }
    }

    #[test]
    fn host_backed_identity_signs_without_debugging_key_material() {
        let provider = Arc::new(TestProvider(SigningKey::from_bytes(&[7; 32])));
        let identity = HostBackedDeviceIdentity::new(
            AgentId::new(),
            SecureKeyHandle::new("test/key".into()).unwrap(),
            provider,
        );
        assert_ne!(identity.sign(b"canonical request").unwrap(), [0; 64]);
        let debug = format!("{identity:?}");
        assert!(!debug.contains("test/key"));
        assert!(!debug.contains("canonical request"));
    }
}
