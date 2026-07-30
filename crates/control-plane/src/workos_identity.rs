#![allow(clippy::missing_errors_doc)]

use crate::{AppState, error::AppError, repository::RepositoryError};
use axum::{Json, body::Bytes, extract::State, http::HeaderMap};
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use piqae_storage_postgres::{WorkOsIdentityData, WorkOsIdentityEvent, WorkOsProjectionResult};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;
const WORKOS_SIGNATURE_TOLERANCE_MILLISECONDS: i64 = 300_000;

#[derive(Debug, Deserialize)]
struct WorkOsEnvelope {
    id: String,
    event: String,
    created_at: DateTime<Utc>,
    data: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct WorkOsWebhookReceipt {
    received: bool,
    duplicate: bool,
    applied: bool,
}

pub async fn webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<WorkOsWebhookReceipt>, AppError> {
    let secret = state
        .workos_webhook_secret
        .as_deref()
        .ok_or_else(|| AppError::service_unavailable("workos_webhook_unavailable"))?;
    verify_workos_signature(&headers, &body, secret)?;
    let envelope: WorkOsEnvelope = serde_json::from_slice(&body)?;
    let event = workos_projection(envelope, &body)?;
    let result = state
        .repository
        .project_workos_identity_event(&event)
        .await
        .map_err(|error| match error {
            RepositoryError::IdempotencyConflict => AppError::conflict(
                "workos_event_conflict",
                "The WorkOS event ID was already used with different content.",
            ),
            other => other.into(),
        })?;
    Ok(Json(WorkOsWebhookReceipt {
        received: true,
        duplicate: result == WorkOsProjectionResult::Duplicate,
        applied: result == WorkOsProjectionResult::Applied,
    }))
}

fn verify_workos_signature(headers: &HeaderMap, body: &[u8], secret: &str) -> Result<(), AppError> {
    verify_workos_signature_at(headers, body, secret, Utc::now().timestamp_millis())
}

fn verify_workos_signature_at(
    headers: &HeaderMap,
    body: &[u8],
    secret: &str,
    now_milliseconds: i64,
) -> Result<(), AppError> {
    let value = headers
        .get("workos-signature")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(AppError::unauthorized)?;
    let mut timestamp = None;
    let mut signatures = Vec::new();
    for component in value.split(',').map(str::trim) {
        if let Some(value) = component.strip_prefix("t=") {
            timestamp = value.parse::<i64>().ok();
        } else if let Some(value) = component.strip_prefix("v1=")
            && let Ok(decoded) = hex::decode(value)
        {
            signatures.push(decoded);
        }
    }
    let timestamp = timestamp.ok_or_else(AppError::unauthorized)?;
    if (now_milliseconds - timestamp).abs() > WORKOS_SIGNATURE_TOLERANCE_MILLISECONDS {
        return Err(AppError::unauthorized());
    }
    let mut signed = timestamp.to_string().into_bytes();
    signed.push(b'.');
    signed.extend_from_slice(body);
    let verified = signatures.iter().any(|signature| {
        HmacSha256::new_from_slice(secret.as_bytes())
            .map(|mut mac| {
                mac.update(&signed);
                mac.verify_slice(signature).is_ok()
            })
            .unwrap_or(false)
    });
    if verified {
        Ok(())
    } else {
        Err(AppError::unauthorized())
    }
}

fn workos_projection(
    envelope: WorkOsEnvelope,
    body: &[u8],
) -> Result<WorkOsIdentityEvent, AppError> {
    if envelope.id.is_empty() || envelope.event.is_empty() {
        return Err(invalid_workos_event());
    }
    let data = match envelope.event.as_str() {
        "organization.created" | "organization.updated" | "organization.deleted" => {
            let organization_id = required_string(&envelope.data, "id")?;
            let name =
                optional_string(&envelope.data, "name").unwrap_or_else(|| organization_id.clone());
            WorkOsIdentityData::Organization {
                organization_id,
                name,
                status: if envelope.event == "organization.deleted" {
                    "cancelled".into()
                } else {
                    "active".into()
                },
                event_at: data_timestamp(&envelope.data, envelope.created_at)?,
            }
        }
        "user.created" | "user.updated" | "user.deleted" => {
            let first_name = optional_string(&envelope.data, "first_name");
            let last_name = optional_string(&envelope.data, "last_name");
            let display_name = [first_name, last_name]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" ");
            WorkOsIdentityData::User {
                user_id: required_string(&envelope.data, "id")?,
                email: optional_string(&envelope.data, "email"),
                display_name: (!display_name.is_empty()).then_some(display_name),
                status: if envelope.event == "user.deleted" {
                    "inactive".into()
                } else {
                    "active".into()
                },
                event_at: data_timestamp(&envelope.data, envelope.created_at)?,
            }
        }
        "organization_membership.created"
        | "organization_membership.updated"
        | "organization_membership.deleted" => {
            let role = envelope
                .data
                .pointer("/role/slug")
                .and_then(serde_json::Value::as_str)
                .or_else(|| {
                    envelope
                        .data
                        .pointer("/roles/0/slug")
                        .and_then(serde_json::Value::as_str)
                })
                .filter(|value| !value.is_empty())
                .ok_or_else(invalid_workos_event)?
                .to_owned();
            let status = if envelope.event == "organization_membership.deleted" {
                "inactive".into()
            } else {
                optional_string(&envelope.data, "status").unwrap_or_else(|| "active".into())
            };
            WorkOsIdentityData::Membership {
                membership_id: required_string(&envelope.data, "id")?,
                organization_id: required_string(&envelope.data, "organization_id")?,
                user_id: required_string(&envelope.data, "user_id")?,
                role,
                status,
                event_at: data_timestamp(&envelope.data, envelope.created_at)?,
            }
        }
        _ => WorkOsIdentityData::Ignored,
    };
    Ok(WorkOsIdentityEvent {
        id: envelope.id,
        event_type: envelope.event,
        payload_sha256: format!("{:x}", Sha256::digest(body)),
        data,
    })
}

