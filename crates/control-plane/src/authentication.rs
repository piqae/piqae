use async_trait::async_trait;
use base64::{Engine, engine::general_purpose::STANDARD};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header, jwk::JwkSet};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use spool_auth::{
    Scope, api_key_lookup_prefix, local_owner_session_id, platform_service_account_key_id,
    verify_api_key, verify_local_owner_session, verify_platform_service_account_key,
};
use spool_domain::{EnvironmentId, WorkspaceId};
use spool_storage_postgres::PostgresStore;
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;
use thiserror::Error;
use tokio::sync::RwLock;

#[derive(Clone, Copy, Debug)]
pub struct TenantContext {
    pub workspace_id: WorkspaceId,
    pub environment_id: EnvironmentId,
    permissions: Permissions,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlatformManagerContext {
    pub service_account_id: String,
    pub owner_workspace_id: WorkspaceId,
}

#[derive(Clone, Copy, Debug, Default)]
struct Permissions(u16);

impl Permissions {
    const ALL: Self = Self(u16::MAX);

    fn from_names(names: &[String]) -> Self {
        let mut value = 0;
        for name in names {
            value |= match name.as_str() {
                "api_keys_read" => 1 << 0,
                "api_keys_write" => 1 << 1,
                "agents_read" => 1 << 2,
                "agents_write" => 1 << 3,
                "printers_read" => 1 << 4,
                "printers_write" => 1 << 5,
                "jobs_read" => 1 << 6,
                "jobs_write" => 1 << 7,
                "webhooks_read" => 1 << 8,
                "webhooks_write" => 1 << 9,
                "usage_read" => 1 << 10,
                "audit_read" => 1 << 11,
                _ => 0,
            };
        }
        Self(value)
    }

    const fn allows(self, scope: Scope) -> bool {
        let bit = match scope {
            Scope::ApiKeysRead => 1 << 0,
            Scope::ApiKeysWrite => 1 << 1,
            Scope::AgentsRead => 1 << 2,
            Scope::AgentsWrite => 1 << 3,
            Scope::PrintersRead => 1 << 4,
            Scope::PrintersWrite => 1 << 5,
            Scope::JobsRead => 1 << 6,
            Scope::JobsWrite => 1 << 7,
            Scope::WebhooksRead => 1 << 8,
            Scope::WebhooksWrite => 1 << 9,
            Scope::UsageRead => 1 << 10,
            Scope::AuditRead => 1 << 11,
        };
        self.0 & bit != 0
    }
}

impl TenantContext {
    #[must_use]
    pub const fn unrestricted(workspace_id: WorkspaceId, environment_id: EnvironmentId) -> Self {
        Self {
            workspace_id,
            environment_id,
            permissions: Permissions::ALL,
        }
    }

    #[must_use]
    pub const fn allows(self, scope: Scope) -> bool {
        self.permissions.allows(scope)
    }

    #[must_use]
    pub fn with_scopes(
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        scopes: &[Scope],
    ) -> Self {
        Self {
            workspace_id,
            environment_id,
            permissions: Permissions::from_names(
                &scopes
                    .iter()
                    .map(|scope| scope.as_str().to_owned())
                    .collect::<Vec<_>>(),
            ),
        }
    }
}

#[derive(Debug, Error)]
#[error("authentication failed")]
pub struct AuthenticationError;

#[async_trait]
pub trait Authenticator: Send + Sync + 'static {
    async fn authenticate_bearer(
        &self,
        authorization: &str,
    ) -> Result<TenantContext, AuthenticationError>;
    async fn authenticate_basic(
        &self,
        authorization: &str,
    ) -> Result<TenantContext, AuthenticationError>;
    async fn authenticate_platform_bearer(
        &self,
        _authorization: &str,
        _workspace_id: WorkspaceId,
        _environment_id: EnvironmentId,
        _required_scope: Scope,
        _request_id: &str,
    ) -> Result<TenantContext, AuthenticationError> {
        Err(AuthenticationError)
    }
    async fn authenticate_platform_manager(
        &self,
        _authorization: &str,
    ) -> Result<PlatformManagerContext, AuthenticationError> {
        Err(AuthenticationError)
    }
}

#[derive(Clone, Debug, Default)]
pub struct StaticAuthenticator {
    principals: Arc<RwLock<HashMap<[u8; 32], TenantContext>>>,
}

