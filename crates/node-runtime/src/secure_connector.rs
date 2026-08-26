//! Host-backed connector identity with non-exportable private key material.

use ed25519_dalek::{Signature, Verifier as _, VerifyingKey};
use piqae_agent_client::{ClientError, DeviceRequestSigner};
use piqae_domain::AgentId;
use piqae_node_host_api::{
    ConnectorKeyError, GeneratedConnectorKey, SecureConnectorSigner, SecureKeyHandle,
};
use rand::RngCore as _;
use std::sync::Arc;

pub const INSTALLATION_KEY_SCOPE_PREFIX: &str = "installation/";
pub const CONNECTOR_KEY_SCOPE_PREFIX: &str = "connector/";

/// Creates the stable provider scope for the installation signing key.
///
/// # Errors
///
/// Returns `InvalidKeyMaterial` for an unsafe application scope.
pub fn installation_key_scope(application_scope: &str) -> Result<String, ConnectorKeyError> {
    scoped_key_label(INSTALLATION_KEY_SCOPE_PREFIX, application_scope, None)
}

/// Creates a unique provider scope for one pending connector key.
///
/// # Errors
///
/// Returns `InvalidKeyMaterial` for an unsafe application scope.
pub fn connector_key_scope(application_scope: &str) -> Result<String, ConnectorKeyError> {
    scoped_key_label(
        CONNECTOR_KEY_SCOPE_PREFIX,
        application_scope,
        Some(uuid::Uuid::new_v4()),
    )
}

fn scoped_key_label(
    prefix: &str,
    application_scope: &str,
    suffix: Option<uuid::Uuid>,
) -> Result<String, ConnectorKeyError> {
    if application_scope.is_empty()
        || application_scope.len() > 160
        || !application_scope
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    {
        return Err(ConnectorKeyError::InvalidKeyMaterial);
    }
    Ok(suffix.map_or_else(
        || format!("{prefix}{application_scope}"),
        |suffix| format!("{prefix}{application_scope}/{suffix}"),
    ))
}

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

/// Proves that an opaque provider handle signs with the public key it returned
/// before either value is persisted or sent to an authority.
///
/// # Errors
///
/// Returns a provider error or `InvalidKeyMaterial` when the returned public
/// key and opaque signing handle do not form one Ed25519 identity.
pub fn verify_generated_key(
    provider: &dyn SecureConnectorSigner,
    generated: &GeneratedConnectorKey,
    scope: &str,
) -> Result<(), piqae_node_host_api::ConnectorKeyError> {
    if scope.is_empty() || scope.len() > 256 {
        return Err(piqae_node_host_api::ConnectorKeyError::InvalidKeyMaterial);
    }
    let mut nonce = [0_u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let mut challenge = b"piqae-secure-connector-key-proof-v1\0".to_vec();
    challenge.extend_from_slice(scope.as_bytes());
    challenge.push(0);
    challenge.extend_from_slice(&nonce);
    let signature = provider.sign(&generated.handle, &challenge)?;
    let key = VerifyingKey::from_bytes(&generated.public_key)
        .map_err(|_| piqae_node_host_api::ConnectorKeyError::InvalidKeyMaterial)?;
    key.verify(&challenge, &Signature::from_bytes(&signature))
        .map_err(|_| piqae_node_host_api::ConnectorKeyError::InvalidKeyMaterial)
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

    #[test]
    fn generated_key_integrity_rejects_wrong_public_key_and_scopes_are_role_separated() {
        let provider = TestProvider(SigningKey::from_bytes(&[19; 32]));
        let scope = connector_key_scope("com.example.pos").unwrap();
        assert!(scope.starts_with("connector/com.example.pos/"));
        assert_eq!(
            installation_key_scope("com.example.pos").unwrap(),
            "installation/com.example.pos"
        );
        let mut generated = provider.generate(&scope).unwrap();
        verify_generated_key(&provider, &generated, &scope).unwrap();
        generated.public_key = SigningKey::from_bytes(&[23; 32]).verifying_key().to_bytes();
        assert_eq!(
            verify_generated_key(&provider, &generated, &scope).unwrap_err(),
            ConnectorKeyError::InvalidKeyMaterial
        );
        assert!(connector_key_scope("unsafe/scope").is_err());
    }
}
