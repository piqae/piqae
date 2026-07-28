//! Verification for Spool's signed, staged update metadata.
//!
//! Root trust is supplied by the package or administrator. This crate performs
//! no network or installation operations and therefore remains independently
//! testable.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use chrono::{DateTime, Utc};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReleaseMetadata {
    pub role: MetadataRole,
    pub version: u64,
    pub expires_at: DateTime<Utc>,
    pub channel: UpdateChannel,
    pub release: Version,
    pub rollout_percent: u8,
    pub targets: Vec<UpdateTarget>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MetadataRole {
    Root,
    Targets,
    Snapshot,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateChannel {
    Canary,
    Stable,
    Pinned,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct UpdateTarget {
    pub platform: String,
    pub architecture: String,
    pub url: String,
    pub sha256: String,
    pub length: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SignedMetadata {
    pub key_id: String,
    pub signed: ReleaseMetadata,
    /// Standard base64-encoded Ed25519 signature over canonical struct JSON.
    pub signature: String,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum VerificationError {
    #[error("metadata key id does not match trusted key")]
    KeyIdMismatch,
    #[error("metadata signature is not valid base64")]
    InvalidSignatureEncoding,
    #[error("metadata signature is invalid")]
    InvalidSignature,
    #[error("metadata serialization failed")]
    Serialization,
    #[error("metadata expired at {0}")]
    Expired(DateTime<Utc>),
    #[error("metadata rollout must be between 0 and 100 percent")]
    InvalidRollout,
    #[error("metadata version {received} would roll back trusted version {trusted}")]
    MetadataRollback { trusted: u64, received: u64 },
    #[error("release {received} would roll back installed release {installed}")]
    ReleaseRollback {
        installed: Version,
        received: Version,
    },
    #[error("target checksum is not a lowercase SHA-256 digest")]
    InvalidTargetChecksum,
    #[error("target URL must use HTTPS")]
    InsecureTargetUrl,
}

impl SignedMetadata {
    /// Verifies trust, expiry, rollback protection, target policy, and the
    /// Ed25519 signature.
    ///
    /// # Errors
    ///
    /// Returns [`VerificationError`] when any trust or policy check fails.
    pub fn verify(
        &self,
        trusted_key: &VerifyingKey,
        trusted_metadata_version: u64,
        installed_release: &Version,
        now: DateTime<Utc>,
    ) -> Result<(), VerificationError> {
        if self.key_id != key_id(trusted_key) {
            return Err(VerificationError::KeyIdMismatch);
        }
        if self.signed.expires_at <= now {
            return Err(VerificationError::Expired(self.signed.expires_at));
        }
        if self.signed.rollout_percent > 100 {
            return Err(VerificationError::InvalidRollout);
        }
        if self.signed.version < trusted_metadata_version {
            return Err(VerificationError::MetadataRollback {
                trusted: trusted_metadata_version,
                received: self.signed.version,
            });
        }
        if self.signed.release < *installed_release {
            return Err(VerificationError::ReleaseRollback {
                installed: installed_release.clone(),
                received: self.signed.release.clone(),
            });
        }
        for target in &self.signed.targets {
            if !is_sha256(&target.sha256) {
                return Err(VerificationError::InvalidTargetChecksum);
            }
            if !target.url.starts_with("https://") {
                return Err(VerificationError::InsecureTargetUrl);
            }
        }

        let payload =
            serde_json::to_vec(&self.signed).map_err(|_| VerificationError::Serialization)?;
        let encoded = STANDARD
            .decode(&self.signature)
            .map_err(|_| VerificationError::InvalidSignatureEncoding)?;
        let signature = Signature::from_slice(&encoded)
            .map_err(|_| VerificationError::InvalidSignatureEncoding)?;
        trusted_key
            .verify(&payload, &signature)
            .map_err(|_| VerificationError::InvalidSignature)
    }
}

#[must_use]
pub fn key_id(key: &VerifyingKey) -> String {
    let digest = Sha256::digest(key.as_bytes());
    hex::encode(&digest[..16])
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::{
        MetadataRole, ReleaseMetadata, SignedMetadata, UpdateChannel, UpdateTarget,
        VerificationError, key_id,
    };
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use chrono::{Duration, Utc};
    use ed25519_dalek::{Signer, SigningKey};
    use semver::Version;

    fn signed() -> (SignedMetadata, SigningKey) {
        let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
        let metadata = ReleaseMetadata {
            role: MetadataRole::Targets,
            version: 4,
            expires_at: Utc::now() + Duration::hours(1),
            channel: UpdateChannel::Stable,
            release: Version::new(1, 2, 3),
            rollout_percent: 25,
            targets: vec![UpdateTarget {
                platform: "windows".into(),
                architecture: "x86_64".into(),
                url: "https://updates.example.invalid/spool.msi".into(),
                sha256: "a".repeat(64),
                length: 42,
            }],
        };
        let payload = serde_json::to_vec(&metadata).unwrap_or_default();
        let signature = signing_key.sign(&payload);
        (
            SignedMetadata {
                key_id: key_id(&signing_key.verifying_key()),
                signed: metadata,
                signature: STANDARD.encode(signature.to_bytes()),
            },
            signing_key,
        )
    }

    #[test]
    fn accepts_valid_forward_metadata() {
        let (metadata, signing_key) = signed();
        assert_eq!(
            metadata.verify(
                &signing_key.verifying_key(),
                3,
                &Version::new(1, 0, 0),
                Utc::now(),
            ),
            Ok(())
        );
    }

    #[test]
    fn rejects_tampering_and_rollback() {
        let (mut metadata, signing_key) = signed();
        metadata.signed.rollout_percent = 26;
        assert_eq!(
            metadata.verify(
                &signing_key.verifying_key(),
                3,
                &Version::new(1, 0, 0),
                Utc::now(),
            ),
            Err(VerificationError::InvalidSignature)
        );

        let (metadata, signing_key) = signed();
        assert!(matches!(
            metadata.verify(
                &signing_key.verifying_key(),
                5,
                &Version::new(1, 0, 0),
                Utc::now(),
            ),
            Err(VerificationError::MetadataRollback { .. })
        ));
    }
}
