#![allow(clippy::missing_errors_doc)]

use crate::{AppState, api::authenticate_native, error::AppError};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use piqae_auth::Scope;
use piqae_domain::AgentId;
use piqae_protocol::agent::AgentCommand;
use piqae_storage_postgres::{NodeUpdatePolicy, StoredNodeUpdate};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct RequestUpdate {
    version: String,
    metadata_url: String,
}

#[derive(Debug, Deserialize)]
pub struct RequestRollback {
    metadata_url: String,
}

pub async fn get(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(node_id): Path<AgentId>,
) -> Result<Json<StoredNodeUpdate>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::AgentsRead).await?;
    Ok(Json(
        state
            .repository
            .get_node_update(
                tenant.workspace_id,
                tenant.environment_id,
                node_id,
                default_mode(&state),
            )
            .await?,
    ))
}

pub async fn patch_policy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(node_id): Path<AgentId>,
    Json(policy): Json<NodeUpdatePolicy>,
) -> Result<Json<StoredNodeUpdate>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::AgentsWrite).await?;
    validate_policy(&policy)?;
    let update = state
        .repository
        .update_node_policy(
            tenant.workspace_id,
            tenant.environment_id,
            node_id,
            &policy,
            default_mode(&state),
        )
        .await?;
    state
        .publish(tenant, "node.update_policy.updated", &update)
        .await?;
    Ok(Json(update))
}

pub async fn request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(node_id): Path<AgentId>,
    Json(request): Json<RequestUpdate>,
) -> Result<Response, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::AgentsWrite).await?;
    validate_version(&request.version)?;
    validate_metadata_url(&state, &request.metadata_url)?;
    let current = state
        .repository
        .get_node_update(
            tenant.workspace_id,
            tenant.environment_id,
            node_id,
            default_mode(&state),
        )
        .await?;
    if current.policy.mode == "disabled" {
        return Err(AppError::invalid(
            "updates_disabled",
            "Updates are disabled for this node.",
        ));
    }
    let update = state
        .repository
        .request_node_update(
            tenant.workspace_id,
            tenant.environment_id,
            node_id,
            &request.version,
            default_mode(&state),
        )
        .await?;
    state
        .repository
        .enqueue_agent_command(
            tenant.workspace_id,
            tenant.environment_id,
            node_id,
            &AgentCommand::UpdateAvailable {
                version: request.version,
                channel: update.policy.channel.clone(),
                metadata_url: request.metadata_url,
            },
        )
        .await?;
    state
        .publish(tenant, "node.update.requested", &update)
        .await?;
    Ok((StatusCode::ACCEPTED, Json(update)).into_response())
}

pub async fn rollback(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(node_id): Path<AgentId>,
    Json(request): Json<RequestRollback>,
) -> Result<Response, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::AgentsWrite).await?;
    validate_metadata_url(&state, &request.metadata_url)?;
    let current = state
        .repository
        .get_node_update(
            tenant.workspace_id,
            tenant.environment_id,
            node_id,
            default_mode(&state),
        )
        .await?;
    let rollback_version = current.status.rollback_version.ok_or_else(|| {
        AppError::invalid(
            "rollback_unavailable",
            "This node has no verified rollback version.",
        )
    })?;
    let update = state
        .repository
        .request_node_update(
            tenant.workspace_id,
            tenant.environment_id,
            node_id,
            &rollback_version,
            default_mode(&state),
        )
        .await?;
    state
        .repository
        .enqueue_agent_command(
            tenant.workspace_id,
            tenant.environment_id,
            node_id,
            &AgentCommand::UpdateAvailable {
                version: rollback_version,
                channel: "rollback".into(),
                metadata_url: request.metadata_url,
            },
        )
        .await?;
    state
        .publish(tenant, "node.update.rollback_requested", &update)
        .await?;
    Ok((StatusCode::ACCEPTED, Json(update)).into_response())
}

fn default_mode(state: &AppState) -> &'static str {
    if state.capabilities.deployment == "cloud" {
        "automatic"
    } else {
        "prompt"
    }
}

fn validate_policy(policy: &NodeUpdatePolicy) -> Result<(), AppError> {
    if !matches!(policy.channel.as_str(), "stable" | "canary" | "pinned")
        || !matches!(policy.mode.as_str(), "automatic" | "prompt" | "disabled")
        || (policy.channel == "pinned"
            && policy
                .pinned_version
                .as_deref()
                .is_none_or(|version| validate_version(version).is_err()))
        || policy
            .maintenance_window
            .as_ref()
            .is_some_and(|window| !window.is_object())
    {
        return Err(AppError::invalid(
            "invalid_update_policy",
            "The channel, mode, pinned version, or maintenance window is invalid.",
        ));
    }
    if let Some(version) = &policy.pinned_version {
        validate_version(version)?;
    }
    Ok(())
}

fn validate_version(version: &str) -> Result<(), AppError> {
    if version.is_empty()
        || version.len() > 64
        || !version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+'))
    {
        return Err(AppError::invalid(
            "invalid_update_version",
            "The update version is invalid.",
        ));
    }
    Ok(())
}

fn validate_metadata_url(state: &AppState, value: &str) -> Result<(), AppError> {
    let url = url::Url::parse(value).map_err(|_| {
        AppError::invalid(
            "invalid_update_metadata_url",
            "The update metadata URL is invalid.",
        )
    })?;
    let permitted = url.scheme() == "https"
        || (state.capabilities.deployment != "cloud"
            && url.scheme() == "http"
            && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "::1")));
    if !permitted || url.username() != "" || url.password().is_some() {
        return Err(AppError::invalid(
            "invalid_update_metadata_url",
            "Update metadata must use HTTPS without embedded credentials.",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_versions_are_bounded_and_path_safe() {
        assert!(validate_version("0.2.0-canary.1").is_ok());
        assert!(validate_version("../secrets").is_err());
        assert!(validate_version("").is_err());
    }
}
