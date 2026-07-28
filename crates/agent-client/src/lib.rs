//! Authenticated outbound HTTPS transport for Spool agents.

use base64::{Engine as _, engine::general_purpose::STANDARD_NO_PAD};
use chrono::Utc;
use ed25519_dalek::{Signer, SigningKey};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::Serialize;
use sha2::{Digest, Sha256};
use spool_domain::AgentId;
use spool_protocol::agent::{AgentSyncRequest, AgentSyncResponse, EnrolRequest, EnrolResponse};
use std::time::Duration;
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
    #[error("control plane returned HTTP {status}: {body}")]
    Status { status: u16, body: String },
    #[error("control-plane response exceeds {MAX_RESPONSE_BYTES} bytes")]
    ResponseTooLarge,
}

#[derive(Debug)]
pub struct DeviceIdentity {
    agent_id: AgentId,
    signing_key: SigningKey,
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
        STANDARD_NO_PAD.encode(self.signing_key.verifying_key().as_bytes())
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
        insert_header(&mut headers, "x-spool-agent-id", &self.agent_id.to_string())?;
        insert_header(
            &mut headers,
            "x-spool-timestamp",
            &timestamp_unix_ms.to_string(),
        )?;
        insert_header(&mut headers, "x-spool-nonce", &nonce.to_string())?;
        insert_header(&mut headers, "x-spool-body-sha256", &digest)?;
        insert_header(
            &mut headers,
            "x-spool-signature",
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
            .user_agent(concat!("spool-agent/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self { base_url, client })
    }

    /// Consumes a one-time enrolment token without device authentication.
    ///
    /// # Errors
    ///
    /// Returns an error for serialization, transport, status, size, or
    /// response-decoding failure.
    pub async fn enrol(&self, request: &EnrolRequest) -> Result<EnrolResponse, ClientError> {
        self.post_json("v1/agent/enrol", request, None).await
    }

    /// Performs one signed, resumable long-poll synchronization request.
    ///
    /// # Errors
    ///
    /// Returns an error for signing, serialization, transport, status, size,
    /// or response-decoding failure.
    pub async fn sync(
        &self,
        identity: &DeviceIdentity,
        request: &AgentSyncRequest,
    ) -> Result<AgentSyncResponse, ClientError> {
        self.post_json("v1/agent/sync", request, Some(identity))
            .await
    }

    async fn post_json<Req: Serialize + Sync, Res: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        request: &Req,
        identity: Option<&DeviceIdentity>,
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
                Utc::now().timestamp_millis(),
                Uuid::new_v4(),
            )?);
        }
        let response = builder.send().await?;
        let status = response.status();
        let bytes = response.bytes().await?;
        if bytes.len() > MAX_RESPONSE_BYTES {
            return Err(ClientError::ResponseTooLarge);
        }
        if !status.is_success() {
            return Err(ClientError::Status {
                status: status.as_u16(),
                body: String::from_utf8_lossy(&bytes).chars().take(1024).collect(),
            });
        }
        Ok(serde_json::from_slice(&bytes)?)
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD_NO_PAD;
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};

    #[test]
    fn signed_headers_cover_method_path_time_nonce_and_body() {
        let secret = [7_u8; 32];
        let identity = DeviceIdentity::new(AgentId::new(), SigningKey::from_bytes(&secret));
        let nonce = Uuid::nil();
        let headers = identity
            .signed_headers("POST", "/v1/agent/sync", b"{}", 123, nonce)
            .expect("headers");
        let digest = format!("{:x}", Sha256::digest(b"{}"));
        assert_eq!(headers["x-spool-body-sha256"], digest);
        let signature_bytes = STANDARD_NO_PAD
            .decode(headers["x-spool-signature"].as_bytes())
            .expect("base64");
        let signature = Signature::from_slice(&signature_bytes).expect("signature");
        let key = VerifyingKey::from_bytes(&identity.signing_key.verifying_key().to_bytes())
            .expect("key");
        let canonical = format!("POST\n/v1/agent/sync\n123\n{nonce}\n{digest}");
        key.verify(canonical.as_bytes(), &signature)
            .expect("valid signature");
    }

    #[test]
    fn public_key_and_secret_round_trip() {
        let identity = DeviceIdentity::generate(AgentId::new());
        assert_eq!(
            STANDARD_NO_PAD
                .decode(identity.public_key_base64())
                .expect("base64"),
            identity.signing_key.verifying_key().as_bytes()
        );
        assert_eq!(identity.secret_bytes().len(), 32);
    }
}
