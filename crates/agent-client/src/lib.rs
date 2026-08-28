//! Authenticated outbound HTTPS transport for Piqae agents.

use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD_NO_PAD, URL_SAFE_NO_PAD},
};
use chrono::Utc;
use ed25519_dalek::{Signer, SigningKey};
use piqae_domain::{AgentId, JobId};
use piqae_protocol::agent::{
    AgentAcceptJobRequest, AgentAcceptJobResponse, AgentIdentityUpdateRequest,
    AgentIdentityUpdateResponse, AgentReleaseLeaseRequest, AgentRenewLeaseRequest,
    AgentRenewLeaseResponse, AgentSyncRequest, AgentSyncResponse, ConnectSessionPreview,
    ConnectSessionPreviewRequest, CreateDeviceAuthorizationRequest, CreatedDeviceAuthorization,
    DeviceAuthorizationExchange, DeviceAuthorizationStatus, EnrolRequest, EnrolResponse,
};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
    time::Duration,
};
use thiserror::Error;
use url::Url;
use uuid::Uuid;

const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("invalid control-plane URL: {0}")]
    Url(#[from] url::ParseError),
    #[error("invalid request header: {0}")]
    Header(#[from] reqwest::header::InvalidHeaderValue),
    #[error("request serialization failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("control-plane request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("control plane rejected this node's identity: {code}")]
    Unauthorized { code: String },
    #[error("control plane returned HTTP {status}: {body}")]
    Status { status: u16, body: String },
    #[error("control-plane response exceeds {MAX_RESPONSE_BYTES} bytes")]
    ResponseTooLarge,
    #[error("device authorization request failed")]
    DeviceAuthorization,
    #[error("device request signing failed")]
    Signing,
    #[error("node identity revision conflict; current revision is {current_revision}")]
    NodeIdentityRevisionConflict { current_revision: u64 },
}

impl ClientError {
    /// Reports whether the control plane rejected this node's signature rather
    /// than failing for a transport or server-side reason.
    #[must_use]
    pub const fn is_unauthorized(&self) -> bool {
        matches!(self, Self::Unauthorized { .. })
    }

    /// Reports that an additive recovery endpoint is unavailable on an N-1
    /// or older self-hosted authority.
    #[must_use]
    pub const fn is_endpoint_unsupported(&self) -> bool {
        matches!(
            self,
            Self::Status {
                status: 404 | 405,
                ..
            }
        )
    }

    /// The control-plane error code for a rejected node request.
    #[must_use]
    pub const fn unauthorized_code(&self) -> Option<&str> {
        match self {
            Self::Unauthorized { code } => Some(code.as_str()),
            _ => None,
        }
    }
}

/// The node's running estimate of the control plane's clock.
///
/// Signed requests carry a timestamp the control plane checks against a bounded
/// window, so a node whose own clock has drifted — a suspended laptop, a
/// virtual machine, an appliance with no NTP — would otherwise be rejected
/// forever with no way to discover why. Every response, *including the
/// rejection*, carries the server clock, so observing it here lets the node
/// correct itself on the next attempt without operator involvement.
#[derive(Clone, Debug, Default)]
pub struct ServerClock {
    offset_ms: Arc<AtomicI64>,
}

impl ServerClock {
    /// Records the control plane's clock from one response.
    pub fn observe(&self, headers: &HeaderMap) {
        let Some(server_ms) = server_time_ms(headers) else {
            return;
        };
        self.offset_ms
            .store(server_ms - Utc::now().timestamp_millis(), Ordering::Relaxed);
    }

    /// The timestamp to sign with, corrected by the last observed offset.
    #[must_use]
    pub fn signing_timestamp_ms(&self) -> i64 {
        Utc::now()
            .timestamp_millis()
            .saturating_add(self.offset_ms.load(Ordering::Relaxed))
    }

    /// How far this node's clock is behind the control plane, in milliseconds.
    #[must_use]
    pub fn offset_ms(&self) -> i64 {
        self.offset_ms.load(Ordering::Relaxed)
    }
}

fn server_time_ms(headers: &HeaderMap) -> Option<i64> {
    if let Some(milliseconds) = headers
        .get("x-piqae-server-time")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<i64>().ok())
    {
        return Some(milliseconds);
    }
    // Any HTTP/1.1 origin sends `Date`, so a node still corrects itself when a
    // proxy strips unknown headers. One-second resolution is far finer than the
    // drift that actually causes rejection.
    headers
        .get(reqwest::header::DATE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| chrono::DateTime::parse_from_rfc2822(value).ok())
        .map(|value| value.timestamp_millis())
}