impl StaticAuthenticator {
    pub async fn insert(&self, token: &str, tenant: TenantContext) {
        self.principals
            .write()
            .await
            .insert(Sha256::digest(token.as_bytes()).into(), tenant);
    }

    async fn authenticate_token(&self, token: &str) -> Result<TenantContext, AuthenticationError> {
        let supplied: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        self.principals
            .read()
            .await
            .iter()
            .find(|(candidate, _)| candidate.ct_eq(&supplied).into())
            .map(|(_, tenant)| *tenant)
            .ok_or(AuthenticationError)
    }
}

#[async_trait]
impl Authenticator for StaticAuthenticator {
    async fn authenticate_bearer(
        &self,
        authorization: &str,
    ) -> Result<TenantContext, AuthenticationError> {
        let token = authorization
            .strip_prefix("Bearer ")
            .filter(|value| !value.is_empty())
            .ok_or(AuthenticationError)?;
        self.authenticate_token(token).await
    }

    async fn authenticate_basic(
        &self,
        authorization: &str,
    ) -> Result<TenantContext, AuthenticationError> {
        let encoded = authorization
            .strip_prefix("Basic ")
            .ok_or(AuthenticationError)?;
        let decoded = STANDARD.decode(encoded).map_err(|_| AuthenticationError)?;
        let decoded = String::from_utf8(decoded).map_err(|_| AuthenticationError)?;
        let (username, password) = decoded.split_once(':').ok_or(AuthenticationError)?;
        if !password.is_empty() {
            return Err(AuthenticationError);
        }
        self.authenticate_token(username).await
    }
}

#[derive(Clone, Debug)]
pub struct PostgresAuthenticator {
    store: PostgresStore,
}

#[derive(Clone, Debug)]
pub struct LocalSessionAuthenticator {
    store: PostgresStore,
}

impl LocalSessionAuthenticator {
    #[must_use]
    pub const fn new(store: PostgresStore) -> Self {
        Self { store }
    }

    async fn authenticate_token(&self, token: &str) -> Result<TenantContext, AuthenticationError> {
        let id = local_owner_session_id(token).map_err(|_| AuthenticationError)?;
        let record = self
            .store
            .local_owner_session_for_authentication(&id.to_string())
            .await
            .map_err(|_| AuthenticationError)?;
        let token = token.to_owned();
        let secret_hash = record.secret_hash.clone();
        tokio::task::spawn_blocking(move || verify_local_owner_session(&token, &secret_hash))
            .await
            .map_err(|_| AuthenticationError)?
            .map_err(|_| AuthenticationError)?;
        Ok(TenantContext::unrestricted(
            record.workspace_id,
            record.environment_id,
        ))
    }
}

#[async_trait]
impl Authenticator for LocalSessionAuthenticator {
    async fn authenticate_bearer(
        &self,
        authorization: &str,
    ) -> Result<TenantContext, AuthenticationError> {
        let token = authorization
            .strip_prefix("Bearer ")
            .filter(|value| !value.is_empty())
            .ok_or(AuthenticationError)?;
        self.authenticate_token(token).await
    }

    async fn authenticate_basic(
        &self,
        _authorization: &str,
    ) -> Result<TenantContext, AuthenticationError> {
        Err(AuthenticationError)
    }
}

impl PostgresAuthenticator {
    #[must_use]
    pub const fn new(store: PostgresStore) -> Self {
        Self { store }
    }

    async fn authenticate_token(&self, token: &str) -> Result<TenantContext, AuthenticationError> {
        let prefix = api_key_lookup_prefix(token).map_err(|_| AuthenticationError)?;
        let record = self
            .store
            .api_key_for_authentication(&prefix)
            .await
            .map_err(|_| AuthenticationError)?;
        verify_api_key(token, &record.secret_hash).map_err(|_| AuthenticationError)?;
        if let Err(error) = self.store.mark_api_key_used(&prefix).await {
            tracing::warn!(%error, "failed to record API key use");
        }
        Ok(TenantContext {
            workspace_id: record.workspace_id,
            environment_id: record.environment_id,
            permissions: Permissions::from_names(&record.scopes),
        })
    }
}

