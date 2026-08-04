#![allow(clippy::missing_errors_doc)]

use crate::{AppState, api::authenticate_native, error::AppError};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD_NO_PAD, URL_SAFE_NO_PAD},
};
use chrono::{Duration, Utc};
use piqae_auth::Scope;
use piqae_storage_postgres::NewDeviceAuthorization;
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const DEVICE_AUTHORIZATION_LIFETIME_SECONDS: i64 = 600;
const USER_CODE_ALPHABET: &[u8] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";

#[derive(Debug, Deserialize)]
pub struct CreateDeviceAuthorizationRequest {
    pub public_key: String,
    pub installation_id: String,
    pub proposed_name: String,
    pub hostname: String,
    pub platform: String,
    pub architecture: String,
    pub installation_mode: String,
    pub agent_version: String,
    pub protocol_version: u16,
}

#[derive(Debug, Serialize)]
pub struct CreatedDeviceAuthorization {
    pub id: String,
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: &'static str,
    pub expires_in: i64,
    pub interval: u8,
}

#[derive(Debug, Serialize)]
pub struct DeviceAuthorizationStatus {
    pub id: String,
    pub state: String,
    pub expires_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct DeviceAuthorizationReview {
    pub id: String,
    pub proposed_name: String,
    pub hostname: String,
    pub platform: String,
    pub architecture: String,
    pub state: String,
    pub expires_at: chrono::DateTime<Utc>,
    /// The node whose device key this approval would replace.
    ///
    /// Present when the request comes from an installation already paired to
    /// this workspace — an in-place key rotation. Approving retires that node's
    /// current key, which is a materially different decision from admitting a
    /// new node, so it must be visible before approval rather than after.
    pub replaces_node_id: Option<piqae_domain::AgentId>,
}

#[derive(Debug, Deserialize)]
pub struct DecideDeviceAuthorizationRequest {
    pub user_code: String,
}

/// Carries the device code in a request body instead of the request path.
///
/// The device code is a bearer secret for the pairing exchange. In a path it
/// reaches every proxy access log, CDN log, and trace between the node and the
/// control plane, none of which Piqae controls.
#[derive(Debug, Deserialize)]
pub struct DeviceCodeRequest {
    pub device_code: String,
}

#[derive(Debug, Serialize)]
pub struct DeviceAuthorizationExchange {
    pub node_id: piqae_domain::AgentId,
    pub workspace_id: piqae_domain::WorkspaceId,
    pub environment_id: piqae_domain::EnvironmentId,
    pub server_time: chrono::DateTime<Utc>,
    pub sync_after_ms: u64,
}

pub async fn create(
    State(state): State<AppState>,
    Json(request): Json<CreateDeviceAuthorizationRequest>,
) -> Result<Response, AppError> {
    validate_request(&request)?;
    let public_key = URL_SAFE_NO_PAD
        .decode(&request.public_key)
        .or_else(|_| STANDARD_NO_PAD.decode(&request.public_key))
        .map_err(|_| {
            AppError::invalid("invalid_public_key", "The device public key is invalid.")
        })?;
    if public_key.len() != 32 {
        return Err(AppError::invalid(
            "invalid_public_key",
            "The device public key must contain exactly 32 bytes.",
        ));
    }
    let mut secret = [0_u8; 32];
    OsRng.fill_bytes(&mut secret);
    let device_code = format!("piq_dev_{}", URL_SAFE_NO_PAD.encode(secret));
    let user_code = generate_user_code();
    let id = format!("dva_{}", ulid::Ulid::new());
    let expires_at = Utc::now() + Duration::seconds(DEVICE_AUTHORIZATION_LIFETIME_SECONDS);
    state
        .repository
        .create_device_authorization(&NewDeviceAuthorization {
            id: &id,
            device_code_hash: &digest(&device_code),
            user_code_hash: &digest(&user_code.replace('-', "")),
            user_code_display: &user_code,
            device_public_key: &public_key,
            installation_id: request.installation_id.trim(),
            proposed_name: request.proposed_name.trim(),
            hostname: request.hostname.trim(),
            platform: request.platform.trim(),
            architecture: request.architecture.trim(),
            installation_mode: &request.installation_mode,
            agent_version: request.agent_version.trim(),
            protocol_version: request.protocol_version,
            expires_at,
        })
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(CreatedDeviceAuthorization {
            id,
            device_code,
            user_code,
            verification_uri: "/pair",
            expires_in: DEVICE_AUTHORIZATION_LIFETIME_SECONDS,
            interval: 2,
        }),
    )
        .into_response())
}

pub async fn status(
    State(state): State<AppState>,
    Json(request): Json<DeviceCodeRequest>,
) -> Result<Json<DeviceAuthorizationStatus>, AppError> {
    status_for_code(&state, &request.device_code).await
}

/// Polls pairing state with the device code in the request path.
///
/// Retained for nodes released before the code moved into the request body.
/// New callers must use `POST /v1/device-authorizations/status`.
pub async fn status_by_path(
    State(state): State<AppState>,
    Path(device_code): Path<String>,
) -> Result<Json<DeviceAuthorizationStatus>, AppError> {
    warn_deprecated_path_code("status");
    status_for_code(&state, &device_code).await
}

async fn status_for_code(
    state: &AppState,
    device_code: &str,
) -> Result<Json<DeviceAuthorizationStatus>, AppError> {
    let authorization = state
        .repository
        .device_authorization_by_hash(&digest(device_code))
        .await?;
    Ok(Json(DeviceAuthorizationStatus {
        id: authorization.id,
        state: authorization.state,
        expires_at: authorization.expires_at,
    }))
}