fn status_error(status: u16, bytes: &[u8]) -> ClientError {
    if status == 409
        && let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes)
        && value
            .pointer("/error/code")
            .and_then(serde_json::Value::as_str)
            == Some("node_identity_revision_conflict")
        && let Some(current_revision) = value
            .pointer("/error/details/current_revision")
            .and_then(serde_json::Value::as_u64)
    {
        return ClientError::NodeIdentityRevisionConflict { current_revision };
    }
    let body: String = String::from_utf8_lossy(bytes).chars().take(1024).collect();
    if status == 401 {
        let code = serde_json::from_slice::<serde_json::Value>(bytes)
            .ok()
            .and_then(|value| {
                value
                    .get("error")?
                    .get("code")?
                    .as_str()
                    .map(ToOwned::to_owned)
            })
            .unwrap_or_else(|| "unauthorized".to_owned());
        return ClientError::Unauthorized { code };
    }
    ClientError::Status { status, body }
}

#[derive(Debug)]
pub struct DeviceIdentity {
    agent_id: AgentId,
    signing_key: SigningKey,
}

/// Signing contract shared by file-backed installed nodes and embedded hosts
/// whose connector private keys never leave Keychain/Credential Manager.
pub trait DeviceRequestSigner: std::fmt::Debug + Send + Sync {
    fn agent_id(&self) -> &AgentId;
    /// Signs one bounded canonical HTTP request.
    ///
    /// # Errors
    ///
    /// Returns `Signing` when the backing identity cannot sign safely.
    fn sign(&self, message: &[u8]) -> Result<[u8; 64], ClientError>;

    /// Builds the complete authenticated header set for one exact request.
    ///
    /// # Errors
    ///
    /// Returns a signing or invalid-header error.
    fn signed_headers(
        &self,
        method: &str,
        path: &str,
        body: &[u8],
        timestamp_unix_ms: i64,
        nonce: Uuid,
    ) -> Result<HeaderMap, ClientError> {
        let digest = format!("{:x}", Sha256::digest(body));
        let canonical = format!(
            "{}\n{}\n{}\n{}\n{}",
            method.to_ascii_uppercase(),
            path,
            timestamp_unix_ms,
            nonce,
            digest
        );
        let signature = self.sign(canonical.as_bytes())?;
        let mut headers = HeaderMap::new();
        insert_header(
            &mut headers,
            "x-piqae-agent-id",
            &self.agent_id().to_string(),
        )?;
        insert_header(
            &mut headers,
            "x-piqae-timestamp",
            &timestamp_unix_ms.to_string(),
        )?;
        insert_header(&mut headers, "x-piqae-nonce", &nonce.to_string())?;
        insert_header(&mut headers, "x-piqae-body-sha256", &digest)?;
        insert_header(
            &mut headers,
            "x-piqae-signature",
            &STANDARD_NO_PAD.encode(signature),
        )?;
        Ok(headers)
    }
}

impl DeviceRequestSigner for DeviceIdentity {
    fn agent_id(&self) -> &AgentId {
        &self.agent_id
    }

    fn sign(&self, message: &[u8]) -> Result<[u8; 64], ClientError> {
        Ok(self.signing_key.sign(message).to_bytes())
    }
}

impl DeviceIdentity {
    #[must_use]
    pub const fn new(agent_id: AgentId, signing_key: SigningKey) -> Self {
        Self {
            agent_id,
            signing_key,
        }
    }

