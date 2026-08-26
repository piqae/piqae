#![allow(clippy::missing_errors_doc, clippy::too_many_lines)]

use crate::{AppState, authentication::PlatformManagerContext, error::AppError};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use piqae_auth::{
    Scope, generate_platform_service_account_key, rotate_platform_service_account_key,
};
use piqae_storage_postgres::{StoredPlatformAccount, StoredPlatformCredential};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const MAX_OPERATION_CUSTOMERS: usize = 25;
const MAX_RESOURCES_PER_CUSTOMER: usize = 100;

#[derive(Debug, Deserialize)]
pub struct PlatformOperationsQuery {
    #[serde(default = "default_operations_limit")]
    limit: usize,
    after: Option<String>,
}

const fn default_operations_limit() -> usize {
    MAX_OPERATION_CUSTOMERS
}

#[derive(Debug, Serialize)]
pub struct PlatformOperationsCustomer {
    id: String,
    external_id: String,
    name: String,
}

#[derive(Debug, Serialize)]
pub struct PlatformOperationsEnvironment {
    id: String,
    kind: &'static str,
}

#[derive(Debug, Serialize)]
pub struct PlatformOperationsRow {
    customer: PlatformOperationsCustomer,
    environment: PlatformOperationsEnvironment,
    agents: Vec<serde_json::Value>,
    printers: Vec<serde_json::Value>,
    jobs: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    physical_destinations: Vec<crate::destination_topology::PhysicalDestinationResponse>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    routes: Vec<crate::destination_topology::PrinterRouteResponse>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    route_observations: Vec<crate::destination_topology::RouteObservationResponse>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    runtime_observations: Vec<crate::destination_topology::NodeRuntimeObservationResponse>,
}

#[derive(Debug, Serialize)]
pub struct PlatformOperationsPage {
    data: Vec<PlatformOperationsRow>,
    next_cursor: Option<String>,
    has_more: bool,
}

fn canonical_resource<T: Serialize>(
    resource: &T,
    identifiers: &[(&str, String)],
) -> Result<serde_json::Value, AppError> {
    let mut value = serde_json::to_value(resource)
        .map_err(|_| AppError::service_unavailable("platform_operations_serialization_failed"))?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| AppError::service_unavailable("platform_operations_serialization_failed"))?;
    for (field, identifier) in identifiers {
        object.insert(
            (*field).to_owned(),
            serde_json::Value::String(identifier.clone()),
        );
    }
    Ok(value)
}

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

#[derive(Serialize)]
pub struct PlatformEnableResponse {
    enabled: bool,
    secret: String,
}

#[derive(Serialize)]
pub struct PlatformCredentialSecretResponse {
    #[serde(flatten)]
    credential: StoredPlatformCredential,
    secret: String,
}

impl std::fmt::Debug for PlatformCredentialSecretResponse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PlatformCredentialSecretResponse")
            .field("credential", &self.credential)
            .field("secret", &"[REDACTED]")
            .finish()
    }
}

async fn authenticate_human_manager(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<crate::authentication::TenantContext, AppError> {
    if headers.contains_key("x-piqae-workspace-id")
        || headers.contains_key("x-piqae-environment-id")
        || headers.contains_key("x-spool-workspace-id")
        || headers.contains_key("x-spool-environment-id")
        || headers.contains_key("x-piqae-managed-workspace-id")
        || headers.contains_key("x-piqae-managed-environment-id")
    {
        return Err(AppError::unauthorized());
    }
    let authorization = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(AppError::unauthorized)?;
    let tenant = state
        .authenticator
        .authenticate_human(authorization)
        .await
        .map_err(|_| AppError::unauthorized())?;
    if !tenant.allows(Scope::ApiKeysWrite) {
        return Err(AppError::forbidden());
    }
    Ok(tenant)
}

impl std::fmt::Debug for PlatformEnableResponse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PlatformEnableResponse")
            .field("enabled", &self.enabled)
            .field("secret", &"[REDACTED]")
            .finish()
    }
}

pub async fn enable(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let tenant = authenticate_human_manager(&state, &headers).await?;
    let credential = generate_platform_service_account_key()
        .map_err(|_| AppError::service_unavailable("credential_generation_failed"))?;
    state
        .repository
        .enable_platform_manager(
            &credential.id.to_string(),
            "Piqae platform integration",
            &credential.password_hash,
            tenant.workspace_id,
            &crate::request_id::current(),
        )
        .await?;
    let mut response = (
        StatusCode::CREATED,
        Json(PlatformEnableResponse {
            enabled: true,
            secret: credential.plaintext,
        }),
    )
        .into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
        .headers_mut()
        .insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    Ok(response)
}

