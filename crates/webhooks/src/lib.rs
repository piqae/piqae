//! Native webhook signing and deterministic retry policy.

use base64::{Engine, engine::general_purpose::STANDARD};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::time::Duration;
use subtle::ConstantTimeEq;
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookEnvelope<T> {
    pub id: Uuid,
    #[serde(rename = "type")]
    pub event_type: String,
    pub created_at: OffsetDateTime,
    pub data: T,
}

pub fn signature(secret: &[u8], timestamp: i64, body: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
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
}
