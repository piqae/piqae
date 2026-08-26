//! Durable local responsibility boundary for the Piqae agent.
//!
//! A job is accepted only by [`AgentStore::accept_job`] which atomically
//! records the inbox receipt, job, per-printer FIFO sequence, initial event,
//! content reference, and outbound acknowledgement.

use piqae_domain::{
    DriverFingerprint, EventId, JobOptions, NativePrinterOption, NativeProfileKind,
    PRINTER_PROFILE_SCHEMA_VERSION, PrinterCapabilities, PrinterCapabilityProfile, PrinterId,
    ProfileDependency, ProfileId, ProfileStatus, ProfileSummary, SafeProfileOverride,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::path::Path;
use thiserror::Error;

const SCHEMA: &str = include_str!("../schema.sql");

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("`SQLite` error: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("job {0} already exists with different immutable fields")]
    JobConflict(String),
    #[error("job {0} was not found")]
    JobNotFound(String),
    #[error("event sequence conflict for job {job_id}: expected {expected}, got {actual}")]
    EventSequence {
        job_id: String,
        expected: i64,
        actual: i64,
    },
    #[error("invalid durable local event: {0}")]
    InvalidLocalEvent(String),
    #[error("printer {0} was not found")]
    PrinterNotFound(String),
    #[error("printer profile revision conflict: expected {expected}, current {current}")]
    ProfileRevisionConflict { expected: u64, current: u64 },
    #[error("invalid printer profile: {0}")]
    InvalidPrinterProfile(String),
    #[error("profile capture session {0} was not found")]
    CaptureSessionNotFound(String),
    #[error("profile capture session {0} is no longer authorized")]
    CaptureSessionNotAuthorized(String),
    #[error("profile capture token is invalid")]
    InvalidCaptureToken,
    #[error("native profile blob exceeds the {0} byte limit")]
    NativeBlobTooLarge(usize),
    #[error("document resource {0} was not found")]
    DocumentResourceNotFound(String),
    #[error("content {0} is being reclaimed")]
    ContentReclaimInProgress(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredDocumentResource {
    pub digest: String,
    pub media_type: String,
    pub byte_length: u64,
    pub relative_path: String,
    pub verified_unix_ms: i64,
    pub last_accessed_unix_ms: i64,
    pub reference_count: u64,
}

pub const MAX_NATIVE_PROFILE_BLOB_BYTES: usize = 1024 * 1024;
const MAX_PROFILE_CAPTURE_SESSION_LIFETIME_MS: i64 = 10 * 60 * 1000;

fn map_document_resource(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredDocumentResource> {
    let byte_length: i64 = row.get(2)?;
    let reference_count: i64 = row.get(6)?;
    let byte_length = u64::try_from(byte_length).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            2,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })?;
    let reference_count = u64::try_from(reference_count).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            6,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })?;
    Ok(StoredDocumentResource {
        digest: row.get(0)?,
        media_type: row.get(1)?,
        byte_length,
        relative_path: row.get(3)?,
        verified_unix_ms: row.get(4)?,
        last_accessed_unix_ms: row.get(5)?,
        reference_count,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptedJob {
    pub job_id: String,
    pub submission_id: String,
    pub printer_id: String,
    pub printer_native_id: String,
    pub title: String,
    pub content_sha256: String,
    pub content_path: String,
    pub content_kind: String,
    pub options_json: String,
    pub expires_unix_ms: Option<i64>,
    pub accepted_unix_ms: i64,
    pub cloud_managed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalJob {
    pub job_id: String,
    pub submission_id: String,
    pub printer_id: String,
    pub printer_native_id: String,
    pub printer_sequence: i64,
    pub title: String,
    pub content_sha256: String,
    pub content_path: String,
    pub content_kind: String,
    pub options_json: String,
    pub state: String,
    pub expires_unix_ms: Option<i64>,
    pub native_job_id: Option<String>,
    pub target_id: Option<String>,
    pub binding_id: Option<String>,
    pub profile_id: Option<String>,
    pub profile_revision: Option<u64>,
    pub stock_id: Option<String>,
    pub loaded_media_snapshot_json: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingEvent {
    pub outbox_sequence: i64,
    pub event_id: String,
    pub job_id: String,
    pub job_sequence: i64,
    pub state: String,
    pub reason: Option<String>,
    pub message: Option<String>,
    pub details_json: String,
    pub observed_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredPrinter {
    pub printer_id: String,
    pub native_id: String,
    pub name: String,
    pub state: String,
    pub is_default: bool,
    pub present: bool,
    pub exposed: bool,
    pub capabilities_json: String,
    pub native_options_json: String,
    pub profile_revision: u64,
    pub observed_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredPrinterProfile {
    pub printer_id: String,
    pub revision: u64,
    pub schema_version: u16,
    pub portable_json: String,
    pub native_options_json: String,
    pub observed_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredNamedProfile {
    pub profile_id: String,
    pub printer_id: String,
    pub revision: u64,
    pub name: String,
    pub is_default: bool,
    pub options_json: String,
    pub status: String,
    pub native_kind: String,
    pub native_blob_id: Option<String>,
    pub native_digest: Option<String>,
    pub driver_fingerprint_json: String,
    pub summary_json: String,
    pub stock_id: Option<String>,
    pub safe_overrides_json: String,
    pub last_validated_unix_ms: Option<i64>,
    pub last_test_job_id: Option<String>,
    pub published: bool,
    /// This revision intentionally delegates non-job settings to the
    /// printer driver's current defaults instead of replaying a snapshot.
    pub uses_current_printer_defaults: bool,
    pub updated_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeProfileCapture {
    pub name: String,
    pub is_default: bool,
    pub options_json: String,
    pub status: String,
    pub native_kind: String,
    pub native_schema_version: u16,
    pub native_digest: String,
    pub native_blob: Vec<u8>,
    pub driver_fingerprint_json: String,
    pub summary_json: String,
    pub stock_id: Option<String>,
    pub dependencies_json: String,
    pub safe_overrides_json: String,
    pub published: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredNativeProfileBlob {
    pub blob_id: String,
    pub profile_id: String,
    pub profile_revision: u64,
    pub native_kind: String,
    pub schema_version: u16,
    pub digest: String,
    pub native_blob: Vec<u8>,
    pub created_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredCaptureSession {
    pub session_id: String,
    pub printer_id: String,
    pub profile_id: Option<String>,
    pub expected_revision: Option<u64>,
    pub operation: String,
    pub status: String,
    pub peer_user_id: String,
    pub expires_unix_ms: i64,
    pub created_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredStock {
    pub stock_id: String,
    pub name: String,
    pub sku: Option<String>,
    pub kind: String,
    pub definition_json: String,
    pub retired: bool,
    pub updated_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredLoadedMedia {
    pub device_id: String,
    pub source: String,
    pub stock_id: Option<String>,
    pub confidence: String,
    pub confirmed_unix_ms: i64,
    pub confirmed_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredTarget {
    pub target_id: String,
    pub name: String,
    pub stock_id: Option<String>,
    pub routing_policy: String,
    pub published: bool,
    pub retired: bool,
    pub updated_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredTargetBinding {
    pub binding_id: String,
    pub target_id: String,
    pub agent_id: String,
    pub printer_id: String,
    pub profile_id: String,
    pub profile_revision: u64,
    pub role: String,
    pub priority: u16,
    pub enabled: bool,
    pub created_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReconciliationItem {
    pub job: LocalJob,
    pub next_observe_unix_ms: i64,
    pub uncertainty_deadline_unix_ms: i64,
    pub attempt_count: u32,
    pub cancel_requested: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueCounts {
    pub queued: u32,
    pub active: u32,
}

#[derive(Clone, PartialEq, Eq)]
pub struct CloudAcceptIntent {
    pub job_id: String,
    pub lease_id: String,
    pub lease_token: String,
    pub lease_expires_unix_ms: i64,
    pub content_sha256: String,
    pub local_sequence: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfidentialFile {
    pub job_id: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReclaimableContent {
    pub sha256: String,
    pub path: String,
}

impl std::fmt::Debug for CloudAcceptIntent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CloudAcceptIntent")
            .field("job_id", &self.job_id)
            .field("lease_id", &self.lease_id)
            .field("lease_token", &"[REDACTED]")
            .field("lease_expires_unix_ms", &self.lease_expires_unix_ms)
            .field("content_sha256", &self.content_sha256)
            .field("local_sequence", &self.local_sequence)
            .finish()
    }
}

#[derive(Debug)]
pub struct AgentStore {
    connection: Connection,
}

impl AgentStore {
    /// Records a digest-verified document resource. Callers must publish the
    /// corresponding file atomically before this transaction commits.
    ///
    /// # Errors
    /// Returns an error when `SQLite` cannot persist the metadata.
    pub fn upsert_document_resource(
        &self,
        resource: &StoredDocumentResource,
    ) -> Result<(), StorageError> {
        self.connection.execute(
            "INSERT INTO document_resources(
               digest, media_type, byte_length, relative_path, verified_unix_ms,
               last_accessed_unix_ms, reference_count
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(digest) DO UPDATE SET
               media_type=excluded.media_type,
               byte_length=excluded.byte_length,
               relative_path=excluded.relative_path,
               verified_unix_ms=excluded.verified_unix_ms,
               last_accessed_unix_ms=excluded.last_accessed_unix_ms,
               evicting=0",
            params![
                resource.digest,
                resource.media_type,
                i64::try_from(resource.byte_length).unwrap_or(i64::MAX),
                resource.relative_path,
                resource.verified_unix_ms,
                resource.last_accessed_unix_ms,
                i64::try_from(resource.reference_count).unwrap_or(i64::MAX),
            ],
        )?;
        Ok(())
    }

    /// Loads resource metadata by digest.
    ///
    /// # Errors
    /// Returns an error when `SQLite` cannot query the metadata.
    pub fn document_resource(
        &self,
        digest: &str,
    ) -> Result<Option<StoredDocumentResource>, StorageError> {
        self.connection
            .query_row(
                "SELECT digest, media_type, byte_length, relative_path,
                        verified_unix_ms, last_accessed_unix_ms, reference_count
                 FROM document_resources WHERE digest=?1",
                [digest],
                map_document_resource,
            )
            .optional()
            .map_err(StorageError::from)
    }

    /// Updates LRU access time.
    ///
    /// # Errors
    /// Returns an error for an unknown digest or `SQLite` failure.
    pub fn touch_document_resource(&self, digest: &str, now: i64) -> Result<(), StorageError> {
        let changed = self.connection.execute(
            "UPDATE document_resources SET last_accessed_unix_ms=?2 WHERE digest=?1",
            params![digest, now],
        )?;
        if changed == 0 {
            return Err(StorageError::DocumentResourceNotFound(digest.into()));
        }
        Ok(())
    }

    /// Pins a resource against eviction.
    ///
    /// # Errors
    /// Returns an error for an unknown digest or `SQLite` failure.
    pub fn retain_document_resource(&self, digest: &str) -> Result<(), StorageError> {
        let changed = self.connection.execute(
            "UPDATE document_resources SET reference_count=reference_count+1
             WHERE digest=?1 AND evicting=0",
            [digest],
        )?;
        if changed == 0 {
            return Err(StorageError::DocumentResourceNotFound(digest.into()));
        }
        Ok(())
    }

    /// Releases one durable eviction pin.
    ///
    /// # Errors
    /// Returns an error for an unknown/unpinned digest or `SQLite` failure.
    pub fn release_document_resource(&self, digest: &str) -> Result<(), StorageError> {
        let changed = self.connection.execute(
            "UPDATE document_resources SET reference_count=reference_count-1
             WHERE digest=?1 AND reference_count>0",
            [digest],
        )?;
        if changed == 0 {
            return Err(StorageError::DocumentResourceNotFound(digest.into()));
        }
        Ok(())
    }

    /// Returns the total persisted cache byte count.
    ///
    /// # Errors
    /// Returns an error when `SQLite` cannot calculate usage.
    pub fn document_resource_usage(&self) -> Result<u64, StorageError> {
        let value: i64 = self.connection.query_row(
            "SELECT COALESCE(SUM(byte_length), 0) FROM document_resources",
            [],
            |row| row.get(0),
        )?;
        Ok(u64::try_from(value).unwrap_or_default())
    }

    /// Lists eviction candidates oldest first.
    ///
    /// # Errors
    /// Returns an error when `SQLite` cannot query candidates.
    pub fn unreferenced_document_resources_lru(
        &self,
    ) -> Result<Vec<StoredDocumentResource>, StorageError> {
        let mut statement = self.connection.prepare(
            "SELECT digest, media_type, byte_length, relative_path,
                    verified_unix_ms, last_accessed_unix_ms, reference_count
             FROM document_resources WHERE reference_count=0
             ORDER BY last_accessed_unix_ms, digest",
        )?;
        statement
            .query_map([], map_document_resource)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StorageError::from)
    }

    /// Lists recent cache entries for a bounded readiness snapshot.
    ///
    /// # Errors
    /// Returns an error when `SQLite` cannot query the cache.
    pub fn recent_document_resources(
        &self,
        limit: usize,
    ) -> Result<Vec<StoredDocumentResource>, StorageError> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let mut statement = self.connection.prepare(
            "SELECT digest, media_type, byte_length, relative_path,
                    verified_unix_ms, last_accessed_unix_ms, reference_count
             FROM document_resources WHERE evicting=0
             ORDER BY last_accessed_unix_ms DESC, digest LIMIT ?1",
        )?;
        statement
            .query_map([limit], map_document_resource)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StorageError::from)
    }

    /// Deletes metadata only when no durable reference protects it.
    ///
    /// # Errors
    /// Returns an error when `SQLite` cannot perform the conditional delete.
    pub fn delete_unreferenced_document_resource(
        &self,
        digest: &str,
    ) -> Result<bool, StorageError> {
        Ok(self.connection.execute(
            "DELETE FROM document_resources WHERE digest=?1 AND reference_count=0",
            [digest],
        )? == 1)
    }

    /// Claims an unreferenced resource for filesystem eviction.
    ///
    /// # Errors
    /// Returns an error when `SQLite` cannot perform the atomic claim.
    pub fn claim_document_resource_eviction(&self, digest: &str) -> Result<bool, StorageError> {
        Ok(self.connection.execute(
            "UPDATE document_resources SET evicting=1
             WHERE digest=?1 AND reference_count=0 AND evicting=0",
            [digest],
        )? == 1)
    }

    /// Cancels a failed filesystem eviction claim.
    ///
    /// # Errors
    /// Returns an error when `SQLite` cannot clear the claim.
    pub fn cancel_document_resource_eviction(&self, digest: &str) -> Result<(), StorageError> {
        self.connection.execute(
            "UPDATE document_resources SET evicting=0 WHERE digest=?1",
            [digest],
        )?;
        Ok(())
    }

    /// Deletes metadata after a claimed resource file is gone.
    ///
    /// # Errors
    /// Returns an error when `SQLite` cannot finalize eviction.
    pub fn finish_document_resource_eviction(&self, digest: &str) -> Result<bool, StorageError> {
        Ok(self.connection.execute(
            "DELETE FROM document_resources
             WHERE digest=?1 AND reference_count=0 AND evicting=1",
            [digest],
        )? == 1)
    }

    /// Clears process-local cache guards once, when the cache owner starts.
    ///
    /// # Errors
    /// Returns an error when `SQLite` cannot reset stale guards.
    pub fn reset_document_resource_transient_state(&self) -> Result<(), StorageError> {
        self.connection.execute(
            "UPDATE document_resources SET reference_count=0, evicting=0",
            [],
        )?;
        Ok(())
    }
    /// Opens or creates the durable agent database and applies its schema.
    ///
    /// # Errors
    ///
    /// Returns an error when `SQLite` cannot open, configure, or migrate the
    /// database.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let connection = Connection::open(path)?;
        Self::configure(connection)
    }

    /// Creates a fully configured in-memory store for deterministic tests.
    ///
    /// # Errors
    ///
    /// Returns an error when `SQLite` initialization or migration fails.
    pub fn in_memory() -> Result<Self, StorageError> {
        Self::configure(Connection::open_in_memory()?)
    }

    #[allow(clippy::too_many_lines)]
    fn configure(connection: Connection) -> Result<Self, StorageError> {
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        connection.execute_batch(SCHEMA)?;
        ensure_column(
            &connection,
            "document_resources",
            "evicting",
            "INTEGER NOT NULL DEFAULT 0 CHECK (evicting IN (0, 1))",
        )?;
        ensure_column(
            &connection,
            "content_files",
            "reclaiming",
            "INTEGER NOT NULL DEFAULT 0 CHECK (reclaiming IN (0, 1))",
        )?;
        // A claim is process-transient but persisted to close the delete race.
        // On restart the claimant is gone, so make the file eligible for a
        // fresh existence check and claim/finalize attempt.
        connection.execute(
            "UPDATE content_files SET reclaiming = 0 WHERE reclaiming = 1",
            [],
        )?;
        let has_cloud_managed: bool = connection.query_row(
            "SELECT EXISTS (
               SELECT 1 FROM pragma_table_info('jobs') WHERE name = 'cloud_managed'
             )",
            [],
            |row| row.get(0),
        )?;
        if !has_cloud_managed {
            connection.execute(
                "ALTER TABLE jobs ADD COLUMN cloud_managed INTEGER NOT NULL
                 DEFAULT 0 CHECK (cloud_managed IN (0, 1))",
                [],
            )?;
        }
        let has_confidential: bool = connection.query_row(
            "SELECT EXISTS (SELECT 1 FROM pragma_table_info('jobs') WHERE name = 'confidential')",
            [],
            |row| row.get(0),
        )?;
        if !has_confidential {
            connection.execute("ALTER TABLE jobs ADD COLUMN confidential INTEGER NOT NULL DEFAULT 0 CHECK (confidential IN (0, 1))", [])?;
        }
        let has_confidential_delete_after: bool = connection.query_row(
            "SELECT EXISTS (SELECT 1 FROM pragma_table_info('jobs') WHERE name = 'confidential_delete_after_unix_ms')",
            [],
            |row| row.get(0),
        )?;
        if !has_confidential_delete_after {
            connection.execute(
                "ALTER TABLE jobs ADD COLUMN confidential_delete_after_unix_ms INTEGER",
                [],
            )?;
        }
        // Older releases assigned confidential plaintext an unconditional
        // deletion deadline. A queued or retryable job may still need that
        // file after the deadline, so neutralize legacy deadlines unless the
        // job has reached a truthful terminal state.
        connection.execute(
            "UPDATE jobs SET confidential_delete_after_unix_ms = NULL
             WHERE confidential = 1
               AND state NOT IN ('completed_reported','delivery_uncertain','failed_terminal','cancelled','expired')",
            [],
        )?;
        let has_is_default: bool = connection.query_row(
            "SELECT EXISTS (
               SELECT 1 FROM pragma_table_info('printers') WHERE name = 'is_default'
             )",
            [],
            |row| row.get(0),
        )?;
        if !has_is_default {
            connection.execute(
                "ALTER TABLE printers ADD COLUMN is_default INTEGER NOT NULL
                 DEFAULT 0 CHECK (is_default IN (0, 1))",
                [],
            )?;
        }
        let has_present: bool = connection.query_row(
            "SELECT EXISTS (
               SELECT 1 FROM pragma_table_info('printers') WHERE name = 'present'
             )",
            [],
            |row| row.get(0),
        )?;
        if !has_present {
            connection.execute(
                "ALTER TABLE printers ADD COLUMN present INTEGER NOT NULL
                 DEFAULT 1 CHECK (present IN (0, 1))",
                [],
            )?;
        }
        for (name, definition) in [
            ("status", "TEXT NOT NULL DEFAULT 'needs_test'"),
            ("native_kind", "TEXT NOT NULL DEFAULT 'portable_options'"),
            ("native_blob_id", "TEXT"),
            ("native_digest", "TEXT"),
            (
                "driver_fingerprint_json",
                "TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(driver_fingerprint_json))",
            ),
            (
                "summary_json",
                "TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(summary_json))",
            ),
            ("stock_id", "TEXT"),
            (
                "safe_overrides_json",
                "TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(safe_overrides_json))",
            ),
            ("last_validated_unix_ms", "INTEGER"),
            ("last_test_job_id", "TEXT"),
            (
                "published",
                "INTEGER NOT NULL DEFAULT 0 CHECK (published IN (0, 1))",
            ),
            (
                "uses_current_printer_defaults",
                "INTEGER NOT NULL DEFAULT 0 CHECK (uses_current_printer_defaults IN (0, 1))",
            ),
        ] {
            ensure_column(&connection, "printer_profiles", name, definition)?;
        }
        for (name, definition) in [
            ("target_id", "TEXT"),
            ("binding_id", "TEXT"),
            ("profile_id", "TEXT"),
            ("profile_revision", "INTEGER"),
            ("stock_id", "TEXT"),
            (
                "loaded_media_snapshot_json",
                "TEXT CHECK (loaded_media_snapshot_json IS NULL OR json_valid(loaded_media_snapshot_json))",
            ),
        ] {
            ensure_column(&connection, "jobs", name, definition)?;
        }
        connection.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, applied_unix_ms)
             VALUES (4, CAST(unixepoch('subsec') * 1000 AS INTEGER))",
            [],
        )?;
        Ok(Self { connection })
    }

    /// Runs `SQLite`'s quick integrity check.
    ///
    /// # Errors
    ///
    /// Returns an error if `SQLite` cannot execute the check.
    pub fn integrity_check(&self) -> Result<bool, StorageError> {
        let result: String = self
            .connection
            .query_row("PRAGMA quick_check", [], |row| row.get(0))?;
        Ok(result == "ok")
    }

    /// Returns durable failure counters reported as node health.
    ///
    /// Executor crashes are counted from the append-only job event history, so
    /// restarting the agent cannot make an unhealthy installation appear clean.
    /// The last error is the newest recorded failure reason, if any.
    ///
    /// # Errors
    ///
    /// Returns an error if the durable event history cannot be queried.
    pub fn failure_health(&self) -> Result<(u64, Option<String>), StorageError> {
        let crashes = self
            .setting("executor_crashes")?
            .and_then(|value| value.parse().ok())
            .unwrap_or(0);
        Ok((crashes, self.setting("last_runtime_error_code")?))
    }

    /// Durably records a classified executor failure before job-state mapping
    /// can intentionally replace it with `ambiguous_handoff`.
    ///
    /// # Errors
    ///
    /// Returns an error if the health settings cannot be updated atomically.
    pub fn record_executor_failure(&mut self, code: &str) -> Result<(), StorageError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current: Option<String> = transaction
            .query_row(
                "SELECT value_json FROM settings WHERE key = 'executor_crashes'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let crashes = current
            .and_then(|value| serde_json::from_str::<String>(&value).ok())
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0)
            .saturating_add(u64::from(code == "executor_crashed"));
        for (key, value) in [
            ("executor_crashes", crashes.to_string()),
            ("last_runtime_error_code", code.to_owned()),
        ] {
            transaction.execute(
                "INSERT INTO settings(key, value_json, updated_unix_ms)
                 VALUES (?1, ?2, CAST(unixepoch('subsec') * 1000 AS INTEGER))
                 ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json,
                   updated_unix_ms = excluded.updated_unix_ms",
                params![key, serde_json::to_string(&value)?],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// Reads a durable agent setting.
    ///
    /// # Errors
    ///
    /// Returns an error when `SQLite` cannot execute or decode the query.
    pub fn setting(&self, key: &str) -> Result<Option<String>, StorageError> {
        let encoded: Option<String> = self
            .connection
            .query_row(
                "SELECT value_json FROM settings WHERE key = ?1",
                [key],
                |row| row.get(0),
            )
            .optional()?;
        encoded
            .map(|value| serde_json::from_str(&value))
            .transpose()
            .map_err(Into::into)
    }

    /// Atomically creates or replaces a durable agent setting.
    ///
    /// # Errors
    ///
    /// Returns an error when `SQLite` cannot update the setting.
    pub fn set_setting(&mut self, key: &str, value: &str) -> Result<(), StorageError> {
        let encoded = serde_json::to_string(value)?;
        self.connection.execute(
            "INSERT INTO settings(key, value_json, updated_unix_ms)
             VALUES (?1, ?2, CAST(unixepoch('subsec') * 1000 AS INTEGER))
             ON CONFLICT(key) DO UPDATE SET
               value_json = excluded.value_json,
               updated_unix_ms = excluded.updated_unix_ms",
            params![key, encoded],
        )?;
        Ok(())
    }

    /// Upserts a discovered native printer while preserving its logical ID.
    ///
    /// # Errors
    ///
    /// Returns an error if capabilities are not valid JSON or `SQLite` cannot
    /// atomically persist the inventory record.
    pub fn upsert_printer(
        &mut self,
        native_id: &str,
        name: &str,
        state: &str,
        is_default: bool,
        capabilities_json: &str,
        observed_unix_ms: i64,
    ) -> Result<StoredPrinter, StorageError> {
        let _: serde_json::Value = serde_json::from_str(capabilities_json)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let printer_id = transaction
            .query_row(
                "SELECT printer_id FROM printers WHERE native_id = ?1",
                [native_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .unwrap_or_else(|| PrinterId::new().to_string());
        transaction.execute(
            "INSERT INTO printers(
               printer_id, native_id, name, state, is_default, present,
               observed_unix_ms
             )
             VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6)
             ON CONFLICT(printer_id) DO UPDATE SET
               native_id = excluded.native_id,
               name = excluded.name,
               state = excluded.state,
               is_default = excluded.is_default,
               present = 1,
               observed_unix_ms = excluded.observed_unix_ms",
            params![
                printer_id,
                native_id,
                name,
                state,
                is_default,
                observed_unix_ms
            ],
        )?;
        transaction.execute(
            "INSERT INTO printer_capabilities(
               printer_id, revision, capabilities_json, observed_unix_ms
             ) VALUES (?1, 'current', ?2, ?3)
             ON CONFLICT(printer_id, revision) DO UPDATE SET
               capabilities_json = excluded.capabilities_json,
               observed_unix_ms = excluded.observed_unix_ms",
            params![printer_id, capabilities_json, observed_unix_ms],
        )?;
        transaction.commit()?;
        Ok(StoredPrinter {
            printer_id,
            native_id: native_id.to_owned(),
            name: name.to_owned(),
            state: state.to_owned(),
            is_default,
            present: true,
            exposed: false,
            capabilities_json: capabilities_json.to_owned(),
            native_options_json: "{}".into(),
            profile_revision: 0,
            observed_unix_ms,
        })
    }

    /// Reconciles one successful native discovery as the authoritative set.
    ///
    /// Missing queues are retained for job/profile history but marked absent.
    /// A later upsert or reconciliation restores the same logical printer.
    ///
    /// # Errors
    ///
    /// Returns an error when the presence update cannot be committed.
    pub fn reconcile_printer_presence(
        &mut self,
        present_native_ids: &[String],
    ) -> Result<(), StorageError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute("UPDATE printers SET present = 0", [])?;
        for native_id in present_native_ids {
            transaction.execute(
                "UPDATE printers SET present = 1 WHERE native_id = ?1",
                [native_id],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// Persists a new immutable profile revision when capabilities change.
    ///
    /// `expected_revision` enables optimistic concurrency for administrator
    /// edits. Discovery passes `None` and receives the existing revision when
    /// the profile payload is unchanged.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown printer, invalid profile, stale
    /// revision, or failed transaction.
    pub fn store_printer_profile(
        &mut self,
        printer_id: &str,
        expected_revision: Option<u64>,
        portable_json: &str,
        native_options_json: &str,
        observed_unix_ms: i64,
    ) -> Result<StoredPrinterProfile, StorageError> {
        let portable: PrinterCapabilities = serde_json::from_str(portable_json)?;
        let native_options: BTreeMap<String, NativePrinterOption> =
            serde_json::from_str(native_options_json)?;
        PrinterCapabilityProfile::draft(portable, native_options)
            .validate()
            .map_err(|error| StorageError::InvalidPrinterProfile(error.to_string()))?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM printers WHERE printer_id = ?1)",
            [printer_id],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(StorageError::PrinterNotFound(printer_id.to_owned()));
        }
        let current = transaction
            .query_row(
                "SELECT revision, schema_version, portable_json,
                        native_options_json, observed_unix_ms
                 FROM printer_capability_snapshots
                 WHERE printer_id = ?1 ORDER BY revision DESC LIMIT 1",
                [printer_id],
                |row| {
                    Ok(StoredPrinterProfile {
                        printer_id: printer_id.to_owned(),
                        revision: row.get(0)?,
                        schema_version: row.get(1)?,
                        portable_json: row.get(2)?,
                        native_options_json: row.get(3)?,
                        observed_unix_ms: row.get(4)?,
                    })
                },
            )
            .optional()?;
        let current_revision = current.as_ref().map_or(0, |profile| profile.revision);
        if let Some(expected) = expected_revision
            && expected != current_revision
        {
            return Err(StorageError::ProfileRevisionConflict {
                expected,
                current: current_revision,
            });
        }
        if let Some(profile) = current
            && profile.schema_version == PRINTER_PROFILE_SCHEMA_VERSION
            && profile.portable_json == portable_json
            && profile.native_options_json == native_options_json
        {
            transaction.commit()?;
            return Ok(profile);
        }
        let revision = current_revision.saturating_add(1);
        transaction.execute(
            "INSERT INTO printer_capability_snapshots(
               printer_id, revision, schema_version, portable_json,
               native_options_json, observed_unix_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                printer_id,
                revision,
                PRINTER_PROFILE_SCHEMA_VERSION,
                portable_json,
                native_options_json,
                observed_unix_ms
            ],
        )?;
        transaction.commit()?;
        Ok(StoredPrinterProfile {
            printer_id: printer_id.to_owned(),
            revision,
            schema_version: PRINTER_PROFILE_SCHEMA_VERSION,
            portable_json: portable_json.to_owned(),
            native_options_json: native_options_json.to_owned(),
            observed_unix_ms,
        })
    }

    /// Creates a named print profile at revision one.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid options, an empty name, an unknown
    /// printer, or failed transaction.
    pub fn create_named_profile(
        &mut self,
        printer_id: &str,
        name: &str,
        is_default: bool,
        options_json: &str,
        updated_unix_ms: i64,
    ) -> Result<StoredNamedProfile, StorageError> {
        validate_named_profile(name, options_json)?;
        if self.printer(printer_id)?.is_none() {
            return Err(StorageError::PrinterNotFound(printer_id.to_owned()));
        }
        let profile_id = format!("prf_{}", PrinterId::new());
        self.insert_named_profile(
            &profile_id,
            printer_id,
            1,
            name,
            is_default,
            options_json,
            false,
            updated_unix_ms,
        )?;
        self.named_profile(printer_id, &profile_id)?
            .ok_or_else(|| StorageError::InvalidPrinterProfile("profile insert failed".into()))
    }

    /// Creates the one live driver-default profile for a discovered printer.
    ///
    /// This is idempotent. The generated profile is the job default only when
    /// the printer does not already have an active default profile.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown printer or failed transaction.
    pub fn ensure_current_printer_defaults_profile(
        &mut self,
        printer_id: &str,
        updated_unix_ms: i64,
    ) -> Result<StoredNamedProfile, StorageError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let printer_exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM printers WHERE printer_id = ?1)",
            [printer_id],
            |row| row.get(0),
        )?;
        if !printer_exists {
            return Err(StorageError::PrinterNotFound(printer_id.to_owned()));
        }
        if let Some(profile) = query_current_printer_defaults_profile(&transaction, printer_id)? {
            transaction.commit()?;
            return Ok(profile);
        }

        let profile_id = format!("prf_{}", PrinterId::new());
        let options_json = serde_json::to_string(&JobOptions::default())?;
        let is_default: bool = transaction.query_row(
            "SELECT NOT EXISTS(
               SELECT 1
               FROM printer_profiles p
               WHERE p.printer_id = ?1
                 AND p.is_default = 1
                 AND p.deleted = 0
                 AND p.revision = (
                   SELECT MAX(latest.revision)
                   FROM printer_profiles latest
                   WHERE latest.profile_id = p.profile_id
                 )
             )",
            [printer_id],
            |row| row.get(0),
        )?;
        transaction.execute(
            "INSERT INTO printer_profiles(
               profile_id, printer_id, revision, name, is_default,
               options_json, status, native_kind, native_blob_id,
               native_digest, driver_fingerprint_json, summary_json,
               stock_id, safe_overrides_json, published,
               uses_current_printer_defaults, deleted, updated_unix_ms
             ) VALUES (
               ?1, ?2, 1, 'Current printer defaults', ?3,
               ?4, 'ready', 'portable_options', NULL,
               NULL, '{}', '{}', NULL, '[]', 0, 1, 0, ?5
             )",
            params![
                profile_id,
                printer_id,
                is_default,
                options_json,
                updated_unix_ms
            ],
        )?;
        let profile = query_named_profile(&transaction, printer_id, &profile_id)?
            .ok_or_else(|| StorageError::InvalidPrinterProfile("profile insert failed".into()))?;
        transaction.commit()?;
        Ok(profile)
    }

    /// Appends a named print profile revision using optimistic concurrency.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid options, unknown profile, stale revision,
    /// or failed transaction.
    #[allow(
        clippy::too_many_arguments,
        reason = "keeps immutable profile revision fields explicit at the storage boundary"
    )]
    pub fn update_named_profile(
        &mut self,
        printer_id: &str,
        profile_id: &str,
        expected_revision: u64,
        name: &str,
        is_default: bool,
        options_json: &str,
        updated_unix_ms: i64,
    ) -> Result<StoredNamedProfile, StorageError> {
        validate_named_profile(name, options_json)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = query_named_profile(&transaction, printer_id, profile_id)?
            .ok_or_else(|| StorageError::InvalidPrinterProfile("profile was not found".into()))?;
        if current.revision != expected_revision {
            return Err(StorageError::ProfileRevisionConflict {
                expected: expected_revision,
                current: current.revision,
            });
        }
        insert_named_profile_tx(
            &transaction,
            profile_id,
            printer_id,
            current.revision + 1,
            name.trim(),
            is_default,
            options_json,
            false,
            updated_unix_ms,
        )?;
        transaction.commit()?;
        self.named_profile(printer_id, profile_id)?
            .ok_or_else(|| StorageError::InvalidPrinterProfile("profile update failed".into()))
    }

    /// Appends a tombstone revision for a named profile.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown profile, stale revision, or failed
    /// transaction.
    pub fn delete_named_profile(
        &mut self,
        printer_id: &str,
        profile_id: &str,
        expected_revision: u64,
        updated_unix_ms: i64,
    ) -> Result<(), StorageError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = query_named_profile(&transaction, printer_id, profile_id)?
            .ok_or_else(|| StorageError::InvalidPrinterProfile("profile was not found".into()))?;
        if current.revision != expected_revision {
            return Err(StorageError::ProfileRevisionConflict {
                expected: expected_revision,
                current: current.revision,
            });
        }
        insert_named_profile_tx(
            &transaction,
            profile_id,
            printer_id,
            current.revision + 1,
            &current.name,
            false,
            &current.options_json,
            true,
            updated_unix_ms,
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Returns all active named profiles for a printer.
    ///
    /// # Errors
    ///
    /// Returns an error when `SQLite` cannot execute or decode the query.
    pub fn named_profiles(
        &self,
        printer_id: &str,
    ) -> Result<Vec<StoredNamedProfile>, StorageError> {
        let mut statement = self.connection.prepare(
            "SELECT p.profile_id, p.printer_id, p.revision, p.name,
                    p.is_default, p.options_json, p.status, p.native_kind,
                    p.native_blob_id, p.native_digest,
                    p.driver_fingerprint_json, p.summary_json, p.stock_id,
                    p.safe_overrides_json, p.last_validated_unix_ms,
                    p.last_test_job_id, p.published,
                    p.uses_current_printer_defaults, p.updated_unix_ms
             FROM printer_profiles p
             WHERE p.printer_id = ?1
               AND p.revision = (
                 SELECT MAX(latest.revision) FROM printer_profiles latest
                 WHERE latest.profile_id = p.profile_id
               )
               AND p.deleted = 0
             ORDER BY p.is_default DESC, p.name, p.profile_id",
        )?;
        let rows = statement.query_map([printer_id], named_profile_from_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Returns one active named profile.
    ///
    /// # Errors
    ///
    /// Returns an error when `SQLite` cannot execute or decode the query.
    pub fn named_profile(
        &self,
        printer_id: &str,
        profile_id: &str,
    ) -> Result<Option<StoredNamedProfile>, StorageError> {
        self.connection
            .query_row(
                NAMED_PROFILE_QUERY,
                params![printer_id, profile_id],
                named_profile_from_row,
            )
            .optional()
            .map_err(Into::into)
    }

    /// Returns one exact immutable profile revision.
    ///
    /// Historical and tombstoned rows remain readable so a job that was
    /// durably pinned before a later edit or retirement can replay exactly
    /// what routing selected.
    ///
    /// # Errors
    ///
    /// Returns an error when `SQLite` cannot execute or decode the query.
    pub fn named_profile_revision(
        &self,
        printer_id: &str,
        profile_id: &str,
        revision: u64,
    ) -> Result<Option<StoredNamedProfile>, StorageError> {
        self.connection
            .query_row(
                "SELECT profile_id, printer_id, revision, name, is_default,
                        options_json, status, native_kind, native_blob_id,
                        native_digest, driver_fingerprint_json, summary_json,
                        stock_id, safe_overrides_json,
                        last_validated_unix_ms, last_test_job_id, published,
                        uses_current_printer_defaults, updated_unix_ms
                 FROM printer_profiles
                 WHERE printer_id = ?1 AND profile_id = ?2 AND revision = ?3
                 LIMIT 1",
                params![printer_id, profile_id, revision],
                named_profile_from_row,
            )
            .optional()
            .map_err(Into::into)
    }

    /// Returns one exact immutable profile revision without requiring its
    /// destination identifier.
    ///
    /// # Errors
    ///
    /// Returns an error when `SQLite` cannot execute or decode the query.
    pub fn profile_revision(
        &self,
        profile_id: &str,
        revision: u64,
    ) -> Result<Option<StoredNamedProfile>, StorageError> {
        self.connection
            .query_row(
                "SELECT profile_id, printer_id, revision, name, is_default,
                        options_json, status, native_kind, native_blob_id,
                        native_digest, driver_fingerprint_json, summary_json,
                        stock_id, safe_overrides_json,
                        last_validated_unix_ms, last_test_job_id, published,
                        uses_current_printer_defaults, updated_unix_ms
                 FROM printer_profiles
                 WHERE profile_id = ?1 AND revision = ?2
                 LIMIT 1",
                params![profile_id, revision],
                named_profile_from_row,
            )
            .optional()
            .map_err(Into::into)
    }

    /// Records a successful integrity validation without changing profile
    /// readiness or the immutable native settings.
    ///
    /// # Errors
    ///
    /// Returns an error when the exact revision does not exist or `SQLite`
    /// cannot update it.
    pub fn record_profile_validation(
        &mut self,
        profile_id: &str,
        revision: u64,
        validated_unix_ms: i64,
    ) -> Result<(), StorageError> {
        let changed = self.connection.execute(
            "UPDATE printer_profiles
             SET last_validated_unix_ms = ?3, updated_unix_ms = ?3
             WHERE profile_id = ?1 AND revision = ?2 AND deleted = 0",
            params![profile_id, revision, validated_unix_ms],
        )?;
        if changed != 1 {
            return Err(StorageError::InvalidPrinterProfile(format!(
                "profile {profile_id} revision {revision} was not found"
            )));
        }
        Ok(())
    }

    /// Returns the dependency manifest for one immutable profile revision.
    ///
    /// # Errors
    ///
    /// Returns an error when `SQLite` cannot execute or decode the query.
    pub fn profile_dependencies(
        &self,
        profile_id: &str,
        revision: u64,
    ) -> Result<Vec<ProfileDependency>, StorageError> {
        let mut statement = self.connection.prepare(
            "SELECT kind, value
             FROM profile_dependencies
             WHERE profile_id = ?1 AND profile_revision = ?2
             ORDER BY dependency_index",
        )?;
        let rows = statement.query_map(params![profile_id, revision], |row| {
            Ok(ProfileDependency {
                kind: row.get(0)?,
                value: row.get(1)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Records the operational result of a driver test without changing the
    /// immutable native configuration or its revision identity.
    ///
    /// # Errors
    ///
    /// Returns an error when the exact revision no longer exists or `SQLite`
    /// cannot update it.
    pub fn record_profile_test_result(
        &mut self,
        profile_id: &str,
        revision: u64,
        job_id: &str,
        passed_native_handoff: bool,
        validated_unix_ms: i64,
    ) -> Result<(), StorageError> {
        let changed = self.connection.execute(
            "UPDATE printer_profiles
             SET status = ?4, last_validated_unix_ms = ?5,
                 last_test_job_id = ?3,
                 published = CASE
                   WHEN uses_current_printer_defaults = 1 THEN 0
                   ELSE ?6
                 END,
                 updated_unix_ms = ?5
             WHERE profile_id = ?1 AND revision = ?2 AND deleted = 0",
            params![
                profile_id,
                revision,
                job_id,
                if passed_native_handoff {
                    "ready"
                } else {
                    "needs_test"
                },
                validated_unix_ms,
                passed_native_handoff
            ],
        )?;
        if changed != 1 {
            return Err(StorageError::InvalidPrinterProfile(format!(
                "profile {profile_id} revision {revision} was not found"
            )));
        }
        Ok(())
    }

    /// Authorizes one short-lived native profile capture without storing its
    /// plaintext bearer token.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown destination, invalid operation, or
    /// failed transaction.
    #[allow(clippy::too_many_arguments)]
    pub fn create_profile_capture_session(
        &mut self,
        session_id: &str,
        token_digest: &str,
        printer_id: &str,
        profile_id: Option<&str>,
        expected_revision: Option<u64>,
        operation: &str,
        peer_user_id: &str,
        expires_unix_ms: i64,
        created_unix_ms: i64,
    ) -> Result<StoredCaptureSession, StorageError> {
        if self.printer(printer_id)?.is_none() {
            return Err(StorageError::PrinterNotFound(printer_id.to_owned()));
        }
        if !matches!(operation, "create" | "edit" | "clone")
            || token_digest.trim().is_empty()
            || peer_user_id.trim().is_empty()
            || expires_unix_ms <= created_unix_ms
            || expires_unix_ms.saturating_sub(created_unix_ms)
                > MAX_PROFILE_CAPTURE_SESSION_LIFETIME_MS
        {
            return Err(StorageError::InvalidPrinterProfile(
                "invalid profile capture authorization".into(),
            ));
        }
        if operation == "create" && (profile_id.is_some() || expected_revision.is_some()) {
            return Err(StorageError::InvalidPrinterProfile(
                "create captures cannot reference an existing profile".into(),
            ));
        }
        if matches!(operation, "edit" | "clone") {
            let Some(profile_id) = profile_id else {
                return Err(StorageError::InvalidPrinterProfile(
                    "edit and clone captures require a profile".into(),
                ));
            };
            let current = self.named_profile(printer_id, profile_id)?.ok_or_else(|| {
                StorageError::InvalidPrinterProfile("profile was not found".into())
            })?;
            if expected_revision != Some(current.revision) {
                return Err(StorageError::ProfileRevisionConflict {
                    expected: expected_revision.unwrap_or(0),
                    current: current.revision,
                });
            }
        }
        self.connection.execute(
            "INSERT INTO profile_capture_sessions(
               session_id, token_digest, printer_id, profile_id,
               expected_revision, operation, status, peer_user_id,
               expires_unix_ms, created_unix_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'authorized', ?7, ?8, ?9)",
            params![
                session_id,
                token_digest,
                printer_id,
                profile_id,
                expected_revision,
                operation,
                peer_user_id,
                expires_unix_ms,
                created_unix_ms
            ],
        )?;
        self.capture_session(session_id)?
            .ok_or_else(|| StorageError::CaptureSessionNotFound(session_id.to_owned()))
    }

    /// Reads non-secret capture-session metadata.
    ///
    /// # Errors
    ///
    /// Returns an error when the row cannot be decoded.
    pub fn capture_session(
        &self,
        session_id: &str,
    ) -> Result<Option<StoredCaptureSession>, StorageError> {
        self.connection
            .query_row(
                "SELECT session_id, printer_id, profile_id, expected_revision,
                        operation, status, peer_user_id, expires_unix_ms,
                        created_unix_ms
                 FROM profile_capture_sessions WHERE session_id = ?1",
                [session_id],
                |row| {
                    Ok(StoredCaptureSession {
                        session_id: row.get(0)?,
                        printer_id: row.get(1)?,
                        profile_id: row.get(2)?,
                        expected_revision: row.get(3)?,
                        operation: row.get(4)?,
                        status: row.get(5)?,
                        peer_user_id: row.get(6)?,
                        expires_unix_ms: row.get(7)?,
                        created_unix_ms: row.get(8)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    /// Atomically appends a native profile revision, its opaque local blob and
    /// dependency manifest, then consumes the one-time capture session.
    ///
    /// # Errors
    ///
    /// Returns an error for a bad/expired token, stale revision, invalid
    /// metadata, oversized or incorrectly digested blob, or failed commit.
    #[allow(
        clippy::too_many_lines,
        reason = "one immediate transaction keeps capture authorization, immutable blob, metadata, dependencies, and session consumption atomic"
    )]
    pub fn commit_profile_capture(
        &mut self,
        session_id: &str,
        token_digest: &str,
        peer_user_id: &str,
        capture: &NativeProfileCapture,
        committed_unix_ms: i64,
    ) -> Result<StoredNamedProfile, StorageError> {
        validate_native_capture(capture)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let authorization = transaction
            .query_row(
                "SELECT token_digest, printer_id, profile_id, expected_revision,
                        operation, status, peer_user_id, expires_unix_ms
                 FROM profile_capture_sessions WHERE session_id = ?1",
                [session_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<u64>>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, i64>(7)?,
                    ))
                },
            )
            .optional()?
            .ok_or_else(|| StorageError::CaptureSessionNotFound(session_id.to_owned()))?;
        if authorization.5 != "authorized"
            || authorization.7 < committed_unix_ms
            || authorization.6 != peer_user_id
        {
            return Err(StorageError::CaptureSessionNotAuthorized(
                session_id.to_owned(),
            ));
        }
        if !constant_time_str_eq(&authorization.0, token_digest) {
            return Err(StorageError::InvalidCaptureToken);
        }

        let current = authorization
            .2
            .as_deref()
            .map(|profile_id| query_named_profile(&transaction, &authorization.1, profile_id))
            .transpose()?
            .flatten();
        let current_revision = current.as_ref().map_or(0, |profile| profile.revision);
        if authorization.3.unwrap_or(0) != current_revision {
            return Err(StorageError::ProfileRevisionConflict {
                expected: authorization.3.unwrap_or(0),
                current: current_revision,
            });
        }
        let (profile_id, revision) = if authorization.4 == "edit" {
            (
                authorization.2.clone().ok_or_else(|| {
                    StorageError::InvalidPrinterProfile("edit capture requires a profile".into())
                })?,
                current_revision + 1,
            )
        } else {
            (ProfileId::new().to_string(), 1)
        };
        // Editing a profile changes its immutable settings revision, not its
        // routing identity. In particular, capturing the generated driver-
        // default profile must not unexpectedly stop it being the default.
        let is_default = if authorization.4 == "edit" {
            current.as_ref().is_some_and(|profile| profile.is_default)
        } else {
            capture.is_default
        };
        let blob_id = piqae_domain::NativeProfileBlobId::new().to_string();
        if is_default {
            demote_default_profiles(
                &transaction,
                &authorization.1,
                &profile_id,
                committed_unix_ms,
            )?;
        }
        transaction.execute(
            "INSERT INTO printer_profiles(
               profile_id, printer_id, revision, name, is_default, options_json,
               status, native_kind, native_blob_id, native_digest,
               driver_fingerprint_json, summary_json, stock_id,
               safe_overrides_json, published, uses_current_printer_defaults,
               deleted, updated_unix_ms
             ) VALUES (
               ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
               ?14, ?15, 0, 0, ?16
             )",
            params![
                profile_id,
                authorization.1,
                revision,
                capture.name.trim(),
                is_default,
                capture.options_json,
                capture.status,
                capture.native_kind,
                blob_id,
                capture.native_digest,
                capture.driver_fingerprint_json,
                capture.summary_json,
                capture.stock_id,
                capture.safe_overrides_json,
                capture.published,
                committed_unix_ms
            ],
        )?;
        transaction.execute(
            "INSERT INTO profile_native_blobs(
               blob_id, profile_id, profile_revision, native_kind,
               schema_version, digest, native_blob, created_unix_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                blob_id,
                profile_id,
                revision,
                capture.native_kind,
                capture.native_schema_version,
                capture.native_digest,
                capture.native_blob,
                committed_unix_ms
            ],
        )?;
        let dependencies: Vec<ProfileDependency> =
            serde_json::from_str(&capture.dependencies_json)?;
        for (index, dependency) in dependencies.iter().enumerate() {
            transaction.execute(
                "INSERT INTO profile_dependencies(
                   profile_id, profile_revision, dependency_index, kind, value
                 ) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    profile_id,
                    revision,
                    i64::try_from(index).map_err(|error| {
                        StorageError::InvalidPrinterProfile(error.to_string())
                    })?,
                    dependency.kind,
                    dependency.value
                ],
            )?;
        }
        transaction.execute(
            "UPDATE profile_capture_sessions
             SET status = 'committed', completed_unix_ms = ?2
             WHERE session_id = ?1 AND status = 'authorized'",
            params![session_id, committed_unix_ms],
        )?;
        let stored = query_named_profile(&transaction, &authorization.1, &profile_id)?
            .ok_or_else(|| StorageError::InvalidPrinterProfile("profile insert failed".into()))?;
        transaction.commit()?;
        Ok(stored)
    }

    /// Consumes an authorized capture session without creating a profile.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid token/session or failed update.
    pub fn cancel_profile_capture(
        &mut self,
        session_id: &str,
        token_digest: &str,
        peer_user_id: &str,
        cancelled_unix_ms: i64,
    ) -> Result<(), StorageError> {
        let changed = self.connection.execute(
            "UPDATE profile_capture_sessions
             SET status = 'cancelled', completed_unix_ms = ?4
             WHERE session_id = ?1 AND token_digest = ?2
               AND peer_user_id = ?3 AND status = 'authorized'
               AND expires_unix_ms >= ?4",
            params![session_id, token_digest, peer_user_id, cancelled_unix_ms],
        )?;
        if changed == 0 {
            return Err(StorageError::CaptureSessionNotAuthorized(
                session_id.to_owned(),
            ));
        }
        Ok(())
    }

    /// Loads one opaque native profile blob by exact immutable revision.
    ///
    /// # Errors
    ///
    /// Returns an error when the row cannot be decoded.
    pub fn native_profile_blob(
        &self,
        profile_id: &str,
        revision: u64,
    ) -> Result<Option<StoredNativeProfileBlob>, StorageError> {
        self.connection
            .query_row(
                "SELECT blob_id, profile_id, profile_revision, native_kind,
                        schema_version, digest, native_blob, created_unix_ms
                 FROM profile_native_blobs
                 WHERE profile_id = ?1 AND profile_revision = ?2",
                params![profile_id, revision],
                |row| {
                    Ok(StoredNativeProfileBlob {
                        blob_id: row.get(0)?,
                        profile_id: row.get(1)?,
                        profile_revision: row.get(2)?,
                        native_kind: row.get(3)?,
                        schema_version: row.get(4)?,
                        digest: row.get(5)?,
                        native_blob: row.get(6)?,
                        created_unix_ms: row.get(7)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    /// Pins an accepted job to the exact target, binding, profile revision and
    /// stock facts selected by routing.
    ///
    /// # Errors
    ///
    /// Returns an error for a missing job/profile or invalid media snapshot.
    #[allow(clippy::too_many_arguments)]
    pub fn pin_job_profile(
        &mut self,
        job_id: &str,
        target_id: Option<&str>,
        binding_id: Option<&str>,
        profile_id: &str,
        profile_revision: u64,
        stock_id: Option<&str>,
        loaded_media_snapshot_json: Option<&str>,
    ) -> Result<(), StorageError> {
        if let Some(snapshot) = loaded_media_snapshot_json {
            let _: serde_json::Value = serde_json::from_str(snapshot)?;
        }
        let profile_exists: bool = self.connection.query_row(
            "SELECT EXISTS(
               SELECT 1 FROM printer_profiles
               WHERE profile_id = ?1 AND revision = ?2 AND deleted = 0
             )",
            params![profile_id, profile_revision],
            |row| row.get(0),
        )?;
        if !profile_exists {
            return Err(StorageError::InvalidPrinterProfile(format!(
                "profile {profile_id} revision {profile_revision} was not found"
            )));
        }
        let changed = self.connection.execute(
            "UPDATE jobs SET target_id = ?2, binding_id = ?3, profile_id = ?4,
                    profile_revision = ?5, stock_id = ?6,
                    loaded_media_snapshot_json = ?7
             WHERE job_id = ?1
               AND (profile_id IS NULL OR (
                    profile_id = ?4 AND profile_revision = ?5
               ))",
            params![
                job_id,
                target_id,
                binding_id,
                profile_id,
                profile_revision,
                stock_id,
                loaded_media_snapshot_json
            ],
        )?;
        if changed != 1 {
            let job_exists: bool = self.connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM jobs WHERE job_id = ?1)",
                [job_id],
                |row| row.get(0),
            )?;
            return Err(if job_exists {
                StorageError::JobConflict(job_id.to_owned())
            } else {
                StorageError::JobNotFound(job_id.to_owned())
            });
        }
        Ok(())
    }

    /// Creates or updates a physical-device identity used by loaded-media
    /// state. Destination grouping remains explicit and confidence-labelled.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid metadata JSON or failed persistence.
    #[allow(clippy::too_many_arguments)]
    pub fn upsert_physical_device(
        &mut self,
        device_id: &str,
        display_name: &str,
        hardware_fingerprint: Option<&str>,
        identity_confidence: &str,
        metadata_json: &str,
        updated_unix_ms: i64,
    ) -> Result<(), StorageError> {
        let _: serde_json::Value = serde_json::from_str(metadata_json)?;
        self.connection.execute(
            "INSERT INTO physical_devices(
               device_id, display_name, hardware_fingerprint,
               identity_confidence, metadata_json, updated_unix_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(device_id) DO UPDATE SET
               display_name = excluded.display_name,
               hardware_fingerprint = excluded.hardware_fingerprint,
               identity_confidence = excluded.identity_confidence,
               metadata_json = excluded.metadata_json,
               updated_unix_ms = excluded.updated_unix_ms",
            params![
                device_id,
                display_name,
                hardware_fingerprint,
                identity_confidence,
                metadata_json,
                updated_unix_ms
            ],
        )?;
        Ok(())
    }

    /// Associates an installed destination with a physical device.
    ///
    /// # Errors
    ///
    /// Returns an error for unknown resources or failed persistence.
    pub fn bind_printer_device(
        &mut self,
        printer_id: &str,
        device_id: &str,
        confidence: &str,
        confirmed_by: Option<&str>,
        updated_unix_ms: i64,
    ) -> Result<(), StorageError> {
        self.connection.execute(
            "INSERT INTO printer_device_bindings(
               printer_id, device_id, binding_confidence, confirmed_by,
               updated_unix_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(printer_id) DO UPDATE SET
               device_id = excluded.device_id,
               binding_confidence = excluded.binding_confidence,
               confirmed_by = excluded.confirmed_by,
               updated_unix_ms = excluded.updated_unix_ms",
            params![
                printer_id,
                device_id,
                confidence,
                confirmed_by,
                updated_unix_ms
            ],
        )?;
        Ok(())
    }

    /// Creates or updates a portable stock definition.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid definition JSON or failed persistence.
    #[allow(clippy::too_many_arguments)]
    pub fn upsert_stock(
        &mut self,
        stock_id: &str,
        name: &str,
        sku: Option<&str>,
        kind: &str,
        definition_json: &str,
        retired: bool,
        updated_unix_ms: i64,
    ) -> Result<StoredStock, StorageError> {
        let _: serde_json::Value = serde_json::from_str(definition_json)?;
        if name.trim().is_empty() {
            return Err(StorageError::InvalidPrinterProfile(
                "stock name cannot be empty".into(),
            ));
        }
        self.connection.execute(
            "INSERT INTO stocks(
               stock_id, name, sku, kind, definition_json, retired,
               updated_unix_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(stock_id) DO UPDATE SET
               name = excluded.name, sku = excluded.sku, kind = excluded.kind,
               definition_json = excluded.definition_json,
               retired = excluded.retired,
               updated_unix_ms = excluded.updated_unix_ms",
            params![
                stock_id,
                name.trim(),
                sku,
                kind,
                definition_json,
                retired,
                updated_unix_ms
            ],
        )?;
        Ok(StoredStock {
            stock_id: stock_id.to_owned(),
            name: name.trim().to_owned(),
            sku: sku.map(str::to_owned),
            kind: kind.to_owned(),
            definition_json: definition_json.to_owned(),
            retired,
            updated_unix_ms,
        })
    }

    /// Confirms the stock loaded in one physical source/tray.
    ///
    /// # Errors
    ///
    /// Returns an error for unknown device/stock or failed persistence.
    pub fn confirm_loaded_media(&mut self, media: &StoredLoadedMedia) -> Result<(), StorageError> {
        self.connection.execute(
            "INSERT INTO loaded_media(
               device_id, source, stock_id, confidence, confirmed_unix_ms,
               confirmed_by
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(device_id, source) DO UPDATE SET
               stock_id = excluded.stock_id,
               confidence = excluded.confidence,
               confirmed_unix_ms = excluded.confirmed_unix_ms,
               confirmed_by = excluded.confirmed_by",
            params![
                media.device_id,
                media.source,
                media.stock_id,
                media.confidence,
                media.confirmed_unix_ms,
                media.confirmed_by
            ],
        )?;
        Ok(())
    }

    /// Reads loaded-media state for one device and source.
    ///
    /// # Errors
    ///
    /// Returns an error when the row cannot be decoded.
    pub fn loaded_media(
        &self,
        device_id: &str,
        source: &str,
    ) -> Result<Option<StoredLoadedMedia>, StorageError> {
        self.connection
            .query_row(
                "SELECT device_id, source, stock_id, confidence,
                        confirmed_unix_ms, confirmed_by
                 FROM loaded_media WHERE device_id = ?1 AND source = ?2",
                params![device_id, source],
                |row| {
                    Ok(StoredLoadedMedia {
                        device_id: row.get(0)?,
                        source: row.get(1)?,
                        stock_id: row.get(2)?,
                        confidence: row.get(3)?,
                        confirmed_unix_ms: row.get(4)?,
                        confirmed_by: row.get(5)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    /// Creates or updates a stable logical print target.
    ///
    /// # Errors
    ///
    /// Returns an error for an empty name or failed persistence.
    pub fn upsert_target(&mut self, target: &StoredTarget) -> Result<(), StorageError> {
        if target.name.trim().is_empty() {
            return Err(StorageError::InvalidPrinterProfile(
                "target name cannot be empty".into(),
            ));
        }
        self.connection.execute(
            "INSERT INTO targets(
               target_id, name, stock_id, routing_policy, published, retired,
               updated_unix_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(target_id) DO UPDATE SET
               name = excluded.name, stock_id = excluded.stock_id,
               routing_policy = excluded.routing_policy,
               published = excluded.published, retired = excluded.retired,
               updated_unix_ms = excluded.updated_unix_ms",
            params![
                target.target_id,
                target.name.trim(),
                target.stock_id,
                target.routing_policy,
                target.published,
                target.retired,
                target.updated_unix_ms
            ],
        )?;
        Ok(())
    }

    /// Creates or updates one node/destination/profile target binding.
    ///
    /// # Errors
    ///
    /// Returns an error for a missing pinned profile revision or failed write.
    pub fn upsert_target_binding(
        &mut self,
        binding: &StoredTargetBinding,
    ) -> Result<(), StorageError> {
        let profile_exists: bool = self.connection.query_row(
            "SELECT EXISTS(
               SELECT 1 FROM printer_profiles
               WHERE profile_id = ?1 AND printer_id = ?2
                 AND revision = ?3 AND deleted = 0
             )",
            params![
                binding.profile_id,
                binding.printer_id,
                binding.profile_revision
            ],
            |row| row.get(0),
        )?;
        if !profile_exists {
            return Err(StorageError::InvalidPrinterProfile(format!(
                "profile {} revision {} was not found for printer {}",
                binding.profile_id, binding.profile_revision, binding.printer_id
            )));
        }
        self.connection.execute(
            "INSERT INTO target_bindings(
               binding_id, target_id, agent_id, printer_id, profile_id,
               profile_revision, role, priority, enabled, created_unix_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(binding_id) DO UPDATE SET
               target_id = excluded.target_id, agent_id = excluded.agent_id,
               printer_id = excluded.printer_id,
               profile_id = excluded.profile_id,
               profile_revision = excluded.profile_revision,
               role = excluded.role, priority = excluded.priority,
               enabled = excluded.enabled",
            params![
                binding.binding_id,
                binding.target_id,
                binding.agent_id,
                binding.printer_id,
                binding.profile_id,
                binding.profile_revision,
                binding.role,
                binding.priority,
                binding.enabled,
                binding.created_unix_ms
            ],
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_named_profile(
        &mut self,
        profile_id: &str,
        printer_id: &str,
        revision: u64,
        name: &str,
        is_default: bool,
        options_json: &str,
        deleted: bool,
        updated_unix_ms: i64,
    ) -> Result<(), StorageError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        insert_named_profile_tx(
            &transaction,
            profile_id,
            printer_id,
            revision,
            name.trim(),
            is_default,
            options_json,
            deleted,
            updated_unix_ms,
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Changes whether a discovered queue may receive jobs or be advertised.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown printer or failed transaction.
    pub fn set_printer_exposed(
        &mut self,
        printer_id: &str,
        exposed: bool,
        updated_unix_ms: i64,
    ) -> Result<(), StorageError> {
        let exists: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM printers WHERE printer_id = ?1)",
            [printer_id],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(StorageError::PrinterNotFound(printer_id.to_owned()));
        }
        self.connection.execute(
            "INSERT INTO printer_exposure(printer_id, exposed, updated_unix_ms)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(printer_id) DO UPDATE SET
               exposed = excluded.exposed,
               updated_unix_ms = excluded.updated_unix_ms",
            params![printer_id, exposed, updated_unix_ms],
        )?;
        Ok(())
    }

    /// Returns one persisted printer and its latest immutable profile.
    ///
    /// # Errors
    ///
    /// Returns an error when `SQLite` cannot execute or decode the query.
    pub fn printer(&self, printer_id: &str) -> Result<Option<StoredPrinter>, StorageError> {
        self.connection
            .query_row(STORED_PRINTER_QUERY, [printer_id], stored_printer_from_row)
            .optional()
            .map_err(Into::into)
    }

    /// Returns persisted printer inventory ordered by friendly name.
    ///
    /// # Errors
    ///
    /// Returns an error when `SQLite` cannot execute or decode the query.
    pub fn printers(&self) -> Result<Vec<StoredPrinter>, StorageError> {
        let query = format!("{STORED_PRINTER_QUERY} ORDER BY p.name, p.printer_id");
        let mut statement = self.connection.prepare(&query)?;
        let rows = statement.query_map([Option::<String>::None], stored_printer_from_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Returns only queues confirmed by the latest successful discovery.
    ///
    /// # Errors
    ///
    /// Returns an error when `SQLite` cannot execute or decode the query.
    pub fn present_printers(&self) -> Result<Vec<StoredPrinter>, StorageError> {
        let query =
            format!("{STORED_PRINTER_QUERY} AND p.present = 1 ORDER BY p.name, p.printer_id");
        let mut statement = self.connection.prepare(&query)?;
        let rows = statement.query_map([Option::<String>::None], stored_printer_from_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Counts currently present queues reporting an actionable degraded state.
    ///
    /// # Errors
    ///
    /// Returns an error when `SQLite` cannot execute the aggregate query.
    pub fn present_printer_warning_count(&self) -> Result<u32, StorageError> {
        self.connection
            .query_row(
                "SELECT COUNT(*) FROM printers
                 WHERE present = 1 AND state IN ('offline', 'error', 'paper_out')",
                [],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    /// Resolves a cloud logical printer ID to the current native queue key.
    ///
    /// # Errors
    ///
    /// Returns an error when `SQLite` cannot execute or decode the query.
    pub fn native_printer_id(&self, printer_id: &str) -> Result<Option<String>, StorageError> {
        self.connection
            .query_row(
                "SELECT native_id FROM printers WHERE printer_id = ?1",
                [printer_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    /// Atomically accepts local responsibility for a job. Duplicate delivery
    /// of identical immutable metadata is a successful no-op.
    ///
    /// # Errors
    ///
    /// Returns an error for conflicting immutable metadata or if any part of
    /// the acceptance transaction fails.
    pub fn accept_job(&mut self, job: &AcceptedJob) -> Result<LocalJob, StorageError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        if let Some(existing) = query_job(&tx, &job.job_id)? {
            let same = existing.submission_id == job.submission_id
                && existing.printer_id == job.printer_id
                && existing.content_sha256 == job.content_sha256
                && existing.content_kind == job.content_kind;
            if !same {
                return Err(StorageError::JobConflict(job.job_id.clone()));
            }
            tx.commit()?;
            return Ok(existing);
        }

        tx.execute(
            "INSERT INTO printer_sequences (printer_id, next_sequence)
             VALUES (?1, 2)
             ON CONFLICT (printer_id)
             DO UPDATE SET next_sequence = next_sequence + 1",
            [&job.printer_id],
        )?;
        let printer_sequence: i64 = tx.query_row(
            "SELECT next_sequence - 1 FROM printer_sequences WHERE printer_id = ?1",
            [&job.printer_id],
            |row| row.get(0),
        )?;

        tx.execute(
            "INSERT INTO inbox_receipts
             (receipt_id, job_id, content_sha256, received_unix_ms)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                format!("accept:{}", job.job_id),
                job.job_id,
                job.content_sha256,
                job.accepted_unix_ms
            ],
        )?;
        let content_recorded = tx.execute(
            "INSERT INTO content_files
             (sha256, path, reference_count, verified_unix_ms)
             VALUES (?1, ?2, 1, ?3)
             ON CONFLICT (sha256)
             DO UPDATE SET reference_count = reference_count + 1
             WHERE content_files.reclaiming = 0",
            params![job.content_sha256, job.content_path, job.accepted_unix_ms],
        )?;
        if content_recorded == 0 {
            return Err(StorageError::ContentReclaimInProgress(
                job.content_sha256.clone(),
            ));
        }
        tx.execute(
            "INSERT INTO jobs
             (job_id, submission_id, printer_id, printer_native_id,
              printer_sequence, title, content_sha256, content_path,
              content_kind, options_json, state, expires_unix_ms,
              accepted_unix_ms, updated_unix_ms, cloud_managed)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                     'queued_local', ?11, ?12, ?12, ?13)",
            params![
                job.job_id,
                job.submission_id,
                job.printer_id,
                job.printer_native_id,
                printer_sequence,
                job.title,
                job.content_sha256,
                job.content_path,
                job.content_kind,
                job.options_json,
                job.expires_unix_ms,
                job.accepted_unix_ms,
                job.cloud_managed,
            ],
        )?;
        append_event_tx(
            &tx,
            &EventId::new().to_string(),
            &job.job_id,
            1,
            "queued_local",
            None,
            Some("Job is durable in the local queue"),
            "{}",
            job.accepted_unix_ms,
        )?;
        tx.commit()?;
        self.get_job(&job.job_id)?
            .ok_or_else(|| StorageError::JobNotFound(job.job_id.clone()))
    }

    /// Durably prepares a cloud job and its exact lease acceptance request
    /// without making the job runnable.
    ///
    /// A repeated offer with identical immutable job data replaces only the
    /// lease credentials. No local event is emitted until
    /// [`Self::activate_cloud_job`] confirms the remote acceptance.
    ///
    /// # Errors
    ///
    /// Returns an error for a non-cloud job, conflicting immutable metadata,
    /// invalid lease data, or a failed transaction.
    #[allow(clippy::too_many_lines)]
    pub fn prepare_cloud_job(
        &mut self,
        job: &AcceptedJob,
        lease_id: &str,
        lease_token: &str,
        lease_expires_unix_ms: i64,
    ) -> Result<LocalJob, StorageError> {
        if !job.cloud_managed || lease_token.is_empty() {
            return Err(StorageError::InvalidLocalEvent(
                "cloud acceptance requires a managed job and lease token".into(),
            ));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(existing) = query_job(&transaction, &job.job_id)? {
            let same = existing.submission_id == job.submission_id
                && existing.printer_id == job.printer_id
                && existing.content_sha256 == job.content_sha256
                && existing.content_kind == job.content_kind;
            if !same {
                return Err(StorageError::JobConflict(job.job_id.clone()));
            }
            if existing.state != "cloud_accept_pending" {
                return Err(StorageError::InvalidLocalEvent(format!(
                    "cloud job {} is already in local state {}",
                    job.job_id, existing.state
                )));
            }
            upsert_cloud_accept_intent(
                &transaction,
                &existing,
                lease_id,
                lease_token,
                lease_expires_unix_ms,
                job.accepted_unix_ms,
            )?;
            transaction.commit()?;
            return Ok(existing);
        }

        transaction.execute(
            "INSERT INTO printer_sequences (printer_id, next_sequence)
             VALUES (?1, 2)
             ON CONFLICT (printer_id)
             DO UPDATE SET next_sequence = next_sequence + 1",
            [&job.printer_id],
        )?;
        let printer_sequence: i64 = transaction.query_row(
            "SELECT next_sequence - 1 FROM printer_sequences WHERE printer_id = ?1",
            [&job.printer_id],
            |row| row.get(0),
        )?;
        transaction.execute(
            "INSERT INTO inbox_receipts
             (receipt_id, job_id, content_sha256, received_unix_ms)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                format!("accept:{}", job.job_id),
                job.job_id,
                job.content_sha256,
                job.accepted_unix_ms
            ],
        )?;
        let content_recorded = transaction.execute(
            "INSERT INTO content_files
             (sha256, path, reference_count, verified_unix_ms)
             VALUES (?1, ?2, 1, ?3)
             ON CONFLICT (sha256)
             DO UPDATE SET reference_count = reference_count + 1
             WHERE content_files.reclaiming = 0",
            params![job.content_sha256, job.content_path, job.accepted_unix_ms],
        )?;
        if content_recorded == 0 {
            return Err(StorageError::ContentReclaimInProgress(
                job.content_sha256.clone(),
            ));
        }
        let confidential = std::path::Path::new(&job.content_path)
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("confidential-"));
        transaction.execute(
            "INSERT INTO jobs
             (job_id, submission_id, printer_id, printer_native_id,
              printer_sequence, title, content_sha256, content_path,
              content_kind, options_json, state, expires_unix_ms,
              accepted_unix_ms, updated_unix_ms, cloud_managed,
              confidential, confidential_delete_after_unix_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                     'cloud_accept_pending', ?11, ?12, ?12, 1, ?13, NULL)",
            params![
                job.job_id,
                job.submission_id,
                job.printer_id,
                job.printer_native_id,
                printer_sequence,
                job.title,
                job.content_sha256,
                job.content_path,
                job.content_kind,
                job.options_json,
                job.expires_unix_ms,
                job.accepted_unix_ms,
                i64::from(confidential),
            ],
        )?;
        let local = query_job(&transaction, &job.job_id)?
            .ok_or_else(|| StorageError::JobNotFound(job.job_id.clone()))?;
        upsert_cloud_accept_intent(
            &transaction,
            &local,
            lease_id,
            lease_token,
            lease_expires_unix_ms,
            job.accepted_unix_ms,
        )?;
        transaction.commit()?;
        self.get_job(&job.job_id)?
            .ok_or_else(|| StorageError::JobNotFound(job.job_id.clone()))
    }

    /// Returns confidential plaintext files belonging to terminal jobs.
    ///
    /// # Errors
    ///
    /// Returns an error when `SQLite` cannot decode an intent.
    pub fn confidential_files_due(&self) -> Result<Vec<ConfidentialFile>, StorageError> {
        let mut statement = self.connection.prepare(
            "SELECT job_id, content_path FROM jobs
             WHERE confidential = 1
               AND state IN ('completed_reported','delivery_uncertain','failed_terminal','cancelled','expired')
             LIMIT 256",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(ConfidentialFile {
                job_id: row.get(0)?,
                path: row.get(1)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Removes the durable confidential-file record after its plaintext file is gone.
    ///
    /// # Errors
    ///
    /// Returns an error if the local database update fails.
    pub fn mark_confidential_file_deleted(&self, job_id: &str) -> Result<(), StorageError> {
        self.connection.execute(
            "UPDATE jobs SET confidential = 0, confidential_delete_after_unix_ms = NULL
             WHERE job_id = ?1 AND confidential = 1",
            [job_id],
        )?;
        Ok(())
    }

    /// Returns content referenced exclusively by truthful terminal jobs.
    /// Delivery-uncertain jobs deliberately remain retention barriers.
    ///
    /// # Errors
    ///
    /// Returns an error when the bounded candidate query cannot be executed.
    pub fn claim_reclaimable_terminal_content(
        &mut self,
        limit: usize,
    ) -> Result<Vec<ReclaimableContent>, StorageError> {
        let bounded = i64::try_from(limit.clamp(1, 256)).unwrap_or(256);
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut statement = transaction.prepare(
            "SELECT c.sha256, c.path
             FROM content_files c
             WHERE c.reclaiming = 0 AND NOT EXISTS (
               SELECT 1 FROM jobs retained
               WHERE retained.content_sha256 = c.sha256
                 AND retained.content_path = c.path
                 AND retained.state NOT IN (
                   'completed_reported','failed_terminal','cancelled','expired'
                 )
             )
             ORDER BY c.verified_unix_ms, c.sha256
             LIMIT ?1",
        )?;
        let candidates = statement
            .query_map([bounded], |row| {
                Ok(ReclaimableContent {
                    sha256: row.get(0)?,
                    path: row.get(1)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(statement);
        let mut claimed = Vec::new();
        for candidate in candidates {
            let changed = transaction.execute(
                "UPDATE content_files SET reclaiming = 1
                 WHERE sha256 = ?1 AND path = ?2 AND reclaiming = 0
                   AND NOT EXISTS (
                     SELECT 1 FROM jobs retained
                     WHERE retained.content_sha256 = ?1
                       AND retained.content_path = ?2
                       AND retained.state NOT IN (
                         'completed_reported','failed_terminal','cancelled','expired'
                       )
                   )",
                params![candidate.sha256, candidate.path],
            )?;
            if changed == 1 {
                claimed.push(candidate);
            }
        }
        transaction.commit()?;
        Ok(claimed)
    }

    /// Retires one file after the caller has removed it. The reference check
    /// is repeated transactionally so an active or uncertain job can never
    /// lose its document through a stale cleanup candidate.
    ///
    /// # Errors
    ///
    /// Returns an error when the transactional reference check or retirement
    /// cannot be persisted.
    pub fn mark_terminal_content_reclaimed(
        &mut self,
        sha256: &str,
        path: &str,
    ) -> Result<bool, StorageError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let retained: bool = transaction.query_row(
            "SELECT EXISTS(
               SELECT 1 FROM jobs
               WHERE content_sha256 = ?1 AND content_path = ?2
                 AND state NOT IN (
                   'completed_reported','failed_terminal','cancelled','expired'
                 )
             )",
            params![sha256, path],
            |row| row.get(0),
        )?;
        if retained {
            transaction.execute(
                "UPDATE content_files SET reclaiming = 0
                 WHERE sha256 = ?1 AND path = ?2 AND reclaiming = 1",
                params![sha256, path],
            )?;
            transaction.commit()?;
            return Ok(false);
        }
        transaction.execute(
            "UPDATE jobs SET content_path = 'retired:' || content_sha256
             WHERE content_sha256 = ?1 AND content_path = ?2
               AND state IN (
                 'completed_reported','failed_terminal','cancelled','expired'
               )",
            params![sha256, path],
        )?;
        transaction.execute(
            "DELETE FROM content_files
             WHERE sha256 = ?1 AND path = ?2 AND reclaiming = 1",
            params![sha256, path],
        )?;
        transaction.commit()?;
        Ok(true)
    }

    /// Releases a reclaim claim after filesystem deletion failed.
    ///
    /// # Errors
    ///
    /// Returns an error when the durable claim cannot be cleared.
    pub fn cancel_terminal_content_reclaim(
        &self,
        sha256: &str,
        path: &str,
    ) -> Result<(), StorageError> {
        self.connection.execute(
            "UPDATE content_files SET reclaiming = 0
             WHERE sha256 = ?1 AND path = ?2 AND reclaiming = 1",
            params![sha256, path],
        )?;
        Ok(())
    }

    /// Returns accepted cloud jobs whose acknowledgement has not reached the control plane.
    ///
    /// # Errors
    ///
    /// Returns an error if the durable queue cannot be read or decoded.
    pub fn pending_cloud_accepts(&self) -> Result<Vec<CloudAcceptIntent>, StorageError> {
        let mut statement = self.connection.prepare(
            "SELECT job_id, lease_id, lease_token, lease_expires_unix_ms,
                    content_sha256, local_sequence
             FROM cloud_accept_intents ORDER BY prepared_unix_ms, job_id",
        )?;
        let rows = statement.query_map([], row_to_cloud_accept_intent)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Atomically makes a remotely confirmed cloud job runnable, emits its
    /// first local event, and deletes the persisted lease capability.
    ///
    /// # Errors
    ///
    /// Returns an error for a missing/unprepared job or a failed transaction.
    pub fn activate_cloud_job(
        &mut self,
        job_id: &str,
        observed_unix_ms: i64,
    ) -> Result<LocalJob, StorageError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let job = query_job(&transaction, job_id)?
            .ok_or_else(|| StorageError::JobNotFound(job_id.to_owned()))?;
        if job.state == "queued_local" {
            transaction.commit()?;
            return Ok(job);
        }
        if job.state != "cloud_accept_pending" {
            return Err(StorageError::InvalidLocalEvent(format!(
                "cloud job {job_id} cannot activate from {}",
                job.state
            )));
        }
        let has_intent: bool = transaction.query_row(
            "SELECT EXISTS (
               SELECT 1 FROM cloud_accept_intents WHERE job_id = ?1
             )",
            [job_id],
            |row| row.get(0),
        )?;
        if !has_intent {
            return Err(StorageError::InvalidLocalEvent(format!(
                "cloud job {job_id} has no acceptance intent"
            )));
        }
        transaction.execute(
            "UPDATE jobs SET state = 'queued_local', updated_unix_ms = ?2
             WHERE job_id = ?1",
            params![job_id, observed_unix_ms],
        )?;
        append_event_tx(
            &transaction,
            &EventId::new().to_string(),
            job_id,
            1,
            "queued_local",
            None,
            Some("Job is durable in the local queue"),
            "{}",
            observed_unix_ms,
        )?;
        transaction.execute(
            "DELETE FROM cloud_accept_intents WHERE job_id = ?1",
            [job_id],
        )?;
        transaction.commit()?;
        self.get_job(job_id)?
            .ok_or_else(|| StorageError::JobNotFound(job_id.to_owned()))
    }

    /// Looks up one locally durable job.
    ///
    /// # Errors
    ///
    /// Returns an error when `SQLite` cannot execute or decode the query.
    pub fn get_job(&self, job_id: &str) -> Result<Option<LocalJob>, StorageError> {
        Ok(query_job(&self.connection, job_id)?)
    }

    /// Returns aggregate local queue counts for health and backpressure.
    ///
    /// # Errors
    ///
    /// Returns an error when `SQLite` cannot execute or decode the query.
    pub fn queue_counts(&self) -> Result<QueueCounts, StorageError> {
        self.connection
            .query_row(
                "SELECT
                   COUNT(*) FILTER (
                     WHERE state IN ('queued_local', 'failed_retryable')
                   ),
                   COUNT(*) FILTER (
                     WHERE state IN ('preparing', 'rendering', 'spool_intent',
                                     'accepted_by_spooler', 'spooling', 'printing')
                   )
                 FROM jobs",
                [],
                |row| {
                    let queued = row.get::<_, u32>(0)?;
                    let active = row.get::<_, u32>(1)?;
                    Ok(QueueCounts { queued, active })
                },
            )
            .map_err(Into::into)
    }

    /// Returns the newest local jobs for one logical printer.
    ///
    /// # Errors
    ///
    /// Returns an error when `SQLite` cannot execute or decode the query.
    pub fn jobs_for_printer(
        &self,
        printer_id: &str,
        limit: usize,
    ) -> Result<Vec<LocalJob>, StorageError> {
        let bounded = i64::try_from(limit.clamp(1, 500)).unwrap_or(500);
        let mut statement = self.connection.prepare(
            "SELECT job_id, submission_id, printer_id, printer_native_id,
                    printer_sequence, title, content_sha256, content_path,
                    content_kind, options_json, state, expires_unix_ms,
                    native_job_id, target_id, binding_id, profile_id,
                    profile_revision, stock_id, loaded_media_snapshot_json
             FROM jobs WHERE printer_id = ?1
             ORDER BY printer_sequence DESC LIMIT ?2",
        )?;
        let rows = statement.query_map(params![printer_id, bounded], row_to_job)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Returns retained local jobs newest first across every printer.
    ///
    /// This is used by the loopback node queue. Pagination keeps the agent
    /// control channel bounded even when the durable history is large.
    ///
    /// # Errors
    ///
    /// Returns an error when `SQLite` cannot query or decode the retained jobs.
    pub fn local_job_history(
        &self,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<LocalJob>, StorageError> {
        let bounded_limit = i64::try_from(limit.clamp(1, 200)).unwrap_or(200);
        let bounded_offset = i64::try_from(offset.min(i32::MAX as usize)).unwrap_or(i64::MAX);
        let mut statement = self.connection.prepare(
            "SELECT job_id, submission_id, printer_id, printer_native_id,
                    printer_sequence, title, content_sha256, content_path,
                    content_kind, options_json, state, expires_unix_ms,
                    native_job_id, target_id, binding_id, profile_id,
                    profile_revision, stock_id, loaded_media_snapshot_json
             FROM jobs ORDER BY accepted_unix_ms DESC, job_id DESC LIMIT ?1 OFFSET ?2",
        )?;
        let rows = statement.query_map(params![bounded_limit, bounded_offset], row_to_job)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Returns exact local queue counts for one logical printer.
    ///
    /// # Errors
    ///
    /// Returns an error when `SQLite` cannot execute the aggregate query.
    pub fn printer_queue_counts(&self, printer_id: &str) -> Result<QueueCounts, StorageError> {
        self.connection
            .query_row(
                "SELECT
                   COUNT(*) FILTER (
                     WHERE state IN ('queued_local', 'failed_retryable')
                   ),
                   COUNT(*) FILTER (
                     WHERE state IN ('preparing', 'rendering', 'spool_intent',
                                     'accepted_by_spooler', 'spooling',
                                     'printing', 'blocked')
                   )
                 FROM jobs WHERE printer_id = ?1",
                [printer_id],
                |row| {
                    Ok(QueueCounts {
                        queued: row.get(0)?,
                        active: row.get(1)?,
                    })
                },
            )
            .map_err(Into::into)
    }

    /// Returns at most one FIFO head for each printer. A printer already
    /// executing a job is withheld to preserve the serial handoff invariant.
    ///
    /// # Errors
    ///
    /// Returns an error when `SQLite` cannot execute or decode the query.
    pub fn runnable_heads(&self, now_unix_ms: i64) -> Result<Vec<LocalJob>, StorageError> {
        let mut statement = self.connection.prepare(
            "SELECT j.job_id, j.submission_id, j.printer_id, j.printer_native_id,
                    j.printer_sequence, j.title, j.content_sha256, j.content_path,
                    j.content_kind, j.options_json, j.state, j.expires_unix_ms,
                    j.native_job_id, j.target_id, j.binding_id, j.profile_id,
                    j.profile_revision, j.stock_id,
                    j.loaded_media_snapshot_json
             FROM jobs j
             WHERE j.state IN ('queued_local', 'failed_retryable')
               AND (j.next_attempt_unix_ms IS NULL OR j.next_attempt_unix_ms <= ?1)
               AND (j.expires_unix_ms IS NULL OR j.expires_unix_ms > ?1)
               AND NOT EXISTS (
                 SELECT 1 FROM jobs active
                 WHERE active.printer_id = j.printer_id
                   AND active.state IN ('preparing', 'rendering', 'spool_intent',
                                        'accepted_by_spooler', 'spooling', 'printing')
               )
               AND j.printer_sequence = (
                 SELECT MIN(head.printer_sequence) FROM jobs head
                 WHERE head.printer_id = j.printer_id
                   AND head.state IN ('queued_local', 'failed_retryable')
               )
             ORDER BY j.accepted_unix_ms, j.job_id",
        )?;
        let rows = statement.query_map([now_unix_ms], row_to_job)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    /// Appends an event at an explicitly expected per-job sequence.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid JSON, an unexpected sequence, an unknown
    /// job, or a failed transaction.
    pub fn append_event(
        &mut self,
        event_id: &str,
        job_id: &str,
        expected_sequence: i64,
        state: &str,
        reason: Option<&str>,
        message: Option<&str>,
        details_json: &str,
        observed_unix_ms: i64,
    ) -> Result<(), StorageError> {
        let _: serde_json::Value = serde_json::from_str(details_json)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let actual: i64 = tx.query_row(
            "SELECT COALESCE(MAX(job_sequence), 0) + 1 FROM job_events WHERE job_id = ?1",
            [job_id],
            |row| row.get(0),
        )?;
        if expected_sequence != actual {
            return Err(StorageError::EventSequence {
                job_id: job_id.to_owned(),
                expected: expected_sequence,
                actual,
            });
        }
        append_event_tx(
            &tx,
            event_id,
            job_id,
            expected_sequence,
            state,
            reason,
            message,
            details_json,
            observed_unix_ms,
        )?;
        tx.execute(
            "UPDATE jobs SET state = ?1, updated_unix_ms = ?2 WHERE job_id = ?3",
            params![state, observed_unix_ms, job_id],
        )?;
        if tx.changes() != 1 {
            return Err(StorageError::JobNotFound(job_id.to_owned()));
        }
        tx.commit()?;
        Ok(())
    }

    /// Appends the next event sequence atomically from the caller's
    /// perspective. The explicit [`Self::append_event`] variant remains
    /// available when replay code already knows the expected sequence.
    ///
    /// # Errors
    ///
    /// Returns an error when the current sequence cannot be read or the event
    /// transaction fails.
    #[allow(clippy::too_many_arguments)]
    pub fn append_next_event(
        &mut self,
        event_id: &str,
        job_id: &str,
        state: &str,
        reason: Option<&str>,
        message: Option<&str>,
        details_json: &str,
        observed_unix_ms: i64,
    ) -> Result<i64, StorageError> {
        let sequence: i64 = self.connection.query_row(
            "SELECT COALESCE(MAX(job_sequence), 0) + 1
             FROM job_events WHERE job_id = ?1",
            [job_id],
            |row| row.get(0),
        )?;
        self.append_event(
            event_id,
            job_id,
            sequence,
            state,
            reason,
            message,
            details_json,
            observed_unix_ms,
        )?;
        Ok(sequence)
    }

    /// Associates the OS spooler identifier with a local job.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown job or a failed update.
    pub fn set_native_job_id(
        &mut self,
        job_id: &str,
        native_job_id: &str,
        updated_unix_ms: i64,
    ) -> Result<(), StorageError> {
        let changed = self.connection.execute(
            "UPDATE jobs
             SET native_job_id = ?1, updated_unix_ms = ?2
             WHERE job_id = ?3",
            params![native_job_id, updated_unix_ms, job_id],
        )?;
        if changed != 1 {
            return Err(StorageError::JobNotFound(job_id.to_owned()));
        }
        Ok(())
    }

    /// Persists the native identifier and restart-safe reconciliation
    /// schedule after the spooler accepts a job.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown job or failed transaction.
    pub fn schedule_native_reconciliation(
        &mut self,
        job_id: &str,
        native_job_id: &str,
        next_observe_unix_ms: i64,
        uncertainty_deadline_unix_ms: i64,
    ) -> Result<(), StorageError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = transaction.execute(
            "UPDATE jobs
             SET native_job_id = ?1, updated_unix_ms = ?2
             WHERE job_id = ?3",
            params![native_job_id, next_observe_unix_ms, job_id],
        )?;
        if changed != 1 {
            return Err(StorageError::JobNotFound(job_id.to_owned()));
        }
        transaction.execute(
            "INSERT INTO job_reconciliation(
               job_id, next_observe_unix_ms, uncertainty_deadline_unix_ms
             ) VALUES (?1, ?2, ?3)
             ON CONFLICT(job_id) DO UPDATE SET
               next_observe_unix_ms = excluded.next_observe_unix_ms,
               uncertainty_deadline_unix_ms = excluded.uncertainty_deadline_unix_ms",
            params![job_id, next_observe_unix_ms, uncertainty_deadline_unix_ms],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Atomically records native acceptance, publishes its durable event, and
    /// creates the restart-safe reconciliation schedule.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown job, invalid details, or failed
    /// transaction.
    #[allow(clippy::too_many_arguments)]
    pub fn record_native_acceptance(
        &mut self,
        event_id: &str,
        job_id: &str,
        native_job_id: &str,
        details_json: &str,
        observed_unix_ms: i64,
        next_observe_unix_ms: i64,
        uncertainty_deadline_unix_ms: i64,
    ) -> Result<(), StorageError> {
        let _: serde_json::Value = serde_json::from_str(details_json)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let sequence: i64 = transaction.query_row(
            "SELECT COALESCE(MAX(job_sequence), 0) + 1
             FROM job_events WHERE job_id = ?1",
            [job_id],
            |row| row.get(0),
        )?;
        append_event_tx(
            &transaction,
            event_id,
            job_id,
            sequence,
            "accepted_by_spooler",
            None,
            Some("Operating system accepted the job"),
            details_json,
            observed_unix_ms,
        )?;
        let changed = transaction.execute(
            "UPDATE jobs
             SET native_job_id = ?1, state = 'accepted_by_spooler',
                 updated_unix_ms = ?2
             WHERE job_id = ?3 AND state = 'spool_intent'",
            params![native_job_id, observed_unix_ms, job_id],
        )?;
        if changed != 1 {
            return Err(StorageError::JobNotFound(job_id.to_owned()));
        }
        transaction.execute(
            "INSERT INTO job_reconciliation(
               job_id, next_observe_unix_ms, uncertainty_deadline_unix_ms
             ) VALUES (?1, ?2, ?3)
             ON CONFLICT(job_id) DO UPDATE SET
               next_observe_unix_ms = excluded.next_observe_unix_ms,
               uncertainty_deadline_unix_ms = excluded.uncertainty_deadline_unix_ms",
            params![job_id, next_observe_unix_ms, uncertainty_deadline_unix_ms],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Returns active native jobs whose durable observation deadline is due.
    ///
    /// # Errors
    ///
    /// Returns an error when `SQLite` cannot execute or decode the query.
    pub fn due_reconciliations(
        &self,
        now_unix_ms: i64,
        limit: usize,
    ) -> Result<Vec<ReconciliationItem>, StorageError> {
        let bounded = i64::try_from(limit.clamp(1, 100)).unwrap_or(100);
        let mut statement = self.connection.prepare(
            "SELECT j.job_id, j.submission_id, j.printer_id, j.printer_native_id,
                    j.printer_sequence, j.title, j.content_sha256, j.content_path,
                    j.content_kind, j.options_json, j.state, j.expires_unix_ms,
                    j.native_job_id, j.target_id, j.binding_id, j.profile_id,
                    j.profile_revision, j.stock_id, j.loaded_media_snapshot_json,
                    r.next_observe_unix_ms,
                    r.uncertainty_deadline_unix_ms, r.attempt_count,
                    r.cancel_requested
             FROM job_reconciliation r
             JOIN jobs j ON j.job_id = r.job_id
             WHERE r.next_observe_unix_ms <= ?1
               AND j.state IN (
                 'accepted_by_spooler', 'spooling', 'printing', 'blocked',
                 'cancel_requested'
               )
             ORDER BY r.next_observe_unix_ms, j.job_id
             LIMIT ?2",
        )?;
        let rows = statement.query_map(params![now_unix_ms, bounded], |row| {
            Ok(ReconciliationItem {
                job: LocalJob {
                    job_id: row.get(0)?,
                    submission_id: row.get(1)?,
                    printer_id: row.get(2)?,
                    printer_native_id: row.get(3)?,
                    printer_sequence: row.get(4)?,
                    title: row.get(5)?,
                    content_sha256: row.get(6)?,
                    content_path: row.get(7)?,
                    content_kind: row.get(8)?,
                    options_json: row.get(9)?,
                    state: row.get(10)?,
                    expires_unix_ms: row.get(11)?,
                    native_job_id: row.get(12)?,
                    target_id: row.get(13)?,
                    binding_id: row.get(14)?,
                    profile_id: row.get(15)?,
                    profile_revision: row.get(16)?,
                    stock_id: row.get(17)?,
                    loaded_media_snapshot_json: row.get(18)?,
                },
                next_observe_unix_ms: row.get(19)?,
                uncertainty_deadline_unix_ms: row.get(20)?,
                attempt_count: row.get(21)?,
                cancel_requested: row.get(22)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Records one native observation and schedules the next bounded attempt.
    ///
    /// # Errors
    ///
    /// Returns an error when the job is unknown or the transaction fails.
    #[allow(clippy::too_many_arguments)]
    pub fn record_reconciliation_attempt(
        &mut self,
        job_id: &str,
        native_job_id: &str,
        native_state: &str,
        authority: &str,
        details_json: &str,
        error_code: Option<&str>,
        observed_unix_ms: i64,
        next_observe_unix_ms: i64,
    ) -> Result<(), StorageError> {
        let _: serde_json::Value = serde_json::from_str(details_json)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT INTO native_observations(
               observation_id, job_id, native_job_id, state, authority,
               details_json, observed_unix_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                EventId::new().to_string(),
                job_id,
                native_job_id,
                native_state,
                authority,
                details_json,
                observed_unix_ms
            ],
        )?;
        let changed = transaction.execute(
            "UPDATE job_reconciliation SET
               next_observe_unix_ms = ?1,
               attempt_count = attempt_count + 1,
               last_native_state = ?2,
               last_error_code = ?3,
               cancel_requested = 0
             WHERE job_id = ?4",
            params![next_observe_unix_ms, native_state, error_code, job_id],
        )?;
        if changed != 1 {
            return Err(StorageError::JobNotFound(job_id.to_owned()));
        }
        transaction.commit()?;
        Ok(())
    }

    /// Stops native reconciliation after a truthful terminal event.
    ///
    /// # Errors
    ///
    /// Returns an error when `SQLite` cannot remove the schedule.
    pub fn finish_reconciliation(&mut self, job_id: &str) -> Result<(), StorageError> {
        self.connection
            .execute("DELETE FROM job_reconciliation WHERE job_id = ?1", [job_id])?;
        Ok(())
    }

    /// Marks an active native job for cancellation and immediate executor
    /// attention. Pre-handoff jobs use the existing local cancellation path.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown job or failed durable transition.
    pub fn request_cancel(
        &mut self,
        job_id: &str,
        observed_unix_ms: i64,
    ) -> Result<bool, StorageError> {
        let Some(job) = self.get_job(job_id)? else {
            return Err(StorageError::JobNotFound(job_id.to_owned()));
        };
        if matches!(
            job.state.as_str(),
            "cloud_accept_pending" | "queued_local" | "failed_retryable"
        ) {
            return self.cancel_before_handoff(job_id, observed_unix_ms);
        }
        if !matches!(
            job.state.as_str(),
            "accepted_by_spooler" | "spooling" | "printing" | "blocked"
        ) {
            return Ok(false);
        }
        self.append_next_event(
            &EventId::new().to_string(),
            job_id,
            "cancel_requested",
            Some("cancelled_by_server"),
            Some("Cancellation requested after native handoff"),
            "{}",
            observed_unix_ms,
        )?;
        self.connection.execute(
            "UPDATE job_reconciliation
             SET cancel_requested = 1, next_observe_unix_ms = ?1
             WHERE job_id = ?2",
            params![observed_unix_ms, job_id],
        )?;
        Ok(true)
    }

    /// Returns unacknowledged outbound events after the supplied cursor.
    ///
    /// # Errors
    ///
    /// Returns an error when `SQLite` cannot execute or decode the query.
    pub fn pending_events(
        &self,
        after_sequence: i64,
        limit: usize,
    ) -> Result<Vec<PendingEvent>, StorageError> {
        let bounded = i64::try_from(limit.clamp(1, 500)).unwrap_or(500);
        let mut statement = self.connection.prepare(
            "SELECT outbox_sequence, event_id, job_id, job_sequence, state,
                    reason, message, details_json, observed_unix_ms
             FROM event_outbox
             WHERE acknowledged_unix_ms IS NULL AND outbox_sequence > ?1
             ORDER BY outbox_sequence LIMIT ?2",
        )?;
        let rows = statement.query_map(params![after_sequence, bounded], |row| {
            Ok(PendingEvent {
                outbox_sequence: row.get(0)?,
                event_id: row.get(1)?,
                job_id: row.get(2)?,
                job_sequence: row.get(3)?,
                state: row.get(4)?,
                reason: row.get(5)?,
                message: row.get(6)?,
                details_json: row.get(7)?,
                observed_unix_ms: row.get(8)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Returns only cloud-managed events for signed agent synchronization.
    ///
    /// # Errors
    ///
    /// Returns an error when `SQLite` cannot execute or decode the query.
    pub fn pending_cloud_events(
        &self,
        after_sequence: i64,
        limit: usize,
    ) -> Result<Vec<PendingEvent>, StorageError> {
        let bounded = i64::try_from(limit.clamp(1, 500)).unwrap_or(500);
        let mut statement = self.connection.prepare(
            "SELECT outbox.outbox_sequence, outbox.event_id, outbox.job_id,
                    outbox.job_sequence, outbox.state, outbox.reason,
                    outbox.message, outbox.details_json, outbox.observed_unix_ms
             FROM event_outbox outbox
             JOIN jobs ON jobs.job_id = outbox.job_id
             WHERE jobs.cloud_managed = 1
               AND outbox.acknowledged_unix_ms IS NULL
               AND outbox.outbox_sequence > ?1
             ORDER BY outbox.outbox_sequence LIMIT ?2",
        )?;
        let rows = statement.query_map(params![after_sequence, bounded], |row| {
            Ok(PendingEvent {
                outbox_sequence: row.get(0)?,
                event_id: row.get(1)?,
                job_id: row.get(2)?,
                job_sequence: row.get(3)?,
                state: row.get(4)?,
                reason: row.get(5)?,
                message: row.get(6)?,
                details_json: row.get(7)?,
                observed_unix_ms: row.get(8)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Returns the newest unacknowledged cloud event sequence.
    ///
    /// This lets the runtime wake cloud synchronization when the local
    /// executor records a new state without reading the full outbox.
    ///
    /// # Errors
    ///
    /// Returns an error when `SQLite` cannot execute the query.
    pub fn latest_pending_cloud_event_sequence(&self) -> Result<i64, StorageError> {
        self.connection
            .query_row(
                "SELECT COALESCE(MAX(outbox.outbox_sequence), 0)
                 FROM event_outbox outbox
                 JOIN jobs ON jobs.job_id = outbox.job_id
                 WHERE jobs.cloud_managed = 1
                   AND outbox.acknowledged_unix_ms IS NULL",
                [],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    /// Marks every outbound event through a cursor as acknowledged.
    ///
    /// # Errors
    ///
    /// Returns an error if `SQLite` cannot update the outbox.
    pub fn acknowledge_events(
        &mut self,
        through_sequence: i64,
        acknowledged_unix_ms: i64,
    ) -> Result<usize, StorageError> {
        Ok(self.connection.execute(
            "UPDATE event_outbox SET acknowledged_unix_ms = ?1
             WHERE outbox_sequence <= ?2 AND acknowledged_unix_ms IS NULL",
            params![acknowledged_unix_ms, through_sequence],
        )?)
    }

    /// Acknowledges cloud-managed events through a server-returned event ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the event cannot be found or `SQLite` cannot update
    /// the cloud outbox projection.
    pub fn acknowledge_cloud_event(
        &mut self,
        event_id: &str,
        acknowledged_unix_ms: i64,
    ) -> Result<usize, StorageError> {
        let sequence: i64 = self
            .connection
            .query_row(
                "SELECT outbox_sequence FROM event_outbox WHERE event_id = ?1",
                [event_id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| StorageError::JobNotFound(format!("event:{event_id}")))?;
        Ok(self.connection.execute(
            "UPDATE event_outbox
             SET acknowledged_unix_ms = ?1
             WHERE outbox_sequence <= ?2
               AND acknowledged_unix_ms IS NULL
               AND job_id IN (
                 SELECT job_id FROM jobs WHERE cloud_managed = 1
               )",
            params![acknowledged_unix_ms, sequence],
        )?)
    }

    /// Cancels a job that has not crossed the native handoff boundary.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown job or if either cancellation event
    /// cannot be persisted.
    pub fn cancel_before_handoff(
        &mut self,
        job_id: &str,
        observed_unix_ms: i64,
    ) -> Result<bool, StorageError> {
        let Some(job) = self.get_job(job_id)? else {
            return Err(StorageError::JobNotFound(job_id.to_owned()));
        };
        if job.state == "cloud_accept_pending" {
            self.terminalize_prepared_cloud_job(
                job_id,
                "cancelled",
                "cancelled_by_server",
                "Cancelled by the control plane before cloud acceptance",
                observed_unix_ms,
            )?;
            return Ok(true);
        }
        if !matches!(job.state.as_str(), "queued_local" | "failed_retryable") {
            return Ok(false);
        }
        self.append_next_event(
            &EventId::new().to_string(),
            job_id,
            "cancel_requested",
            Some("cancelled_by_server"),
            Some("Cancellation requested by control plane"),
            "{}",
            observed_unix_ms,
        )?;
        self.append_next_event(
            &EventId::new().to_string(),
            job_id,
            "cancelled",
            Some("cancelled_by_server"),
            Some("Cancelled before native handoff"),
            "{}",
            observed_unix_ms,
        )?;
        Ok(true)
    }

    /// Expires locally queued jobs whose policy deadline has elapsed.
    ///
    /// # Errors
    ///
    /// Returns an error if candidate discovery or any event transaction fails.
    pub fn expire_waiting(&mut self, now_unix_ms: i64) -> Result<usize, StorageError> {
        let job_ids = {
            let mut statement = self.connection.prepare(
                "SELECT job_id FROM jobs
                 WHERE state IN (
                    'cloud_accept_pending', 'queued_local', 'failed_retryable'
                 )
                   AND expires_unix_ms IS NOT NULL
                   AND expires_unix_ms <= ?1",
            )?;
            let rows = statement.query_map([now_unix_ms], |row| row.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        for job_id in &job_ids {
            if self
                .get_job(job_id)?
                .is_some_and(|job| job.state == "cloud_accept_pending")
            {
                self.terminalize_prepared_cloud_job(
                    job_id,
                    "expired",
                    "expired_before_handoff",
                    "Job expired before cloud acceptance",
                    now_unix_ms,
                )?;
                continue;
            }
            let sequence: i64 = self.connection.query_row(
                "SELECT COALESCE(MAX(job_sequence), 0) + 1 FROM job_events WHERE job_id = ?1",
                [job_id],
                |row| row.get(0),
            )?;
            self.append_event(
                &EventId::new().to_string(),
                job_id,
                sequence,
                "expired",
                Some("expired_before_handoff"),
                Some("Job expired before native submission"),
                "{}",
                now_unix_ms,
            )?;
        }
        Ok(job_ids.len())
    }

    fn terminalize_prepared_cloud_job(
        &mut self,
        job_id: &str,
        state: &str,
        reason: &str,
        message: &str,
        observed_unix_ms: i64,
    ) -> Result<(), StorageError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = query_job(&transaction, job_id)?
            .ok_or_else(|| StorageError::JobNotFound(job_id.to_owned()))?;
        if current.state != "cloud_accept_pending" {
            return Err(StorageError::InvalidLocalEvent(format!(
                "cloud job {job_id} cannot terminate from {}",
                current.state
            )));
        }
        transaction.execute(
            "DELETE FROM cloud_accept_intents WHERE job_id = ?1",
            [job_id],
        )?;
        transaction.execute(
            "UPDATE jobs SET state = ?2, updated_unix_ms = ?3 WHERE job_id = ?1",
            params![job_id, state, observed_unix_ms],
        )?;
        append_event_tx(
            &transaction,
            &EventId::new().to_string(),
            job_id,
            1,
            state,
            Some(reason),
            Some(message),
            "{}",
            observed_unix_ms,
        )?;
        transaction.commit()?;
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
fn append_event_tx(
    tx: &rusqlite::Transaction<'_>,
    event_id: &str,
    job_id: &str,
    job_sequence: i64,
    state: &str,
    reason: Option<&str>,
    message: Option<&str>,
    details_json: &str,
    observed_unix_ms: i64,
) -> Result<(), rusqlite::Error> {
    tx.execute(
        "INSERT INTO job_events
         (event_id, job_id, job_sequence, state, reason, message,
          details_json, observed_unix_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            event_id,
            job_id,
            job_sequence,
            state,
            reason,
            message,
            details_json,
            observed_unix_ms
        ],
    )?;
    tx.execute(
        "INSERT INTO event_outbox
         (event_id, job_id, job_sequence, state, reason, message,
          details_json, observed_unix_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            event_id,
            job_id,
            job_sequence,
            state,
            reason,
            message,
            details_json,
            observed_unix_ms
        ],
    )?;
    Ok(())
}

fn query_job(connection: &Connection, job_id: &str) -> Result<Option<LocalJob>, rusqlite::Error> {
    connection
        .query_row(
            "SELECT job_id, submission_id, printer_id, printer_native_id,
                    printer_sequence, title, content_sha256, content_path,
                    content_kind, options_json, state, expires_unix_ms,
                    native_job_id, target_id, binding_id, profile_id,
                    profile_revision, stock_id, loaded_media_snapshot_json
             FROM jobs WHERE job_id = ?1",
            [job_id],
            row_to_job,
        )
        .optional()
}

fn ensure_column(
    connection: &Connection,
    table: &str,
    name: &str,
    definition: &str,
) -> Result<(), rusqlite::Error> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    if !columns.iter().any(|column| column == name) {
        connection.execute_batch(&format!(
            "ALTER TABLE {table} ADD COLUMN {name} {definition}"
        ))?;
    }
    Ok(())
}

const STORED_PRINTER_QUERY: &str = "
    SELECT p.printer_id, p.native_id, p.name, p.state, p.is_default,
           p.present, COALESCE(e.exposed, 0),
           COALESCE(profile.portable_json, legacy.capabilities_json, '{}'),
           COALESCE(profile.native_options_json, '{}'),
           COALESCE(profile.revision, 0),
           p.observed_unix_ms
    FROM printers p
    LEFT JOIN printer_exposure e ON e.printer_id = p.printer_id
    LEFT JOIN printer_capabilities legacy
      ON legacy.printer_id = p.printer_id AND legacy.revision = 'current'
    LEFT JOIN printer_capability_snapshots profile
      ON profile.printer_id = p.printer_id
     AND profile.revision = (
       SELECT MAX(latest.revision) FROM printer_capability_snapshots latest
       WHERE latest.printer_id = p.printer_id
     )
    WHERE (?1 IS NULL OR p.printer_id = ?1)";

fn stored_printer_from_row(row: &rusqlite::Row<'_>) -> Result<StoredPrinter, rusqlite::Error> {
    Ok(StoredPrinter {
        printer_id: row.get(0)?,
        native_id: row.get(1)?,
        name: row.get(2)?,
        state: row.get(3)?,
        is_default: row.get(4)?,
        present: row.get(5)?,
        exposed: row.get(6)?,
        capabilities_json: row.get(7)?,
        native_options_json: row.get(8)?,
        profile_revision: row.get(9)?,
        observed_unix_ms: row.get(10)?,
    })
}

fn named_profile_from_row(row: &rusqlite::Row<'_>) -> Result<StoredNamedProfile, rusqlite::Error> {
    Ok(StoredNamedProfile {
        profile_id: row.get(0)?,
        printer_id: row.get(1)?,
        revision: row.get(2)?,
        name: row.get(3)?,
        is_default: row.get(4)?,
        options_json: row.get(5)?,
        status: row.get(6)?,
        native_kind: row.get(7)?,
        native_blob_id: row.get(8)?,
        native_digest: row.get(9)?,
        driver_fingerprint_json: row.get(10)?,
        summary_json: row.get(11)?,
        stock_id: row.get(12)?,
        safe_overrides_json: row.get(13)?,
        last_validated_unix_ms: row.get(14)?,
        last_test_job_id: row.get(15)?,
        published: row.get(16)?,
        uses_current_printer_defaults: row.get(17)?,
        updated_unix_ms: row.get(18)?,
    })
}

const NAMED_PROFILE_QUERY: &str = "
    SELECT profile_id, printer_id, revision, name, is_default,
           options_json, status, native_kind, native_blob_id, native_digest,
           driver_fingerprint_json, summary_json, stock_id,
           safe_overrides_json, last_validated_unix_ms, last_test_job_id,
           published, uses_current_printer_defaults, updated_unix_ms
    FROM printer_profiles
    WHERE printer_id = ?1 AND profile_id = ?2
      AND revision = (
        SELECT MAX(latest.revision) FROM printer_profiles latest
        WHERE latest.profile_id = ?2
      )
      AND deleted = 0
    ORDER BY revision DESC LIMIT 1";

fn query_named_profile(
    connection: &Connection,
    printer_id: &str,
    profile_id: &str,
) -> Result<Option<StoredNamedProfile>, rusqlite::Error> {
    connection
        .query_row(
            NAMED_PROFILE_QUERY,
            params![printer_id, profile_id],
            named_profile_from_row,
        )
        .optional()
}

fn query_current_printer_defaults_profile(
    connection: &Connection,
    printer_id: &str,
) -> Result<Option<StoredNamedProfile>, rusqlite::Error> {
    connection
        .query_row(
            "SELECT p.profile_id, p.printer_id, p.revision, p.name,
                    p.is_default, p.options_json, p.status, p.native_kind,
                    p.native_blob_id, p.native_digest,
                    p.driver_fingerprint_json, p.summary_json, p.stock_id,
                    p.safe_overrides_json, p.last_validated_unix_ms,
                    p.last_test_job_id, p.published,
                    p.uses_current_printer_defaults, p.updated_unix_ms
             FROM printer_profiles p
             WHERE p.printer_id = ?1
               AND p.revision = (
                 SELECT MAX(latest.revision) FROM printer_profiles latest
                 WHERE latest.profile_id = p.profile_id
               )
               AND p.deleted = 0
               AND p.uses_current_printer_defaults = 1
             ORDER BY p.revision DESC
             LIMIT 1",
            [printer_id],
            named_profile_from_row,
        )
        .optional()
}

#[allow(clippy::too_many_arguments)]
fn insert_named_profile_tx(
    transaction: &Connection,
    profile_id: &str,
    printer_id: &str,
    revision: u64,
    name: &str,
    is_default: bool,
    options_json: &str,
    deleted: bool,
    updated_unix_ms: i64,
) -> Result<(), StorageError> {
    if is_default {
        demote_default_profiles(transaction, printer_id, profile_id, updated_unix_ms)?;
    }
    transaction.execute(
        "INSERT INTO printer_profiles(
           profile_id, printer_id, revision, name, is_default,
           options_json, deleted, updated_unix_ms
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            profile_id,
            printer_id,
            revision,
            name,
            is_default,
            options_json,
            deleted,
            updated_unix_ms
        ],
    )?;
    Ok(())
}

fn demote_default_profiles(
    transaction: &Connection,
    printer_id: &str,
    profile_id: &str,
    updated_unix_ms: i64,
) -> Result<(), StorageError> {
    transaction.execute(
        "UPDATE printer_profiles
         SET is_default = 0, updated_unix_ms = ?3
         WHERE printer_id = ?1 AND profile_id <> ?2
           AND is_default = 1 AND deleted = 0
           AND revision = (
             SELECT MAX(latest.revision) FROM printer_profiles latest
             WHERE latest.profile_id = printer_profiles.profile_id
           )",
        params![printer_id, profile_id, updated_unix_ms],
    )?;
    Ok(())
}

fn validate_named_profile(name: &str, options_json: &str) -> Result<(), StorageError> {
    if name.trim().is_empty() {
        return Err(StorageError::InvalidPrinterProfile(
            "profile name cannot be empty".into(),
        ));
    }
    let _: JobOptions = serde_json::from_str(options_json)?;
    Ok(())
}

fn validate_native_capture(capture: &NativeProfileCapture) -> Result<(), StorageError> {
    validate_named_profile(&capture.name, &capture.options_json)?;
    if capture.native_blob.len() > MAX_NATIVE_PROFILE_BLOB_BYTES {
        return Err(StorageError::NativeBlobTooLarge(
            MAX_NATIVE_PROFILE_BLOB_BYTES,
        ));
    }
    if capture.native_schema_version == 0 {
        return Err(StorageError::InvalidPrinterProfile(
            "native profile schema version must be positive".into(),
        ));
    }
    let _: DriverFingerprint = serde_json::from_str(&capture.driver_fingerprint_json)?;
    let _: ProfileSummary = serde_json::from_str(&capture.summary_json)?;
    let _: Vec<ProfileDependency> = serde_json::from_str(&capture.dependencies_json)?;
    let _: Vec<SafeProfileOverride> = serde_json::from_str(&capture.safe_overrides_json)?;
    let encoded_kind = serde_json::to_string(&capture.native_kind)?;
    let _: NativeProfileKind = serde_json::from_str(&encoded_kind)?;
    let encoded_status = serde_json::to_string(&capture.status)?;
    let status: ProfileStatus = serde_json::from_str(&encoded_status)?;
    if !matches!(
        status,
        ProfileStatus::Draft
            | ProfileStatus::Ready
            | ProfileStatus::NeedsTest
            | ProfileStatus::InteractiveOnly
    ) {
        return Err(StorageError::InvalidPrinterProfile(
            "a newly captured profile has an invalid status".into(),
        ));
    }
    let expected = format!("sha256:{:x}", Sha256::digest(&capture.native_blob));
    if !constant_time_str_eq(&expected, &capture.native_digest) {
        return Err(StorageError::InvalidPrinterProfile(
            "native profile digest does not match blob".into(),
        ));
    }
    Ok(())
}

fn constant_time_str_eq(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.as_bytes()
        .iter()
        .zip(right.as_bytes())
        .fold(0_u8, |difference, (a, b)| difference | (a ^ b))
        == 0
}

fn upsert_cloud_accept_intent(
    connection: &Connection,
    job: &LocalJob,
    lease_id: &str,
    lease_token: &str,
    lease_expires_unix_ms: i64,
    prepared_unix_ms: i64,
) -> Result<(), StorageError> {
    connection.execute(
        "INSERT INTO cloud_accept_intents (
            job_id, lease_id, lease_token, lease_expires_unix_ms,
            content_sha256, local_sequence, prepared_unix_ms
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(job_id) DO UPDATE SET
            lease_id = excluded.lease_id,
            lease_token = excluded.lease_token,
            lease_expires_unix_ms = excluded.lease_expires_unix_ms,
            content_sha256 = excluded.content_sha256,
            local_sequence = excluded.local_sequence,
            prepared_unix_ms = excluded.prepared_unix_ms",
        params![
            job.job_id,
            lease_id,
            lease_token,
            lease_expires_unix_ms,
            job.content_sha256,
            job.printer_sequence,
            prepared_unix_ms,
        ],
    )?;
    Ok(())
}

fn row_to_cloud_accept_intent(
    row: &rusqlite::Row<'_>,
) -> Result<CloudAcceptIntent, rusqlite::Error> {
    let local_sequence: i64 = row.get(5)?;
    Ok(CloudAcceptIntent {
        job_id: row.get(0)?,
        lease_id: row.get(1)?,
        lease_token: row.get(2)?,
        lease_expires_unix_ms: row.get(3)?,
        content_sha256: row.get(4)?,
        local_sequence: u64::try_from(local_sequence).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                5,
                rusqlite::types::Type::Integer,
                Box::new(error),
            )
        })?,
    })
}

fn row_to_job(row: &rusqlite::Row<'_>) -> Result<LocalJob, rusqlite::Error> {
    Ok(LocalJob {
        job_id: row.get(0)?,
        submission_id: row.get(1)?,
        printer_id: row.get(2)?,
        printer_native_id: row.get(3)?,
        printer_sequence: row.get(4)?,
        title: row.get(5)?,
        content_sha256: row.get(6)?,
        content_path: row.get(7)?,
        content_kind: row.get(8)?,
        options_json: row.get(9)?,
        state: row.get(10)?,
        expires_unix_ms: row.get(11)?,
        native_job_id: row.get(12)?,
        target_id: row.get(13)?,
        binding_id: row.get(14)?,
        profile_id: row.get(15)?,
        profile_revision: row.get(16)?,
        stock_id: row.get(17)?,
        loaded_media_snapshot_json: row.get(18)?,
    })
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn configure_adds_confidential_retention_column_independently() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(SCHEMA).unwrap();
        connection
            .execute(
                "ALTER TABLE jobs DROP COLUMN confidential_delete_after_unix_ms",
                [],
            )
            .unwrap();
        let store = AgentStore::configure(connection).unwrap();
        let present: bool = store.connection.query_row(
            "SELECT EXISTS (SELECT 1 FROM pragma_table_info('jobs') WHERE name = 'confidential_delete_after_unix_ms')",
            [],
            |row| row.get(0),
        ).unwrap();
        assert!(present);
    }

    fn job(id: &str, printer: &str, accepted: i64) -> AcceptedJob {
        AcceptedJob {
            job_id: id.into(),
            submission_id: format!("sub-{id}"),
            printer_id: printer.into(),
            printer_native_id: format!("native-{printer}"),
            title: "test".into(),
            content_sha256: format!("sha-{id}"),
            content_path: format!("/content/{id}"),
            content_kind: "pdf".into(),
            options_json: "{}".into(),
            expires_unix_ms: None,
            accepted_unix_ms: accepted,
            cloud_managed: false,
        }
    }

    fn native_capture(blob: Vec<u8>) -> NativeProfileCapture {
        NativeProfileCapture {
            name: "A4 colour".into(),
            is_default: true,
            options_json: serde_json::to_string(&JobOptions::default()).unwrap(),
            status: "ready".into(),
            native_kind: "macos_printcore".into(),
            native_schema_version: 1,
            native_digest: format!("sha256:{:x}", Sha256::digest(&blob)),
            native_blob: blob,
            driver_fingerprint_json: serde_json::to_string(&DriverFingerprint {
                platform: "macos".into(),
                driver_name: "HP".into(),
                native_queue_id: "native-hp".into(),
                ..DriverFingerprint::default()
            })
            .unwrap(),
            summary_json: serde_json::to_string(&ProfileSummary::default()).unwrap(),
            stock_id: None,
            dependencies_json: serde_json::to_string(&vec![ProfileDependency {
                kind: "driver".into(),
                value: "HP 1.0".into(),
            }])
            .unwrap(),
            safe_overrides_json: r#"["copies","pages"]"#.into(),
            published: true,
        }
    }

    #[test]
    fn accept_is_atomic_idempotent_and_emits_outbox_event() {
        let mut store = AgentStore::in_memory().unwrap();
        let first = store.accept_job(&job("1", "p1", 10)).unwrap();
        let duplicate = store.accept_job(&job("1", "p1", 10)).unwrap();
        assert_eq!(first, duplicate);
        assert_eq!(first.printer_sequence, 1);
        let events = store.pending_events(0, 100).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].state, "queued_local");
        assert!(store.integrity_check().unwrap());
    }

    #[test]
    fn classified_executor_health_survives_restart() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("health.sqlite");
        {
            let mut store = AgentStore::open(&database).unwrap();
            store.record_executor_failure("executor_crashed").unwrap();
            store.record_executor_failure("executor_timed_out").unwrap();
        }
        let store = AgentStore::open(&database).unwrap();
        assert_eq!(
            store.failure_health().unwrap(),
            (1, Some("executor_timed_out".into()))
        );
    }

    #[test]
    fn prepared_cloud_accept_survives_both_accept_crash_windows() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("agent.sqlite");
        let mut cloud = job("cloud", "p1", 10);
        cloud.cloud_managed = true;
        let lease_id = "lease-before-crash";
        {
            let mut store = AgentStore::open(&database).unwrap();
            let prepared = store
                .prepare_cloud_job(&cloud, lease_id, "secret-token", 30_000)
                .unwrap();
            assert_eq!(prepared.state, "cloud_accept_pending");
            assert!(store.runnable_heads(20).unwrap().is_empty());
            assert!(store.pending_events(0, 10).unwrap().is_empty());
        }

        let restarted_before_accept = AgentStore::open(&database).unwrap();
        let intents = restarted_before_accept.pending_cloud_accepts().unwrap();
        assert_eq!(intents.len(), 1);
        assert_eq!(intents[0].lease_id, lease_id);
        assert_eq!(intents[0].lease_token, "secret-token");
        assert!(!format!("{:?}", intents[0]).contains("secret-token"));
        assert!(
            restarted_before_accept
                .runnable_heads(20)
                .unwrap()
                .is_empty()
        );
        drop(restarted_before_accept);

        // Model a crash after the server durably accepted the exact intent but
        // before the client could activate its local queue.
        let mut restarted = AgentStore::open(&database).unwrap();
        assert_eq!(restarted.pending_cloud_accepts().unwrap(), intents);
        restarted.activate_cloud_job("cloud", 20).unwrap();
        restarted.activate_cloud_job("cloud", 21).unwrap();
        assert_eq!(restarted.runnable_heads(30).unwrap().len(), 1);
        assert_eq!(restarted.pending_events(0, 10).unwrap().len(), 1);
        assert!(restarted.pending_cloud_accepts().unwrap().is_empty());
    }

    #[test]
    fn repeated_cloud_offer_updates_only_lease_intent() {
        let mut store = AgentStore::in_memory().unwrap();
        let mut cloud = job("cloud", "p1", 10);
        cloud.cloud_managed = true;
        store
            .prepare_cloud_job(&cloud, "old-lease", "old-token", 30_000)
            .unwrap();
        let new_lease = "new-lease";
        let duplicate = store
            .prepare_cloud_job(&cloud, new_lease, "new-token", 60_000)
            .unwrap();
        assert_eq!(duplicate.printer_sequence, 1);
        let intents = store.pending_cloud_accepts().unwrap();
        assert_eq!(intents.len(), 1);
        assert_eq!(intents[0].lease_id, new_lease);
        assert_eq!(intents[0].lease_token, "new-token");
        assert!(store.pending_events(0, 10).unwrap().is_empty());
        assert!(store.runnable_heads(20).unwrap().is_empty());
    }

    #[test]
    fn cancel_and_expiry_terminalize_prepared_cloud_jobs_once() {
        let mut store = AgentStore::in_memory().unwrap();
        let mut cancelled = job("cancelled", "p1", 10);
        cancelled.cloud_managed = true;
        store
            .prepare_cloud_job(&cancelled, "cancel-lease", "cancel-token", 30_000)
            .unwrap();
        assert!(store.request_cancel("cancelled", 20).unwrap());
        assert_eq!(
            store.get_job("cancelled").unwrap().unwrap().state,
            "cancelled"
        );

        let mut expired = job("expired", "p2", 11);
        expired.cloud_managed = true;
        expired.expires_unix_ms = Some(25);
        store
            .prepare_cloud_job(&expired, "expire-lease", "expire-token", 30_000)
            .unwrap();
        assert_eq!(store.expire_waiting(26).unwrap(), 1);
        assert_eq!(store.get_job("expired").unwrap().unwrap().state, "expired");
        assert!(store.pending_cloud_accepts().unwrap().is_empty());
        assert!(store.runnable_heads(30).unwrap().is_empty());

        let events = store.pending_events(0, 10).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].state, "cancelled");
        assert_eq!(events[0].reason.as_deref(), Some("cancelled_by_server"));
        assert_eq!(events[1].state, "expired");
        assert_eq!(events[1].reason.as_deref(), Some("expired_before_handoff"));
    }

    #[test]
    fn duplicate_with_changed_digest_is_rejected() {
        let mut store = AgentStore::in_memory().unwrap();
        store.accept_job(&job("1", "p1", 10)).unwrap();
        let mut changed = job("1", "p1", 10);
        changed.content_sha256 = "different".into();
        assert!(matches!(
            store.accept_job(&changed),
            Err(StorageError::JobConflict(_))
        ));
    }

    #[test]
    fn runnable_heads_are_fifo_per_printer_and_parallel_between_printers() {
        let mut store = AgentStore::in_memory().unwrap();
        store.accept_job(&job("one", "p1", 10)).unwrap();
        store.accept_job(&job("two", "p1", 11)).unwrap();
        store.accept_job(&job("three", "p2", 12)).unwrap();
        let heads = store.runnable_heads(20).unwrap();
        assert_eq!(heads.len(), 2);
        assert!(heads.iter().any(|j| j.job_id == "one"));
        assert!(heads.iter().any(|j| j.job_id == "three"));
        assert!(!heads.iter().any(|j| j.job_id == "two"));
        assert_eq!(
            store.queue_counts().unwrap(),
            QueueCounts {
                queued: 3,
                active: 0
            }
        );
    }

    #[test]
    fn outbox_acknowledgement_is_monotonic() {
        let mut store = AgentStore::in_memory().unwrap();
        store.accept_job(&job("one", "p1", 10)).unwrap();
        store
            .append_event("one:2", "one", 2, "preparing", None, None, "{}", 20)
            .unwrap();
        assert_eq!(store.acknowledge_events(1, 30).unwrap(), 1);
        let events = store.pending_events(0, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].job_sequence, 2);
    }

    #[test]
    fn expires_only_jobs_before_handoff() {
        let mut store = AgentStore::in_memory().unwrap();
        let mut expiring = job("one", "p1", 10);
        expiring.expires_unix_ms = Some(15);
        store.accept_job(&expiring).unwrap();
        assert_eq!(store.expire_waiting(16).unwrap(), 1);
        assert_eq!(store.get_job("one").unwrap().unwrap().state, "expired");
    }

    #[test]
    fn cloud_outbox_is_isolated_and_acknowledged_by_event_id() {
        let mut store = AgentStore::in_memory().unwrap();
        store.accept_job(&job("local", "p1", 10)).unwrap();
        let mut cloud = job("cloud", "p2", 11);
        cloud.cloud_managed = true;
        store.accept_job(&cloud).unwrap();

        let cloud_events = store.pending_cloud_events(0, 10).unwrap();
        assert_eq!(cloud_events.len(), 1);
        assert_eq!(cloud_events[0].job_id, "cloud");
        assert_eq!(
            store.latest_pending_cloud_event_sequence().unwrap(),
            cloud_events[0].outbox_sequence
        );
        assert_eq!(
            store
                .acknowledge_cloud_event(&cloud_events[0].event_id, 20)
                .unwrap(),
            1
        );
        assert!(store.pending_cloud_events(0, 10).unwrap().is_empty());
        assert_eq!(store.latest_pending_cloud_event_sequence().unwrap(), 0);

        let remaining = store.pending_events(0, 10).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].job_id, "local");
    }

    #[test]
    fn cancellation_stops_at_the_native_handoff_boundary() {
        let mut store = AgentStore::in_memory().unwrap();
        store.accept_job(&job("waiting", "p1", 10)).unwrap();
        assert!(store.cancel_before_handoff("waiting", 20).unwrap());
        assert_eq!(
            store.get_job("waiting").unwrap().unwrap().state,
            "cancelled"
        );

        store.accept_job(&job("handed-off", "p2", 30)).unwrap();
        store
            .append_event(
                "handed-off:2",
                "handed-off",
                2,
                "preparing",
                None,
                None,
                "{}",
                31,
            )
            .unwrap();
        store
            .append_event(
                "handed-off:3",
                "handed-off",
                3,
                "spool_intent",
                None,
                None,
                "{}",
                32,
            )
            .unwrap();
        assert!(!store.cancel_before_handoff("handed-off", 33).unwrap());
        assert_eq!(
            store.get_job("handed-off").unwrap().unwrap().state,
            "spool_intent"
        );
    }

    #[test]
    fn settings_are_durable_json_strings() {
        let mut store = AgentStore::in_memory().unwrap();
        assert_eq!(store.setting("command_cursor").unwrap(), None);
        store
            .set_setting("command_cursor", "cursor-with-\"quotes\"")
            .unwrap();
        assert_eq!(
            store.setting("command_cursor").unwrap().as_deref(),
            Some("cursor-with-\"quotes\"")
        );
    }

    #[test]
    fn confidential_plaintext_is_retained_until_a_truthful_terminal_state() {
        let mut store = AgentStore::in_memory().unwrap();
        let retained_states = [
            "cloud_accept_pending",
            "queued_local",
            "spool_intent",
            "printing",
            "accepted_by_spooler",
        ];
        let terminal_states = [
            "completed_reported",
            "delivery_uncertain",
            "failed_terminal",
            "cancelled",
            "expired",
        ];

        for (index, state) in retained_states
            .iter()
            .chain(terminal_states.iter())
            .enumerate()
        {
            let job_id = format!("encrypted-{index}");
            let accepted_at = 1_000 + i64::try_from(index).expect("fixture index fits i64");
            let mut encrypted = job(&job_id, "printer", accepted_at);
            encrypted.cloud_managed = true;
            encrypted.content_sha256 = format!("sha-{index}");
            encrypted.content_path = format!("/content/confidential-{index}");
            store
                .prepare_cloud_job(&encrypted, "lease", "secret", 60_000)
                .unwrap();
            store
                .connection
                .execute(
                    "UPDATE jobs SET state = ?2,
                     confidential_delete_after_unix_ms = 1 WHERE job_id = ?1",
                    params![job_id, state],
                )
                .unwrap();
        }

        let mut due = store.confidential_files_due().unwrap();
        due.sort_by(|left, right| left.job_id.cmp(&right.job_id));
        assert_eq!(
            due.iter()
                .map(|file| file.job_id.as_str())
                .collect::<Vec<_>>(),
            (retained_states.len()..retained_states.len() + terminal_states.len())
                .map(|index| format!("encrypted-{index}"))
                .collect::<Vec<_>>()
        );
        assert!(!due.iter().any(|file| file.job_id == "encrypted-4"));

        for file in due {
            store.mark_confidential_file_deleted(&file.job_id).unwrap();
        }
        assert!(store.confidential_files_due().unwrap().is_empty());
    }

    #[test]
    fn opening_an_existing_store_neutralizes_legacy_nonterminal_deadlines() {
        let connection = Connection::open_in_memory().unwrap();
        let mut store = AgentStore::configure(connection).unwrap();
        for (job_id, state) in [
            ("queued", "queued_local"),
            ("handed-off", "accepted_by_spooler"),
            ("done", "completed_reported"),
        ] {
            let mut encrypted = job(job_id, "printer", 1_000);
            encrypted.cloud_managed = true;
            encrypted.content_sha256 = format!("sha-{job_id}");
            encrypted.content_path = format!("/content/confidential-{job_id}");
            store
                .prepare_cloud_job(&encrypted, "lease", "secret", 60_000)
                .unwrap();
            store.connection.execute(
                "UPDATE jobs SET state = ?2, confidential_delete_after_unix_ms = 1 WHERE job_id = ?1",
                params![job_id, state],
            ).unwrap();
        }

        let connection = store.connection;
        let reopened = AgentStore::configure(connection).unwrap();
        for job_id in ["queued", "handed-off"] {
            let deadline: Option<i64> = reopened
                .connection
                .query_row(
                    "SELECT confidential_delete_after_unix_ms FROM jobs WHERE job_id = ?1",
                    [job_id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(deadline, None, "legacy deadline remained for {job_id}");
        }
        let terminal_deadline: Option<i64> = reopened
            .connection
            .query_row(
                "SELECT confidential_delete_after_unix_ms FROM jobs WHERE job_id = 'done'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(terminal_deadline, Some(1));
        assert_eq!(reopened.confidential_files_due().unwrap()[0].job_id, "done");
    }

    #[test]
    fn printer_inventory_preserves_logical_to_native_mapping() {
        let mut store = AgentStore::in_memory().unwrap();
        let first = store
            .upsert_printer(
                "native-queue",
                "Office",
                "online",
                true,
                r#"{"color":true}"#,
                10,
            )
            .unwrap();
        let refreshed = store
            .upsert_printer(
                "native-queue",
                "Office renamed",
                "busy",
                false,
                r#"{"color":false}"#,
                20,
            )
            .unwrap();
        assert_eq!(first.printer_id, refreshed.printer_id);
        assert_eq!(
            store
                .native_printer_id(&first.printer_id)
                .unwrap()
                .as_deref(),
            Some("native-queue")
        );
        assert_eq!(refreshed.name, "Office renamed");
    }

    #[test]
    fn printer_exposure_defaults_off_and_profile_revisions_are_immutable() {
        let mut store = AgentStore::in_memory().unwrap();
        let printer = store
            .upsert_printer(
                "native",
                "Office",
                "online",
                true,
                &serde_json::to_string(&PrinterCapabilities::default()).unwrap(),
                10,
            )
            .unwrap();
        assert!(!store.printer(&printer.printer_id).unwrap().unwrap().exposed);
        let first = store
            .store_printer_profile(
                &printer.printer_id,
                None,
                &serde_json::to_string(&PrinterCapabilities::default()).unwrap(),
                "{}",
                10,
            )
            .unwrap();
        let unchanged = store
            .store_printer_profile(&printer.printer_id, None, &first.portable_json, "{}", 20)
            .unwrap();
        assert_eq!(first.revision, unchanged.revision);

        let changed = PrinterCapabilities {
            color: true,
            ..Default::default()
        };
        let second = store
            .store_printer_profile(
                &printer.printer_id,
                Some(first.revision),
                &serde_json::to_string(&changed).unwrap(),
                "{}",
                30,
            )
            .unwrap();
        assert_eq!(second.revision, first.revision + 1);
        assert!(matches!(
            store.store_printer_profile(
                &printer.printer_id,
                Some(first.revision),
                &second.portable_json,
                "{}",
                40
            ),
            Err(StorageError::ProfileRevisionConflict { .. })
        ));

        store
            .set_printer_exposed(&printer.printer_id, true, 50)
            .unwrap();
        let stored = store.printer(&printer.printer_id).unwrap().unwrap();
        assert!(stored.exposed);
        assert_eq!(stored.profile_revision, second.revision);
        assert!(stored.is_default);
    }

    #[test]
    fn discovery_default_profile_is_idempotent_and_restored_after_deletion() {
        let mut store = AgentStore::in_memory().unwrap();
        let printer = store
            .upsert_printer(
                "native-defaults",
                "Office",
                "online",
                true,
                &serde_json::to_string(&PrinterCapabilities::default()).unwrap(),
                10,
            )
            .unwrap();

        let first = store
            .ensure_current_printer_defaults_profile(&printer.printer_id, 10)
            .unwrap();
        let unchanged = store
            .ensure_current_printer_defaults_profile(&printer.printer_id, 20)
            .unwrap();
        assert_eq!(unchanged.profile_id, first.profile_id);
        assert_eq!(first.name, "Current printer defaults");
        assert!(first.is_default);
        assert!(!first.published);
        assert!(first.uses_current_printer_defaults);
        assert_eq!(first.native_blob_id, None);
        assert_eq!(
            serde_json::from_str::<JobOptions>(&first.options_json).unwrap(),
            JobOptions::default()
        );
        assert_eq!(store.named_profiles(&printer.printer_id).unwrap().len(), 1);
        store
            .record_profile_test_result(&first.profile_id, first.revision, "job_test", true, 25)
            .unwrap();
        assert!(
            !store
                .named_profile(&printer.printer_id, &first.profile_id)
                .unwrap()
                .unwrap()
                .published
        );

        store
            .delete_named_profile(&printer.printer_id, &first.profile_id, first.revision, 30)
            .unwrap();
        let restored = store
            .ensure_current_printer_defaults_profile(&printer.printer_id, 40)
            .unwrap();
        assert_ne!(restored.profile_id, first.profile_id);
        assert!(restored.uses_current_printer_defaults);
        assert_eq!(store.named_profiles(&printer.printer_id).unwrap().len(), 1);
    }

    #[test]
    fn discovery_default_supplements_user_profiles_without_replacing_them() {
        let mut store = AgentStore::in_memory().unwrap();
        let printer = store
            .upsert_printer(
                "native-user-profile",
                "Office",
                "online",
                true,
                &serde_json::to_string(&PrinterCapabilities::default()).unwrap(),
                10,
            )
            .unwrap();
        let user = store
            .create_named_profile(
                &printer.printer_id,
                "Labels",
                false,
                &serde_json::to_string(&JobOptions::default()).unwrap(),
                11,
            )
            .unwrap();

        let generated = store
            .ensure_current_printer_defaults_profile(&printer.printer_id, 20)
            .unwrap();
        assert_ne!(generated.profile_id, user.profile_id);
        assert!(generated.uses_current_printer_defaults);
        assert!(generated.is_default);
        assert_eq!(store.named_profiles(&printer.printer_id).unwrap().len(), 2);
    }

    #[test]
    fn named_profiles_are_multi_profile_versioned_and_tombstoned() {
        let mut store = AgentStore::in_memory().unwrap();
        let printer = store
            .upsert_printer(
                "native",
                "Office",
                "online",
                false,
                &serde_json::to_string(&PrinterCapabilities::default()).unwrap(),
                10,
            )
            .unwrap();
        let colour = store
            .create_named_profile(
                &printer.printer_id,
                "A4 Colour",
                true,
                &serde_json::to_string(&JobOptions::default()).unwrap(),
                20,
            )
            .unwrap();
        let black_mark = store
            .create_named_profile(
                &printer.printer_id,
                "Black Mark",
                false,
                &serde_json::to_string(&JobOptions::default()).unwrap(),
                21,
            )
            .unwrap();
        assert_ne!(colour.profile_id, black_mark.profile_id);
        assert_eq!(store.named_profiles(&printer.printer_id).unwrap().len(), 2);
        let updated = store
            .update_named_profile(
                &printer.printer_id,
                &black_mark.profile_id,
                black_mark.revision,
                "Black Mark Labels",
                true,
                &black_mark.options_json,
                30,
            )
            .unwrap();
        assert_eq!(updated.revision, 2);
        let profiles = store.named_profiles(&printer.printer_id).unwrap();
        assert_eq!(
            profiles.iter().filter(|profile| profile.is_default).count(),
            1
        );
        assert_eq!(
            profiles
                .iter()
                .find(|profile| profile.profile_id == colour.profile_id)
                .unwrap()
                .revision,
            1
        );
        store
            .record_profile_test_result(&updated.profile_id, updated.revision, "job_test", true, 35)
            .unwrap();
        let tested = store
            .named_profile_revision(&printer.printer_id, &updated.profile_id, updated.revision)
            .unwrap()
            .unwrap();
        assert_eq!(tested.status, "ready");
        assert!(tested.published);
        assert_eq!(tested.last_test_job_id.as_deref(), Some("job_test"));
        store
            .delete_named_profile(
                &printer.printer_id,
                &updated.profile_id,
                updated.revision,
                40,
            )
            .unwrap();
        assert!(
            store
                .named_profile(&printer.printer_id, &updated.profile_id)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn per_printer_queue_counts_are_not_truncated_by_detail_limits() {
        let mut store = AgentStore::in_memory().unwrap();
        for index in 0..501 {
            store
                .accept_job(&job(&format!("job-{index}"), "large-queue", index))
                .unwrap();
        }
        assert_eq!(
            store.printer_queue_counts("large-queue").unwrap(),
            QueueCounts {
                queued: 501,
                active: 0
            }
        );
        assert_eq!(
            store.jobs_for_printer("large-queue", 200).unwrap().len(),
            200
        );
    }

    #[test]
    fn authoritative_discovery_hides_absent_queues_without_losing_history() {
        let mut store = AgentStore::in_memory().unwrap();
        let present = store
            .upsert_printer(
                "native-present",
                "Present",
                "online",
                true,
                &serde_json::to_string(&PrinterCapabilities::default()).unwrap(),
                10,
            )
            .unwrap();
        let stale = store
            .upsert_printer(
                "native-stale",
                "Stale",
                "offline",
                false,
                &serde_json::to_string(&PrinterCapabilities::default()).unwrap(),
                10,
            )
            .unwrap();
        let profile = store
            .create_named_profile(
                &stale.printer_id,
                "Historical profile",
                true,
                &serde_json::to_string(&JobOptions::default()).unwrap(),
                11,
            )
            .unwrap();
        store
            .reconcile_printer_presence(&["native-present".into()])
            .unwrap();

        let active = store.present_printers().unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].printer_id, present.printer_id);
        let retained = store.printer(&stale.printer_id).unwrap().unwrap();
        assert!(!retained.present);
        assert_eq!(
            store
                .named_profile(&stale.printer_id, &profile.profile_id)
                .unwrap()
                .unwrap()
                .name,
            "Historical profile"
        );
        assert_eq!(store.present_printer_warning_count().unwrap(), 0);

        let reappeared = store
            .upsert_printer(
                "native-stale",
                "Stale renamed",
                "error",
                false,
                &serde_json::to_string(&PrinterCapabilities::default()).unwrap(),
                20,
            )
            .unwrap();
        store
            .reconcile_printer_presence(&["native-present".into(), "native-stale".into()])
            .unwrap();
        assert_eq!(reappeared.printer_id, stale.printer_id);
        assert_eq!(store.present_printers().unwrap().len(), 2);
        assert_eq!(store.present_printer_warning_count().unwrap(), 1);
    }

    #[test]
    fn capturing_discovery_default_preserves_default_identity_and_fixes_snapshot() {
        let mut store = AgentStore::in_memory().unwrap();
        let printer = store
            .upsert_printer(
                "native-live-default",
                "Office",
                "online",
                true,
                &serde_json::to_string(&PrinterCapabilities::default()).unwrap(),
                10,
            )
            .unwrap();
        let dynamic = store
            .ensure_current_printer_defaults_profile(&printer.printer_id, 11)
            .unwrap();
        store
            .create_profile_capture_session(
                "pcs_default_edit",
                "token-digest",
                &printer.printer_id,
                Some(&dynamic.profile_id),
                Some(dynamic.revision),
                "edit",
                "501",
                310_000,
                12,
            )
            .unwrap();
        let captured = store
            .commit_profile_capture(
                "pcs_default_edit",
                "token-digest",
                "501",
                &NativeProfileCapture {
                    is_default: false,
                    ..native_capture(b"fixed-driver-settings".to_vec())
                },
                20,
            )
            .unwrap();
        assert_eq!(captured.profile_id, dynamic.profile_id);
        assert_eq!(captured.revision, dynamic.revision + 1);
        assert!(captured.is_default);
        assert!(!captured.uses_current_printer_defaults);
        assert!(captured.native_blob_id.is_some());
    }

    #[test]
    fn native_capture_is_single_use_digest_checked_and_revisioned() {
        let mut store = AgentStore::in_memory().unwrap();
        let printer = store
            .upsert_printer(
                "native-hp",
                "HP",
                "online",
                true,
                &serde_json::to_string(&PrinterCapabilities::default()).unwrap(),
                10,
            )
            .unwrap();
        store
            .create_profile_capture_session(
                "pcs_create",
                "token-digest",
                &printer.printer_id,
                None,
                None,
                "create",
                "501",
                310_000,
                10_000,
            )
            .unwrap();
        let blob = b"opaque-native-settings".to_vec();
        let capture = native_capture(blob.clone());
        let first = store
            .commit_profile_capture("pcs_create", "token-digest", "501", &capture, 20_000)
            .unwrap();
        assert_eq!(first.revision, 1);
        assert_eq!(first.status, "ready");
        assert_eq!(first.native_kind, "macos_printcore");
        assert!(first.published);
        assert_eq!(
            store
                .native_profile_blob(&first.profile_id, 1)
                .unwrap()
                .unwrap()
                .native_blob,
            blob
        );
        assert!(matches!(
            store.commit_profile_capture("pcs_create", "token-digest", "501", &capture, 21_000),
            Err(StorageError::CaptureSessionNotAuthorized(_))
        ));

        store
            .create_profile_capture_session(
                "pcs_edit",
                "edit-token-digest",
                &printer.printer_id,
                Some(&first.profile_id),
                Some(1),
                "edit",
                "501",
                320_000,
                30_000,
            )
            .unwrap();
        let second = store
            .commit_profile_capture(
                "pcs_edit",
                "edit-token-digest",
                "501",
                &NativeProfileCapture {
                    name: "A4 colour best".into(),
                    ..capture.clone()
                },
                40_000,
            )
            .unwrap();
        assert_eq!(second.profile_id, first.profile_id);
        assert_eq!(second.revision, 2);
        assert!(
            store
                .native_profile_blob(&first.profile_id, 1)
                .unwrap()
                .is_some()
        );
        assert!(
            store
                .native_profile_blob(&first.profile_id, 2)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn default_demotion_preserves_native_revision_blob_and_dependencies() {
        let mut store = AgentStore::in_memory().unwrap();
        let printer = store
            .upsert_printer(
                "native-hp",
                "HP",
                "online",
                true,
                &serde_json::to_string(&PrinterCapabilities::default()).unwrap(),
                10,
            )
            .unwrap();
        let blob = b"opaque-native-settings".to_vec();
        let capture = native_capture(blob.clone());
        for (session_id, token, name) in [
            ("pcs_first", "first-token", "A4 colour"),
            ("pcs_second", "second-token", "A4 draft"),
        ] {
            store
                .create_profile_capture_session(
                    session_id,
                    token,
                    &printer.printer_id,
                    None,
                    None,
                    "create",
                    "501",
                    310_000,
                    10_000,
                )
                .unwrap();
            store
                .commit_profile_capture(
                    session_id,
                    token,
                    "501",
                    &NativeProfileCapture {
                        name: name.into(),
                        ..capture.clone()
                    },
                    20_000,
                )
                .unwrap();
        }

        let profiles = store.named_profiles(&printer.printer_id).unwrap();
        let first = profiles
            .iter()
            .find(|profile| profile.name == "A4 colour")
            .unwrap();
        assert_eq!(first.revision, 1);
        assert!(!first.is_default);
        assert_eq!(
            store
                .native_profile_blob(&first.profile_id, first.revision)
                .unwrap()
                .unwrap()
                .native_blob,
            blob
        );
        assert_eq!(
            store
                .profile_dependencies(&first.profile_id, first.revision)
                .unwrap(),
            vec![ProfileDependency {
                kind: "driver".into(),
                value: "HP 1.0".into(),
            }]
        );
    }

    #[test]
    fn capture_authorization_rejects_invalid_create_context_and_long_lifetimes() {
        let mut store = AgentStore::in_memory().unwrap();
        let printer = store
            .upsert_printer(
                "native-hp",
                "HP",
                "online",
                true,
                &serde_json::to_string(&PrinterCapabilities::default()).unwrap(),
                10,
            )
            .unwrap();

        for (profile_id, expected_revision) in [(Some("prf_existing"), None), (None, Some(1))] {
            assert!(matches!(
                store.create_profile_capture_session(
                    "pcs_invalid_create",
                    "digest",
                    &printer.printer_id,
                    profile_id,
                    expected_revision,
                    "create",
                    "501",
                    20_000,
                    10_000,
                ),
                Err(StorageError::InvalidPrinterProfile(_))
            ));
        }
        assert!(matches!(
            store.create_profile_capture_session(
                "pcs_too_long",
                "digest",
                &printer.printer_id,
                None,
                None,
                "create",
                "501",
                610_001,
                10_000,
            ),
            Err(StorageError::InvalidPrinterProfile(_))
        ));
        store
            .create_profile_capture_session(
                "pcs_max_lifetime",
                "digest",
                &printer.printer_id,
                None,
                None,
                "create",
                "501",
                610_000,
                10_000,
            )
            .unwrap();
    }

    #[test]
    fn stock_target_and_job_profile_pin_are_durable() {
        let mut store = AgentStore::in_memory().unwrap();
        let printer = store
            .upsert_printer(
                "native-label",
                "Label",
                "online",
                false,
                &serde_json::to_string(&PrinterCapabilities::default()).unwrap(),
                1,
            )
            .unwrap();
        store
            .upsert_physical_device("dev_test", "Label device", None, "operator", "{}", 2)
            .unwrap();
        store
            .bind_printer_device(&printer.printer_id, "dev_test", "operator", Some("user"), 3)
            .unwrap();
        store
            .upsert_stock(
                "stk_test",
                "80 mm matte",
                Some("LABEL-80"),
                "roll_label",
                r#"{"width_mm":80}"#,
                false,
                4,
            )
            .unwrap();
        let media = StoredLoadedMedia {
            device_id: "dev_test".into(),
            source: "roll".into(),
            stock_id: Some("stk_test".into()),
            confidence: "operator_confirmed".into(),
            confirmed_unix_ms: 5,
            confirmed_by: Some("user".into()),
        };
        store.confirm_loaded_media(&media).unwrap();
        assert_eq!(store.loaded_media("dev_test", "roll").unwrap(), Some(media));

        let profile = store
            .create_named_profile(
                &printer.printer_id,
                "Legacy profile",
                true,
                &serde_json::to_string(&JobOptions::default()).unwrap(),
                6,
            )
            .unwrap();
        assert_eq!(profile.status, "needs_test");
        assert_eq!(profile.native_kind, "portable_options");
        store
            .upsert_target(&StoredTarget {
                target_id: "tgt_test".into(),
                name: "80 mm target".into(),
                stock_id: Some("stk_test".into()),
                routing_policy: "primary_only".into(),
                published: true,
                retired: false,
                updated_unix_ms: 7,
            })
            .unwrap();
        store
            .upsert_target_binding(&StoredTargetBinding {
                binding_id: "bnd_test".into(),
                target_id: "tgt_test".into(),
                agent_id: "agt_test".into(),
                printer_id: printer.printer_id.clone(),
                profile_id: profile.profile_id.clone(),
                profile_revision: profile.revision,
                role: "primary".into(),
                priority: 0,
                enabled: true,
                created_unix_ms: 8,
            })
            .unwrap();
        store
            .accept_job(&job("pinned", &printer.printer_id, 9))
            .unwrap();
        store
            .pin_job_profile(
                "pinned",
                Some("tgt_test"),
                Some("bnd_test"),
                &profile.profile_id,
                profile.revision,
                Some("stk_test"),
                Some(r#"{"roll":"stk_test"}"#),
            )
            .unwrap();
        let pinned = store.get_job("pinned").unwrap().unwrap();
        assert_eq!(pinned.target_id.as_deref(), Some("tgt_test"));
        assert_eq!(pinned.binding_id.as_deref(), Some("bnd_test"));
        assert_eq!(
            pinned.profile_id.as_deref(),
            Some(profile.profile_id.as_str())
        );
        assert_eq!(pinned.profile_revision, Some(1));
        assert_eq!(pinned.stock_id.as_deref(), Some("stk_test"));
    }

    #[test]
    fn binding_and_job_pins_require_exact_existing_records() {
        let mut store = AgentStore::in_memory().unwrap();
        let printer = store
            .upsert_printer(
                "native-label",
                "Label",
                "online",
                false,
                &serde_json::to_string(&PrinterCapabilities::default()).unwrap(),
                1,
            )
            .unwrap();
        let first = store
            .create_named_profile(
                &printer.printer_id,
                "First",
                false,
                &serde_json::to_string(&JobOptions::default()).unwrap(),
                2,
            )
            .unwrap();
        store
            .upsert_target(&StoredTarget {
                target_id: "tgt_test".into(),
                name: "Target".into(),
                stock_id: None,
                routing_policy: "primary_only".into(),
                published: true,
                retired: false,
                updated_unix_ms: 3,
            })
            .unwrap();
        assert!(matches!(
            store.upsert_target_binding(&StoredTargetBinding {
                binding_id: "bnd_missing_revision".into(),
                target_id: "tgt_test".into(),
                agent_id: "agt_test".into(),
                printer_id: printer.printer_id.clone(),
                profile_id: first.profile_id.clone(),
                profile_revision: first.revision + 1,
                role: "primary".into(),
                priority: 0,
                enabled: true,
                created_unix_ms: 4,
            }),
            Err(StorageError::InvalidPrinterProfile(_))
        ));
        assert!(matches!(
            store.pin_job_profile(
                "missing",
                None,
                None,
                &first.profile_id,
                first.revision,
                None,
                None,
            ),
            Err(StorageError::JobNotFound(_))
        ));

        store
            .accept_job(&job("pinned", &printer.printer_id, 5))
            .unwrap();
        store
            .pin_job_profile(
                "pinned",
                None,
                None,
                &first.profile_id,
                first.revision,
                None,
                None,
            )
            .unwrap();
        let second = store
            .create_named_profile(
                &printer.printer_id,
                "Second",
                false,
                &serde_json::to_string(&JobOptions::default()).unwrap(),
                6,
            )
            .unwrap();
        assert!(matches!(
            store.pin_job_profile(
                "pinned",
                None,
                None,
                &second.profile_id,
                second.revision,
                None,
                None,
            ),
            Err(StorageError::JobConflict(_))
        ));
    }

    #[test]
    fn version_three_tables_gain_profile_and_job_columns() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("agent.sqlite3");
        {
            let old = Connection::open(&path).unwrap();
            old.execute_batch(
                "CREATE TABLE printer_profiles (
                   profile_id TEXT NOT NULL,
                   printer_id TEXT NOT NULL,
                   revision INTEGER NOT NULL,
                   name TEXT NOT NULL,
                   is_default INTEGER NOT NULL DEFAULT 0,
                   options_json TEXT NOT NULL,
                   deleted INTEGER NOT NULL DEFAULT 0,
                   updated_unix_ms INTEGER NOT NULL,
                   PRIMARY KEY(profile_id, revision)
                 );
                 CREATE TABLE jobs (
                   job_id TEXT PRIMARY KEY,
                   submission_id TEXT NOT NULL,
                   printer_id TEXT NOT NULL,
                   printer_native_id TEXT NOT NULL,
                   printer_sequence INTEGER NOT NULL,
                   title TEXT NOT NULL,
                   content_sha256 TEXT NOT NULL,
                   content_path TEXT NOT NULL,
                   content_kind TEXT NOT NULL,
                   options_json TEXT NOT NULL,
                   state TEXT NOT NULL,
                   expires_unix_ms INTEGER,
                   next_attempt_unix_ms INTEGER,
                   attempt_count INTEGER NOT NULL DEFAULT 0,
                   native_job_id TEXT,
                   accepted_unix_ms INTEGER NOT NULL,
                   updated_unix_ms INTEGER NOT NULL,
                   cloud_managed INTEGER NOT NULL DEFAULT 0
                 );",
            )
            .unwrap();
        }
        let store = AgentStore::open(&path).unwrap();
        for (table, expected) in [
            (
                "printer_profiles",
                vec!["status", "native_kind", "native_blob_id", "published"],
            ),
            (
                "jobs",
                vec!["target_id", "profile_id", "profile_revision", "stock_id"],
            ),
        ] {
            let mut statement = store
                .connection
                .prepare(&format!("PRAGMA table_info({table})"))
                .unwrap();
            let columns = statement
                .query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            for name in expected {
                assert!(columns.iter().any(|column| column == name));
            }
        }
    }

    #[test]
    fn local_history_pages_across_printers_newest_first() {
        let mut store = AgentStore::in_memory().unwrap();
        store.accept_job(&job("older", "printer-a", 10)).unwrap();
        store.accept_job(&job("newer", "printer-b", 20)).unwrap();

        let first = store.local_job_history(0, 1).unwrap();
        let second = store.local_job_history(1, 1).unwrap();
        assert_eq!(first[0].job_id, "newer");
        assert_eq!(second[0].job_id, "older");
    }

    #[test]
    fn resource_digest_is_canonical_and_upsert_recovers_eviction_claim() {
        let store = AgentStore::in_memory().unwrap();
        let digest = "a".repeat(64);
        let resource = StoredDocumentResource {
            digest: digest.clone(),
            media_type: "image/jpeg".into(),
            byte_length: 4,
            relative_path: format!("sha256/aa/{digest}"),
            verified_unix_ms: 1,
            last_accessed_unix_ms: 1,
            reference_count: 0,
        };
        store.upsert_document_resource(&resource).unwrap();
        assert!(store.claim_document_resource_eviction(&digest).unwrap());
        store.upsert_document_resource(&resource).unwrap();
        store.retain_document_resource(&digest).unwrap();

        let uppercase = "A".repeat(64);
        assert!(
            store
                .connection
                .execute(
                    "INSERT INTO document_resources(
               digest, media_type, byte_length, relative_path, verified_unix_ms,
               last_accessed_unix_ms, reference_count, evicting
             ) VALUES (?1, 'image/jpeg', 1, ?2, 1, 1, 0, 0)",
                    params![uppercase, "sha256/AA/uppercase"],
                )
                .is_err()
        );
    }

    #[test]
    fn content_reclaim_claim_blocks_new_references_until_cancel_or_finalize() {
        let mut store = AgentStore::in_memory().unwrap();
        let original = job("terminal", "printer", 1);
        store.accept_job(&original).unwrap();
        store
            .connection
            .execute(
                "UPDATE jobs SET state = 'completed_reported' WHERE job_id = 'terminal'",
                [],
            )
            .unwrap();
        let claimed = store.claim_reclaimable_terminal_content(1).unwrap();
        assert_eq!(claimed.len(), 1);

        let mut concurrent = job("concurrent", "printer", 2);
        concurrent.content_sha256 = original.content_sha256.clone();
        concurrent.content_path = original.content_path.clone();
        assert!(matches!(
            store.accept_job(&concurrent),
            Err(StorageError::ContentReclaimInProgress(_))
        ));
        store
            .cancel_terminal_content_reclaim(&claimed[0].sha256, &claimed[0].path)
            .unwrap();
        store.accept_job(&concurrent).unwrap();
    }

    #[test]
    fn content_reclaim_claim_is_safely_reset_on_restart() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("agent.db");
        let mut store = AgentStore::open(&database).unwrap();
        store.accept_job(&job("terminal", "printer", 1)).unwrap();
        store
            .connection
            .execute(
                "UPDATE jobs SET state = 'completed_reported' WHERE job_id = 'terminal'",
                [],
            )
            .unwrap();
        assert_eq!(
            store.claim_reclaimable_terminal_content(1).unwrap().len(),
            1
        );
        drop(store);

        let mut restarted = AgentStore::open(&database).unwrap();
        assert_eq!(
            restarted
                .claim_reclaimable_terminal_content(1)
                .unwrap()
                .len(),
            1
        );
    }
}