#[async_trait]
impl Authenticator for PostgresAuthenticator {
    async fn authenticate_bearer(
        &self,
        authorization: &str,
    ) -> Result<TenantContext, AuthenticationError> {
        let token = authorization
            .strip_prefix("Bearer ")
            .filter(|value| !value.is_empty())
            .ok_or(AuthenticationError)?;
        self.authenticate_token(token).await
    }

    async fn authenticate_basic(
        &self,
        authorization: &str,
    ) -> Result<TenantContext, AuthenticationError> {
        let encoded = authorization
            .strip_prefix("Basic ")
            .ok_or(AuthenticationError)?;
        let decoded = STANDARD.decode(encoded).map_err(|_| AuthenticationError)?;
        let decoded = String::from_utf8(decoded).map_err(|_| AuthenticationError)?;
        let (username, password) = decoded.split_once(':').ok_or(AuthenticationError)?;
        if !password.is_empty() {
            return Err(AuthenticationError);
        }
        self.authenticate_token(username).await
    }

    async fn authenticate_platform_bearer(
        &self,
        authorization: &str,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        required_scope: Scope,
        request_id: &str,
    ) -> Result<TenantContext, AuthenticationError> {
        let token = authorization
            .strip_prefix("Bearer ")
            .filter(|value| !value.is_empty())
            .ok_or(AuthenticationError)?;
        let id = platform_service_account_key_id(token).map_err(|_| AuthenticationError)?;
        let record = self
            .store
            .platform_grant_for_authentication(&id.to_string(), workspace_id, environment_id)
            .await
            .map_err(|_| AuthenticationError)?;
        let token = token.to_owned();
        let secret_hash = record.secret_hash;
        tokio::task::spawn_blocking(move || {
            verify_platform_service_account_key(&token, &secret_hash)
        })
        .await
        .map_err(|_| AuthenticationError)?
        .map_err(|_| AuthenticationError)?;
        let permissions = Permissions::from_names(&record.scopes);
        self.store
            .record_platform_service_account_use(
                &id.to_string(),
                workspace_id,
                environment_id,
                required_scope.as_str(),
                permissions.allows(required_scope),
                request_id,
            )
            .await
            .map_err(|_| AuthenticationError)?;
        Ok(TenantContext {
            workspace_id,
            environment_id,
            permissions,
        })
    }

    async fn authenticate_platform_manager(
        &self,
        authorization: &str,
    ) -> Result<PlatformManagerContext, AuthenticationError> {
        let token = authorization
            .strip_prefix("Bearer ")
            .filter(|value| !value.is_empty())
            .ok_or(AuthenticationError)?;
        let id = platform_service_account_key_id(token).map_err(|_| AuthenticationError)?;
        let record = self
            .store
            .platform_manager_for_authentication(&id.to_string())
            .await
            .map_err(|_| AuthenticationError)?;
        let token = token.to_owned();
        tokio::task::spawn_blocking(move || {
            verify_platform_service_account_key(&token, &record.secret_hash)
        })
        .await
        .map_err(|_| AuthenticationError)?
        .map_err(|_| AuthenticationError)?;
        Ok(PlatformManagerContext {
            service_account_id: id.to_string(),
            owner_workspace_id: record.owner_workspace_id,
        })
    }
}

#[derive(Clone, Debug)]
pub struct CombinedAuthenticator {
    postgres: PostgresAuthenticator,
    local_session: Option<LocalSessionAuthenticator>,
    bootstrap: Option<StaticAuthenticator>,
    oidc: Option<OidcAuthenticator>,
}

impl CombinedAuthenticator {
    #[must_use]
    pub const fn new(
        postgres: PostgresAuthenticator,
        local_session: Option<LocalSessionAuthenticator>,
        bootstrap: Option<StaticAuthenticator>,
        oidc: Option<OidcAuthenticator>,
    ) -> Self {
        Self {
            postgres,
            local_session,
            bootstrap,
            oidc,
        }
    }
}

#[async_trait]
impl Authenticator for CombinedAuthenticator {
    async fn authenticate_bearer(
        &self,
        authorization: &str,
    ) -> Result<TenantContext, AuthenticationError> {
        if let Ok(tenant) = self.postgres.authenticate_bearer(authorization).await {
            Ok(tenant)
        } else {
            if let Some(local_session) = &self.local_session
                && let Ok(tenant) = local_session.authenticate_bearer(authorization).await
            {
                return Ok(tenant);
            }
            if let Some(bootstrap) = &self.bootstrap
                && let Ok(tenant) = bootstrap.authenticate_bearer(authorization).await
            {
                return Ok(tenant);
            }
            match &self.oidc {
                Some(oidc) => oidc.authenticate_bearer(authorization).await,
                None => Err(AuthenticationError),
            }
        }
    }