    #[must_use]
    pub fn from_secret_bytes(agent_id: AgentId, secret: &[u8; 32]) -> Self {
        Self::new(agent_id, SigningKey::from_bytes(secret))
    }

    #[must_use]
    pub fn generate(agent_id: AgentId) -> Self {
        Self {
            agent_id,
            signing_key: SigningKey::generate(&mut rand::rngs::OsRng),
        }
    }

    #[must_use]
    pub fn public_key_base64(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.signing_key.verifying_key().as_bytes())
    }

    #[must_use]
    pub fn sign_base64(&self, message: &[u8]) -> String {
        STANDARD_NO_PAD.encode(self.signing_key.sign(message).to_bytes())
    }

    #[must_use]
    pub fn secret_bytes(&self) -> [u8; 32] {
        self.signing_key.to_bytes()
    }

    /// Builds the signed device-authentication headers for an exact body.
    ///
    /// # Errors
    ///
    /// Returns an error if a generated header value is not valid HTTP.
    pub fn signed_headers(
        &self,
        method: &str,
        path: &str,
        body: &[u8],
        timestamp_unix_ms: i64,
        nonce: Uuid,
    ) -> Result<HeaderMap, ClientError> {
        let digest = format!("{:x}", Sha256::digest(body));
        let canonical = format!(
            "{}\n{}\n{}\n{}\n{}",
            method.to_ascii_uppercase(),
            path,
            timestamp_unix_ms,
            nonce,
            digest
        );
        let signature = self.signing_key.sign(canonical.as_bytes());
        let mut headers = HeaderMap::new();
        insert_header(&mut headers, "x-piqae-agent-id", &self.agent_id.to_string())?;
        insert_header(
            &mut headers,
            "x-piqae-timestamp",
            &timestamp_unix_ms.to_string(),
        )?;
        insert_header(&mut headers, "x-piqae-nonce", &nonce.to_string())?;
        insert_header(&mut headers, "x-piqae-body-sha256", &digest)?;
        insert_header(
            &mut headers,
            "x-piqae-signature",
            &STANDARD_NO_PAD.encode(signature.to_bytes()),
        )?;
        Ok(headers)
    }
}

fn insert_header(
    headers: &mut HeaderMap,
    name: &'static str,
    value: &str,
) -> Result<(), ClientError> {
    headers.insert(HeaderName::from_static(name), HeaderValue::from_str(value)?);
    Ok(())
}

#[derive(Debug, Clone)]
pub struct AgentClient {
    base_url: Url,
    client: reqwest::Client,
    clock: ServerClock,
}