pub async fn credential(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<StoredPlatformCredential>, AppError> {
    let tenant = authenticate_human_manager(&state, &headers).await?;
    Ok(Json(
        state
            .repository
            .get_platform_credential(tenant.workspace_id)
            .await?,
    ))
}

pub async fn rotate_credential(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let tenant = authenticate_human_manager(&state, &headers).await?;
    let current = state
        .repository
        .get_platform_credential(tenant.workspace_id)
        .await?;
    let id = current
        .id
        .parse()
        .map_err(|_| AppError::service_unavailable("invalid_platform_credential_id"))?;
    let generated = rotate_platform_service_account_key(id)
        .map_err(|_| AppError::service_unavailable("credential_generation_failed"))?;
    let credential = state
        .repository
        .rotate_platform_manager(tenant.workspace_id, &generated.password_hash)
        .await?;
    let mut response = Json(PlatformCredentialSecretResponse {
        credential,
        secret: generated.plaintext,
    })
    .into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
        .headers_mut()
        .insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    Ok(response)
}

pub async fn revoke_credential(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<StatusCode, AppError> {
    let tenant = authenticate_human_manager(&state, &headers).await?;
    state
        .repository
        .revoke_platform_manager(tenant.workspace_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
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
        || headers.contains_key("x-piqae-managed-workspace-id")
        || headers.contains_key("x-piqae-managed-environment-id")
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

/// Returns a bounded, owner-scoped operational snapshot.
///
/// Resources remain nested under their immutable customer identity so equal
/// resource IDs in separate tenants cannot be confused by callers.
pub async fn operations(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<PlatformOperationsQuery>,
) -> Result<Json<PlatformOperationsPage>, AppError> {
    let manager = authenticate_manager(&state, &headers).await?;
    let limit = query.limit.clamp(1, MAX_OPERATION_CUSTOMERS);
    if let Some(after) = query.after.as_deref() {
        validate_external_id(after)?;
    }

    let mut accounts = state
        .repository
        .list_platform_accounts(&manager.service_account_id)
        .await?;
    accounts.retain(|account| {
        account.status == "active"
            && query
                .after
                .as_ref()
                .is_none_or(|after| account.external_id > *after)
    });
    accounts.sort_by(|left, right| left.external_id.cmp(&right.external_id));
    let has_more = accounts.len() > limit;
    accounts.truncate(limit);

    let mut data = Vec::with_capacity(accounts.len());
    for account in accounts {
        let workspace_id = account.id;
        let environment_id = account.environments.live.id;
        let mut agents = state
            .repository
            .list_agents(workspace_id, environment_id)
            .await?;
        agents.truncate(MAX_RESOURCES_PER_CUSTOMER);
        let mut printers = state
            .repository
            .list_printers(workspace_id, environment_id, None, 100)
            .await?;
        printers.truncate(MAX_RESOURCES_PER_CUSTOMER);
        let mut jobs = state
            .repository
            .list_jobs(workspace_id, environment_id, None, 100)
            .await?;
        jobs.truncate(MAX_RESOURCES_PER_CUSTOMER);
        let agents = agents
            .iter()
            .map(|agent| canonical_resource(agent, &[("id", agent.id.to_string())]))
            .collect::<Result<Vec<_>, _>>()?;
        let printers = printers
            .iter()
            .map(|printer| {
                canonical_resource(
                    printer,
                    &[
                        ("id", printer.id.to_string()),
                        ("agent_id", printer.agent_id.to_string()),
                    ],
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let jobs = jobs
            .into_iter()
            .map(crate::api::JobResponse::from)
            .map(|job| {
                canonical_resource(
                    &job,
                    &[
                        ("id", job.id.to_string()),
                        ("printer_id", job.printer_id.to_string()),
                    ],
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let (physical_destinations, routes, route_observations) =
            crate::destination_topology::operational_snapshot(
                &state,
                piqae_storage_postgres::destination_topology::TenantScope {
                    workspace_id,
                    environment_id,
                },
                MAX_RESOURCES_PER_CUSTOMER,
            )
            .await?;
        let runtime_observations = state
            .destination_topology
            .list_latest_node_runtime_observations(
                piqae_storage_postgres::destination_topology::TenantScope {
                    workspace_id,
                    environment_id,
                },
                None,
                u32::try_from(MAX_RESOURCES_PER_CUSTOMER).unwrap_or(100),
            )
            .await
            .map_err(crate::destination_topology::map_storage_error)?
            .into_iter()
            .map(crate::destination_topology::runtime_observation_response)
            .collect();
        data.push(PlatformOperationsRow {
            customer: PlatformOperationsCustomer {
                id: workspace_id.to_string(),
                external_id: account.external_id,
                name: account.name,
            },
            environment: PlatformOperationsEnvironment {
                id: environment_id.to_string(),
                kind: "live",
            },
            agents,
            printers,
            jobs,
            physical_destinations,
            routes,
            route_observations,
            runtime_observations,
        });
    }
    let next_cursor = has_more
        .then(|| data.last().map(|row| row.customer.external_id.clone()))
        .flatten();
    Ok(Json(PlatformOperationsPage {
        data,
        next_cursor,
        has_more,
    }))
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