fn required_string(value: &serde_json::Value, key: &str) -> Result<String, AppError> {
    optional_string(value, key).ok_or_else(invalid_workos_event)
}

fn optional_string(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn data_timestamp(
    value: &serde_json::Value,
    fallback: DateTime<Utc>,
) -> Result<DateTime<Utc>, AppError> {
    let Some(timestamp) = value
        .get("updated_at")
        .or_else(|| value.get("created_at"))
        .and_then(serde_json::Value::as_str)
    else {
        return Ok(fallback);
    };
    timestamp.parse().map_err(|_| invalid_workos_event())
}

fn invalid_workos_event() -> AppError {
    AppError::invalid("invalid_workos_event", "The WorkOS event is invalid.")
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn signature_accepts_exact_body_and_rejects_tampering() {
        let body = br#"{"id":"event_test"}"#;
        let secret = "test-signing-secret";
        let timestamp = 1_700_000_000_000_i64;
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("valid HMAC key");
        mac.update(timestamp.to_string().as_bytes());
        mac.update(b".");
        mac.update(body);
        let signature = hex::encode(mac.finalize().into_bytes());
        let mut headers = HeaderMap::new();
        headers.insert(
            "workos-signature",
            HeaderValue::from_str(&format!("t={timestamp}, v1={signature}"))
                .expect("valid test header"),
        );
        assert!(verify_workos_signature_at(&headers, body, secret, timestamp).is_ok());
        assert!(
            verify_workos_signature_at(&headers, br#"{"id":"changed"}"#, secret, timestamp)
                .is_err()
        );
    }

    #[test]
    fn signature_rejects_stale_timestamp() {
        let mut headers = HeaderMap::new();
        headers.insert("workos-signature", HeaderValue::from_static("t=1000,v1=00"));
        assert!(verify_workos_signature_at(&headers, b"{}", "secret", 301_001).is_err());
    }

    #[test]
    fn membership_projection_uses_role_and_identity_claims() {
        let body = br#"{
          "id":"event_01",
          "event":"organization_membership.updated",
          "created_at":"2026-07-30T00:00:01Z",
          "data":{
            "id":"om_01",
            "organization_id":"org_01",
            "user_id":"user_01",
            "status":"active",
            "role":{"slug":"operator"},
            "updated_at":"2026-07-30T00:00:00Z"
          }
        }"#;
        let envelope: WorkOsEnvelope = serde_json::from_slice(body).expect("valid event");
        let event = workos_projection(envelope, body).expect("valid projection");
        assert!(matches!(
            event.data,
            WorkOsIdentityData::Membership {
                role,
                status,
                ..
            } if role == "operator" && status == "active"
        ));
    }
}
