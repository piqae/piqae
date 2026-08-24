//! Local-owner identity and provider-independent workspace projections.
//!
//! Bootstrap, credential exchange and session rotation are local-owner
//! operations and require [`LocalIdentityState`]. The identity and workspace
//! projections are tenant reads keyed off the authenticated workspace, so they
//! go through [`AppState::repository`] and work under every identity provider.

use crate::{AppState, authentication::TenantContext, error::AppError};
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use chrono::{Duration, Utc};
use piqae_auth::{
    GeneratedLocalSecret, generate_local_owner_credential, generate_local_owner_session,
    local_owner_credential_id, local_owner_session_id, verify_local_owner_credential,
    verify_local_owner_session,
};
use piqae_domain::{EnvironmentId, WorkspaceId};
use piqae_storage_postgres::{
    BootstrappedLocalOwner, LocalOwnerAuthenticationRecord, PostgresStore, StorageError,
    StoredWorkspace, StoredWorkspaceMember,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use uuid::Uuid;

const DEFAULT_SESSION_SECONDS: i64 = 12 * 60 * 60;

#[derive(Clone)]
pub struct LocalIdentityState {
    store: PostgresStore,
    bootstrap_token_digest: Option<[u8; 32]>,
    session_seconds: i64,
}

impl std::fmt::Debug for LocalIdentityState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LocalIdentityState")
            .field("bootstrap_enabled", &self.bootstrap_token_digest.is_some())
            .field("session_seconds", &self.session_seconds)
            .finish_non_exhaustive()
    }
}

impl LocalIdentityState {
    #[must_use]
    pub fn new(
        store: PostgresStore,
        bootstrap_token: Option<&str>,
        session_seconds: Option<i64>,
    ) -> Self {
        Self {
            store,
            bootstrap_token_digest: bootstrap_token
                .filter(|token| !token.is_empty())
                .map(|token| Sha256::digest(token.as_bytes()).into()),
            session_seconds: bounded_session_seconds(session_seconds),
        }
    }
}

const fn bounded_session_seconds(value: Option<i64>) -> i64 {
    match value {
        Some(value) if value < 15 * 60 => 15 * 60,
        Some(value) if value > 24 * 60 * 60 => 24 * 60 * 60,
        Some(value) => value,
        None => DEFAULT_SESSION_SECONDS,
    }
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/identity/local/bootstrap", post(bootstrap))
        .route("/v1/identity/local/exchange", post(exchange))
        .route("/v1/identity/local/sessions/rotate", post(rotate_session))
        .route("/v1/identity/local/sessions/revoke", post(revoke_session))
        .route("/v1/identity/me", get(current_identity))
        .route(
            "/v1/workspaces/current",
            get(current_workspace).patch(rename_current_workspace),
        )
        .route(
            "/v1/workspaces/current/members",
            get(current_workspace_members),
        )
}

#[derive(Debug, Deserialize)]
struct BootstrapRequest {
    workspace_name: String,
    email: String,
    display_name: Option<String>,
}

#[derive(Debug, Serialize)]
struct BootstrapResponse {
    credential: String,
    workspace: StoredWorkspace,
    member: StoredWorkspaceMember,
}

async fn bootstrap(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<BootstrapRequest>,
) -> Result<(StatusCode, Json<BootstrapResponse>), AppError> {
    let identity = identity_state(&state)?;
    authorize_bootstrap(identity, &headers)?;
    let workspace_name = request.workspace_name.trim();
    let email = request.email.trim().to_ascii_lowercase();
    let display_name = request
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if workspace_name.is_empty()
        || workspace_name.len() > 120
        || email.len() > 320
        || !email.contains('@')
        || display_name.is_some_and(|value| value.len() > 120)
    {
        return Err(AppError::invalid(
            "invalid_local_owner",
            "Workspace name, email, or display name is invalid.",
        ));
    }
    let generated = generate_credential().await?;
    let workspace_id = WorkspaceId::new();
    let environment_id = EnvironmentId::new();
    let user_id = format!("usr_{}", Uuid::now_v7());
    let BootstrappedLocalOwner { workspace, member } = identity
        .store
        .bootstrap_local_owner(
            workspace_id,
            environment_id,
            &generated.id.to_string(),
            &generated.password_hash,
            workspace_name,
            &user_id,
            &email,
            display_name,
        )
        .await
        .map_err(map_bootstrap_error)?;
    Ok((
        StatusCode::CREATED,
        Json(BootstrapResponse {
            credential: generated.plaintext,
            workspace,
            member,
        }),
    ))
}

