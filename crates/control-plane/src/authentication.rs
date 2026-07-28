use async_trait::async_trait;
use base64::{Engine, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha256};
use spool_domain::{EnvironmentId, WorkspaceId};
use std::{collections::HashMap, sync::Arc};
use subtle::ConstantTimeEq;
use thiserror::Error;
use tokio::sync::RwLock;

#[derive(Clone, Copy, Debug)]
pub struct TenantContext {
    pub workspace_id: WorkspaceId,
    pub environment_id: EnvironmentId,
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
