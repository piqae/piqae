use crate::{AppState, authentication::TenantContext, error::AppError};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE_NO_PAD},
};
use chrono::{Duration, Utc};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use piqae_domain::AgentId;
use sha2::{Digest, Sha256};
use std::str::FromStr;
use subtle::ConstantTimeEq;

/// Tolerated difference between a node's signing clock and the server clock.
///
/// Nodes on suspended laptops, virtual machines, and appliances without NTP
/// routinely drift by minutes. A window this wide keeps ordinary drift out of
/// the support queue, and `NONCE_RETENTION` is derived from it so widening the
/// window can never shorten replay protection.
const MAXIMUM_CLOCK_SKEW_MS: i64 = 300_000;

/// How long a consumed nonce stays reserved.
///
/// A signed request stays acceptable until its own timestamp plus the skew
/// window, so a nonce observed now cannot be replayed after this point. The
/// extra minute absorbs clock movement on the control plane itself.
const NONCE_RETENTION_MS: i64 = MAXIMUM_CLOCK_SKEW_MS + 60_000;

/// A signature stays acceptable until its own timestamp plus the skew window,
/// so retaining nonces for any less would let a captured request be replayed
/// once its reservation expired. Enforced at compile time because widening the
/// window is exactly the change that would otherwise break it silently.
const _: () = assert!(NONCE_RETENTION_MS > MAXIMUM_CLOCK_SKEW_MS);

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
    authenticate_agent_inner(state, headers, method, path, body, false).await
}

/// Verifies the connector's final signed revocation request.
///
/// This path intentionally permits an exact already-revoked credential so a
/// lost response or failed local commit can retry only the idempotent
/// revocation operation.
///
/// # Errors
///
/// Returns an authentication error for missing, stale, replayed,
/// digest-mismatched, or cryptographically invalid requests, and a service
/// error if persistence fails.
pub async fn authenticate_agent_for_revocation(
    state: &AppState,
    headers: &axum::http::HeaderMap,
    method: &str,
    path: &str,
    body: &[u8],
) -> Result<AgentIdentity, AppError> {
    authenticate_agent_inner(state, headers, method, path, body, true).await
}

async fn authenticate_agent_inner(
    state: &AppState,
    headers: &axum::http::HeaderMap,
    method: &str,
    path: &str,
    body: &[u8],
    allow_revoked_connector: bool,
) -> Result<AgentIdentity, AppError> {
    let agent_id_text = required_product_header(headers, "x-piqae-agent-id", "x-spool-agent-id")?;
    let agent_id = AgentId::from_str(agent_id_text)
        .map_err(|_| AppError::device_unauthorized("invalid_agent_id"))?;
    let timestamp_text =
        required_product_header(headers, "x-piqae-timestamp", "x-spool-timestamp")?;
    let timestamp_ms = timestamp_text
        .parse::<i64>()
        .map_err(|_| AppError::device_unauthorized("invalid_agent_timestamp"))?;
    let now_ms = Utc::now().timestamp_millis();
    let skew_ms = now_ms - timestamp_ms;
    if skew_ms.abs() > MAXIMUM_CLOCK_SKEW_MS {
        tracing::warn!(
            agent.id = %agent_id_text,
            clock.skew_ms = skew_ms,
            error.type = "stale_agent_request",
            "rejected a node request outside the signing clock window"
        );
        return Err(AppError::device_unauthorized("stale_agent_request"));
    }
    let nonce = required_product_header(headers, "x-piqae-nonce", "x-spool-nonce")?;
    if nonce.len() < 16 || nonce.len() > 128 {
        return Err(AppError::device_unauthorized("invalid_agent_nonce"));
    }
    let supplied_digest =
        required_product_header(headers, "x-piqae-body-sha256", "x-spool-body-sha256")?;
    let calculated_digest = format!("{:x}", Sha256::digest(body));
    if supplied_digest
        .as_bytes()
        .ct_eq(calculated_digest.as_bytes())
        .unwrap_u8()
        != 1
    {
        return Err(AppError::device_unauthorized("agent_digest_mismatch"));
    }
    let record = if allow_revoked_connector {
        state
            .repository
            .agent_for_revocation_authentication(agent_id)
            .await
    } else {
        state.repository.agent_for_authentication(agent_id).await
    }
    .map_err(|_| AppError::device_unauthorized("unknown_agent"))?;
    let public_key: [u8; 32] = record
        .public_key
        .try_into()
        .map_err(|_| AppError::device_unauthorized("invalid_agent_public_key"))?;
    let verifying_key = VerifyingKey::from_bytes(&public_key)
        .map_err(|_| AppError::device_unauthorized("invalid_agent_public_key"))?;
    let signature_text =
        required_product_header(headers, "x-piqae-signature", "x-spool-signature")?;
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
        .reserve_agent_nonce(
            agent_id,
            nonce,
            Utc::now() + Duration::milliseconds(NONCE_RETENTION_MS),
        )
        .await
        .map_err(|_| AppError::device_unauthorized("agent_nonce_replayed"))?;
    Ok(AgentIdentity {
        agent_id,
        tenant: TenantContext::unrestricted(record.workspace_id, record.environment_id),
    })
}

fn required_product_header<'a>(
    headers: &'a axum::http::HeaderMap,
    name: &'static str,
    legacy_name: &'static str,
) -> Result<&'a str, AppError> {
    let canonical = header_value(headers, name);
    let legacy = header_value(headers, legacy_name);
    match (canonical, legacy) {
        (Some(value), None) | (None, Some(value)) => Ok(value),
        (Some(canonical), Some(legacy)) if canonical == legacy => Ok(canonical),
        (Some(_), Some(_)) => Err(AppError::device_unauthorized(
            "conflicting_agent_authentication",
        )),
        (None, None) => Err(AppError::device_unauthorized(
            "missing_agent_authentication",
        )),
    }
}

fn header_value<'a>(headers: &'a axum::http::HeaderMap, name: &'static str) -> Option<&'a str> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue};

    #[test]
    fn legacy_agent_headers_remain_accepted() {
        let mut headers = HeaderMap::new();
        headers.insert("x-spool-agent-id", HeaderValue::from_static("agt_legacy"));
        assert!(matches!(
            required_product_header(&headers, "x-piqae-agent-id", "x-spool-agent-id"),
            Ok("agt_legacy")
        ));
    }

    #[test]
    fn conflicting_product_headers_fail_closed() {
        let mut headers = HeaderMap::new();
        headers.insert("x-piqae-agent-id", HeaderValue::from_static("agt_new"));
        headers.insert("x-spool-agent-id", HeaderValue::from_static("agt_old"));
        assert!(required_product_header(&headers, "x-piqae-agent-id", "x-spool-agent-id").is_err());
    }
}