impl AgentClient {
    /// Builds a bounded, Rustls-backed control-plane client.
    ///
    /// # Errors
    ///
    /// Returns an error when the HTTP client cannot be configured.
    pub fn new(base_url: Url) -> Result<Self, ClientError> {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(40))
            // Enrollment and signed node requests are origin-pinned. Following
            // redirects could forward invitation or lease capabilities to a
            // different authority selected by a compromised proxy response.
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("piqae-agent/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self {
            base_url,
            client,
            clock: ServerClock::default(),
        })
    }

    /// The clock this client signs with, corrected against the control plane.
    #[must_use]
    pub const fn clock(&self) -> &ServerClock {
        &self.clock
    }

    /// Consumes a one-time enrolment token without device authentication.
    ///
    /// # Errors
    ///
    /// Returns an error for serialization, transport, status, size, or
    /// response-decoding failure.
    pub async fn enrol(&self, request: &EnrolRequest) -> Result<EnrolResponse, ClientError> {
        self.post_json("v1/agents/enrol", request, None).await
    }

    /// Inspects a one-time connect invitation without consuming it. Errors are
    /// redacted so the capability is never copied into diagnostics.
    ///
    /// # Errors
    ///
    /// Returns a redacted authorization error when transport, validation, or decoding fails.
    pub async fn preview_connect_session(
        &self,
        token: &str,
    ) -> Result<ConnectSessionPreview, ClientError> {
        self.post_json(
            "v1/node-connect-sessions/preview",
            &ConnectSessionPreviewRequest {
                token: token.to_owned(),
            },
            None,
        )
        .await
        .map_err(|_| ClientError::DeviceAuthorization)
    }

    /// Starts a ten-minute browser authorization without sending the private key.
    ///
    /// # Errors
    ///
    /// Returns an error for serialization, transport, status, size, or decoding failures.
    pub async fn create_device_authorization(
        &self,
        request: &CreateDeviceAuthorizationRequest,
    ) -> Result<CreatedDeviceAuthorization, ClientError> {
        self.post_json("v1/device-authorizations", request, None)
            .await
    }

    /// Polls one device authorization without exposing its code in error values.
    ///
    /// The device code travels in the request body. In a request path it would
    /// be recorded by every proxy and CDN access log between this node and the
    /// control plane.
    ///
    /// # Errors
    ///
    /// Returns a redacted device-authorization error when polling fails.
    pub async fn device_authorization_status(
        &self,
        device_code: &str,
    ) -> Result<DeviceAuthorizationStatus, ClientError> {
        self.post_json(
            "v1/device-authorizations/status",
            &serde_json::json!({ "device_code": device_code }),
            None,
        )
        .await
        .map_err(|_| ClientError::DeviceAuthorization)
    }

    /// Exchanges one approved device authorization for its assigned node identity.
    ///
    /// # Errors
    ///
    /// Returns a redacted device-authorization error when exchange fails.
    pub async fn exchange_device_authorization(
        &self,
        device_code: &str,
    ) -> Result<DeviceAuthorizationExchange, ClientError> {
        self.post_json(
            "v1/device-authorizations/exchange",
            &serde_json::json!({ "device_code": device_code }),
            None,
        )
        .await
        .map_err(|_| ClientError::DeviceAuthorization)
    }

    /// Performs one signed, resumable long-poll synchronization request.
    ///
    /// # Errors
    ///
    /// Returns an error for signing, serialization, transport, status, size,
    /// or response-decoding failure.
    pub async fn sync(
        &self,
        identity: &dyn DeviceRequestSigner,
        request: &AgentSyncRequest,
    ) -> Result<AgentSyncResponse, ClientError> {
        self.post_json("v1/agent/sync", request, Some(identity))
            .await
    }

    /// Updates only this connector's tenant-visible node metadata using an
    /// independent server revision. Exact response-loss retries are
    /// idempotent and never rotate node credentials or queue identity.
    ///
    /// # Errors
    ///
    /// Returns `NodeIdentityRevisionConflict` with the current server revision
    /// when an operator or another client updated this connector first.
    pub async fn update_node_identity(
        &self,
        identity: &dyn DeviceRequestSigner,
        request: &AgentIdentityUpdateRequest,
    ) -> Result<AgentIdentityUpdateResponse, ClientError> {
        let body = serde_json::to_vec(request)?;
        let path = "/v1/agent/identity";
        let mut builder = self
            .client
            .put(self.base_url.join(path.trim_start_matches('/'))?)
            .header("content-type", "application/json")
            .body(body.clone());
        builder = builder.headers(identity.signed_headers(
            "PUT",
            path,
            &body,
            self.clock.signing_timestamp_ms(),
            Uuid::new_v4(),
        )?);
        let mut response = builder.send().await?;
        self.clock.observe(response.headers());
        let status = response.status();
        let bytes = bounded_response_bytes(&mut response).await?;
        if !status.is_success() {
            return Err(status_error(status.as_u16(), &bytes));
        }
        Ok(serde_json::from_slice(&bytes)?)
    }

    /// Durably revokes this connector's server-side grant using its final
    /// signed request. A successful response means the credential is denied
    /// by subsequent agent authentication and may be deleted locally.
    ///
    /// # Errors
    ///
    /// Returns an error when the connector id is malformed, request signing
    /// fails, the authority cannot be reached, or the authority rejects the
    /// exact connector credential.
    pub async fn revoke_connector(
        &self,
        identity: &dyn DeviceRequestSigner,
        connector_id: &str,
    ) -> Result<(), ClientError> {
        if !connector_id.starts_with("ncon_")
            || connector_id.len() > 128
            || !connector_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return Err(ClientError::Status {
                status: 400,
                body: "invalid connector id".into(),
            });
        }
        self.post_json::<_, serde_json::Value>(
            &format!("v1/agent/connectors/{connector_id}/revoke"),
            &serde_json::json!({}),
            Some(identity),
        )
        .await
        .map(|_| ())
    }

    /// Reconciles one exact durable local acceptance against authority state.
    ///
    /// # Errors
    ///
    /// Returns an error when signing, transport, status validation, or decoding fails.
    pub async fn reconcile_acceptance(
        &self,
        identity: &dyn DeviceRequestSigner,
        job_id: piqae_domain::JobId,
        request: &piqae_protocol::agent::AgentAcceptJobRequest,
    ) -> Result<piqae_protocol::agent::AgentAcceptanceReconciliationResponse, ClientError> {
        self.post_json::<_, piqae_protocol::agent::AgentAcceptanceReconciliationResponse>(
            &format!("v1/agent/jobs/{job_id}/acceptance/reconcile"),
            request,
            Some(identity),
        )
        .await
    }

    /// Compensates one exact accepted job before local queue activation.
    ///
    /// # Errors
    ///
    /// Returns an error when signing, transport, status validation, or decoding fails.
    pub async fn abandon_acceptance(
        &self,
        identity: &dyn DeviceRequestSigner,
        job_id: piqae_domain::JobId,
        request: &piqae_protocol::agent::AgentAcceptJobRequest,
    ) -> Result<bool, ClientError> {
        self.post_json::<_, piqae_protocol::agent::AgentAcceptanceAbandonResponse>(
            &format!("v1/agent/jobs/{job_id}/acceptance/abandon"),
            request,
            Some(identity),
        )
        .await
        .map(|response| response.abandoned)
    }

    /// Registers this node's public content-encryption key using device authentication.
    ///
    /// # Errors
    ///
    /// Returns an error for signing, serialization, transport, status, or decoding failure.
    pub async fn register_content_encryption_key(
        &self,
        identity: &dyn DeviceRequestSigner,
        key_id: &str,
        public_key_spki: &str,
    ) -> Result<serde_json::Value, ClientError> {
        let body = serde_json::to_vec(&serde_json::json!({
            "key_id": key_id, "algorithm": "ECDH-P256-HKDF-SHA256", "public_key_spki": public_key_spki
        }))?;
        let path = "/v1/agent/content-encryption-key";
        let mut builder = self
            .client
            .put(self.base_url.join(path.trim_start_matches('/'))?)
            .header("content-type", "application/json")
            .body(body.clone());
        builder = builder.headers(identity.signed_headers(
            "PUT",
            path,
            &body,
            self.clock.signing_timestamp_ms(),
            Uuid::new_v4(),
        )?);
        let mut response = builder.send().await?;
        self.clock.observe(response.headers());
        let status = response.status();
        let bytes = bounded_response_bytes(&mut response).await?;
        if !status.is_success() {
            return Err(status_error(status.as_u16(), &bytes));
        }
        Ok(serde_json::from_slice(&bytes)?)
    }

    /// Confirms a lease only after the job is durable in local `SQLite`.
    ///
    /// # Errors
    ///
    /// Returns an error for signing, serialization, transport, status, size,
    /// or response-decoding failure.
    pub async fn accept_job(
        &self,
        identity: &dyn DeviceRequestSigner,
        job_id: JobId,
        request: &AgentAcceptJobRequest,
    ) -> Result<AgentAcceptJobResponse, ClientError> {
        self.post_json(
            &format!("v1/agent/jobs/{job_id}/accept"),
            request,
            Some(identity),
        )
        .await
    }

    /// Renews an active download/acceptance lease.
    ///
    /// # Errors
    ///
    /// Returns an error for signing, serialization, transport, status, size,
    /// or response-decoding failure.
    pub async fn renew_lease(
        &self,
        identity: &dyn DeviceRequestSigner,
        job_id: JobId,
        request: &AgentRenewLeaseRequest,
    ) -> Result<AgentRenewLeaseResponse, ClientError> {
        self.post_json(
            &format!("v1/agent/jobs/{job_id}/lease"),
            request,
            Some(identity),
        )
        .await
    }

    /// Releases a lease the agent cannot safely accept.
    ///
    /// # Errors
    ///
    /// Returns an error for signing, serialization, transport, status, size,
    /// or response-decoding failure.
    pub async fn release_lease(
        &self,
        identity: &dyn DeviceRequestSigner,
        job_id: JobId,
        request: &AgentReleaseLeaseRequest,
    ) -> Result<serde_json::Value, ClientError> {
        self.post_json(
            &format!("v1/agent/jobs/{job_id}/release"),
            request,
            Some(identity),
        )
        .await
    }

    /// Opens the authenticated content stream protected by an active lease.
    ///
    /// The opaque lease token is sent only as a request header and is never
    /// retained in an error value.
    ///
    /// # Errors
    ///
    /// Returns an error for signing, transport, URL, header, or HTTP status
    /// failures.
    pub async fn download_content(
        &self,
        identity: &dyn DeviceRequestSigner,
        job_id: JobId,
        lease_id: Uuid,
        lease_token: &str,
    ) -> Result<reqwest::Response, ClientError> {
        let path = format!("v1/agent/jobs/{job_id}/content");
        let request_path = format!("/{path}");
        let mut headers = identity.signed_headers(
            "GET",
            &request_path,
            &[],
            self.clock.signing_timestamp_ms(),
            Uuid::new_v4(),
        )?;
        insert_header(&mut headers, "x-piqae-lease-id", &lease_id.to_string())?;
        insert_header(&mut headers, "x-piqae-lease-token", lease_token)?;
        let response = self
            .client
            .get(self.base_url.join(&path)?)
            .headers(headers)
            .send()
            .await?;
        self.clock.observe(response.headers());
        let status = response.status();
        if !status.is_success() {
            let bytes = response.bytes().await?;
            return Err(status_error(status.as_u16(), &bytes));
        }
        Ok(response)
    }

    /// Opens one lease-scoped, content-addressed document resource stream.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid digest, signing, transport, URL, header,
    /// or HTTP status failure.
    pub async fn download_document_resource(
        &self,
        identity: &dyn DeviceRequestSigner,
        job_id: JobId,
        lease_id: Uuid,
        lease_token: &str,
        digest: &str,
    ) -> Result<reqwest::Response, ClientError> {
        if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(ClientError::Status {
                status: 400,
                body: "invalid document resource digest".into(),
            });
        }
        let path = format!(
            "v1/agent/jobs/{job_id}/resources/{}",
            digest.to_ascii_lowercase()
        );
        let request_path = format!("/{path}");
        let mut headers = identity.signed_headers(
            "GET",
            &request_path,
            &[],
            self.clock.signing_timestamp_ms(),
            Uuid::new_v4(),
        )?;
        insert_header(&mut headers, "x-piqae-lease-id", &lease_id.to_string())?;
        insert_header(&mut headers, "x-piqae-lease-token", lease_token)?;
        let response = self
            .client
            .get(self.base_url.join(&path)?)
            .headers(headers)
            .send()
            .await?;
        self.clock.observe(response.headers());
        let status = response.status();
        if !status.is_success() {
            let bytes = response.bytes().await?;
            return Err(status_error(status.as_u16(), &bytes));
        }
        Ok(response)
    }

    async fn post_json<Req: Serialize + Sync, Res: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        request: &Req,
        identity: Option<&dyn DeviceRequestSigner>,
    ) -> Result<Res, ClientError> {
        let body = serde_json::to_vec(request)?;
        let url = self.base_url.join(path)?;
        let request_path = format!("/{}", path.trim_start_matches('/'));
        let mut builder = self
            .client
            .post(url)
            .header("content-type", "application/json")
            .body(body.clone());
        if let Some(identity) = identity {
            builder = builder.headers(identity.signed_headers(
                "POST",
                &request_path,
                &body,
                self.clock.signing_timestamp_ms(),
                Uuid::new_v4(),
            )?);
        }
        let mut response = builder.send().await?;
        self.clock.observe(response.headers());
        let status = response.status();
        let bytes = bounded_response_bytes(&mut response).await?;
        if !status.is_success() {
            return Err(status_error(status.as_u16(), &bytes));
        }
        Ok(serde_json::from_slice(&bytes)?)
    }
}

