use crate::{AppState, authentication::TenantContext, error::AppError};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE_NO_PAD},
};
use chrono::{Duration, Utc};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};
use spool_domain::AgentId;
use std::str::FromStr;
use subtle::ConstantTimeEq;

#[derive(Clone, Copy, Debug)]
pub struct AgentIdentity {
    pub agent_id: AgentId,
    pub tenant: TenantContext,
}

/// Verifies one exact signed agent request and atomically consumes its nonce.
///
/// # Errors
///
/// Returns an authentication error for missing, stale, replayed, digest-mismatched,
/// or cryptographically invalid requests, and a service error if persistence fails.
pub async fn authenticate_agent(
    state: &AppState,
    headers: &axum::http::HeaderMap,
    method: &str,
    path: &str,
    body: &[u8],
) -> Result<AgentIdentity, AppError> {
    let agent_id_text = required_header(headers, "x-spool-agent-id")?;
    let agent_id = AgentId::from_str(agent_id_text)
        .map_err(|_| AppError::device_unauthorized("invalid_agent_id"))?;
    let timestamp_text = required_header(headers, "x-spool-timestamp")?;
    let timestamp_ms = timestamp_text
        .parse::<i64>()
        .map_err(|_| AppError::device_unauthorized("invalid_agent_timestamp"))?;
    let now_ms = Utc::now().timestamp_millis();
    if now_ms.abs_diff(timestamp_ms) > 60_000 {
        return Err(AppError::device_unauthorized("stale_agent_request"));
    }
    let nonce = required_header(headers, "x-spool-nonce")?;
    if nonce.len() < 16 || nonce.len() > 128 {
        return Err(AppError::device_unauthorized("invalid_agent_nonce"));
    }
    let supplied_digest = required_header(headers, "x-spool-body-sha256")?;
    let calculated_digest = format!("{:x}", Sha256::digest(body));
    if supplied_digest
        .as_bytes()
        .ct_eq(calculated_digest.as_bytes())
        .unwrap_u8()
        != 1
    {
        return Err(AppError::device_unauthorized("agent_digest_mismatch"));
    }
    let record = state
        .repository
        .agent_for_authentication(agent_id)
        .await
        .map_err(|_| AppError::device_unauthorized("unknown_agent"))?;
    let public_key: [u8; 32] = record
        .public_key
        .try_into()
        .map_err(|_| AppError::device_unauthorized("invalid_agent_public_key"))?;
    let verifying_key = VerifyingKey::from_bytes(&public_key)
        .map_err(|_| AppError::device_unauthorized("invalid_agent_public_key"))?;
    let signature_text = required_header(headers, "x-spool-signature")?;
    let signature_bytes = URL_SAFE_NO_PAD
        .decode(signature_text)
        .or_else(|_| STANDARD_NO_PAD.decode(signature_text))
        .or_else(|_| STANDARD.decode(signature_text))
        .map_err(|_| AppError::device_unauthorized("invalid_agent_signature"))?;
    let signature = Signature::from_slice(&signature_bytes)
        .map_err(|_| AppError::device_unauthorized("invalid_agent_signature"))?;
    let canonical = format!("{method}\n{path}\n{timestamp_text}\n{nonce}\n{calculated_digest}");
    verifying_key
        .verify(canonical.as_bytes(), &signature)
        .map_err(|_| AppError::device_unauthorized("invalid_agent_signature"))?;
    state
        .repository
        .reserve_agent_nonce(agent_id, nonce, Utc::now() + Duration::minutes(2))
        .await
        .map_err(|_| AppError::device_unauthorized("agent_nonce_replayed"))?;
    Ok(AgentIdentity {
        agent_id,
        tenant: TenantContext::unrestricted(record.workspace_id, record.environment_id),
    })
}

fn required_header<'a>(
    headers: &'a axum::http::HeaderMap,
    name: &'static str,
) -> Result<&'a str, AppError> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| AppError::device_unauthorized("missing_agent_authentication"))
}
