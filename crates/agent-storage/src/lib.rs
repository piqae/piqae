//! Durable local responsibility boundary for the Spool agent.
//!
//! A job is accepted only by [`AgentStore::accept_job`] which atomically
//! records the inbox receipt, job, per-printer FIFO sequence, initial event,
//! content reference, and outbound acknowledgement.

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueCounts {
    pub queued: u32,
    pub active: u32,
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
              accepted_unix_ms, updated_unix_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                     'queued_local', ?11, ?12, ?12)",
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
        append_event_tx(
            &tx,
            &format!("{}:1", job.job_id),
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

    /// Expires locally queued jobs whose policy deadline has elapsed.
    ///
    /// # Errors
    ///
    /// Returns an error if candidate discovery or any event transaction fails.
    pub fn expire_waiting(&mut self, now_unix_ms: i64) -> Result<usize, StorageError> {
        let job_ids = {
            let mut statement = self.connection.prepare(
                "SELECT job_id FROM jobs
                 WHERE state IN ('queued_local', 'failed_retryable')
                   AND expires_unix_ms IS NOT NULL
                   AND expires_unix_ms <= ?1",
            )?;
            let rows = statement.query_map([now_unix_ms], |row| row.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        for job_id in &job_ids {
            let sequence: i64 = self.connection.query_row(
                "SELECT COALESCE(MAX(job_sequence), 0) + 1 FROM job_events WHERE job_id = ?1",
                [job_id],
                |row| row.get(0),
            )?;
            self.append_event(
                &format!("{job_id}:{sequence}"),
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
}
