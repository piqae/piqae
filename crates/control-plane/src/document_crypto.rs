use chacha20poly1305::{
    ChaCha20Poly1305, KeyInit,
    aead::{Aead, OsRng, Payload, rand_core::RngCore},
};
use piqae_storage_postgres::{DocumentCiphertextField, PostgresStore, StorageError};
use std::{collections::BTreeMap, fmt};
use thiserror::Error;

const MAGIC: &[u8; 4] = b"PDOC";
const VERSION: u8 = 2;
pub const LEGACY_KEY_ID: &str = "legacy-v1";

#[derive(Clone)]
pub struct DocumentSecretBox {
    active_key_id: String,
    keys: BTreeMap<String, [u8; 32]>,
}

#[derive(Debug, Error)]
pub enum DocumentCryptoError {
    #[error("document encryption failed")]
    Encrypt,
    #[error("document decryption failed")]
    Decrypt,
    #[error("document encryption key is invalid")]
    InvalidKey,
    #[error("document encryption key is unavailable")]
    UnknownKey,
    #[error("document encryption maintenance persistence failed")]
    Storage(#[from] StorageError),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RewrapReport {
    pub references_before: i64,
    pub scanned: usize,
    pub rewrapped: usize,
    pub concurrent_changes: usize,
    pub unreadable: usize,
    pub references_after: i64,
}

impl fmt::Debug for DocumentSecretBox {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DocumentSecretBox")
            .field("active_key_id", &self.active_key_id)
            .field("key_ids", &self.keys.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl DocumentSecretBox {
    #[must_use]
    pub fn new(key: [u8; 32]) -> Self {
        Self {
            active_key_id: LEGACY_KEY_ID.to_owned(),
            keys: BTreeMap::from([(LEGACY_KEY_ID.to_owned(), key)]),
        }
    }

    /// Builds a keyring. Only `active_key_id` encrypts; every other key is
    /// decrypt-only. Removing a key is safe only after the database reference
    /// audit reports zero retained ciphertexts for it.
    ///
    /// # Errors
    ///
    /// Returns [`DocumentCryptoError::InvalidKey`] when an id is malformed or
    /// the active id has no corresponding key material.
    pub fn keyring(
        active_key_id: impl Into<String>,
        keys: impl IntoIterator<Item = (String, [u8; 32])>,
    ) -> Result<Self, DocumentCryptoError> {
        let active_key_id = active_key_id.into();
        let keys = keys.into_iter().collect::<BTreeMap<_, _>>();
        if !valid_key_id(&active_key_id)
            || !keys.contains_key(&active_key_id)
            || keys.keys().any(|id| !valid_key_id(id))
        {
            return Err(DocumentCryptoError::InvalidKey);
        }
        Ok(Self {
            active_key_id,
            keys,
        })
    }

    #[must_use]
    pub fn active_key_id(&self) -> &str {
        &self.active_key_id
    }

    pub fn key_ids(&self) -> impl Iterator<Item = &str> {
        self.keys.keys().map(String::as_str)
    }

    /// Encrypts with the active key and authenticates the resource context.
    ///
    /// # Errors
    ///
    /// Returns an error if the active key is unavailable or encryption fails.
    pub fn encrypt(&self, aad: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, DocumentCryptoError> {
        let key = self
            .keys
            .get(&self.active_key_id)
            .ok_or(DocumentCryptoError::UnknownKey)?;
        let key_id = self.active_key_id.as_bytes();
        let key_id_len = u8::try_from(key_id.len()).map_err(|_| DocumentCryptoError::Encrypt)?;
        let mut nonce = [0_u8; 12];
        OsRng.fill_bytes(&mut nonce);
        let ciphertext = ChaCha20Poly1305::new(key.into())
            .encrypt(
                (&nonce).into(),
                Payload {
                    msg: plaintext,
                    aad,
                },
            )
            .map_err(|_| DocumentCryptoError::Encrypt)?;
        let mut encrypted = Vec::with_capacity(6 + key_id.len() + 12 + ciphertext.len());
        encrypted.extend_from_slice(MAGIC);
        encrypted.push(VERSION);
        encrypted.push(key_id_len);
        encrypted.extend_from_slice(key_id);
        encrypted.extend_from_slice(&nonce);
        encrypted.extend(ciphertext);
        Ok(encrypted)
    }

    /// Decrypts v2 or legacy-v1 ciphertext using its retained key generation.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed/tampered data, mismatched context, or a
    /// key generation that is no longer configured.
    pub fn decrypt(&self, aad: &[u8], encrypted: &[u8]) -> Result<Vec<u8>, DocumentCryptoError> {
        let (key_id, nonce, ciphertext) = if encrypted.starts_with(MAGIC) {
            if encrypted.get(4) != Some(&VERSION) {
                return Err(DocumentCryptoError::Decrypt);
            }
            let key_id_len = usize::from(*encrypted.get(5).ok_or(DocumentCryptoError::Decrypt)?);
            let key_end = 6_usize
                .checked_add(key_id_len)
                .ok_or(DocumentCryptoError::Decrypt)?;
            let nonce_end = key_end
                .checked_add(12)
                .ok_or(DocumentCryptoError::Decrypt)?;
            let key_id = std::str::from_utf8(
                encrypted
                    .get(6..key_end)
                    .ok_or(DocumentCryptoError::Decrypt)?,
            )
            .map_err(|_| DocumentCryptoError::Decrypt)?;
            (
                key_id,
                encrypted
                    .get(key_end..nonce_end)
                    .ok_or(DocumentCryptoError::Decrypt)?,
                encrypted
                    .get(nonce_end..)
                    .ok_or(DocumentCryptoError::Decrypt)?,
            )
        } else {
            let (nonce, ciphertext) = encrypted
                .split_at_checked(12)
                .ok_or(DocumentCryptoError::Decrypt)?;
            (LEGACY_KEY_ID, nonce, ciphertext)
        };
        let key = self
            .keys
            .get(key_id)
            .ok_or(DocumentCryptoError::UnknownKey)?;
        ChaCha20Poly1305::new(key.into())
            .decrypt(
                nonce.into(),
                Payload {
                    msg: ciphertext,
                    aad,
                },
            )
            .map_err(|_| DocumentCryptoError::Decrypt)
    }

    /// Re-encrypts a bounded batch of retained `PostgreSQL` records under the
    /// active key. Calls are restartable and idempotent; compare-and-swap
    /// updates never overwrite concurrent application work.
    ///
    /// # Errors
    ///
    /// Returns an error if a retained value cannot be decrypted/encrypted or
    /// `PostgreSQL` maintenance operations fail.
    pub async fn rewrap_postgres_batch(
        &self,
        store: &PostgresStore,
        old_key_id: &str,
        limit: i64,
        dry_run: bool,
    ) -> Result<RewrapReport, DocumentCryptoError> {
        if old_key_id == self.active_key_id {
            return Err(DocumentCryptoError::InvalidKey);
        }
        let references_before = store
            .document_encryption_key_reference_count(old_key_id)
            .await?;
        let records = store
            .document_ciphertexts_for_rewrap(old_key_id, limit)
            .await?;
        let scanned = records.len();
        let mut rewrapped = 0;
        let mut concurrent_changes = 0;
        let mut unreadable = 0;
        if !dry_run {
            for record in records {
                let workspace = record.workspace_id.to_string();
                let environment = record.environment_id.to_string();
                let aad = match record.field {
                    DocumentCiphertextField::TemplateDraft
                    | DocumentCiphertextField::TemplateRevision => crate::documents::document_aad(
                        &workspace,
                        &environment,
                        &record.aad_resource_id,
                    ),
                    DocumentCiphertextField::RenderInput => crate::documents::render_input_aad(
                        &workspace,
                        &environment,
                        &record.aad_resource_id,
                    ),
                    DocumentCiphertextField::RenderPreviewInput => {
                        crate::documents::preview_render_input_aad(
                            &workspace,
                            &environment,
                            &record.aad_resource_id,
                        )
                    }
                    DocumentCiphertextField::RenderPreviewSpecification => {
                        crate::documents::preview_render_spec_aad(
                            &workspace,
                            &environment,
                            &record.aad_resource_id,
                        )
                    }
                    DocumentCiphertextField::RenderArtifactReference => {
                        crate::documents::artifact_key_aad(
                            &workspace,
                            &environment,
                            &record.aad_resource_id,
                        )
                    }
                };
                let Some(replacement) = self.rewrap_payload(&aad, &record.ciphertext) else {
                    unreadable += 1;
                    continue;
                };
                if store
                    .rewrap_document_ciphertext(&record, &replacement)
                    .await?
                {
                    rewrapped += 1;
                } else {
                    concurrent_changes += 1;
                }
            }
        }
        let references_after = store
            .document_encryption_key_reference_count(old_key_id)
            .await?;
        Ok(RewrapReport {
            references_before,
            scanned,
            rewrapped,
            concurrent_changes,
            unreadable,
            references_after,
        })
    }

    fn rewrap_payload(&self, aad: &[u8], ciphertext: &[u8]) -> Option<Vec<u8>> {
        let plaintext = self.decrypt(aad, ciphertext).ok()?;
        self.encrypt(aad, &plaintext).ok()
    }
}

fn valid_key_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn legacy_ciphertext(
        key: [u8; 32],
        aad: &[u8],
        plaintext: &[u8],
    ) -> Result<Vec<u8>, DocumentCryptoError> {
        let nonce = [7_u8; 12];
        let ciphertext = ChaCha20Poly1305::new((&key).into())
            .encrypt(
                (&nonce).into(),
                Payload {
                    msg: plaintext,
                    aad,
                },
            )
            .map_err(|_| DocumentCryptoError::Encrypt)?;
        Ok([nonce.as_slice(), ciphertext.as_slice()].concat())
    }

    #[test]
    fn rotation_keeps_queued_ciphertexts_decryptable() -> Result<(), DocumentCryptoError> {
        let aad = b"wsp_a/env_a/render_queued";
        let old = DocumentSecretBox::keyring("key-2026-01", [("key-2026-01".into(), [1; 32])])?;
        let queued = old.encrypt(aad, b"queued input")?;
        let rotated = DocumentSecretBox::keyring(
            "key-2026-02",
            [
                ("key-2026-01".into(), [1; 32]),
                ("key-2026-02".into(), [2; 32]),
            ],
        )?;
        assert_eq!(rotated.decrypt(aad, &queued)?, b"queued input");
        let new = rotated.encrypt(aad, b"new input")?;
        assert!(
            new.windows("key-2026-02".len())
                .any(|v| v == b"key-2026-02")
        );
        assert_eq!(rotated.decrypt(aad, &new)?, b"new input");
        Ok(())
    }

    #[test]
    fn legacy_v1_remains_readable_during_migration() -> Result<(), DocumentCryptoError> {
        let aad = b"wsp_a/env_a/tpl_a";
        let ciphertext = legacy_ciphertext([9; 32], aad, b"legacy")?;
        let rotated = DocumentSecretBox::keyring(
            "key-2026-02",
            [
                (LEGACY_KEY_ID.into(), [9; 32]),
                ("key-2026-02".into(), [2; 32]),
            ],
        )?;
        assert_eq!(rotated.decrypt(aad, &ciphertext)?, b"legacy");
        Ok(())
    }

    #[test]
    fn missing_retired_key_fails_closed() -> Result<(), DocumentCryptoError> {
        let old = DocumentSecretBox::keyring("old", [("old".into(), [1; 32])])?;
        let ciphertext = old.encrypt(b"aad", b"payload")?;
        let new = DocumentSecretBox::keyring("new", [("new".into(), [2; 32])])?;
        assert!(matches!(
            new.decrypt(b"aad", &ciphertext),
            Err(DocumentCryptoError::UnknownKey)
        ));
        Ok(())
    }

    #[test]
    fn tenant_aad_prevents_cross_tenant_decryption() -> Result<(), DocumentCryptoError> {
        let secret_box = DocumentSecretBox::new([9; 32]);
        let ciphertext = secret_box.encrypt(b"wsp_a/env_a/tpl_a", b"sensitive")?;
        assert_eq!(
            secret_box.decrypt(b"wsp_a/env_a/tpl_a", &ciphertext)?,
            b"sensitive"
        );
        assert!(
            secret_box
                .decrypt(b"wsp_b/env_b/tpl_a", &ciphertext)
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn debug_output_never_contains_key_material() -> Result<(), DocumentCryptoError> {
        let keyring = DocumentSecretBox::keyring(
            "current",
            [
                ("current".into(), [91_u8; 32]),
                ("previous".into(), [37_u8; 32]),
            ],
        )?;
        let debug = format!("{keyring:?}");
        assert!(debug.contains("current"));
        assert!(debug.contains("previous"));
        assert!(!debug.contains("91, 91"));
        assert!(!debug.contains("37, 37"));
        Ok(())
    }

    #[test]
    fn malformed_record_is_classified_unreadable_without_affecting_valid_records()
    -> Result<(), DocumentCryptoError> {
        let old = DocumentSecretBox::keyring("old", [("old".into(), [1_u8; 32])])?;
        let valid = old.encrypt(b"tenant/resource", b"payload")?;
        let rotated = DocumentSecretBox::keyring(
            "new",
            [("old".into(), [1_u8; 32]), ("new".into(), [2_u8; 32])],
        )?;
        assert!(
            rotated
                .rewrap_payload(b"tenant/resource", b"malformed")
                .is_none()
        );
        let replacement = rotated
            .rewrap_payload(b"tenant/resource", &valid)
            .ok_or(DocumentCryptoError::Encrypt)?;
        assert_eq!(
            rotated.decrypt(b"tenant/resource", &replacement)?,
            b"payload"
        );
        Ok(())
    }
}
