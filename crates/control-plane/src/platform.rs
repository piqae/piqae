#![allow(clippy::missing_errors_doc)]

use crate::{AppState, authentication::PlatformManagerContext, error::AppError};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use piqae_storage_postgres::StoredPlatformAccount;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpsertPlatformAccountRequest {
    name: String,
    #[serde(default)]
    metadata: BTreeMap<String, String>,
}

#[derive(Debug, Serialize)]
pub struct PlatformStatusResponse {
    enabled: bool,
}

pub async fn status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<PlatformStatusResponse>, AppError> {
    if headers.contains_key("x-piqae-workspace-id")
        || headers.contains_key("x-piqae-environment-id")
        || headers.contains_key("x-spool-workspace-id")
        || headers.contains_key("x-spool-environment-id")
    {
        return Err(AppError::unauthorized());
    }
    let authorization = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(AppError::unauthorized)?;
    let tenant = state
        .authenticator
        .authenticate_bearer(authorization)
        .await
        .map_err(|_| AppError::unauthorized())?;
    Ok(Json(PlatformStatusResponse {
        enabled: state
            .repository
            .has_platform_manager(tenant.workspace_id)
            .await?,
    }))
}

async fn authenticate_manager(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<PlatformManagerContext, AppError> {
    if headers.contains_key("x-piqae-workspace-id")
        || headers.contains_key("x-piqae-environment-id")
        || headers.contains_key("x-spool-workspace-id")
        || headers.contains_key("x-spool-environment-id")
    {
        return Err(AppError::unauthorized());
    }
    let authorization = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(AppError::unauthorized)?;
    state
        .authenticator
        .authenticate_platform_manager(authorization)
        .await
        .map_err(|_| AppError::unauthorized())
}

fn validate_external_id(external_id: &str) -> Result<(), AppError> {
    let mut characters = external_id.chars();
    let valid = (1..=120).contains(&external_id.len())
        && characters
            .next()
            .is_some_and(|value| value.is_ascii_alphanumeric())
        && characters
            .all(|value| value.is_ascii_alphanumeric() || matches!(value, '_' | '.' | ':' | '-'));
    if !valid {
        return Err(AppError::invalid(
            "invalid_external_id",
            "The platform external ID is invalid.",
        ));
    }
    Ok(())
}

fn validate_request(request: &UpsertPlatformAccountRequest) -> Result<(), AppError> {
    if request.name.trim().is_empty() || request.name.chars().count() > 120 {
        return Err(AppError::invalid(
            "invalid_platform_account_name",
            "Platform account names must contain 1 to 120 characters.",
        ));
    }
    if request.metadata.len() > 20
        || request
            .metadata
            .values()
            .any(|value| value.chars().count() > 500)
    {
        return Err(AppError::invalid(
            "invalid_platform_account_metadata",
            "Platform account metadata exceeds the supported limits.",
        ));
    }
    Ok(())
}

pub async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<StoredPlatformAccount>>, AppError> {
    let manager = authenticate_manager(&state, &headers).await?;
    Ok(Json(
        state
            .repository
            .list_platform_accounts(&manager.service_account_id)
            .await?,
    ))
}

pub async fn get(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(external_id): Path<String>,
) -> Result<Json<StoredPlatformAccount>, AppError> {
    validate_external_id(&external_id)?;
    let manager = authenticate_manager(&state, &headers).await?;
    Ok(Json(
        state
            .repository
            .get_platform_account(&manager.service_account_id, &external_id)
            .await?,
    ))
}

pub async fn upsert(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(external_id): Path<String>,
    Json(request): Json<UpsertPlatformAccountRequest>,
) -> Result<Response, AppError> {
    validate_external_id(&external_id)?;
    validate_request(&request)?;
    let manager = authenticate_manager(&state, &headers).await?;
    let result = state
        .repository
        .upsert_platform_account(
            &manager.service_account_id,
            &external_id,
            request.name.trim(),
            &request.metadata,
            &crate::request_id::current(),
        )
        .await?;
    Ok((
        if result.created {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        },
        Json(result.account),
    )
        .into_response())
}

pub async fn archive(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(external_id): Path<String>,
) -> Result<StatusCode, AppError> {
    validate_external_id(&external_id)?;
    let manager = authenticate_manager(&state, &headers).await?;
    state
        .repository
        .archive_platform_account(
            &manager.service_account_id,
            &external_id,
            &crate::request_id::current(),
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
