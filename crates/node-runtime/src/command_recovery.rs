//! Durable, bounded command replay bookkeeping shared by installed and
//! embedded node hosts.

use piqae_agent_storage::{AgentStore, StorageError};
use piqae_protocol::agent::AgentCommand;
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{Error as _, MapAccess, Visitor},
};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io;
use std::time::Duration;

const COMMAND_RECOVERY_SETTING: &str = "cloud_command_recovery_v1";
const INITIAL_RETRY_MS: i64 = 30_000;
const MAX_RETRY_MS: i64 = 5 * 60_000;
const MAX_RECOVERY_ENTRIES: usize = 100;
const MAX_ERROR_CODE_LEN: usize = 64;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct CommandRecoveryLedger {
    #[serde(default, deserialize_with = "deserialize_entries")]
    entries: BTreeMap<String, CommandRecoveryEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct CommandRecoveryEntry {
    applied: bool,
    attempts: u32,
    next_retry_unix_ms: Option<i64>,
    last_error_code: Option<String>,
}

impl CommandRecoveryLedger {
    /// Loads only bounded, classification-only recovery data. Command payloads
    /// and credentials are never retained.
    ///
    /// # Errors
    ///
    /// Returns an error when the durable setting cannot be read or decoded.
    pub fn load(store: &AgentStore) -> Result<Self, StorageError> {
        let ledger: Self = store
            .setting(COMMAND_RECOVERY_SETTING)?
            .map(|encoded| serde_json::from_str(&encoded))
            .transpose()
            .map(Option::unwrap_or_default)
            .map_err(StorageError::from)?;
        for (key, entry) in &ledger.entries {
            if key.len() != 64 || !key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(invalid_recovery_data(
                    "cloud command recovery key is invalid",
                ));
            }
            if entry
                .last_error_code
                .as_ref()
                .is_some_and(|code| code.chars().count() > MAX_ERROR_CODE_LEN)
            {
                return Err(invalid_recovery_data(
                    "cloud command recovery error code exceeds its limit",
                ));
            }
        }
        Ok(ledger)
    }

    /// Removes recovery entries that are no longer in the authority batch.
    ///
    /// # Errors
    ///
    /// Returns an error when a command cannot be encoded for its opaque key.
    pub fn retain_batch(&mut self, commands: &[AgentCommand]) -> Result<(), StorageError> {
        let keys = commands
            .iter()
            .map(command_key)
            .collect::<Result<BTreeSet<_>, _>>()?;
        self.entries.retain(|key, _| keys.contains(key));
        Ok(())
    }

    /// Derives a payload-free stable key for one authenticated command.
    ///
    /// # Errors
    ///
    /// Returns an error when the command cannot be encoded.
    pub fn key(command: &AgentCommand) -> Result<String, StorageError> {
        command_key(command)
    }

    #[must_use]
    pub fn is_applied(&self, key: &str) -> bool {
        self.entries.get(key).is_some_and(|entry| entry.applied)
    }

    #[must_use]
    pub fn is_due(&self, key: &str, now_unix_ms: i64) -> bool {
        self.entries
            .get(key)
            .and_then(|entry| entry.next_retry_unix_ms)
            .is_none_or(|next| next <= now_unix_ms)
    }

    pub fn record_applied(&mut self, key: String) {
        self.entries.insert(
            key,
            CommandRecoveryEntry {
                applied: true,
                attempts: 0,
                next_retry_unix_ms: None,
                last_error_code: None,
            },
        );
    }

    pub fn record_retry(&mut self, key: String, now_unix_ms: i64, code: &str) {
        let attempts = self
            .entries
            .get(&key)
            .map_or(1, |entry| entry.attempts.saturating_add(1));
        let exponent = attempts.saturating_sub(1).min(4);
        let delay = INITIAL_RETRY_MS
            .saturating_mul(1_i64.checked_shl(exponent).unwrap_or(16))
            .min(MAX_RETRY_MS);
        self.entries.insert(
            key,
            CommandRecoveryEntry {
                applied: false,
                attempts,
                next_retry_unix_ms: Some(now_unix_ms.saturating_add(delay)),
                last_error_code: Some(code.chars().take(MAX_ERROR_CODE_LEN).collect()),
            },
        );
    }

    #[must_use]
    pub fn retry_after(&self, now_unix_ms: i64) -> Option<Duration> {
        self.entries
            .values()
            .filter(|entry| !entry.applied)
            .filter_map(|entry| entry.next_retry_unix_ms)
            .map(|next| u64::try_from(next.saturating_sub(now_unix_ms).max(1)).unwrap_or(1))
            .min()
            .map(Duration::from_millis)
    }

    #[must_use]
    pub fn complete(&self) -> bool {
        self.entries.values().all(|entry| entry.applied)
    }

    /// Persists bounded recovery classifications without command payloads.
    ///
    /// # Errors
    ///
    /// Returns an error when encoding or durable storage fails.
    pub fn persist(&self, store: &mut AgentStore) -> Result<(), StorageError> {
        store.set_setting(COMMAND_RECOVERY_SETTING, &serde_json::to_string(self)?)
    }

    /// Clears the recovery ledger after the authority cursor is durable.
    ///
    /// # Errors
    ///
    /// Returns an error when the cleared ledger cannot be persisted.
    pub fn clear(&mut self, store: &mut AgentStore) -> Result<(), StorageError> {
        self.entries.clear();
        self.persist(store)
    }
}

