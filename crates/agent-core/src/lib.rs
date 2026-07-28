//! Headless Spool agent orchestration.
//!
//! This crate contains no operating-system APIs. It enforces durable local
//! acceptance, per-printer ordering, and the non-transactional spooler
//! handoff policy around a replaceable [`Executor`].

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use spool_agent_storage::{AcceptedJob, AgentStore, LocalJob, StorageError};
use spool_domain::{JobOptions, JobState, validate_transition};
use std::path::{Path, PathBuf};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("local storage failed: {0}")]
    Storage(#[from] StorageError),
    #[error("content I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("content digest mismatch: expected {expected}, got {actual}")]
    ContentDigestMismatch { expected: String, actual: String },
    #[error("invalid stored print options: {0}")]
    InvalidOptions(#[from] serde_json::Error),
    #[error("invalid local state {0}")]
    InvalidState(String),
    #[error("invalid state transition from {from:?} to {to:?}")]
    InvalidTransition { from: JobState, to: JobState },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalSubmission {
    pub job_id: String,
    pub submission_id: String,
    pub printer_native_id: String,
    pub title: String,
    pub content_path: PathBuf,
    pub content_kind: String,
    pub options: JobOptions,
    pub deadline_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeAcceptance {
    pub native_job_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutorFailure {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    /// True if the executor crossed or may have crossed the OS handoff
    /// boundary. Such failures are never retried automatically.
    pub handoff_may_have_succeeded: bool,
    pub native_code: Option<String>,
}

#[async_trait]
pub trait Executor: Send {
    async fn submit(
        &mut self,
        submission: LocalSubmission,
    ) -> Result<NativeAcceptance, ExecutorFailure>;
}

pub trait Clock: Send + Sync {
    fn unix_ms(&self) -> i64;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn unix_ms(&self) -> i64 {
        let elapsed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
    }
}

#[derive(Debug)]
pub struct ContentStore {
    root: PathBuf,
}

impl ContentStore {
    /// Creates the content-addressed storage directory.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created.
    pub async fn open(root: impl Into<PathBuf>) -> Result<Self, AgentError> {
        let root = root.into();
        tokio::fs::create_dir_all(&root).await?;
        Ok(Self { root })
    }

    /// Streams content into the local store, verifies SHA-256, and only then
    /// atomically publishes the digest-addressed file.
    ///
    /// # Errors
    ///
    /// Returns an error for stream or filesystem I/O, or when the computed
    /// digest differs from `expected_sha256`.
    pub async fn put_verified(
        &self,
        expected_sha256: &str,
        mut input: impl AsyncRead + Unpin,
    ) -> Result<PathBuf, AgentError> {
        let expected = expected_sha256.to_ascii_lowercase();
        let final_path = self.root.join(&expected);
        if tokio::fs::try_exists(&final_path).await? {
            return Ok(final_path);
        }

        let temporary_path = self.root.join(format!(".{}.part", Uuid::new_v4()));
        let mut output = tokio::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary_path)
            .await?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0_u8; 64 * 1024];
        loop {
            let count = input.read(&mut buffer).await?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
            output.write_all(&buffer[..count]).await?;
        }
        output.sync_all().await?;
        drop(output);

        let actual = format!("{:x}", hasher.finalize());
        if actual != expected {
            let _ = tokio::fs::remove_file(&temporary_path).await;
            return Err(AgentError::ContentDigestMismatch { expected, actual });
        }

        match tokio::fs::rename(&temporary_path, &final_path).await {
            Ok(()) => Ok(final_path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let _ = tokio::fs::remove_file(&temporary_path).await;
                Ok(final_path)
            }
            Err(error) => {
                let _ = tokio::fs::remove_file(&temporary_path).await;
                Err(error.into())
            }
        }
    }
}

#[derive(Debug)]
pub struct AgentEngine<E, C = SystemClock> {
    store: AgentStore,
    executor: E,
    clock: C,
    execution_deadline_ms: i64,
}

impl<E: Executor, C: Clock> AgentEngine<E, C> {
    pub const DEFAULT_EXECUTION_DEADLINE_MS: i64 = 120_000;

    pub const fn new(store: AgentStore, executor: E, clock: C) -> Self {
        Self {
            store,
            executor,
            clock,
            execution_deadline_ms: Self::DEFAULT_EXECUTION_DEADLINE_MS,
        }
    }

    /// Durably accepts local responsibility for a fully downloaded job.
    ///
    /// # Errors
    ///
    /// Returns an error when the local acceptance transaction fails or
    /// conflicts with an earlier offer.
    pub fn accept(&mut self, job: &AcceptedJob) -> Result<LocalJob, AgentError> {
        Ok(self.store.accept_job(job)?)
    }

    pub const fn store(&self) -> &AgentStore {
        &self.store
    }

    pub const fn store_mut(&mut self) -> &mut AgentStore {
        &mut self.store
    }

    pub fn into_parts(self) -> (AgentStore, E, C) {
        (self.store, self.executor, self.clock)
    }

    /// Executes every currently runnable printer head once.
    ///
    /// # Errors
    ///
    /// Returns an error when queue maintenance, state persistence, option
    /// decoding, or native-executor orchestration fails.
    pub async fn run_once(&mut self) -> Result<usize, AgentError> {
        let now = self.clock.unix_ms();
        self.store.expire_waiting(now)?;
        let jobs = self.store.runnable_heads(now)?;
        let count = jobs.len();
        for job in jobs {
            self.execute_job(job).await?;
        }
        Ok(count)
    }

    async fn execute_job(&mut self, job: LocalJob) -> Result<(), AgentError> {
        self.transition(
            &job.job_id,
            &job.state,
            JobState::Preparing,
            None,
            "Validating local job",
            "{}",
        )?;

        if job.content_kind == "pdf" {
            self.transition(
                &job.job_id,
                "preparing",
                JobState::Rendering,
                None,
                "Preparing PDF for native submission",
                "{}",
            )?;
        }

        let before_intent = if job.content_kind == "pdf" {
            "rendering"
        } else {
            "preparing"
        };
        self.transition(
            &job.job_id,
            before_intent,
            JobState::SpoolIntent,
            None,
            "Persisted native spool intent",
            "{}",
        )?;

        let options: JobOptions = serde_json::from_str(&job.options_json)?;
        let submission = LocalSubmission {
            job_id: job.job_id.clone(),
            submission_id: job.submission_id,
            printer_native_id: job.printer_native_id,
            title: job.title,
            content_path: Path::new(&job.content_path).to_path_buf(),
            content_kind: job.content_kind,
            options,
            deadline_unix_ms: self.clock.unix_ms() + self.execution_deadline_ms,
        };

        match self.executor.submit(submission).await {
            Ok(acceptance) => {
                let now = self.clock.unix_ms();
                self.store
                    .set_native_job_id(&job.job_id, &acceptance.native_job_id, now)?;
                self.transition(
                    &job.job_id,
                    "spool_intent",
                    JobState::AcceptedBySpooler,
                    None,
                    "Operating system accepted the job",
                    &serde_json::json!({
                        "native_job_id": acceptance.native_job_id
                    })
                    .to_string(),
                )?;
            }
            Err(error) => {
                let (state, reason) = if error.handoff_may_have_succeeded {
                    (JobState::DeliveryUncertain, "ambiguous_handoff")
                } else if error.retryable {
                    (JobState::FailedRetryable, error.code.as_str())
                } else {
                    (JobState::FailedTerminal, error.code.as_str())
                };
                self.transition(
                    &job.job_id,
                    "spool_intent",
                    state,
                    Some(reason),
                    &error.message,
                    &serde_json::to_string(&error)?,
                )?;
            }
        }
        Ok(())
    }

    fn transition(
        &mut self,
        job_id: &str,
        from: &str,
        to: JobState,
        reason: Option<&str>,
        message: &str,
        details_json: &str,
    ) -> Result<(), AgentError> {
        let from = parse_state(from)?;
        validate_transition(from, to).map_err(|_| AgentError::InvalidTransition { from, to })?;
        let now = self.clock.unix_ms();
        self.store.append_next_event(
            &format!("evt_{}", Uuid::new_v4()),
            job_id,
            state_name(to),
            reason,
            Some(message),
            details_json,
            now,
        )?;
        Ok(())
    }
}

fn parse_state(value: &str) -> Result<JobState, AgentError> {
    let state = match value {
        "registered" => JobState::Registered,
        "content_pending" => JobState::ContentPending,
        "waiting_for_agent" => JobState::WaitingForAgent,
        "agent_downloading" => JobState::AgentDownloading,
        "agent_accepted" => JobState::AgentAccepted,
        "queued_local" => JobState::QueuedLocal,
        "preparing" => JobState::Preparing,
        "rendering" => JobState::Rendering,
        "spool_intent" => JobState::SpoolIntent,
        "accepted_by_spooler" => JobState::AcceptedBySpooler,
        "spooling" => JobState::Spooling,
        "printing" => JobState::Printing,
        "blocked" => JobState::Blocked,
        "completed_reported" => JobState::CompletedReported,
        "delivery_uncertain" => JobState::DeliveryUncertain,
        "cancel_requested" => JobState::CancelRequested,
        "cancelled" => JobState::Cancelled,
        "expired" => JobState::Expired,
        "failed_retryable" => JobState::FailedRetryable,
        "failed_terminal" => JobState::FailedTerminal,
        unknown => return Err(AgentError::InvalidState(unknown.to_owned())),
    };
    Ok(state)
}

const fn state_name(state: JobState) -> &'static str {
    match state {
        JobState::Registered => "registered",
        JobState::ContentPending => "content_pending",
        JobState::WaitingForAgent => "waiting_for_agent",
        JobState::AgentDownloading => "agent_downloading",
        JobState::AgentAccepted => "agent_accepted",
        JobState::QueuedLocal => "queued_local",
        JobState::Preparing => "preparing",
        JobState::Rendering => "rendering",
        JobState::SpoolIntent => "spool_intent",
        JobState::AcceptedBySpooler => "accepted_by_spooler",
        JobState::Spooling => "spooling",
        JobState::Printing => "printing",
        JobState::Blocked => "blocked",
        JobState::CompletedReported => "completed_reported",
        JobState::DeliveryUncertain => "delivery_uncertain",
        JobState::CancelRequested => "cancel_requested",
        JobState::Cancelled => "cancelled",
        JobState::Expired => "expired",
        JobState::FailedRetryable => "failed_retryable",
        JobState::FailedTerminal => "failed_terminal",
    }
}

#[derive(Debug, Default)]
pub struct FakeExecutor {
    pub submitted: Vec<LocalSubmission>,
    pub result: Option<Result<NativeAcceptance, ExecutorFailure>>,
}

#[async_trait]
impl Executor for FakeExecutor {
    async fn submit(
        &mut self,
        submission: LocalSubmission,
    ) -> Result<NativeAcceptance, ExecutorFailure> {
        self.submitted.push(submission);
        self.result.take().unwrap_or_else(|| {
            Ok(NativeAcceptance {
                native_job_id: format!("fake-{}", self.submitted.len()),
            })
        })
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use tokio::io::BufReader;

    #[derive(Debug, Clone, Copy)]
    struct FixedClock(i64);
    impl Clock for FixedClock {
        fn unix_ms(&self) -> i64 {
            self.0
        }
    }

    fn accepted(id: &str, kind: &str) -> AcceptedJob {
        AcceptedJob {
            job_id: id.into(),
            submission_id: format!("sub-{id}"),
            printer_id: "printer".into(),
            printer_native_id: "native".into(),
            title: "Receipt".into(),
            content_sha256: "abc".into(),
            content_path: "/content/abc".into(),
            content_kind: kind.into(),
            options_json: "{}".into(),
            expires_unix_ms: None,
            accepted_unix_ms: 1,
        }
    }

    #[tokio::test]
    async fn successful_job_persists_intent_before_acceptance() {
        let store = AgentStore::in_memory().expect("store");
        let mut engine = AgentEngine::new(store, FakeExecutor::default(), FixedClock(10));
        engine.accept(&accepted("job", "pdf")).expect("accept");
        assert_eq!(engine.run_once().await.expect("run"), 1);
        let events = engine.store().pending_events(0, 20).expect("events");
        let states: Vec<_> = events.iter().map(|event| event.state.as_str()).collect();
        assert_eq!(
            states,
            [
                "queued_local",
                "preparing",
                "rendering",
                "spool_intent",
                "accepted_by_spooler"
            ]
        );
        assert_eq!(
            engine
                .store()
                .get_job("job")
                .expect("query")
                .expect("job")
                .native_job_id
                .as_deref(),
            Some("fake-1")
        );
    }

    #[tokio::test]
    async fn ambiguous_handoff_is_never_automatically_retried() {
        let executor = FakeExecutor {
            result: Some(Err(ExecutorFailure {
                code: "executor_died".into(),
                message: "child exited during native call".into(),
                retryable: true,
                handoff_may_have_succeeded: true,
                native_code: None,
            })),
            ..FakeExecutor::default()
        };
        let store = AgentStore::in_memory().expect("store");
        let mut engine = AgentEngine::new(store, executor, FixedClock(10));
        engine.accept(&accepted("job", "raw")).expect("accept");
        engine.run_once().await.expect("run");
        assert_eq!(
            engine
                .store()
                .get_job("job")
                .expect("query")
                .expect("job")
                .state,
            "delivery_uncertain"
        );
        assert_eq!(engine.run_once().await.expect("second run"), 0);
    }

    #[tokio::test]
    async fn verified_content_is_atomically_published() {
        let directory = tempfile::tempdir().expect("tempdir");
        let content = b"hello spool";
        let digest = format!("{:x}", Sha256::digest(content));
        let store = ContentStore::open(directory.path()).await.expect("open");
        let path = store
            .put_verified(&digest, BufReader::new(content.as_slice()))
            .await
            .expect("write");
        assert_eq!(tokio::fs::read(path).await.expect("read"), content);
    }

    #[tokio::test]
    async fn digest_mismatch_does_not_publish_content() {
        let directory = tempfile::tempdir().expect("tempdir");
        let store = ContentStore::open(directory.path()).await.expect("open");
        let error = store
            .put_verified(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                BufReader::new(b"wrong".as_slice()),
            )
            .await
            .expect_err("mismatch");
        assert!(matches!(error, AgentError::ContentDigestMismatch { .. }));
    }
}
