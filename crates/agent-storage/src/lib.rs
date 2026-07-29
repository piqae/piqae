//! Durable local responsibility boundary for the Spool agent.
//!
//! A job is accepted only by [`AgentStore::accept_job`] which atomically
//! records the inbox receipt, job, per-printer FIFO sequence, initial event,
//! content reference, and outbound acknowledgement.

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use spool_domain::{EventId, PrinterId};
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
    pub capabilities_json: String,
    pub observed_unix_ms: i64,
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

    fn configure(connection: Connection) -> Result<Self, StorageError> {
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        connection.execute_batch(SCHEMA)?;
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
            "INSERT INTO printers(printer_id, native_id, name, state, observed_unix_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(printer_id) DO UPDATE SET
               native_id = excluded.native_id,
               name = excluded.name,
               state = excluded.state,
               observed_unix_ms = excluded.observed_unix_ms",
            params![printer_id, native_id, name, state, observed_unix_ms],
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
            capabilities_json: capabilities_json.to_owned(),
            observed_unix_ms,
        })
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
        tx.execute(
            "INSERT INTO content_files
             (sha256, path, reference_count, verified_unix_ms)
             VALUES (?1, ?2, 1, ?3)
             ON CONFLICT (sha256)
             DO UPDATE SET reference_count = reference_count + 1",
            params![job.content_sha256, job.content_path, job.accepted_unix_ms],
        )?;
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
        transaction.execute(
            "INSERT INTO content_files
             (sha256, path, reference_count, verified_unix_ms)
             VALUES (?1, ?2, 1, ?3)
             ON CONFLICT (sha256)
             DO UPDATE SET reference_count = reference_count + 1",
            params![job.content_sha256, job.content_path, job.accepted_unix_ms],
        )?;
        transaction.execute(
            "INSERT INTO jobs
             (job_id, submission_id, printer_id, printer_native_id,
              printer_sequence, title, content_sha256, content_path,
              content_kind, options_json, state, expires_unix_ms,
              accepted_unix_ms, updated_unix_ms, cloud_managed)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                     'cloud_accept_pending', ?11, ?12, ?12, 1)",
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

    /// Returns durable cloud accept requests that must be retried exactly.
    ///
    /// # Errors
    ///
    /// Returns an error when `SQLite` cannot decode an intent.
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
                    j.native_job_id
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
                    j.native_job_id, r.next_observe_unix_ms,
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
                },
                next_observe_unix_ms: row.get(13)?,
                uncertainty_deadline_unix_ms: row.get(14)?,
                attempt_count: row.get(15)?,
                cancel_requested: row.get(16)?,
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
                    native_job_id
             FROM jobs WHERE job_id = ?1",
            [job_id],
            row_to_job,
        )
        .optional()
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
    })
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

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
            store
                .acknowledge_cloud_event(&cloud_events[0].event_id, 20)
                .unwrap(),
            1
        );
        assert!(store.pending_cloud_events(0, 10).unwrap().is_empty());

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
    fn printer_inventory_preserves_logical_to_native_mapping() {
        let mut store = AgentStore::in_memory().unwrap();
        let first = store
            .upsert_printer("native-queue", "Office", "online", r#"{"color":true}"#, 10)
            .unwrap();
        let refreshed = store
            .upsert_printer(
                "native-queue",
                "Office renamed",
                "busy",
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
}