fn invalid_recovery_data(message: &str) -> StorageError {
    StorageError::Json(serde_json::Error::io(io::Error::new(
        io::ErrorKind::InvalidData,
        message,
    )))
}

fn deserialize_entries<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<String, CommandRecoveryEntry>, D::Error>
where
    D: Deserializer<'de>,
{
    struct BoundedEntriesVisitor;

    impl<'de> Visitor<'de> for BoundedEntriesVisitor {
        type Value = BTreeMap<String, CommandRecoveryEntry>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("at most 100 cloud command recovery entries")
        }

        fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
        where
            A: MapAccess<'de>,
        {
            if map
                .size_hint()
                .is_some_and(|size| size > MAX_RECOVERY_ENTRIES)
            {
                return Err(A::Error::custom(
                    "cloud command recovery entry count exceeds its limit",
                ));
            }
            let mut entries = BTreeMap::new();
            while let Some((key, entry)) = map.next_entry()? {
                if entries.len() >= MAX_RECOVERY_ENTRIES {
                    return Err(A::Error::custom(
                        "cloud command recovery entry count exceeds its limit",
                    ));
                }
                entries.insert(key, entry);
            }
            Ok(entries)
        }
    }

    deserializer.deserialize_map(BoundedEntriesVisitor)
}

fn command_key(command: &AgentCommand) -> Result<String, StorageError> {
    let encoded = serde_json::to_vec(command)?;
    Ok(hex::encode(Sha256::digest(encoded)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context as _;

    #[test]
    fn retry_is_durable_bounded_and_payload_free() -> anyhow::Result<()> {
        let mut store = AgentStore::in_memory()?;
        let job_id = piqae_domain::JobId::new();
        let command = AgentCommand::CancelJob { job_id };
        let key = CommandRecoveryLedger::key(&command)?;
        let mut ledger = CommandRecoveryLedger::load(&store)?;
        ledger.record_retry(key.clone(), 1_000, "local_store_retry");
        ledger.persist(&mut store)?;
        let restarted = CommandRecoveryLedger::load(&store)?;
        assert!(!restarted.is_due(&key, 1_001));
        assert_eq!(restarted.retry_after(1_000), Some(Duration::from_secs(30)));
        let encoded = store
            .setting(COMMAND_RECOVERY_SETTING)?
            .context("recovery setting")?;
        assert!(!encoded.contains(&job_id.to_string()));
        Ok(())
    }

    #[test]
    fn restart_rejects_oversized_or_corrupt_recovery_state() -> anyhow::Result<()> {
        let mut store = AgentStore::in_memory()?;
        let entry = serde_json::json!({
            "applied": false,
            "attempts": 1,
            "next_retry_unix_ms": 31_000,
            "last_error_code": "local_store_retry",
        });
        let entries = (0..=MAX_RECOVERY_ENTRIES)
            .map(|index| (format!("{index:064x}"), entry.clone()))
            .collect::<serde_json::Map<_, _>>();
        store.set_setting(
            COMMAND_RECOVERY_SETTING,
            &serde_json::json!({"entries": entries}).to_string(),
        )?;
        assert!(CommandRecoveryLedger::load(&store).is_err());

        store.set_setting(
            COMMAND_RECOVERY_SETTING,
            &serde_json::json!({
                "entries": {
                    "0".repeat(64): {
                        "applied": false,
                        "attempts": 1,
                        "next_retry_unix_ms": 31_000,
                        "last_error_code": "x".repeat(MAX_ERROR_CODE_LEN + 1),
                    }
                }
            })
            .to_string(),
        )?;
        assert!(CommandRecoveryLedger::load(&store).is_err());
        Ok(())
    }
}
