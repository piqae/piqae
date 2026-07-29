//! First-party service authentication primitives.

use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Environment {
    Test,
    Live,
}

impl Environment {
    fn key_prefix(self) -> &'static str {
        match self {
            Self::Test => "spl_test_",
            Self::Live => "spl_live_",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    ApiKeysRead,
    ApiKeysWrite,
    AgentsRead,
    AgentsWrite,
    PrintersRead,
    PrintersWrite,
    JobsRead,
    JobsWrite,
    WebhooksRead,
    WebhooksWrite,
    UsageRead,
    AuditRead,
}

impl Scope {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ApiKeysRead => "api_keys_read",
            Self::ApiKeysWrite => "api_keys_write",
            Self::AgentsRead => "agents_read",
            Self::AgentsWrite => "agents_write",
            Self::PrintersRead => "printers_read",
            Self::PrintersWrite => "printers_write",
            Self::JobsRead => "jobs_read",
            Self::JobsWrite => "jobs_write",
            Self::WebhooksRead => "webhooks_read",
            Self::WebhooksWrite => "webhooks_write",
            Self::UsageRead => "usage_read",
            Self::AuditRead => "audit_read",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Principal {
    pub subject: String,
    pub workspace_id: Uuid,
    pub environment: Environment,
    pub scopes: BTreeSet<Scope>,
}

impl Principal {
    pub fn require(&self, scope: Scope) -> Result<(), AuthError> {
        self.scopes
            .contains(&scope)
            .then_some(())
            .ok_or(AuthError::InsufficientScope)
    }
}

#[derive(Debug, Clone)]
pub struct GeneratedApiKey {
    pub id: Uuid,
    pub plaintext: String,
    pub lookup_prefix: String,
    pub password_hash: String,
}

pub struct GeneratedLocalSecret {
    pub id: Uuid,
    pub plaintext: String,
    pub password_hash: String,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AuthError {
    #[error("invalid API key")]
    InvalidKey,
    #[error("insufficient scope")]
    InsufficientScope,
    #[error("key hashing failed")]
    Hashing,
}

pub fn generate_api_key(environment: Environment) -> Result<GeneratedApiKey, AuthError> {
    let mut random = [0_u8; 24];
    OsRng.fill_bytes(&mut random);
    let secret = URL_SAFE_NO_PAD.encode(random);
    let plaintext = format!("{}{}", environment.key_prefix(), secret);
    let lookup_prefix = plaintext.chars().take(17).collect::<String>();
    let salt = SaltString::generate(&mut OsRng);
    let password_hash = Argon2::default()
        .hash_password(plaintext.as_bytes(), &salt)
        .map_err(|_| AuthError::Hashing)?
        .to_string();
    Ok(GeneratedApiKey {
        id: Uuid::now_v7(),
        plaintext,
        lookup_prefix,
        password_hash,
    })
}

pub fn generate_local_owner_credential() -> Result<GeneratedLocalSecret, AuthError> {
    generate_local_secret("spl_owner_")
}

pub fn generate_local_owner_session() -> Result<GeneratedLocalSecret, AuthError> {
    generate_local_secret("spl_session_")
}

pub fn generate_platform_service_account_key() -> Result<GeneratedLocalSecret, AuthError> {
    generate_local_secret("spl_platform_")
}

pub fn rotate_platform_service_account_key(id: Uuid) -> Result<GeneratedLocalSecret, AuthError> {
    generate_local_secret_for_id("spl_platform_", id)
}

fn generate_local_secret(prefix: &str) -> Result<GeneratedLocalSecret, AuthError> {
    generate_local_secret_for_id(prefix, Uuid::now_v7())
}

fn generate_local_secret_for_id(prefix: &str, id: Uuid) -> Result<GeneratedLocalSecret, AuthError> {
    let mut random = [0_u8; 32];
    OsRng.fill_bytes(&mut random);
    let plaintext = format!("{prefix}{id}.{}", URL_SAFE_NO_PAD.encode(random));
    let salt = SaltString::generate(&mut OsRng);
    let password_hash = Argon2::default()
        .hash_password(plaintext.as_bytes(), &salt)
        .map_err(|_| AuthError::Hashing)?
        .to_string();
    Ok(GeneratedLocalSecret {
        id,
        plaintext,
        password_hash,
    })
}

pub fn local_owner_credential_id(plaintext: &str) -> Result<Uuid, AuthError> {
    local_secret_id(plaintext, "spl_owner_")
}

pub fn local_owner_session_id(plaintext: &str) -> Result<Uuid, AuthError> {
    local_secret_id(plaintext, "spl_session_")
}

pub fn platform_service_account_key_id(plaintext: &str) -> Result<Uuid, AuthError> {
    local_secret_id(plaintext, "spl_platform_")
}

fn local_secret_id(plaintext: &str, prefix: &str) -> Result<Uuid, AuthError> {
    let value = plaintext
        .strip_prefix(prefix)
        .and_then(|value| value.split_once('.'))
        .filter(|(_, secret)| {
            secret.len() >= 40 && !secret.contains(char::is_whitespace) && !secret.contains('.')
        })
        .ok_or(AuthError::InvalidKey)?;
    value.0.parse().map_err(|_| AuthError::InvalidKey)
}

pub fn verify_local_owner_credential(plaintext: &str, encoded_hash: &str) -> Result<(), AuthError> {
    local_owner_credential_id(plaintext)?;
    verify_local_secret(plaintext, encoded_hash)
}

pub fn verify_local_owner_session(plaintext: &str, encoded_hash: &str) -> Result<(), AuthError> {
    local_owner_session_id(plaintext)?;
    verify_local_secret(plaintext, encoded_hash)
}

pub fn verify_platform_service_account_key(
    plaintext: &str,
    encoded_hash: &str,
) -> Result<(), AuthError> {
    platform_service_account_key_id(plaintext)?;
    verify_local_secret(plaintext, encoded_hash)
}

fn verify_local_secret(plaintext: &str, encoded_hash: &str) -> Result<(), AuthError> {
    let parsed = PasswordHash::new(encoded_hash).map_err(|_| AuthError::InvalidKey)?;
    Argon2::default()
        .verify_password(plaintext.as_bytes(), &parsed)
        .map_err(|_| AuthError::InvalidKey)
}

pub fn verify_api_key(plaintext: &str, encoded_hash: &str) -> Result<(), AuthError> {
    if !(plaintext.starts_with("spl_test_") || plaintext.starts_with("spl_live_")) {
        return Err(AuthError::InvalidKey);
    }
    let parsed = PasswordHash::new(encoded_hash).map_err(|_| AuthError::InvalidKey)?;
    Argon2::default()
        .verify_password(plaintext.as_bytes(), &parsed)
        .map_err(|_| AuthError::InvalidKey)
}

pub fn api_key_lookup_prefix(plaintext: &str) -> Result<String, AuthError> {
    if !(plaintext.starts_with("spl_test_") || plaintext.starts_with("spl_live_"))
        || plaintext.len() < 17
    {
        return Err(AuthError::InvalidKey);
    }
    Ok(plaintext.chars().take(17).collect())
}

pub fn bearer_token(value: &str) -> Result<&str, AuthError> {
    value
        .strip_prefix("Bearer ")
        .filter(|token| !token.is_empty() && !token.contains(char::is_whitespace))
        .ok_or(AuthError::InvalidKey)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_environment_scoped_and_verifiable() {
        let key = generate_api_key(Environment::Live).unwrap();
        assert!(key.plaintext.starts_with("spl_live_"));
        assert!(verify_api_key(&key.plaintext, &key.password_hash).is_ok());
        assert!(verify_api_key("spl_live_wrong", &key.password_hash).is_err());
    }

    #[test]
    fn scopes_are_denied_by_default() {
        let principal = Principal {
            subject: "test".into(),
            workspace_id: Uuid::now_v7(),
            environment: Environment::Test,
            scopes: BTreeSet::from([Scope::JobsRead]),
        };
        assert!(principal.require(Scope::JobsRead).is_ok());
        assert_eq!(
            principal.require(Scope::JobsWrite),
            Err(AuthError::InsufficientScope)
        );
    }

    #[test]
    fn lookup_prefix_never_contains_the_complete_secret() {
        let key = generate_api_key(Environment::Test).unwrap();
        let prefix = api_key_lookup_prefix(&key.plaintext).unwrap();
        assert_eq!(prefix, key.lookup_prefix);
        assert!(prefix.len() < key.plaintext.len());
    }

    #[test]
    fn local_owner_credentials_are_opaque_argon2_secrets() {
        let credential = generate_local_owner_credential().unwrap();
        assert!(credential.plaintext.starts_with("spl_owner_"));
        assert_eq!(
            local_owner_credential_id(&credential.plaintext),
            Ok(credential.id)
        );
        assert!(
            verify_local_owner_credential(&credential.plaintext, &credential.password_hash).is_ok()
        );
        assert!(
            verify_local_owner_credential(
                &format!("spl_owner_{}.wrong", credential.id),
                &credential.password_hash
            )
            .is_err()
        );
    }

    #[test]
    fn local_owner_sessions_cannot_be_used_as_credentials() {
        let session = generate_local_owner_session().unwrap();
        assert_eq!(local_owner_session_id(&session.plaintext), Ok(session.id));
        assert!(local_owner_credential_id(&session.plaintext).is_err());
        assert!(verify_local_owner_session(&session.plaintext, &session.password_hash).is_ok());
    }

    #[test]
    fn platform_keys_are_distinct_opaque_credentials() {
        let key = generate_platform_service_account_key().unwrap();
        assert!(key.plaintext.starts_with("spl_platform_"));
        assert_eq!(platform_service_account_key_id(&key.plaintext), Ok(key.id));
        assert!(verify_platform_service_account_key(&key.plaintext, &key.password_hash).is_ok());
        assert!(local_owner_session_id(&key.plaintext).is_err());
        assert!(api_key_lookup_prefix(&key.plaintext).is_err());
    }

    #[test]
    fn platform_key_rotation_preserves_identity_and_replaces_secret() {
        let original = generate_platform_service_account_key().unwrap();
        let rotated = rotate_platform_service_account_key(original.id).unwrap();
        assert_eq!(rotated.id, original.id);
        assert_ne!(rotated.plaintext, original.plaintext);
        assert_eq!(
            platform_service_account_key_id(&rotated.plaintext),
            Ok(original.id)
        );
        assert!(
            verify_platform_service_account_key(&rotated.plaintext, &rotated.password_hash).is_ok()
        );
        assert!(
            verify_platform_service_account_key(&original.plaintext, &rotated.password_hash)
                .is_err()
        );
    }
}
