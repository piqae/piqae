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

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
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

pub fn verify_api_key(plaintext: &str, encoded_hash: &str) -> Result<(), AuthError> {
    if !(plaintext.starts_with("spl_test_") || plaintext.starts_with("spl_live_")) {
        return Err(AuthError::InvalidKey);
    }
    let parsed = PasswordHash::new(encoded_hash).map_err(|_| AuthError::InvalidKey)?;
    Argon2::default()
        .verify_password(plaintext.as_bytes(), &parsed)
        .map_err(|_| AuthError::InvalidKey)
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
}