    async fn authenticate_basic(
        &self,
        authorization: &str,
    ) -> Result<TenantContext, AuthenticationError> {
        match self.postgres.authenticate_basic(authorization).await {
            Ok(tenant) => Ok(tenant),
            Err(_) => match &self.bootstrap {
                Some(bootstrap) => bootstrap.authenticate_basic(authorization).await,
                None => Err(AuthenticationError),
            },
        }
    }

    async fn authenticate_platform_bearer(
        &self,
        authorization: &str,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        required_scope: Scope,
        request_id: &str,
    ) -> Result<TenantContext, AuthenticationError> {
        self.postgres
            .authenticate_platform_bearer(
                authorization,
                workspace_id,
                environment_id,
                required_scope,
                request_id,
            )
            .await
    }

    async fn authenticate_platform_manager(
        &self,
        authorization: &str,
    ) -> Result<PlatformManagerContext, AuthenticationError> {
        let token = authorization
            .strip_prefix("Bearer ")
            .filter(|value| !value.is_empty())
            .ok_or(AuthenticationError)?;
        if token.starts_with("spl_platform_") {
            return self
                .postgres
                .authenticate_platform_manager(authorization)
                .await;
        }
        if token.starts_with("spl_test_") || token.starts_with("spl_live_") {
            return Err(AuthenticationError);
        }
        let human = if let Some(local_session) = &self.local_session
            && let Ok(tenant) = local_session.authenticate_bearer(authorization).await
        {
            tenant
        } else if let Some(oidc) = &self.oidc
            && let Ok(tenant) = oidc.authenticate_bearer(authorization).await
        {
            tenant
        } else {
            return Err(AuthenticationError);
        };
        if !human.allows(Scope::ApiKeysWrite) {
            return Err(AuthenticationError);
        }
        let service_account_id = self
            .postgres
            .store
            .platform_manager_for_owner_workspace(human.workspace_id)
            .await
            .map_err(|_| AuthenticationError)?;
        Ok(PlatformManagerContext {
            service_account_id,
            owner_workspace_id: human.workspace_id,
        })
    }
}

#[derive(Clone, Debug)]
pub struct OidcConfiguration {
    pub provider: String,
    pub issuer: String,
    pub audience: Option<String>,
    pub binding_claim: Option<String>,
    pub binding_value: Option<String>,
    pub jwks_url: String,
    pub organization_claim: String,
    pub permissions_claim: String,
    pub environment_kind: String,
    pub allow_unrestricted: bool,
}

#[derive(Clone, Debug)]
pub struct OidcAuthenticator {
    store: PostgresStore,
    configuration: Arc<OidcConfiguration>,
    client: reqwest::Client,
    jwks: Arc<RwLock<Option<CachedJwks>>>,
}

#[derive(Clone, Debug)]
struct CachedJwks {
    fetched_at: Instant,
    keys: JwkSet,
}

#[derive(Debug, Deserialize)]
struct OidcClaims {
    #[serde(flatten)]
    values: HashMap<String, serde_json::Value>,
}

