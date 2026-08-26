//! Authenticated embedded connector enrollment.
//!
//! Public SDK callers provide only a short-lived invitation and local consent.
//! Tenant identity and management metadata are taken exclusively from the
//! pinned authority response; callers cannot fabricate a `ConnectorRecord`.

use crate::{
    connector_key_scope,
    connector_registry::{ConnectorRecord, ConnectorRegistry, InstallationSigningIdentity},
    installation_key_scope, verify_generated_key,
};
use anyhow::{Context as _, Result, bail};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{TimeDelta, Utc};
use piqae_agent_client::AgentClient;
use piqae_node_host_api::{SecureConnectorSigner, SecureKeyHandle};
use piqae_protocol::{
    CURRENT_PROTOCOL_VERSION,
    agent::{
        EnrolRequest, InstallationMode, PrinterGrant, connector_grant_proof_message,
        connector_proof_message,
    },
};
use url::Url;

const PREPARED_KEY_LIFETIME: TimeDelta = TimeDelta::minutes(10);

#[derive(Debug, Clone)]
pub struct PreparedConnectorIdentity {
    pub handle: SecureKeyHandle,
    pub public_key_base64: String,
    pub expires_unix_ms: i64,
}

/// Parameters which are safe for an application to choose. Every ownership
/// field in the resulting connector is sourced from authenticated authority
/// responses instead.
pub struct ConnectorInvitationExchange {
    pub control_plane_url: Url,
    pub invitation_token: String,
    pub connector_key_handle: SecureKeyHandle,
    pub application_scope: String,
    pub printer_grant: PrinterGrant,
    pub allowed_printer_ids: Vec<String>,
    pub node_name: String,
    pub hostname: String,
    pub platform: String,
    pub architecture: String,
}

impl std::fmt::Debug for ConnectorInvitationExchange {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ConnectorInvitationExchange")
            .field("control_plane_url", &self.control_plane_url)
            .field("invitation_token", &"[REDACTED]")
            .field("connector_key_handle", &self.connector_key_handle)
            .field("application_scope", &self.application_scope)
            .field("printer_grant", &self.printer_grant)
            .field("allowed_printer_ids", &self.allowed_printer_ids.len())
            .finish_non_exhaustive()
    }
}

/// Creates or returns the stable installation principal. It is not a
/// connector key and is never scheduled for connector cleanup.
///
/// # Errors
///
/// Returns an error when the provider fails integrity verification or the
/// identity cannot be durably committed.
pub fn ensure_installation_identity(
    registry: &mut ConnectorRegistry,
    provider: &dyn SecureConnectorSigner,
    application_scope: &str,
) -> Result<InstallationSigningIdentity> {
    if let Some(identity) = registry.installation_identity() {
        return Ok(identity.clone());
    }
    let scope = installation_key_scope(application_scope)?;
    let generated = provider.generate(&scope)?;
    verify_generated_key(provider, &generated, &scope)?;
    let identity = InstallationSigningIdentity {
        installation_id: format!("ins_{}", uuid::Uuid::new_v4().simple()),
        handle: generated.handle.clone(),
        public_key: generated.public_key,
    };
    if let Err(error) = registry.set_installation_identity_once(identity.clone()) {
        // This key was never published or activated. Best-effort immediate
        // cleanup prevents a persist failure from leaking a new credential.
        let _ = provider.delete(&generated.handle);
        return Err(error);
    }
    Ok(identity)
}

/// Generates, verifies and durably tracks an invitation-scoped connector key.
///
/// # Errors
///
/// Returns an error when generation, integrity verification, or durable
/// preparation fails.
pub fn prepare_connector_identity(
    registry: &mut ConnectorRegistry,
    provider: &dyn SecureConnectorSigner,
    application_scope: &str,
) -> Result<PreparedConnectorIdentity> {
    let scope = connector_key_scope(application_scope)?;
    let generated = provider.generate(&scope)?;
    verify_generated_key(provider, &generated, &scope)?;
    let expires_unix_ms = (Utc::now() + PREPARED_KEY_LIFETIME).timestamp_millis();
    if let Err(error) = registry.register_prepared_key(
        generated.handle.clone(),
        generated.public_key,
        expires_unix_ms,
    ) {
        let _ = provider.delete(&generated.handle);
        return Err(error);
    }
    Ok(PreparedConnectorIdentity {
        handle: generated.handle,
        public_key_base64: URL_SAFE_NO_PAD.encode(generated.public_key),
        expires_unix_ms,
    })
}

