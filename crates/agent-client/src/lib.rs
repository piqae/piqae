//! Authenticated outbound HTTPS transport for Piqae agents.

use base64::{Engine as _, engine::general_purpose::STANDARD_NO_PAD};
use chrono::Utc;
use ed25519_dalek::{Signer, SigningKey};
use piqae_domain::{AgentId, JobId};
use piqae_protocol::agent::{
    AgentAcceptJobRequest, AgentAcceptJobResponse, AgentReleaseLeaseRequest,
    AgentRenewLeaseRequest, AgentRenewLeaseResponse, AgentSyncRequest, AgentSyncResponse,
    CreateDeviceAuthorizationRequest, CreatedDeviceAuthorization, DeviceAuthorizationExchange,
    DeviceAuthorizationStatus, EnrolRequest, EnrolResponse,
};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::Serialize;
use sha2::{Digest, Sha256};
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
    #[error("device authorization request failed")]
    DeviceAuthorization,
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
            .user_agent(concat!("piqae-agent/", env!("CARGO_PKG_VERSION")))
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
        self.post_json("v1/agents/enrol", request, None).await
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
    /// # Errors
    ///
    /// Returns a redacted device-authorization error when polling fails.
    pub async fn device_authorization_status(
        &self,
        device_code: &str,
    ) -> Result<DeviceAuthorizationStatus, ClientError> {
        self.get_json(&format!("v1/device-authorizations/{device_code}"), None)
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
            &format!("v1/device-authorizations/{device_code}/exchange"),
            &serde_json::json!({}),
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
        identity: &DeviceIdentity,
        request: &AgentSyncRequest,
    ) -> Result<AgentSyncResponse, ClientError> {
        self.post_json("v1/agent/sync", request, Some(identity))
            .await
    }

    /// Confirms a lease only after the job is durable in local `SQLite`.
    ///
    /// # Errors
    ///
    /// Returns an error for signing, serialization, transport, status, size,
    /// or response-decoding failure.
    pub async fn accept_job(
        &self,
        identity: &DeviceIdentity,
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
        identity: &DeviceIdentity,
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
        identity: &DeviceIdentity,
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
        identity: &DeviceIdentity,
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
            Utc::now().timestamp_millis(),
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
        let status = response.status();
        if !status.is_success() {
            let bytes = response.bytes().await?;
            return Err(ClientError::Status {
                status: status.as_u16(),
                body: String::from_utf8_lossy(&bytes).chars().take(1024).collect(),
            });
        }
        Ok(response)
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

    async fn get_json<Res: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        identity: Option<&DeviceIdentity>,
    ) -> Result<Res, ClientError> {
        let url = self.base_url.join(path)?;
        let request_path = format!("/{}", path.trim_start_matches('/'));
        let mut builder = self.client.get(url);
        if let Some(identity) = identity {
            builder = builder.headers(identity.signed_headers(
                "GET",
                &request_path,
                &[],
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

    fn parse_http_headers(request: &str) -> std::collections::HashMap<String, String> {
        request
            .lines()
            .skip(1)
            .filter_map(|line| line.split_once(':'))
            .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_owned()))
            .collect()
    }
}
