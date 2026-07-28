//! Native webhook signing and deterministic retry policy.

use base64::{Engine, engine::general_purpose::STANDARD};
use chacha20poly1305::{
    ChaCha20Poly1305, KeyInit,
    aead::{Aead, OsRng, rand_core::RngCore},
};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::time::Duration;
use subtle::ConstantTimeEq;
use thiserror::Error;
use time::OffsetDateTime;
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

pub const RETRY_DELAYS: [Duration; 8] = [
    Duration::from_secs(0),
    Duration::from_secs(5),
    Duration::from_secs(30),
    Duration::from_secs(120),
    Duration::from_secs(600),
    Duration::from_secs(3_600),
    Duration::from_secs(21_600),
    Duration::from_secs(86_400),
];

#[derive(Clone, Debug)]
pub struct WebhookSecretBox {
    key: [u8; 32],
}

#[derive(Debug, Error)]
pub enum SecretBoxError {
    #[error("webhook secret encryption failed")]
    Encrypt,
    #[error("webhook secret decryption failed")]
    Decrypt,
}

impl WebhookSecretBox {
    #[must_use]
    pub const fn new(key: [u8; 32]) -> Self {
        Self { key }
    }

    pub fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, SecretBoxError> {
        let cipher = ChaCha20Poly1305::new((&self.key).into());
        let mut nonce = [0_u8; 12];
        OsRng.fill_bytes(&mut nonce);
        let encrypted = cipher
            .encrypt((&nonce).into(), plaintext)
            .map_err(|_| SecretBoxError::Encrypt)?;
        let mut result = Vec::with_capacity(nonce.len() + encrypted.len());
        result.extend_from_slice(&nonce);
        result.extend_from_slice(&encrypted);
        Ok(result)
    }

    pub fn decrypt(&self, encrypted: &[u8]) -> Result<Vec<u8>, SecretBoxError> {
        let (nonce, ciphertext) = encrypted
            .split_at_checked(12)
            .ok_or(SecretBoxError::Decrypt)?;
        ChaCha20Poly1305::new((&self.key).into())
            .decrypt(nonce.into(), ciphertext)
            .map_err(|_| SecretBoxError::Decrypt)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookEnvelope<T> {
    pub id: Uuid,
    #[serde(rename = "type")]
    pub event_type: String,
    pub created_at: OffsetDateTime,
    pub data: T,
}

pub fn signature(secret: &[u8], timestamp: i64, body: &[u8]) -> String {
    let mut mac = <HmacSha256 as Mac>::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);
    STANDARD.encode(mac.finalize().into_bytes())
}

pub fn verify(secret: &[u8], timestamp: i64, body: &[u8], supplied: &str) -> bool {
    let expected = signature(secret, timestamp, body);
    expected.as_bytes().ct_eq(supplied.as_bytes()).into()
}

pub fn retry_delay(attempt: usize) -> Option<Duration> {
    RETRY_DELAYS.get(attempt).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signatures_cover_timestamp_and_raw_body() {
        let body = br#"{"job":"one"}"#;
        let signed = signature(b"secret", 42, body);
        assert!(verify(b"secret", 42, body, &signed));
        assert!(!verify(b"secret", 43, body, &signed));
        assert!(!verify(b"secret", 42, br#"{"job":"two"}"#, &signed));
    }

    #[test]
    fn retry_schedule_ends_in_dead_letter() {
        assert_eq!(retry_delay(0), Some(Duration::ZERO));
        assert_eq!(retry_delay(7), Some(Duration::from_secs(86_400)));
        assert_eq!(retry_delay(8), None);
    }

    #[test]
    fn secrets_are_encrypted_with_authenticated_ciphertext() {
        let secret_box = WebhookSecretBox::new([7; 32]);
        let encrypted = secret_box.encrypt(b"signing secret").unwrap();
        assert_ne!(encrypted, b"signing secret");
        assert_eq!(secret_box.decrypt(&encrypted).unwrap(), b"signing secret");
        let mut tampered = encrypted;
        tampered[15] ^= 1;
        assert!(secret_box.decrypt(&tampered).is_err());
    }
}