/// Exchanges an invitation against its exact origin and atomically activates
/// only the authenticated response metadata.
///
/// # Errors
///
/// Returns an error for unsafe input, expired/replayed invitations, provider
/// failure, authority rejection, response validation, or durable activation.
pub async fn exchange_connector_invitation(
    registry: &mut ConnectorRegistry,
    provider: &dyn SecureConnectorSigner,
    request: ConnectorInvitationExchange,
) -> Result<ConnectorRecord> {
    validate_exchange_request(&request)?;
    let prepared = registry
        .prepared_key(&request.connector_key_handle)
        .cloned()
        .context("connector key was not prepared or has already been consumed")?;
    if prepared.expires_unix_ms <= Utc::now().timestamp_millis() {
        bail!("connector key preparation expired");
    }
    let client = AgentClient::new(request.control_plane_url.clone())?;
    let preview = client
        .preview_connect_session(&request.invitation_token)
        .await
        .context("preview connector invitation")?;
    if preview.expires_at <= Utc::now() {
        bail!("connector invitation expired");
    }
    validate_preview_grant(&preview.printer_grant, request.printer_grant)?;
    let installation =
        ensure_installation_identity(registry, provider, &request.application_scope)?;
    let public_key = URL_SAFE_NO_PAD.encode(prepared.public_key);
    let proof = match request.printer_grant {
        PrinterGrant::SelectedPrinters => connector_proof_message(
            &request.invitation_token,
            &installation.installation_id,
            &public_key,
            &request.allowed_printer_ids,
        ),
        PrinterGrant::AllLocalPrinters => connector_grant_proof_message(
            &request.invitation_token,
            &installation.installation_id,
            &public_key,
            request.printer_grant,
            &request.allowed_printer_ids,
        ),
    };
    let installation_proof = URL_SAFE_NO_PAD.encode(provider.sign(&installation.handle, &proof)?);
    let enrolled = client
        .enrol(&EnrolRequest {
            token: request.invitation_token,
            public_key,
            name: request.node_name,
            hostname: request.hostname,
            platform: request.platform,
            architecture: request.architecture,
            installation_mode: InstallationMode::User,
            agent_version: env!("CARGO_PKG_VERSION").into(),
            protocol_version: CURRENT_PROTOCOL_VERSION,
            installation_id: Some(installation.installation_id),
            installation_public_key: Some(URL_SAFE_NO_PAD.encode(installation.public_key)),
            printer_grant: request.printer_grant,
            allowed_printer_ids: request.allowed_printer_ids.clone(),
            installation_proof: Some(installation_proof),
        })
        .await
        .context("exchange connector invitation")?;
    let connector_id = enrolled
        .connector_id
        .context("authority response omitted connector id")?;
    let record = ConnectorRecord {
        connector_id,
        agent_id: enrolled.agent_id.to_string(),
        control_plane_url: request.control_plane_url,
        display_name: preview
            .requesting_service_name
            .or_else(|| Some(preview.workspace_name.clone())),
        workspace_name: Some(preview.workspace_name),
        authorization_type: Some(preview.authorization_type),
        workspace_id: Some(preview.workspace_id),
        environment_id: Some(preview.environment_id),
        requesting_service_account_id: preview.requesting_service_account_id,
        manage_url: preview.return_url.and_then(|value| value.parse().ok()),
        device_key_file: None,
        secure_key_handle: Some(request.connector_key_handle),
        enabled: true,
        printer_grant: request.printer_grant,
        allowed_printer_ids: request.allowed_printer_ids,
    };
    registry.complete_prepared(record.clone())?;
    Ok(record)
}

fn validate_exchange_request(request: &ConnectorInvitationExchange) -> Result<()> {
    let url = &request.control_plane_url;
    let local_http = url.scheme() == "http"
        && url
            .host_str()
            .is_some_and(|host| matches!(host, "localhost" | "127.0.0.1" | "::1"));
    if (url.scheme() != "https" && !local_http)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || request.invitation_token.is_empty()
        || request.invitation_token.len() > 4096
        || request.node_name.is_empty()
        || request.node_name.len() > 256
        || request.hostname.is_empty()
        || request.hostname.len() > 256
        || request.allowed_printer_ids.len() > 128
    {
        bail!("connector invitation request is outside supported bounds");
    }
    match request.printer_grant {
        PrinterGrant::AllLocalPrinters if !request.allowed_printer_ids.is_empty() => {
            bail!("all-printer consent cannot include selected printers")
        }
        PrinterGrant::SelectedPrinters if request.allowed_printer_ids.is_empty() => {
            bail!("selected-printer consent requires at least one printer")
        }
        _ => {}
    }
    Ok(())
}

