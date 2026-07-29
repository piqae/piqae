use async_trait::async_trait;
use base64::{Engine, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha256};
use spool_auth::{Scope, api_key_lookup_prefix, verify_api_key};
use spool_domain::{EnvironmentId, WorkspaceId};
use spool_storage_postgres::PostgresStore;
use std::{collections::HashMap, sync::Arc};
use subtle::ConstantTimeEq;
use thiserror::Error;
use tokio::sync::RwLock;

#[derive(Clone, Copy, Debug)]
pub struct TenantContext {
    pub workspace_id: WorkspaceId,
    pub environment_id: EnvironmentId,
    permissions: Permissions,
}

#[derive(Clone, Copy, Debug, Default)]
struct Permissions(u16);

impl Permissions {
    const ALL: Self = Self(u16::MAX);

    fn from_names(names: &[String]) -> Self {
        let mut value = 0;
        for name in names {
            value |= match name.as_str() {
                "agents_read" => 1 << 0,
                "agents_write" => 1 << 1,
                "printers_read" => 1 << 2,
                "printers_write" => 1 << 3,
                "jobs_read" => 1 << 4,
                "jobs_write" => 1 << 5,
                "webhooks_read" => 1 << 6,
                "webhooks_write" => 1 << 7,
                "usage_read" => 1 << 8,
                "audit_read" => 1 << 9,
                _ => 0,
            };
        }
        Self(value)
    }

    const fn allows(self, scope: &Scope) -> bool {
        let bit = match scope {
            Scope::AgentsRead => 1 << 0,
            Scope::AgentsWrite => 1 << 1,
            Scope::PrintersRead => 1 << 2,
            Scope::PrintersWrite => 1 << 3,
            Scope::JobsRead => 1 << 4,
            Scope::JobsWrite => 1 << 5,
            Scope::WebhooksRead => 1 << 6,
            Scope::WebhooksWrite => 1 << 7,
            Scope::UsageRead => 1 << 8,
            Scope::AuditRead => 1 << 9,
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
    pub const fn allows(self, scope: &Scope) -> bool {
        self.permissions.allows(scope)
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
}

#[derive(Clone, Debug)]
pub struct CombinedAuthenticator {
    postgres: PostgresAuthenticator,
    bootstrap: Option<StaticAuthenticator>,
}

impl CombinedAuthenticator {
    #[must_use]
    pub const fn new(
        postgres: PostgresAuthenticator,
        bootstrap: Option<StaticAuthenticator>,
    ) -> Self {
        Self {
            postgres,
            bootstrap,
        }
    }
}

#[async_trait]
impl Authenticator for CombinedAuthenticator {
    async fn authenticate_bearer(
        &self,
        authorization: &str,
    ) -> Result<TenantContext, AuthenticationError> {
        match self.postgres.authenticate_bearer(authorization).await {
            Ok(tenant) => Ok(tenant),
            Err(_) => match &self.bootstrap {
                Some(bootstrap) => bootstrap.authenticate_bearer(authorization).await,
                None => Err(AuthenticationError),
            },
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
}