impl OidcAuthenticator {
    /// Creates an OIDC authenticator with a bounded, no-redirect JWKS client.
    ///
    /// # Errors
    ///
    /// Returns an error when required configuration is empty or invalid.
    pub fn new(
        store: PostgresStore,
        configuration: OidcConfiguration,
    ) -> Result<Self, AuthenticationError> {
        if !matches!(configuration.provider.as_str(), "workos" | "oidc")
            || configuration.issuer.is_empty()
            || configuration.jwks_url.is_empty()
            || configuration.organization_claim.is_empty()
            || configuration.permissions_claim.is_empty()
            || !matches!(configuration.environment_kind.as_str(), "test" | "live")
            || (configuration.audience.is_some()
                == (configuration.binding_claim.is_some() && configuration.binding_value.is_some()))
            || configuration.binding_claim.is_some() != configuration.binding_value.is_some()
        {
            return Err(AuthenticationError);
        }
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| AuthenticationError)?;
        Ok(Self {
            store,
            configuration: Arc::new(configuration),
            client,
            jwks: Arc::new(RwLock::new(None)),
        })
    }

    async fn authenticate_token(&self, token: &str) -> Result<TenantContext, AuthenticationError> {
        let header = decode_header(token).map_err(|_| AuthenticationError)?;
        if !matches!(
            header.alg,
            Algorithm::RS256
                | Algorithm::RS384
                | Algorithm::RS512
                | Algorithm::ES256
                | Algorithm::ES384
                | Algorithm::EdDSA
        ) {
            return Err(AuthenticationError);
        }
        let key_id = header.kid.as_deref().ok_or(AuthenticationError)?;
        let jwk = self.key_for_id(key_id).await?;
        let key = DecodingKey::from_jwk(&jwk).map_err(|_| AuthenticationError)?;
        let mut validation = Validation::new(header.alg);
        validation.set_issuer(&[self.configuration.issuer.as_str()]);
        if let Some(audience) = &self.configuration.audience {
            validation.set_audience(&[audience.as_str()]);
        }
        validation.validate_exp = true;
        let claims = decode::<OidcClaims>(token, &key, &validation)
            .map_err(|_| AuthenticationError)?
            .claims;
        if let (Some(claim), Some(expected)) = (
            &self.configuration.binding_claim,
            &self.configuration.binding_value,
        ) && claims.values.get(claim).and_then(serde_json::Value::as_str) != Some(expected)
        {
            return Err(AuthenticationError);
        }
        let organization_id = claims
            .values
            .get(&self.configuration.organization_claim)
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or(AuthenticationError)?;
        let (workspace_id, environment_id) = self
            .store
            .provision_oidc_tenant(
                &self.configuration.provider,
                organization_id,
                &self.configuration.environment_kind,
            )
            .await
            .map_err(|_| AuthenticationError)?;
        let permissions = if self.configuration.allow_unrestricted {
            Permissions::ALL
        } else {
            claims
                .values
                .get(&self.configuration.permissions_claim)
                .and_then(serde_json::Value::as_array)
                .map(|values| {
                    Permissions::from_names(
                        &values
                            .iter()
                            .filter_map(serde_json::Value::as_str)
                            .map(str::to_owned)
                            .collect::<Vec<_>>(),
                    )
                })
                .unwrap_or_default()
        };
        Ok(TenantContext {
            workspace_id,
            environment_id,
            permissions,
        })
    }

    async fn key_for_id(
        &self,
        key_id: &str,
    ) -> Result<jsonwebtoken::jwk::Jwk, AuthenticationError> {
        if let Some(cached) = self.jwks.read().await.as_ref()
            && cached.fetched_at.elapsed() < Duration::from_secs(300)
            && let Some(key) = cached.keys.find(key_id)
        {
            return Ok(key.clone());
        }
        let keys = self
            .client
            .get(&self.configuration.jwks_url)
            .send()
            .await
            .map_err(|_| AuthenticationError)?
            .error_for_status()
            .map_err(|_| AuthenticationError)?
            .json::<JwkSet>()
            .await
            .map_err(|_| AuthenticationError)?;
        let key = keys.find(key_id).cloned().ok_or(AuthenticationError)?;
        *self.jwks.write().await = Some(CachedJwks {
            fetched_at: Instant::now(),
            keys,
        });
        Ok(key)
    }
}

#[async_trait]
impl Authenticator for OidcAuthenticator {
    async fn authenticate_bearer(
        &self,
        authorization: &str,
    ) -> Result<TenantContext, AuthenticationError> {
        let token = authorization
            .strip_prefix("Bearer ")
            .filter(|value| !value.is_empty())
            .ok_or(AuthenticationError)?;
        self.authenticate_token(token).await
    }

    async fn authenticate_basic(
        &self,
        _authorization: &str,
    ) -> Result<TenantContext, AuthenticationError> {
        Err(AuthenticationError)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oidc_scope_mapping_denies_mutations_by_default() {
        let tenant = TenantContext {
            workspace_id: WorkspaceId::new(),
            environment_id: EnvironmentId::new(),
            permissions: Permissions::from_names(&["jobs_read".into()]),
        };
        assert!(tenant.allows(Scope::JobsRead));
        assert!(!tenant.allows(Scope::JobsWrite));
        assert!(!tenant.allows(Scope::WebhooksWrite));
    }
}