async fn bounded_response_bytes(response: &mut reqwest::Response) -> Result<Vec<u8>, ClientError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(ClientError::ResponseTooLarge);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(ClientError::ResponseTooLarge);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD_NO_PAD;
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    #[test]
    fn signed_headers_cover_method_path_time_nonce_and_body() {
        let secret = [7_u8; 32];
        let identity = DeviceIdentity::new(AgentId::new(), SigningKey::from_bytes(&secret));
        let nonce = Uuid::nil();
        let headers = identity
            .signed_headers("POST", "/v1/agent/sync", b"{}", 123, nonce)
            .expect("headers");
        let digest = format!("{:x}", Sha256::digest(b"{}"));
        assert_eq!(headers["x-piqae-body-sha256"], digest);
        let signature_bytes = STANDARD_NO_PAD
            .decode(headers["x-piqae-signature"].as_bytes())
            .expect("base64");
        let signature = Signature::from_slice(&signature_bytes).expect("signature");
        let key = VerifyingKey::from_bytes(&identity.signing_key.verifying_key().to_bytes())
            .expect("key");
        let canonical = format!("POST\n/v1/agent/sync\n123\n{nonce}\n{digest}");
        key.verify(canonical.as_bytes(), &signature)
            .expect("valid signature");
    }

    #[test]
    fn a_drifted_node_signs_with_the_control_planes_clock() {
        let clock = ServerClock::default();
        assert_eq!(clock.offset_ms(), 0);
        let server_ms = Utc::now().timestamp_millis() + 3_600_000;
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-piqae-server-time",
            HeaderValue::from_str(&server_ms.to_string()).expect("header"),
        );
        clock.observe(&headers);
        // An hour of drift must be corrected to within a small execution delay,
        // not merely reduced.
        assert!(
            (clock.signing_timestamp_ms() - server_ms).abs() < 5_000,
            "offset {}",
            clock.offset_ms()
        );
    }

    #[test]
    fn the_date_header_corrects_a_node_behind_a_header_stripping_proxy() {
        let clock = ServerClock::default();
        let mut headers = HeaderMap::new();
        headers.insert(
            reqwest::header::DATE,
            HeaderValue::from_static("Sun, 06 Nov 1994 08:49:37 GMT"),
        );
        clock.observe(&headers);
        assert_eq!(server_time_ms(&headers), Some(784_111_777_000));
        assert!(clock.offset_ms() != 0);
    }

    #[test]
    fn responses_without_a_clock_leave_the_offset_untouched() {
        let clock = ServerClock::default();
        clock.observe(&HeaderMap::new());
        assert_eq!(clock.offset_ms(), 0);
    }

    #[test]
    fn a_rejected_signature_is_distinguishable_from_a_server_fault() {
        let rejected = status_error(401, br#"{"error":{"code":"stale_agent_request"}}"#);
        assert!(rejected.is_unauthorized());
        assert_eq!(rejected.unauthorized_code(), Some("stale_agent_request"));

        let unparseable = status_error(401, b"not json");
        assert_eq!(unparseable.unauthorized_code(), Some("unauthorized"));

        let outage = status_error(503, b"{}");
        assert!(!outage.is_unauthorized());
        assert!(matches!(outage, ClientError::Status { status: 503, .. }));

        let conflict = status_error(
            409,
            br#"{"error":{"code":"node_identity_revision_conflict","details":{"current_revision":14}}}"#,
        );
        assert!(matches!(
            conflict,
            ClientError::NodeIdentityRevisionConflict {
                current_revision: 14
            }
        ));
    }

    #[test]
    fn public_key_and_secret_round_trip() {
        let identity = DeviceIdentity::generate(AgentId::new());
        assert_eq!(
            URL_SAFE_NO_PAD
                .decode(identity.public_key_base64())
                .expect("base64"),
            identity.signing_key.verifying_key().as_bytes()
        );
        assert_eq!(identity.secret_bytes().len(), 32);
    }

    #[tokio::test]
    async fn content_download_is_signed_and_lease_capability_is_not_in_errors() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let address = listener.local_addr().expect("address");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut request = vec![0_u8; 16 * 1024];
            let read = stream.read(&mut request).await.expect("read");
            request.truncate(read);
            stream
                .write_all(
                    b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 6\r\nConnection: close\r\n\r\ndenied",
                )
                .await
                .expect("response");
            String::from_utf8(request).expect("HTTP request")
        });

        let agent_id = AgentId::new();
        let identity = DeviceIdentity::from_secret_bytes(agent_id, &[9_u8; 32]);
        let job_id = JobId::new();
        let lease_id = Uuid::new_v4();
        let lease_token = "opaque-secret-lease-capability";
        let client =
            AgentClient::new(Url::parse(&format!("http://{address}/")).expect("control-plane URL"))
                .expect("client");
        let error = client
            .download_content(&identity, job_id, lease_id, lease_token)
            .await
            .expect_err("401 must fail");
        let request = server.await.expect("server");
        let headers = parse_http_headers(&request);

        assert!(request.starts_with(&format!("GET /v1/agent/jobs/{job_id}/content HTTP/1.1\r\n")));
        assert_eq!(headers["x-piqae-agent-id"], agent_id.to_string());
        assert_eq!(headers["x-piqae-lease-id"], lease_id.to_string());
        assert_eq!(headers["x-piqae-lease-token"], lease_token);
        assert_eq!(
            headers["x-piqae-body-sha256"],
            format!("{:x}", Sha256::digest([]))
        );
        assert!(!format!("{error:?}").contains(lease_token));

        let nonce = Uuid::parse_str(&headers["x-piqae-nonce"]).expect("nonce");
        let canonical = format!(
            "GET\n/v1/agent/jobs/{job_id}/content\n{}\n{}\n{}",
            headers["x-piqae-timestamp"], nonce, headers["x-piqae-body-sha256"]
        );
        let signature = Signature::from_slice(
            &STANDARD_NO_PAD
                .decode(&headers["x-piqae-signature"])
                .expect("signature encoding"),
        )
        .expect("signature");
        identity
            .signing_key
            .verifying_key()
            .verify(canonical.as_bytes(), &signature)
            .expect("valid content request signature");
    }

    #[tokio::test]
    async fn content_key_registration_is_signed_on_exact_path_and_response_bounded() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = vec![0_u8; 16 * 1024];
            let read = stream.read(&mut request).await.unwrap();
            request.truncate(read);
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        MAX_RESPONSE_BYTES + 1
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            String::from_utf8(request).unwrap()
        });
        let identity = DeviceIdentity::from_secret_bytes(AgentId::new(), &[3_u8; 32]);
        let client = AgentClient::new(Url::parse(&format!("http://{address}/")).unwrap()).unwrap();
        let error = client
            .register_content_encryption_key(&identity, "cek_test", "spki")
            .await
            .expect_err("oversized response must fail before buffering");
        assert!(matches!(error, ClientError::ResponseTooLarge));
        let request = server.await.unwrap();
        assert!(request.starts_with("PUT /v1/agent/content-encryption-key HTTP/1.1\r\n"));
        let headers = parse_http_headers(&request);
        assert!(headers.contains_key("x-piqae-signature"));
    }

    fn parse_http_headers(request: &str) -> std::collections::HashMap<String, String> {
        request
            .lines()
            .skip(1)
            .filter_map(|line| line.split_once(':'))
            .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_owned()))
            .collect()
    }
}