fn validate_preview_grant(preview: &str, consent: PrinterGrant) -> Result<()> {
    let permitted = match consent {
        PrinterGrant::AllLocalPrinters => {
            matches!(preview, "all_local_printers" | "all_printers")
        }
        PrinterGrant::SelectedPrinters => {
            matches!(preview, "selected_printers" | "selected")
        }
    };
    if !permitted {
        bail!("local printer consent does not match the authenticated invitation");
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::too_many_lines)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer as _, SigningKey};
    use piqae_domain::AgentId;
    use piqae_node_host_api::{ConnectorKeyError, GeneratedConnectorKey};
    use piqae_protocol::agent::{ConnectSessionPreview, EnrolResponse};
    use std::{collections::BTreeMap, sync::Mutex};
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    #[derive(Debug, Default)]
    struct TestProvider {
        keys: Mutex<BTreeMap<String, SigningKey>>,
    }

    impl SecureConnectorSigner for TestProvider {
        fn generate(&self, scope: &str) -> Result<GeneratedConnectorKey, ConnectorKeyError> {
            let handle = if scope.starts_with("installation/") {
                "secure/installation"
            } else if scope.starts_with("connector/") {
                "secure/connector"
            } else {
                return Err(ConnectorKeyError::Rejected);
            };
            let key = SigningKey::from_bytes(if handle.ends_with("installation") {
                &[41; 32]
            } else {
                &[43; 32]
            });
            self.keys.lock().unwrap().insert(handle.into(), key.clone());
            Ok(GeneratedConnectorKey {
                handle: SecureKeyHandle::new(handle.into())?,
                public_key: key.verifying_key().to_bytes(),
            })
        }

        fn sign(
            &self,
            handle: &SecureKeyHandle,
            message: &[u8],
        ) -> Result<[u8; 64], ConnectorKeyError> {
            self.keys
                .lock()
                .unwrap()
                .get(handle.as_str())
                .map(|key| key.sign(message).to_bytes())
                .ok_or(ConnectorKeyError::Unavailable)
        }

        fn delete(&self, handle: &SecureKeyHandle) -> Result<(), ConnectorKeyError> {
            self.keys.lock().unwrap().remove(handle.as_str());
            Ok(())
        }
    }

    async fn fake_authority() -> (Url, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let preview = ConnectSessionPreview {
                workspace_id: "wsp_authenticated".into(),
                workspace_name: "Authenticated customer".into(),
                requesting_service_account_id: Some("svc_platform".into()),
                requesting_service_name: Some("Verified platform".into()),
                authorization_type: "platform_customer".into(),
                environment_id: "env_live".into(),
                requested_scopes: vec!["printers:write".into()],
                printer_grant: "all_local_printers".into(),
                expires_at: Utc::now() + TimeDelta::minutes(5),
                return_url: Some("https://app.example/customer".into()),
            };
            let enrolled = EnrolResponse {
                agent_id: AgentId::new(),
                environment: "live".into(),
                server_time: Utc::now(),
                sync_after_ms: 250,
                connector_id: Some("ncon_authenticated".into()),
            };
            for body in [
                serde_json::to_vec(&preview).unwrap(),
                serde_json::to_vec(&enrolled).unwrap(),
            ] {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = vec![0_u8; 16 * 1024];
                let _ = stream.read(&mut request).await.unwrap();
                let header = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    body.len()
                );
                stream.write_all(header.as_bytes()).await.unwrap();
                stream.write_all(&body).await.unwrap();
            }
        });
        (Url::parse(&format!("http://{address}/")).unwrap(), task)
    }

    #[tokio::test]
    async fn authority_fields_are_canonical_and_invitation_replay_is_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let provider = TestProvider::default();
        let mut registry = ConnectorRegistry::load(directory.path()).unwrap();
        ensure_installation_identity(&mut registry, &provider, "com.example.pos").unwrap();
        let prepared =
            prepare_connector_identity(&mut registry, &provider, "com.example.pos").unwrap();
        let (origin, server) = fake_authority().await;
        let make_request = || ConnectorInvitationExchange {
            control_plane_url: origin.clone(),
            invitation_token: "piq_invitation_secret".into(),
            connector_key_handle: prepared.handle.clone(),
            application_scope: "com.example.pos".into(),
            printer_grant: PrinterGrant::AllLocalPrinters,
            allowed_printer_ids: Vec::new(),
            node_name: "POS iPad".into(),
            hostname: "ipad".into(),
            platform: "ios".into(),
            architecture: "arm64".into(),
        };
        let record = exchange_connector_invitation(&mut registry, &provider, make_request())
            .await
            .unwrap();
        assert_eq!(record.connector_id, "ncon_authenticated");
        assert_eq!(record.workspace_id.as_deref(), Some("wsp_authenticated"));
        assert_eq!(record.environment_id.as_deref(), Some("env_live"));
        assert_eq!(
            record.requesting_service_account_id.as_deref(),
            Some("svc_platform")
        );
        assert!(
            exchange_connector_invitation(&mut registry, &provider, make_request())
                .await
                .is_err()
        );
        server.await.unwrap();
    }

    #[test]
    fn hostile_origin_and_consent_substitution_fail_before_network() {
        let request = ConnectorInvitationExchange {
            control_plane_url: Url::parse("https://user:secret@evil.example/").unwrap(),
            invitation_token: "secret".into(),
            connector_key_handle: SecureKeyHandle::new("secure/connector".into()).unwrap(),
            application_scope: "com.example.pos".into(),
            printer_grant: PrinterGrant::AllLocalPrinters,
            allowed_printer_ids: vec!["ptr_fabricated".into()],
            node_name: "POS".into(),
            hostname: "ipad".into(),
            platform: "ios".into(),
            architecture: "arm64".into(),
        };
        assert!(validate_exchange_request(&request).is_err());
        assert!(
            validate_preview_grant("selected_printers", PrinterGrant::AllLocalPrinters).is_err()
        );
    }
}
