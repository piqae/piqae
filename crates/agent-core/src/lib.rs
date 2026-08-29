//! Headless Piqae agent orchestration.
//!
//! This crate contains no operating-system APIs. It enforces durable local
//! acceptance, per-printer ordering, and the non-transactional spooler
//! handoff policy around a replaceable [`Executor`].

use async_trait::async_trait;
use piqae_agent_storage::{
    AcceptedJob, AgentStore, JobPersistenceFacts, LocalJob, PrinterNativeBindingPin, StorageError,
};
use piqae_domain::{
    DriverFingerprint, EventId, JobOptions, JobState, NativeProfileKind, SafeProfileOverride,
    validate_transition,
};
use piqae_protocol::executor::{NativeJobObservation, NativeJobState, NativeProfilePayload};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

pub mod document_render;
pub mod document_resources;

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("local storage failed: {0}")]
    Storage(#[from] StorageError),
    #[error("content I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("content digest mismatch: expected {expected}, got {actual}")]
    ContentDigestMismatch { expected: String, actual: String },
    #[error("content exceeds the {limit} byte local limit")]
    ContentTooLarge { limit: u64 },
    #[error("invalid stored print options: {0}")]
    InvalidOptions(#[from] serde_json::Error),
    #[error("invalid pinned native profile: {0}")]
    InvalidNativeProfile(String),
    #[error("invalid local state {0}")]
    InvalidState(String),
    #[error("invalid state transition from {from:?} to {to:?}")]
    InvalidTransition { from: JobState, to: JobState },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalSubmission {
    pub job_id: String,
    pub submission_id: String,
    pub printer_id: String,
    pub printer_native_id: String,
    pub title: String,
    pub content_path: PathBuf,
    pub content_kind: String,
    pub options: JobOptions,
    pub native_profile: Option<NativeProfilePayload>,
    pub printer_native_binding: Option<PrinterNativeBindingPin>,
    /// Durable queue authority that owns this job's native route reservation.
    /// Local jobs leave this unset and use the executor's queue-scoped owner.
    pub route_connector_id: Option<String>,
    pub deadline_unix_ms: i64,
    pub route_fence: Option<piqae_protocol::executor::LocalRouteFence>,
}

/// Executor-ready profile data stored atomically with cloud responsibility.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DurableProfileSnapshot {
    Portable { profile_id: String, revision: u64 },
    Native { payload: Box<NativeProfilePayload> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeAcceptance {
    pub native_job_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeJobReference {
    pub job_id: String,
    pub printer_native_id: String,
    pub native_job_id: String,
    pub deadline_unix_ms: i64,
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

    async fn observe(
        &mut self,
        reference: NativeJobReference,
    ) -> Result<NativeJobObservation, ExecutorFailure>;

    async fn cancel(&mut self, reference: NativeJobReference) -> Result<(), ExecutorFailure>;
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

#[derive(Debug, Clone)]
pub struct ContentStore {
    root: PathBuf,
}

#[derive(Debug)]
struct PartialContent {
    path: PathBuf,
    published: bool,
}

impl PartialContent {
    const fn new(path: PathBuf) -> Self {
        Self {
            path,
            published: false,
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    const fn published(&mut self) {
        self.published = true;
    }
}

impl Drop for PartialContent {
    fn drop(&mut self) {
        if !self.published {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredContent {
    pub sha256: String,
    pub path: PathBuf,
    pub bytes: u64,
}

impl ContentStore {
    pub const MAX_CONTENT_BYTES: u64 = 50 * 1024 * 1024;
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
        input: impl AsyncRead + Unpin,
    ) -> Result<PathBuf, AgentError> {
        let stored = self
            .put_inner(
                Some(expected_sha256.to_ascii_lowercase()),
                input,
                false,
                Self::MAX_CONTENT_BYTES,
            )
            .await?;
        Ok(stored.path)
    }

    /// Streams and verifies content using a caller-supplied upper bound.
    /// Intended for encoded transport formats whose fixed authentication
    /// overhead is slightly larger than the decoded document limit.
    ///
    /// # Errors
    ///
    /// Returns an error for stream or filesystem I/O, when the input exceeds
    /// `limit`, or when its computed digest differs from `expected_sha256`.
    pub async fn put_verified_with_limit(
        &self,
        expected_sha256: &str,
        input: impl AsyncRead + Unpin,
        limit: u64,
    ) -> Result<PathBuf, AgentError> {
        let stored = self
            .put_inner(
                Some(expected_sha256.to_ascii_lowercase()),
                input,
                false,
                limit,
            )
            .await?;
        Ok(stored.path)
    }

    /// Streams content into the digest-addressed store and returns its
    /// computed identity.
    ///
    /// # Errors
    ///
    /// Returns an error when the input or local filesystem cannot be read or
    /// written.
    pub async fn put(&self, input: impl AsyncRead + Unpin) -> Result<StoredContent, AgentError> {
        self.put_inner(None, input, false, Self::MAX_CONTENT_BYTES)
            .await
    }

    /// Stores decrypted job bytes under a unique, non-content-addressed name.
    /// The engine recognizes this name and removes it after definitive native
    /// handoff or a non-retryable outcome.
    ///
    /// # Errors
    ///
    /// Returns an error when the input or local filesystem cannot be read or written.
    pub async fn put_confidential(
        &self,
        input: impl AsyncRead + Unpin,
    ) -> Result<StoredContent, AgentError> {
        self.put_inner(None, input, true, Self::MAX_CONTENT_BYTES)
            .await
    }

    async fn put_inner(
        &self,
        expected: Option<String>,
        mut input: impl AsyncRead + Unpin,
        confidential: bool,
        limit: u64,
    ) -> Result<StoredContent, AgentError> {
        if let Some(expected) = &expected {
            let final_path = self.root.join(expected);
            if tokio::fs::try_exists(&final_path).await? {
                let bytes = tokio::fs::metadata(&final_path).await?.len();
                if bytes > limit {
                    return Err(AgentError::ContentTooLarge { limit });
                }
                return Ok(StoredContent {
                    sha256: expected.clone(),
                    path: final_path,
                    bytes,
                });
            }
        }
        let temporary_path = self.root.join(format!(".{}.part", Uuid::new_v4()));
        let mut partial = PartialContent::new(temporary_path);
        let mut options = tokio::fs::OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            options.mode(0o600);
        }
        let mut output = options.open(partial.path()).await?;
        let mut hasher = Sha256::new();
        let mut total_bytes = 0_u64;
        let mut buffer = vec![0_u8; 64 * 1024];
        loop {
            let count = input.read(&mut buffer).await?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
            output.write_all(&buffer[..count]).await?;
            total_bytes = total_bytes.saturating_add(u64::try_from(count).unwrap_or(u64::MAX));
            if total_bytes > limit {
                drop(output);
                return Err(AgentError::ContentTooLarge { limit });
            }
        }
        output.sync_all().await?;
        drop(output);

        let actual = format!("{:x}", hasher.finalize());
        if let Some(expected) = expected {
            if actual != expected {
                return Err(AgentError::ContentDigestMismatch { expected, actual });
            }
        }
        let final_path = if confidential {
            self.root.join(format!("confidential-{}", Uuid::new_v4()))
        } else {
            self.root.join(&actual)
        };

        match tokio::fs::rename(partial.path(), &final_path).await {
            Ok(()) => {
                partial.published();
                Ok(StoredContent {
                    sha256: actual,
                    path: final_path,
                    bytes: total_bytes,
                })
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let _ = tokio::fs::remove_file(partial.path()).await;
                partial.published();
                Ok(StoredContent {
                    sha256: actual,
                    path: final_path,
                    bytes: total_bytes,
                })
            }
            Err(error) => Err(error.into()),
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
    pub const DEFAULT_OBSERVATION_DEADLINE_MS: i64 = 30_000;
    pub const DEFAULT_RECONCILIATION_INTERVAL_MS: i64 = 2_000;
    pub const DEFAULT_UNCERTAINTY_AFTER_MS: i64 = 10 * 60 * 1_000;
    pub const DEFAULT_LOCAL_RETRY_DELAY_MS: i64 = 2_000;

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

    /// Durably accepts responsibility together with all immutable execution facts.
    ///
    /// # Errors
    ///
    /// Returns an error when the atomic local acceptance fails or conflicts.
    pub fn accept_with_facts(
        &mut self,
        job: &AcceptedJob,
        facts: &JobPersistenceFacts,
    ) -> Result<LocalJob, AgentError> {
        Ok(self.store.accept_job_with_facts(job, facts)?)
    }

    pub const fn store(&self) -> &AgentStore {
        &self.store
    }

    pub const fn store_mut(&mut self) -> &mut AgentStore {
        &mut self.store
    }

    pub const fn executor_mut(&mut self) -> &mut E {
        &mut self.executor
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
        let mut count = jobs.len();
        for job in jobs {
            self.execute_job(job).await?;
        }
        count += self.reconcile_due().await?;
        Ok(count)
    }

    #[allow(
        clippy::too_many_lines,
        reason = "the pre-handoff state machine keeps persistence and executor outcomes in one audit point"
    )]
    async fn execute_job(&mut self, job: LocalJob) -> Result<(), AgentError> {
        self.begin_execution(&job)?;

        let Some(options) = self.validate_job_options(&job)? else {
            if let Some(path) = confidential_content_path(&job.content_path) {
                let _ = tokio::fs::remove_file(path).await;
            }
            return Ok(());
        };

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

        let native_profile = self.native_profile_for_job(&job)?;
        let confidential_path = confidential_content_path(&job.content_path);
        let submission = LocalSubmission {
            job_id: job.job_id.clone(),
            submission_id: job.submission_id,
            printer_id: job.printer_id,
            printer_native_id: job.printer_native_id,
            title: job.title,
            content_path: Path::new(&job.content_path).to_path_buf(),
            content_kind: job.content_kind,
            options,
            native_profile,
            printer_native_binding: job.printer_native_binding,
            route_connector_id: job.route_connector_id,
            deadline_unix_ms: self.clock.unix_ms() + self.execution_deadline_ms,
            route_fence: None,
        };

        match self.executor.submit(submission).await {
            Ok(acceptance) => {
                let now = self.clock.unix_ms();
                let details = serde_json::json!({
                    "native_job_id": acceptance.native_job_id
                })
                .to_string();
                self.store.record_native_acceptance(
                    &EventId::new().to_string(),
                    &job.job_id,
                    &acceptance.native_job_id,
                    &details,
                    now,
                    now + Self::DEFAULT_RECONCILIATION_INTERVAL_MS,
                    now + Self::DEFAULT_UNCERTAINTY_AFTER_MS,
                )?;
                if let Some(path) = confidential_path.as_ref() {
                    let _ = tokio::fs::remove_file(path).await;
                }
            }
            Err(error) => {
                self.store.record_executor_failure(&error.code)?;
                let (state, reason) = if error.handoff_may_have_succeeded {
                    (JobState::DeliveryUncertain, "ambiguous_handoff")
                } else if error.retryable {
                    (JobState::FailedRetryable, error.code.as_str())
                } else {
                    (JobState::FailedTerminal, error.code.as_str())
                };
                let details = serde_json::to_string(&error)?;
                if error.retryable && !error.handoff_may_have_succeeded && !job.cloud_managed {
                    let now = self.clock.unix_ms();
                    self.store.record_local_retryable_failure(
                        &EventId::new().to_string(),
                        &job.job_id,
                        reason,
                        &error.message,
                        &details,
                        now,
                        now + Self::DEFAULT_LOCAL_RETRY_DELAY_MS,
                    )?;
                } else {
                    self.transition(
                        &job.job_id,
                        "spool_intent",
                        state,
                        Some(reason),
                        &error.message,
                        &details,
                    )?;
                }
                if !error.retryable {
                    if let Some(path) = confidential_path.as_ref() {
                        let _ = tokio::fs::remove_file(path).await;
                    }
                }
            }
        }
        Ok(())
    }

    fn begin_execution(&mut self, job: &LocalJob) -> Result<(), AgentError> {
        let mut state = parse_state(&job.state)?;
        if state == JobState::FailedRetryable {
            if job.cloud_managed {
                return Err(AgentError::InvalidTransition {
                    from: JobState::FailedRetryable,
                    to: JobState::Preparing,
                });
            }
            self.transition(
                &job.job_id,
                &job.state,
                JobState::QueuedLocal,
                Some("local_retry"),
                "Retrying locally queued job after a pre-handoff failure",
                "{}",
            )?;
            state = JobState::QueuedLocal;
        }
        self.transition(
            &job.job_id,
            state_name(state),
            JobState::Preparing,
            None,
            "Validating local job",
            "{}",
        )
    }

    fn validate_job_options(&mut self, job: &LocalJob) -> Result<Option<JobOptions>, AgentError> {
        let options: JobOptions = match serde_json::from_str(&job.options_json) {
            Ok(options) => options,
            Err(error) => {
                self.fail_invalid_job_options(job, &error.to_string())?;
                return Ok(None);
            }
        };
        let Err(error) = options.validate_bounds() else {
            return Ok(Some(options));
        };
        self.fail_invalid_job_options(job, &error.to_string())?;
        Ok(None)
    }

    fn fail_invalid_job_options(&mut self, job: &LocalJob, error: &str) -> Result<(), AgentError> {
        self.transition(
            &job.job_id,
            "preparing",
            JobState::FailedTerminal,
            Some("invalid_print_options"),
            "Print options failed local safety validation before native handoff",
            &serde_json::json!({ "error": error }).to_string(),
        )
    }

    async fn reconcile_due(&mut self) -> Result<usize, AgentError> {
        let now = self.clock.unix_ms();
        let items = self.store.due_reconciliations(now, 32)?;
        let count = items.len();
        for item in items {
            self.reconcile_item(item, now).await?;
        }
        Ok(count)
    }

    async fn reconcile_item(
        &mut self,
        item: piqae_agent_storage::ReconciliationItem,
        now: i64,
    ) -> Result<(), AgentError> {
        let Some(native_job_id) = item.job.native_job_id.clone() else {
            return self.mark_uncertain(
                &item.job,
                "Native reconciliation schedule has no spooler identifier",
                "{}",
                now,
            );
        };
        let reference = NativeJobReference {
            job_id: item.job.job_id.clone(),
            printer_native_id: item.job.printer_native_id.clone(),
            native_job_id: native_job_id.clone(),
            deadline_unix_ms: now + Self::DEFAULT_OBSERVATION_DEADLINE_MS,
        };
        if item.cancel_requested || item.job.state == "cancel_requested" {
            return self
                .reconcile_cancellation(item, reference, &native_job_id, now)
                .await;
        }
        match self.executor.observe(reference).await {
            Ok(observation) => {
                self.apply_native_observation(&item, &native_job_id, &observation, now)
            }
            Err(error) => {
                let details = serde_json::to_string(&error)?;
                self.store.record_reconciliation_attempt(
                    &item.job.job_id,
                    &native_job_id,
                    "observation_error",
                    "executor",
                    &details,
                    Some(&error.code),
                    now,
                    now + Self::DEFAULT_RECONCILIATION_INTERVAL_MS,
                )?;
                if now >= item.uncertainty_deadline_unix_ms {
                    self.mark_uncertain(
                        &item.job,
                        "Native job could not be observed before the uncertainty deadline",
                        &details,
                        now,
                    )?;
                }
                Ok(())
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    fn native_profile_for_job(
        &self,
        job: &LocalJob,
    ) -> Result<Option<NativeProfilePayload>, AgentError> {
        let (profile_id, revision) = match (&job.profile_id, job.profile_revision) {
            (None, None) => return Ok(None),
            (Some(profile_id), Some(revision)) => (profile_id, revision),
            _ => {
                return Err(AgentError::InvalidNativeProfile(format!(
                    "job {} has an incomplete profile revision pin",
                    job.job_id
                )));
            }
        };
        if let Some(encoded) = &job.profile_snapshot_json {
            let snapshot =
                serde_json::from_str::<DurableProfileSnapshot>(encoded).map_err(|error| {
                    AgentError::InvalidNativeProfile(format!(
                        "job {} has an invalid durable profile snapshot: {error}",
                        job.job_id
                    ))
                })?;
            return match snapshot {
                DurableProfileSnapshot::Portable {
                    profile_id: snapshot_id,
                    revision: snapshot_revision,
                } if snapshot_id == *profile_id && snapshot_revision == revision => Ok(None),
                DurableProfileSnapshot::Native { payload }
                    if payload.profile_id == *profile_id && payload.revision == revision =>
                {
                    Ok(Some(*payload))
                }
                _ => Err(AgentError::InvalidNativeProfile(format!(
                    "job {} durable profile snapshot does not match its revision pin",
                    job.job_id
                ))),
            };
        }
        if job.cloud_managed {
            return Err(AgentError::InvalidNativeProfile(format!(
                "job {} is missing its durable profile snapshot",
                job.job_id
            )));
        }
        let metadata = self
            .store
            .named_profile_revision(&job.printer_id, profile_id, revision)?
            .ok_or_else(|| {
                AgentError::InvalidNativeProfile(format!(
                    "profile {profile_id} revision {revision} is missing"
                ))
            })?;
        let metadata_kind = serde_json::from_value::<NativeProfileKind>(serde_json::Value::String(
            metadata.native_kind.clone(),
        ))
        .map_err(|error| {
            AgentError::InvalidNativeProfile(format!(
                "profile {profile_id} revision {revision} has invalid native kind metadata: {error}"
            ))
        })?;
        // Legacy/basic profiles are represented entirely by the already
        // pinned JobOptions. They intentionally have no opaque driver blob.
        if metadata_kind == NativeProfileKind::PortableOptions {
            return Ok(None);
        }
        let native = self
            .store
            .native_profile_blob(profile_id, revision)?
            .ok_or_else(|| {
                AgentError::InvalidNativeProfile(format!(
                    "native payload for profile {profile_id} revision {revision} is missing"
                ))
            })?;
        if metadata.native_blob_id.as_deref() != Some(native.blob_id.as_str())
            || metadata.native_digest.as_deref() != Some(native.digest.as_str())
            || metadata.native_kind != native.native_kind
        {
            return Err(AgentError::InvalidNativeProfile(format!(
                "profile {profile_id} revision {revision} metadata does not match its immutable payload"
            )));
        }
        let kind = serde_json::from_value::<NativeProfileKind>(serde_json::Value::String(
            native.native_kind,
        ))
        .map_err(|error| {
            AgentError::InvalidNativeProfile(format!(
                "profile {profile_id} revision {revision} has invalid native kind metadata: {error}"
            ))
        })?;
        let safe_overrides =
            serde_json::from_str::<Vec<SafeProfileOverride>>(&metadata.safe_overrides_json)
                .map_err(|error| {
                    AgentError::InvalidNativeProfile(format!(
                        "profile {profile_id} revision {revision} has invalid safe override metadata: {error}"
                    ))
                })?;
        let driver_fingerprint = serde_json::from_str::<DriverFingerprint>(
            &metadata.driver_fingerprint_json,
        )
        .map_err(|error| {
            AgentError::InvalidNativeProfile(format!(
                "profile {profile_id} revision {revision} has invalid driver metadata: {error}"
            ))
        })?;
        Ok(Some(NativeProfilePayload {
            profile_id: profile_id.clone(),
            revision,
            kind,
            schema_version: native.schema_version,
            digest: native.digest,
            blob: native.native_blob,
            safe_overrides,
            driver_fingerprint,
        }))
    }

    async fn reconcile_cancellation(
        &mut self,
        item: piqae_agent_storage::ReconciliationItem,
        reference: NativeJobReference,
        native_job_id: &str,
        now: i64,
    ) -> Result<(), AgentError> {
        match self.executor.cancel(reference).await {
            Ok(()) => {
                self.store.record_reconciliation_attempt(
                    &item.job.job_id,
                    native_job_id,
                    "cancelled",
                    "executor",
                    "{}",
                    None,
                    now,
                    now + Self::DEFAULT_RECONCILIATION_INTERVAL_MS,
                )?;
                self.transition(
                    &item.job.job_id,
                    &item.job.state,
                    JobState::Cancelled,
                    Some("cancelled_by_user"),
                    "Operating system accepted cancellation",
                    "{}",
                )?;
                self.store.finish_reconciliation(&item.job.job_id)?;
                Ok(())
            }
            Err(error) => {
                let details = serde_json::to_string(&error)?;
                self.store.record_reconciliation_attempt(
                    &item.job.job_id,
                    native_job_id,
                    "cancel_failed",
                    "executor",
                    &details,
                    Some(&error.code),
                    now,
                    now + Self::DEFAULT_RECONCILIATION_INTERVAL_MS,
                )?;
                if now >= item.uncertainty_deadline_unix_ms {
                    self.mark_uncertain(
                        &item.job,
                        "Cancellation outcome could not be proved",
                        &details,
                        now,
                    )?;
                }
                Ok(())
            }
        }
    }

    fn apply_native_observation(
        &mut self,
        item: &piqae_agent_storage::ReconciliationItem,
        native_job_id: &str,
        observation: &NativeJobObservation,
        now: i64,
    ) -> Result<(), AgentError> {
        let details = serde_json::to_string(&observation)?;
        self.store.record_reconciliation_attempt(
            &item.job.job_id,
            native_job_id,
            native_state_name(observation.state),
            "native_spooler",
            &details,
            None,
            now,
            now + Self::DEFAULT_RECONCILIATION_INTERVAL_MS,
        )?;
        match observation.state {
            NativeJobState::Queued
                if matches!(item.job.state.as_str(), "accepted_by_spooler" | "blocked") =>
            {
                self.transition_if_needed(
                    &item.job,
                    JobState::Spooling,
                    None,
                    "Native spooler reports the job queued",
                    &details,
                )?;
            }
            NativeJobState::Printing
                if matches!(
                    item.job.state.as_str(),
                    "accepted_by_spooler" | "spooling" | "blocked"
                ) =>
            {
                self.transition_if_needed(
                    &item.job,
                    JobState::Printing,
                    None,
                    "Native spooler reports the job printing",
                    &details,
                )?;
            }
            NativeJobState::Blocked => self.transition_if_needed(
                &item.job,
                JobState::Blocked,
                Some("driver_error"),
                "Native spooler reports the job blocked",
                &details,
            )?,
            NativeJobState::Completed => {
                self.transition_if_needed(
                    &item.job,
                    JobState::CompletedReported,
                    None,
                    "Operating system spooler reported completion; physical output is not proven",
                    &details,
                )?;
                self.store.finish_reconciliation(&item.job.job_id)?;
            }
            NativeJobState::Failed => {
                self.transition_if_needed(
                    &item.job,
                    JobState::FailedTerminal,
                    Some("driver_error"),
                    "Native spooler reported terminal failure",
                    &details,
                )?;
                self.store.finish_reconciliation(&item.job.job_id)?;
            }
            NativeJobState::Cancelled => {
                self.transition_if_needed(
                    &item.job,
                    JobState::Cancelled,
                    None,
                    "Native spooler reported cancellation",
                    &details,
                )?;
                self.store.finish_reconciliation(&item.job.job_id)?;
            }
            NativeJobState::Missing | NativeJobState::Unknown => {
                if now >= item.uncertainty_deadline_unix_ms {
                    self.mark_uncertain(
                        &item.job,
                        "Native spooler could not prove the final job outcome",
                        &details,
                        now,
                    )?;
                }
            }
            NativeJobState::Queued | NativeJobState::Printing => {}
        }
        Ok(())
    }

    fn transition_if_needed(
        &mut self,
        job: &LocalJob,
        to: JobState,
        reason: Option<&str>,
        message: &str,
        details_json: &str,
    ) -> Result<(), AgentError> {
        if job.state == state_name(to) {
            return Ok(());
        }
        self.transition(&job.job_id, &job.state, to, reason, message, details_json)
    }

    fn mark_uncertain(
        &mut self,
        job: &LocalJob,
        message: &str,
        details_json: &str,
        _now: i64,
    ) -> Result<(), AgentError> {
        self.transition_if_needed(
            job,
            JobState::DeliveryUncertain,
            Some("ambiguous_handoff"),
            message,
            details_json,
        )?;
        self.store.finish_reconciliation(&job.job_id)?;
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
            &EventId::new().to_string(),
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

fn confidential_content_path(content_path: &str) -> Option<PathBuf> {
    Path::new(content_path)
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("confidential-"))
        .then(|| PathBuf::from(content_path))
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

const fn native_state_name(state: NativeJobState) -> &'static str {
    match state {
        NativeJobState::Queued => "queued",
        NativeJobState::Printing => "printing",
        NativeJobState::Blocked => "blocked",
        NativeJobState::Completed => "completed",
        NativeJobState::Failed => "failed",
        NativeJobState::Cancelled => "cancelled",
        NativeJobState::Missing => "missing",
        NativeJobState::Unknown => "unknown",
    }
}

#[derive(Debug, Default)]
pub struct FakeExecutor {
    pub submitted: Vec<LocalSubmission>,
    pub result: Option<Result<NativeAcceptance, ExecutorFailure>>,
    pub observations: VecDeque<Result<NativeJobObservation, ExecutorFailure>>,
    pub cancellations: usize,
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

    async fn observe(
        &mut self,
        _reference: NativeJobReference,
    ) -> Result<NativeJobObservation, ExecutorFailure> {
        self.observations.pop_front().unwrap_or_else(|| {
            Ok(NativeJobObservation {
                state: NativeJobState::Unknown,
                native_code: Some("fake-observation-exhausted".into()),
                message: None,
            })
        })
    }

    async fn cancel(&mut self, _reference: NativeJobReference) -> Result<(), ExecutorFailure> {
        self.cancellations += 1;
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::time::Duration;
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
            cloud_managed: false,
        }
    }

    fn raw_facts() -> JobPersistenceFacts {
        JobPersistenceFacts {
            printer_native_binding: Some(PrinterNativeBindingPin {
                process_session_id: "virtual-session".into(),
                generation: 1,
                printer_id: "printer".into(),
                output_profile_id: "virtual/raw-v1".into(),
                language_profile_id: "virtual/raw-v1".into(),
                language: "virtual".into(),
                language_version: "1".into(),
                profile_version: "1.0.0".into(),
                media_type: "application/vnd.piqae.virtual-raw".into(),
                driver_fingerprint_sha256: "a".repeat(64),
                support_pack_digest_sha256: "b".repeat(64),
            }),
            ..JobPersistenceFacts::default()
        }
    }

    fn observation(state: NativeJobState) -> NativeJobObservation {
        NativeJobObservation {
            state,
            native_code: Some("fake".into()),
            message: None,
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
    async fn invalid_options_fail_terminally_without_calling_the_executor() {
        let store = AgentStore::in_memory().expect("store");
        let mut engine = AgentEngine::new(store, FakeExecutor::default(), FixedClock(10));
        let mut job = accepted("invalid-options", "pdf");
        job.options_json = serde_json::to_string(&JobOptions {
            copies: Some(0),
            ..Default::default()
        })
        .expect("options");
        engine.accept(&job).expect("accept");

        assert_eq!(engine.run_once().await.expect("run"), 1);
        assert!(engine.executor_mut().submitted.is_empty());
        let stored = engine
            .store()
            .get_job("invalid-options")
            .expect("query")
            .expect("job");
        assert_eq!(stored.state, "failed_terminal");
        let events = engine.store().pending_events(0, 20).expect("events");
        assert_eq!(
            events.last().and_then(|event| event.reason.as_deref()),
            Some("invalid_print_options")
        );
    }

    #[tokio::test]
    async fn invalid_options_remove_confidential_plaintext_after_terminal_failure() {
        let directory = tempfile::tempdir().expect("tempdir");
        let confidential_path = directory.path().join("confidential-invalid-options");
        tokio::fs::write(&confidential_path, b"sensitive print content")
            .await
            .expect("write confidential content");

        let store = AgentStore::in_memory().expect("store");
        let mut engine = AgentEngine::new(store, FakeExecutor::default(), FixedClock(10));
        let mut job = accepted("invalid-confidential-options", "pdf");
        job.content_path = confidential_path.to_string_lossy().into_owned();
        job.options_json = serde_json::to_string(&JobOptions {
            copies: Some(0),
            ..Default::default()
        })
        .expect("options");
        engine.accept(&job).expect("accept");

        assert_eq!(engine.run_once().await.expect("run"), 1);
        assert!(!confidential_path.exists());
        assert!(engine.executor_mut().submitted.is_empty());
        assert_eq!(
            engine
                .store()
                .get_job("invalid-confidential-options")
                .expect("query")
                .expect("job")
                .state,
            "failed_terminal"
        );
    }

    #[tokio::test]
    async fn invalid_durable_option_schema_does_not_leave_a_job_preparing() {
        let store = AgentStore::in_memory().expect("store");
        let mut engine = AgentEngine::new(store, FakeExecutor::default(), FixedClock(10));
        let mut job = accepted("malformed-options", "pdf");
        job.options_json = r#"{"unrecognized_driver_instruction":true}"#.into();
        engine.accept(&job).expect("accept");

        assert_eq!(engine.run_once().await.expect("run"), 1);
        assert!(engine.executor_mut().submitted.is_empty());
        assert_eq!(
            engine
                .store()
                .get_job("malformed-options")
                .expect("query")
                .expect("job")
                .state,
            "failed_terminal"
        );
    }

    #[tokio::test]
    async fn submission_uses_the_exact_pinned_native_profile_revision() {
        let mut store = AgentStore::in_memory().expect("store");
        let printer = store
            .upsert_printer("native", "HP", "online", true, "{}", 1)
            .expect("printer");
        store
            .create_profile_capture_session(
                "capture-1",
                "token-1",
                &printer.printer_id,
                None,
                None,
                "create",
                "user",
                1_000,
                1,
            )
            .expect("capture session");
        let first_blob = br#"{"options":{"media":"iso_a4_210x297mm"}}"#.to_vec();
        let first = piqae_agent_storage::NativeProfileCapture {
            name: "A4".into(),
            is_default: true,
            options_json: "{}".into(),
            status: "ready".into(),
            native_kind: "cups_options".into(),
            native_schema_version: piqae_domain::NATIVE_PROFILE_SCHEMA_VERSION,
            native_digest: format!("sha256:{:x}", Sha256::digest(&first_blob)),
            native_blob: first_blob.clone(),
            driver_fingerprint_json: serde_json::to_string(&DriverFingerprint {
                platform: "macos".into(),
                driver_name: "HP".into(),
                native_queue_id: "native".into(),
                ..Default::default()
            })
            .expect("fingerprint"),
            summary_json: "{}".into(),
            stock_id: None,
            dependencies_json: "[]".into(),
            safe_overrides_json: r#"["copies"]"#.into(),
            published: true,
        };
        let revision_one = store
            .commit_profile_capture("capture-1", "token-1", "user", &first, 2)
            .expect("first revision");

        let mut accepted = accepted("job-profile", "pdf");
        accepted.printer_id.clone_from(&printer.printer_id);
        let mut engine = AgentEngine::new(store, FakeExecutor::default(), FixedClock(10));
        engine.accept(&accepted).expect("accept");
        engine
            .store_mut()
            .pin_job_profile(
                "job-profile",
                None,
                None,
                &revision_one.profile_id,
                revision_one.revision,
                None,
                None,
            )
            .expect("pin revision");

        engine
            .store_mut()
            .create_profile_capture_session(
                "capture-2",
                "token-2",
                &printer.printer_id,
                Some(&revision_one.profile_id),
                Some(revision_one.revision),
                "edit",
                "user",
                1_000,
                3,
            )
            .expect("edit session");
        let second_blob = br#"{"options":{"media":"na_letter_8.5x11in"}}"#.to_vec();
        let mut second = first;
        second.name = "Letter".into();
        second.native_digest = format!("sha256:{:x}", Sha256::digest(&second_blob));
        second.native_blob = second_blob;
        let revision_two = engine
            .store_mut()
            .commit_profile_capture("capture-2", "token-2", "user", &second, 4)
            .expect("second revision");
        assert_eq!(revision_two.revision, revision_one.revision + 1);

        engine.run_once().await.expect("run");
        let submitted = &engine.executor_mut().submitted[0];
        let profile = submitted.native_profile.as_ref().expect("native profile");
        assert_eq!(profile.profile_id, revision_one.profile_id);
        assert_eq!(profile.revision, revision_one.revision);
        assert_eq!(profile.blob, first_blob);
        assert_eq!(profile.safe_overrides, vec![SafeProfileOverride::Copies]);
    }

    #[tokio::test]
    async fn portable_profile_pin_uses_options_without_requiring_a_native_blob() {
        let mut store = AgentStore::in_memory().expect("store");
        let printer = store
            .upsert_printer("native", "HP", "online", true, "{}", 1)
            .expect("printer");
        let profile = store
            .create_named_profile(&printer.printer_id, "Basic A4", true, r#"{"copies":2}"#, 2)
            .expect("portable profile");
        let mut job = accepted("job-portable-profile", "pdf");
        job.printer_id.clone_from(&printer.printer_id);
        job.options_json = profile.options_json.clone();
        let mut engine = AgentEngine::new(store, FakeExecutor::default(), FixedClock(10));
        engine.accept(&job).expect("accept");
        engine
            .store_mut()
            .pin_job_profile(
                &job.job_id,
                None,
                None,
                &profile.profile_id,
                profile.revision,
                None,
                None,
            )
            .expect("pin");

        engine.run_once().await.expect("run");
        let submitted = &engine.executor_mut().submitted[0];
        assert!(submitted.native_profile.is_none());
        assert_eq!(submitted.options.copies, Some(2));
    }

    #[tokio::test]
    async fn ambiguous_handoff_is_never_automatically_retried() {
        let executor = FakeExecutor {
            result: Some(Err(ExecutorFailure {
                code: "executor_crashed".into(),
                message: "child exited during native call".into(),
                retryable: true,
                handoff_may_have_succeeded: true,
                native_code: None,
            })),
            ..FakeExecutor::default()
        };
        let store = AgentStore::in_memory().expect("store");
        let mut engine = AgentEngine::new(store, executor, FixedClock(10));
        engine
            .accept_with_facts(&accepted("job", "raw"), &raw_facts())
            .expect("accept");
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
        assert_eq!(
            engine.store().failure_health().expect("durable health"),
            (1, Some("executor_crashed".into()))
        );
    }

    #[tokio::test]
    async fn retryable_pre_handoff_failure_requeues_before_retrying_once() {
        let executor = FakeExecutor {
            result: Some(Err(ExecutorFailure {
                code: "printer_temporarily_unavailable".into(),
                message: "virtual printer is temporarily unavailable".into(),
                retryable: true,
                handoff_may_have_succeeded: false,
                native_code: None,
            })),
            ..FakeExecutor::default()
        };
        let store = AgentStore::in_memory().expect("store");
        let mut engine = AgentEngine::new(store, executor, FixedClock(10));
        engine
            .accept(&accepted("retryable", "pdf"))
            .expect("accept");

        engine.run_once().await.expect("bounded first failure");
        assert_eq!(
            engine
                .store()
                .get_job("retryable")
                .expect("query")
                .expect("job")
                .state,
            "failed_retryable"
        );

        assert_eq!(engine.run_once().await.expect("retry remains deferred"), 0);
        let (store, executor, _) = engine.into_parts();
        let mut engine = AgentEngine::new(
            store,
            executor,
            FixedClock(10 + AgentEngine::<FakeExecutor, FixedClock>::DEFAULT_LOCAL_RETRY_DELAY_MS),
        );
        engine.run_once().await.expect("safe retry after restart");
        assert_eq!(engine.executor_mut().submitted.len(), 2);
        assert_eq!(
            engine
                .store()
                .get_job("retryable")
                .expect("query")
                .expect("job")
                .state,
            "accepted_by_spooler"
        );
        let states = engine
            .store()
            .pending_events(0, 20)
            .expect("events")
            .into_iter()
            .map(|event| event.state)
            .collect::<Vec<_>>();
        assert_eq!(
            states,
            [
                "queued_local",
                "preparing",
                "rendering",
                "spool_intent",
                "failed_retryable",
                "queued_local",
                "preparing",
                "rendering",
                "spool_intent",
                "accepted_by_spooler",
            ]
        );
    }

    #[tokio::test]
    async fn cloud_prehandoff_rejection_never_self_retries_or_blocks_new_work() {
        let executor = FakeExecutor {
            result: Some(Err(ExecutorFailure {
                code: "printer_temporarily_unavailable".into(),
                message: "virtual printer is temporarily unavailable".into(),
                retryable: true,
                handoff_may_have_succeeded: false,
                native_code: None,
            })),
            ..FakeExecutor::default()
        };
        let store = AgentStore::in_memory().expect("store");
        let mut engine = AgentEngine::new(store, executor, FixedClock(10));
        let mut cloud = accepted("cloud-rejected", "pdf");
        cloud.cloud_managed = true;
        engine
            .accept_with_facts(
                &cloud,
                &JobPersistenceFacts {
                    route_connector_id: Some("legacy".into()),
                    ..JobPersistenceFacts::default()
                },
            )
            .expect("cloud responsibility");

        engine.run_once().await.expect("bounded cloud rejection");
        assert_eq!(
            engine
                .store()
                .get_job("cloud-rejected")
                .expect("query")
                .expect("job")
                .state,
            "failed_retryable"
        );
        assert_eq!(engine.run_once().await.expect("cloud retry quarantined"), 0);

        let (store, mut executor, _) = engine.into_parts();
        executor.result = None;
        let mut restarted = AgentEngine::new(store, executor, FixedClock(20));
        assert_eq!(
            restarted
                .run_once()
                .await
                .expect("restart keeps cloud retry quarantined"),
            0
        );
        let mut later = accepted("new-server-authorized-job", "pdf");
        later.accepted_unix_ms = 2;
        restarted.accept(&later).expect("later local authority");
        assert_eq!(
            restarted.run_once().await.expect("later work progresses"),
            1
        );
        assert_eq!(restarted.executor_mut().submitted.len(), 2);
    }

    #[tokio::test]
    async fn verified_content_is_atomically_published() {
        let directory = tempfile::tempdir().expect("tempdir");
        let content = b"hello piqae";
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

    #[tokio::test]
    async fn verified_content_honours_a_caller_supplied_limit() {
        let directory = tempfile::tempdir().expect("tempdir");
        let store = ContentStore::open(directory.path()).await.expect("open");
        let content = b"authenticated ciphertext";
        let digest = format!("{:x}", Sha256::digest(content));
        let error = store
            .put_verified_with_limit(&digest, BufReader::new(content.as_slice()), 8)
            .await
            .expect_err("limit");
        assert!(matches!(error, AgentError::ContentTooLarge { limit: 8 }));
        let mut entries = tokio::fs::read_dir(directory.path()).await.expect("list");
        assert!(entries.next_entry().await.expect("entry").is_none());
    }

    #[tokio::test]
    async fn cancelled_stream_removes_partial_content() {
        let directory = tempfile::tempdir().expect("tempdir");
        let store = ContentStore::open(directory.path()).await.expect("open");
        let (mut writer, reader) = tokio::io::duplex(64);
        let write = tokio::spawn(async move {
            writer.write_all(b"partial").await.expect("write");
            std::future::pending::<()>().await;
        });
        let put = tokio::spawn(async move { store.put(reader).await });
        tokio::time::sleep(Duration::from_millis(10)).await;
        put.abort();
        let _ = put.await;
        write.abort();
        let mut entries = tokio::fs::read_dir(directory.path()).await.expect("list");
        assert!(entries.next_entry().await.expect("entry").is_none());
    }

    #[tokio::test]
    async fn fake_observations_progress_without_claiming_physical_delivery() {
        let executor = FakeExecutor {
            observations: VecDeque::from([
                Ok(observation(NativeJobState::Queued)),
                Ok(observation(NativeJobState::Printing)),
                Ok(observation(NativeJobState::Completed)),
            ]),
            ..FakeExecutor::default()
        };
        let store = AgentStore::in_memory().expect("store");
        let mut engine = AgentEngine::new(store, executor, FixedClock(10));
        engine
            .accept_with_facts(&accepted("job", "raw"), &raw_facts())
            .expect("accept");
        engine.run_once().await.expect("submit");

        let (store, executor, _) = engine.into_parts();
        let mut engine = AgentEngine::new(store, executor, FixedClock(2_010));
        engine.run_once().await.expect("queued observation");
        let (store, executor, _) = engine.into_parts();
        let mut engine = AgentEngine::new(store, executor, FixedClock(4_010));
        engine.run_once().await.expect("printing observation");
        let (store, executor, _) = engine.into_parts();
        let mut engine = AgentEngine::new(store, executor, FixedClock(6_010));
        engine.run_once().await.expect("completed observation");

        let events = engine.store().pending_events(0, 20).expect("events");
        let states: Vec<_> = events.iter().map(|event| event.state.as_str()).collect();
        assert!(states.ends_with(&["spooling", "printing", "completed_reported"]));
        let completed = events.last().expect("completed event");
        assert!(
            completed
                .message
                .as_deref()
                .is_some_and(|message| message.contains("physical output is not proven"))
        );
        assert!(
            engine
                .store()
                .due_reconciliations(i64::MAX, 10)
                .expect("due")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn reconciliation_resumes_after_sqlite_restart() {
        let directory = tempfile::tempdir().expect("tempdir");
        let database = directory.path().join("agent.sqlite3");
        let store = AgentStore::open(&database).expect("store");
        let mut engine = AgentEngine::new(store, FakeExecutor::default(), FixedClock(10));
        engine
            .accept_with_facts(&accepted("job", "raw"), &raw_facts())
            .expect("accept");
        engine.run_once().await.expect("submit");
        drop(engine);

        let executor = FakeExecutor {
            observations: VecDeque::from([Ok(observation(NativeJobState::Completed))]),
            ..FakeExecutor::default()
        };
        let store = AgentStore::open(&database).expect("reopen");
        let mut restarted = AgentEngine::new(store, executor, FixedClock(2_010));
        assert_eq!(restarted.run_once().await.expect("reconcile"), 1);
        assert_eq!(
            restarted
                .store()
                .get_job("job")
                .expect("query")
                .expect("job")
                .state,
            "completed_reported"
        );
    }

    #[tokio::test]
    async fn unknown_native_outcome_ages_to_delivery_uncertain() {
        let executor = FakeExecutor {
            observations: VecDeque::from([Ok(observation(NativeJobState::Unknown))]),
            ..FakeExecutor::default()
        };
        let store = AgentStore::in_memory().expect("store");
        let mut engine = AgentEngine::new(store, executor, FixedClock(10));
        engine
            .accept_with_facts(&accepted("job", "raw"), &raw_facts())
            .expect("accept");
        engine.run_once().await.expect("submit");
        let (store, executor, _) = engine.into_parts();
        let deadline = 10 + AgentEngine::<FakeExecutor, FixedClock>::DEFAULT_UNCERTAINTY_AFTER_MS;
        let mut engine = AgentEngine::new(store, executor, FixedClock(deadline));
        engine.run_once().await.expect("uncertain");
        assert_eq!(
            engine
                .store()
                .get_job("job")
                .expect("query")
                .expect("job")
                .state,
            "delivery_uncertain"
        );
    }

    #[tokio::test]
    async fn active_cancellation_runs_through_executor() {
        let store = AgentStore::in_memory().expect("store");
        let mut engine = AgentEngine::new(store, FakeExecutor::default(), FixedClock(10));
        engine
            .accept_with_facts(&accepted("job", "raw"), &raw_facts())
            .expect("accept");
        engine.run_once().await.expect("submit");
        engine
            .store_mut()
            .request_cancel("job", 20)
            .expect("request cancellation");
        let (store, executor, _) = engine.into_parts();
        let mut engine = AgentEngine::new(store, executor, FixedClock(20));
        engine.run_once().await.expect("cancel");
        assert_eq!(
            engine
                .store()
                .get_job("job")
                .expect("query")
                .expect("job")
                .state,
            "cancelled"
        );
        let (_, executor, _) = engine.into_parts();
        assert_eq!(executor.cancellations, 1);
    }
}