fn warn_deprecated_path_code(operation: &'static str) {
    tracing::warn!(
        operation,
        error.type = "deprecated_path_device_code",
        "a node sent its device code in the request path; upgrade the node so \
         the code stays out of proxy access logs"
    );
}

pub async fn review(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(authorization_id): Path<String>,
) -> Result<Json<DeviceAuthorizationReview>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::AgentsWrite).await?;
    let authorization = state
        .repository
        .device_authorization_by_id(&authorization_id)
        .await?;
    let replaces_node_id = state
        .repository
        .node_replaced_by_device_authorization(
            &authorization_id,
            tenant.workspace_id,
            tenant.environment_id,
        )
        .await?;
    Ok(Json(DeviceAuthorizationReview {
        id: authorization.id,
        proposed_name: authorization.proposed_name,
        hostname: authorization.hostname,
        platform: authorization.platform,
        architecture: authorization.architecture,
        state: authorization.state,
        expires_at: authorization.expires_at,
        replaces_node_id,
    }))
}

pub async fn approve(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(authorization_id): Path<String>,
    Json(request): Json<DecideDeviceAuthorizationRequest>,
) -> Result<Json<DeviceAuthorizationStatus>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::AgentsWrite).await?;
    let authorization = state
        .repository
        .approve_device_authorization(
            &authorization_id,
            &user_code_hash(&request.user_code)?,
            tenant.workspace_id,
            tenant.environment_id,
            "authenticated_principal",
        )
        .await?;
    state
        .publish(tenant, "node.pairing.approved", &authorization)
        .await?;
    Ok(Json(DeviceAuthorizationStatus {
        id: authorization.id,
        state: authorization.state,
        expires_at: authorization.expires_at,
    }))
}

pub async fn deny(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(authorization_id): Path<String>,
    Json(request): Json<DecideDeviceAuthorizationRequest>,
) -> Result<Json<DeviceAuthorizationStatus>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::AgentsWrite).await?;
    let authorization = state
        .repository
        .deny_device_authorization(&authorization_id, &user_code_hash(&request.user_code)?)
        .await?;
    state
        .publish(tenant, "node.pairing.denied", &authorization)
        .await?;
    Ok(Json(DeviceAuthorizationStatus {
        id: authorization.id,
        state: authorization.state,
        expires_at: authorization.expires_at,
    }))
}

pub async fn exchange(
    State(state): State<AppState>,
    Json(request): Json<DeviceCodeRequest>,
) -> Result<Json<DeviceAuthorizationExchange>, AppError> {
    exchange_for_code(&state, &request.device_code).await
}

/// Exchanges an approved device code supplied in the request path.
///
/// Retained for nodes released before the code moved into the request body.
/// New callers must use `POST /v1/device-authorizations/exchange`.
pub async fn exchange_by_path(
    State(state): State<AppState>,
    Path(device_code): Path<String>,
) -> Result<Json<DeviceAuthorizationExchange>, AppError> {
    warn_deprecated_path_code("exchange");
    exchange_for_code(&state, &device_code).await
}

async fn exchange_for_code(
    state: &AppState,
    device_code: &str,
) -> Result<Json<DeviceAuthorizationExchange>, AppError> {
    let enrolled = state
        .repository
        .exchange_device_authorization_with_billing(
            &digest(device_code),
            state.capabilities.billing.enabled,
        )
        .await?;
    Ok(Json(DeviceAuthorizationExchange {
        node_id: enrolled.agent_id,
        workspace_id: enrolled.workspace_id,
        environment_id: enrolled.environment_id,
        server_time: Utc::now(),
        sync_after_ms: 250,
    }))
}

fn validate_request(request: &CreateDeviceAuthorizationRequest) -> Result<(), AppError> {
    let valid_text = |value: &str, maximum: usize| {
        !value.trim().is_empty() && value.trim().chars().count() <= maximum
    };
    if !valid_text(&request.installation_id, 160)
        || !valid_text(&request.proposed_name, 120)
        || !valid_text(&request.hostname, 255)
        || !valid_text(&request.platform, 40)
        || !valid_text(&request.architecture, 40)
        || !valid_text(&request.agent_version, 40)
        || !matches!(
            request.installation_mode.as_str(),
            "user" | "machine" | "local"
        )
        || request.protocol_version == 0
    {
        return Err(AppError::invalid(
            "invalid_device_authorization",
            "The device metadata is outside the supported limits.",
        ));
    }
    Ok(())
}

fn generate_user_code() -> String {
    let mut random = [0_u8; 8];
    OsRng.fill_bytes(&mut random);
    let characters = random.map(|value| {
        let index = usize::from(value) % USER_CODE_ALPHABET.len();
        char::from(USER_CODE_ALPHABET[index])
    });
    format!(
        "{}{}{}{}-{}{}{}{}",
        characters[0],
        characters[1],
        characters[2],
        characters[3],
        characters[4],
        characters[5],
        characters[6],
        characters[7]
    )
}

fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn user_code_hash(value: &str) -> Result<String, AppError> {
    let normalized = value.trim().replace('-', "").to_ascii_uppercase();
    if normalized.len() != 8
        || !normalized
            .as_bytes()
            .iter()
            .all(|value| USER_CODE_ALPHABET.contains(value))
    {
        return Err(AppError::invalid(
            "invalid_user_code",
            "The pairing user code is invalid.",
        ));
    }
    Ok(digest(&normalized))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_code_avoids_ambiguous_characters() {
        for _ in 0..100 {
            let code = generate_user_code();
            assert_eq!(code.len(), 9);
            assert_eq!(code.as_bytes()[4], b'-');
            assert!(code.chars().filter(|value| *value != '-').all(|value| {
                USER_CODE_ALPHABET.contains(&u8::try_from(value).unwrap_or_default())
            }));
        }
    }
}