#[derive(Debug, Deserialize)]
struct ExchangeRequest {
    credential: String,
}

#[derive(Debug, Serialize)]
struct SessionResponse {
    token: String,
    expires_at: chrono::DateTime<Utc>,
}

async fn exchange(
    State(state): State<AppState>,
    Json(request): Json<ExchangeRequest>,
) -> Result<Json<SessionResponse>, AppError> {
    let identity = identity_state(&state)?;
    let credential_id =
        local_owner_credential_id(&request.credential).map_err(|_| AppError::unauthorized())?;
    let record = identity
        .store
        .local_owner_credential_for_authentication(&credential_id.to_string())
        .await
        .map_err(|_| AppError::unauthorized())?;
    verify_credential(request.credential, record.secret_hash).await?;
    let generated = generate_session().await?;
    let expires_at = Utc::now() + Duration::seconds(identity.session_seconds);
    identity
        .store
        .create_local_owner_session(
            &generated.id.to_string(),
            record.workspace_id,
            &record.credential_id,
            &generated.password_hash,
            expires_at,
        )
        .await
        .map_err(|_| AppError::unauthorized())?;
    Ok(Json(SessionResponse {
        token: generated.plaintext,
        expires_at,
    }))
}

async fn rotate_session(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<SessionResponse>, AppError> {
    let identity = identity_state(&state)?;
    let (old_session_id, record) = authenticate_session(identity, &headers).await?;
    let generated = generate_session().await?;
    let expires_at = Utc::now() + Duration::seconds(identity.session_seconds);
    identity
        .store
        .rotate_local_owner_session(
            record.workspace_id,
            &old_session_id.to_string(),
            &generated.id.to_string(),
            &record.credential_id,
            &generated.password_hash,
            expires_at,
        )
        .await
        .map_err(|_| AppError::unauthorized())?;
    Ok(Json(SessionResponse {
        token: generated.plaintext,
        expires_at,
    }))
}

async fn revoke_session(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<StatusCode, AppError> {
    let identity = identity_state(&state)?;
    let (session_id, record) = authenticate_session(identity, &headers).await?;
    identity
        .store
        .revoke_local_owner_session(record.workspace_id, &session_id.to_string())
        .await
        .map_err(|_| AppError::unauthorized())?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Serialize)]
struct CurrentIdentity {
    id: String,
    email: String,
    name: Option<String>,
    workspace_id: WorkspaceId,
    environment_id: EnvironmentId,
    roles: Vec<String>,
}

async fn current_identity(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<CurrentIdentity>, AppError> {
    let tenant = authenticate_tenant(&state, &headers).await?;
    let member = state
        .repository
        .list_workspace_members(tenant.workspace_id)
        .await?
        .into_iter()
        .find(|member| member.status == "active")
        .ok_or_else(|| AppError::from(crate::repository::RepositoryError::NotFound))?;
    Ok(Json(CurrentIdentity {
        id: member.id,
        email: member.email,
        name: member.name,
        workspace_id: tenant.workspace_id,
        environment_id: tenant.environment_id,
        roles: vec![member.role],
    }))
}

async fn current_workspace(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<StoredWorkspace>, AppError> {
    let tenant = authenticate_tenant(&state, &headers).await?;
    Ok(Json(
        state.repository.get_workspace(tenant.workspace_id).await?,
    ))
}

#[derive(Debug, Deserialize)]
struct RenameWorkspaceRequest {
    name: String,
}

/// Renames the authenticated workspace. Restricted to owners and admins: the
/// name is what every member and connected integration sees, so an ordinary
/// member must not be able to change it.
async fn rename_current_workspace(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<RenameWorkspaceRequest>,
) -> Result<Json<StoredWorkspace>, AppError> {
    let tenant = authenticate_tenant(&state, &headers).await?;
    let name = validated_workspace_name(&request.name)?;
    let members = state
        .repository
        .list_workspace_members(tenant.workspace_id)
        .await?;
    if !can_rename_workspace(&members) {
        return Err(AppError::forbidden());
    }
    Ok(Json(
        state
            .repository
            .rename_workspace(tenant.workspace_id, name)
            .await?,
    ))
}

/// Bounded on characters rather than bytes so a name of multi-byte glyphs is
/// judged by what a person sees rather than by its encoding.
fn validated_workspace_name(raw: &str) -> Result<&str, AppError> {
    let name = raw.trim();
    if name.is_empty() || name.chars().count() > 120 {
        return Err(AppError::invalid(
            "invalid_workspace_name",
            "Workspace name must be between 1 and 120 characters.",
        ));
    }
    Ok(name)
}

fn can_rename_workspace(members: &[StoredWorkspaceMember]) -> bool {
    members.iter().any(|member| {
        member.status == "active" && matches!(member.role.as_str(), "owner" | "admin")
    })
}

async fn current_workspace_members(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<StoredWorkspaceMember>>, AppError> {
    let tenant = authenticate_tenant(&state, &headers).await?;
    Ok(Json(
        state
            .repository
            .list_workspace_members(tenant.workspace_id)
            .await?,
    ))
}

fn identity_state(state: &AppState) -> Result<&LocalIdentityState, AppError> {
    state
        .local_identity
        .as_ref()
        .ok_or_else(|| AppError::service_unavailable("local_identity_disabled"))
}

fn authorize_bootstrap(identity: &LocalIdentityState, headers: &HeaderMap) -> Result<(), AppError> {
    let expected = identity
        .bootstrap_token_digest
        .ok_or_else(|| AppError::service_unavailable("local_owner_bootstrap_disabled"))?;
    let supplied = headers
        .get("x-piqae-bootstrap-token")
        .or_else(|| headers.get("x-spool-bootstrap-token"))
        .and_then(|value| value.to_str().ok())
        .filter(|value| value.len() <= 512)
        .ok_or_else(AppError::unauthorized)?;
    let supplied: [u8; 32] = Sha256::digest(supplied.as_bytes()).into();
    if !bool::from(expected.ct_eq(&supplied)) {
        return Err(AppError::unauthorized());
    }
    Ok(())
}

async fn authenticate_tenant(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<TenantContext, AppError> {
    let authorization = authorization(headers)?;
    state
        .authenticator
        .authenticate_bearer(authorization)
        .await
        .map_err(|_| AppError::unauthorized())
}

async fn authenticate_session(
    identity: &LocalIdentityState,
    headers: &HeaderMap,
) -> Result<(Uuid, LocalOwnerAuthenticationRecord), AppError> {
    let token = authorization(headers)?
        .strip_prefix("Bearer ")
        .ok_or_else(AppError::unauthorized)?;
    let session_id = local_owner_session_id(token).map_err(|_| AppError::unauthorized())?;
    let record = identity
        .store
        .local_owner_session_for_authentication(&session_id.to_string())
        .await
        .map_err(|_| AppError::unauthorized())?;
    verify_session(token.to_owned(), record.secret_hash.clone()).await?;
    Ok((session_id, record))
}

fn authorization(headers: &HeaderMap) -> Result<&str, AppError> {
    headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(AppError::unauthorized)
}

async fn generate_credential() -> Result<GeneratedLocalSecret, AppError> {
    tokio::task::spawn_blocking(generate_local_owner_credential)
        .await
        .map_err(|_| AppError::service_unavailable("local_owner_generation_failed"))?
        .map_err(|_| AppError::service_unavailable("local_owner_generation_failed"))
}

async fn generate_session() -> Result<GeneratedLocalSecret, AppError> {
    tokio::task::spawn_blocking(generate_local_owner_session)
        .await
        .map_err(|_| AppError::service_unavailable("local_session_generation_failed"))?
        .map_err(|_| AppError::service_unavailable("local_session_generation_failed"))
}

async fn verify_credential(plaintext: String, hash: String) -> Result<(), AppError> {
    tokio::task::spawn_blocking(move || verify_local_owner_credential(&plaintext, &hash))
        .await
        .map_err(|_| AppError::unauthorized())?
        .map_err(|_| AppError::unauthorized())
}

async fn verify_session(plaintext: String, hash: String) -> Result<(), AppError> {
    tokio::task::spawn_blocking(move || verify_local_owner_session(&plaintext, &hash))
        .await
        .map_err(|_| AppError::unauthorized())?
        .map_err(|_| AppError::unauthorized())
}

fn map_bootstrap_error(error: StorageError) -> AppError {
    match error {
        StorageError::ConcurrentStateChange => AppError::conflict(
            "local_owner_already_configured",
            "A local owner is already configured for this deployment.",
        ),
        other => crate::repository::RepositoryError::from(other).into(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;
    use crate::{
        authentication::{StaticAuthenticator, TenantContext},
        repository::MemoryRepository,
    };
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use std::sync::Arc;
    use tower::ServiceExt;

    const TOKEN: &str = "piq_test_workspace";

    fn stored_member(role: &str, status: &str) -> StoredWorkspaceMember {
        StoredWorkspaceMember {
            id: "usr_1".into(),
            email: "owner@example.test".into(),
            name: Some("Owner".into()),
            role: role.into(),
            status: status.into(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    /// Builds the real route table over a repository with no local identity
    /// configured, which is exactly how production runs with
    /// `PIQAE_IDENTITY_PROVIDER=workos`.
    async fn workspace_application(members: Vec<StoredWorkspaceMember>) -> Router {
        let repository = MemoryRepository::default();
        let authenticator = StaticAuthenticator::default();
        let tenant = TenantContext::unrestricted(WorkspaceId::new(), EnvironmentId::new());
        repository
            .add_workspace(tenant.workspace_id, "Acme Printing", members)
            .await;
        authenticator.insert(TOKEN, tenant).await;
        let state = AppState::new_for_tests(Arc::new(repository), Arc::new(authenticator));
        assert!(
            state.local_identity.is_none(),
            "the regression only reproduces while local identity is disabled"
        );
        crate::router(state)
    }

    fn signed_request(method: &str, path: &str, body: Option<&str>) -> Request<Body> {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("authorization", format!("Bearer {TOKEN}"));
        if body.is_some() {
            request = request.header("content-type", "application/json");
        }
        request
            .body(body.map_or_else(Body::empty, |value| Body::from(value.to_owned())))
            .expect("valid request")
    }

    async fn json_body(response: axum::response::Response) -> serde_json::Value {
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("readable body")
            .to_bytes();
        serde_json::from_slice(&bytes).expect("JSON body")
    }

    #[tokio::test]
    async fn workspace_projections_do_not_require_local_identity() {
        let router = workspace_application(vec![stored_member("owner", "active")]).await;

        let workspace = router
            .clone()
            .oneshot(signed_request("GET", "/v1/workspaces/current", None))
            .await
            .expect("workspace response");
        assert_eq!(workspace.status(), StatusCode::OK);
        assert_eq!(json_body(workspace).await["name"], "Acme Printing");

        let members = router
            .clone()
            .oneshot(signed_request(
                "GET",
                "/v1/workspaces/current/members",
                None,
            ))
            .await
            .expect("members response");
        assert_eq!(members.status(), StatusCode::OK);
        assert_eq!(json_body(members).await[0]["role"], "owner");

        let identity = router
            .clone()
            .oneshot(signed_request("GET", "/v1/identity/me", None))
            .await
            .expect("identity response");
        assert_eq!(identity.status(), StatusCode::OK);
        assert_eq!(json_body(identity).await["email"], "owner@example.test");
    }

    #[tokio::test]
    async fn workspace_rename_does_not_require_local_identity() {
        let router = workspace_application(vec![stored_member("owner", "active")]).await;
        let response = router
            .clone()
            .oneshot(signed_request(
                "PATCH",
                "/v1/workspaces/current",
                Some(r#"{"name":"  Renamed Workspace  "}"#),
            ))
            .await
            .expect("rename response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = json_body(response).await;
        assert_eq!(body["name"], "Renamed Workspace");

        let reread = router
            .oneshot(signed_request("GET", "/v1/workspaces/current", None))
            .await
            .expect("workspace response");
        assert_eq!(json_body(reread).await["name"], "Renamed Workspace");
    }

    #[tokio::test]
    async fn workspace_rename_still_requires_an_owner_or_admin() {
        let router = workspace_application(vec![stored_member("member", "active")]).await;
        let response = router
            .oneshot(signed_request(
                "PATCH",
                "/v1/workspaces/current",
                Some(r#"{"name":"Renamed Workspace"}"#),
            ))
            .await
            .expect("rename response");
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn workspace_rename_still_rejects_an_over_long_name() {
        let router = workspace_application(vec![stored_member("owner", "active")]).await;
        // 121 multi-byte glyphs: rejected on glyph count, not byte length.
        let body = serde_json::json!({ "name": "é".repeat(121) }).to_string();
        let response = router
            .oneshot(signed_request(
                "PATCH",
                "/v1/workspaces/current",
                Some(&body),
            ))
            .await
            .expect("rename response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    /// The local-owner-only routes must stay closed when local identity is
    /// disabled: widening the workspace projections must not widen these.
    #[tokio::test]
    async fn local_owner_routes_remain_unavailable_without_local_identity() {
        let router = workspace_application(vec![stored_member("owner", "active")]).await;
        for (path, body) in [
            (
                "/v1/identity/local/bootstrap",
                r#"{"workspace_name":"Acme","email":"owner@example.test"}"#,
            ),
            ("/v1/identity/local/exchange", r#"{"credential":"nope"}"#),
            ("/v1/identity/local/sessions/rotate", "{}"),
            ("/v1/identity/local/sessions/revoke", "{}"),
        ] {
            let response = router
                .clone()
                .oneshot(signed_request("POST", path, Some(body)))
                .await
                .expect("local identity response");
            assert_eq!(
                response.status(),
                StatusCode::SERVICE_UNAVAILABLE,
                "{path} must stay disabled"
            );
        }
    }

    #[test]
    fn session_ttl_is_bounded() {
        assert_eq!(bounded_session_seconds(Some(10)), 15 * 60);
        assert_eq!(bounded_session_seconds(Some(99 * 60 * 60)), 24 * 60 * 60);
        assert_eq!(bounded_session_seconds(None), DEFAULT_SESSION_SECONDS);
    }

    #[test]
    fn workspace_names_are_trimmed_and_bounded_by_characters() {
        assert_eq!(validated_workspace_name("  Piqae  ").ok(), Some("Piqae"));
        assert!(validated_workspace_name("   ").is_err());
        assert!(validated_workspace_name(&"é".repeat(120)).is_ok());
        assert!(validated_workspace_name(&"é".repeat(121)).is_err());
    }

    #[test]
    fn only_active_owners_and_admins_may_rename_a_workspace() {
        let member = |role: &str, status: &str| StoredWorkspaceMember {
            id: "usr_1".into(),
            email: "owner@example.test".into(),
            name: None,
            role: role.into(),
            status: status.into(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        assert!(can_rename_workspace(&[member("owner", "active")]));
        assert!(can_rename_workspace(&[member("admin", "active")]));
        assert!(!can_rename_workspace(&[member("member", "active")]));
        // A revoked owner must not retain control of the workspace name.
        assert!(!can_rename_workspace(&[member("owner", "inactive")]));
        assert!(!can_rename_workspace(&[]));
    }
}
