//! Durable, platform-neutral coordination for whole-node updates.
//!
//! This crate deliberately cannot install software itself. Platform packages
//! implement [`RuntimeManager`], while this state machine verifies a candidate,
//! records every intent before a side effect, admits work only while the node
//! is paused and idle, bounds health checks, and requests rollback on failure.
//! The separation keeps update recovery testable without touching an installed
//! application.

use chrono::{DateTime, Utc};
use ed25519_dalek::VerifyingKey;
use piqae_update_metadata::{MetadataRole, SignedMetadata, UpdateTarget, VerificationError};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;

const STATE_SCHEMA_VERSION: u16 = 1;
const MAX_JOURNAL_BYTES: u64 = 8 * 1024 * 1024;
const MAX_JOURNAL_RECORD_BYTES: usize = 64 * 1024;
const HASH_BUFFER_BYTES: usize = 64 * 1024;

/// Immutable policy applied by one guardian instance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GuardianConfig {
    pub platform: String,
    pub architecture: String,
    pub max_artifact_bytes: u64,
    pub health_timeout_ms: i64,
    pub max_health_attempts: u32,
}

impl GuardianConfig {
    /// Rejects configurations that could disable artifact or health bounds.
    ///
    /// # Errors
    ///
    /// Returns [`GuardianError::InvalidConfig`] for an empty target or a
    /// non-positive bound.
    pub fn validate(&self) -> Result<(), GuardianError> {
        if self.platform.trim().is_empty() || self.architecture.trim().is_empty() {
            return Err(GuardianError::InvalidConfig(
                "platform and architecture are required".into(),
            ));
        }
        if self.max_artifact_bytes == 0 {
            return Err(GuardianError::InvalidConfig(
                "artifact limit must be greater than zero".into(),
            ));
        }
        if self.health_timeout_ms <= 0 || self.max_health_attempts == 0 {
            return Err(GuardianError::InvalidConfig(
                "health timeout and attempts must be greater than zero".into(),
            ));
        }
        Ok(())
    }
}

/// Snapshot obtained from the durable agent immediately before admission.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuntimeActivity {
    pub paused: bool,
    pub queued_jobs: u32,
    pub active_jobs: u32,
    pub handoff_in_progress: bool,
    pub profile_edit_active: bool,
}

impl RuntimeActivity {
    #[must_use]
    pub const fn admission(self) -> Admission {
        if !self.paused {
            Admission::Blocked(AdmissionBlock::NotPaused)
        } else if self.active_jobs > 0 {
            Admission::Blocked(AdmissionBlock::ActiveJobs)
        } else if self.queued_jobs > 0 {
            Admission::Blocked(AdmissionBlock::QueuedJobs)
        } else if self.handoff_in_progress {
            Admission::Blocked(AdmissionBlock::SpoolerHandoff)
        } else if self.profile_edit_active {
            Admission::Blocked(AdmissionBlock::ProfileEdit)
        } else {
            Admission::Ready
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Admission {
    Ready,
    Blocked(AdmissionBlock),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdmissionBlock {
    NotPaused,
    QueuedJobs,
    ActiveJobs,
    SpoolerHandoff,
    ProfileEdit,
}

/// A candidate that passed signed metadata, target, size, digest, and platform
/// signature verification.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct VerifiedCandidate {
    pub release: Version,
    pub metadata_version: u64,
    pub artifact_path: PathBuf,
    pub artifact_sha256: String,
    pub artifact_length: u64,
}

impl VerifiedCandidate {
    /// Revalidates persisted artifact bytes immediately before platform
    /// staging. This closes the gap between initial verification and a later
    /// restart or deferred idle window.
    ///
    /// # Errors
    ///
    /// Returns an error if the artifact is no longer the same bounded regular
    /// file covered by the signed metadata.
    pub fn revalidate_local_artifact(&self, maximum: u64) -> Result<(), GuardianError> {
        if self.artifact_length == 0 || self.artifact_length > maximum {
            return Err(GuardianError::ArtifactLengthOutOfBounds {
                length: self.artifact_length,
                maximum,
            });
        }
        let metadata = fs::symlink_metadata(&self.artifact_path)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(GuardianError::ArtifactNotRegularFile);
        }
        if metadata.len() != self.artifact_length {
            return Err(GuardianError::ArtifactLengthMismatch {
                expected: self.artifact_length,
                actual: metadata.len(),
            });
        }
        if sha256_file(&self.artifact_path, maximum)? != self.artifact_sha256 {
            return Err(GuardianError::ArtifactDigestMismatch);
        }
        Ok(())
    }
}

/// Platform-specific signature verification remains mandatory and fail-closed.
pub trait PlatformArtifactVerifier {
    /// Verifies the operating-system signature or notarisation evidence.
    ///
    /// # Errors
    ///
    /// Returns a redacted reason when platform trust cannot be established.
    fn verify_platform_signature(
        &self,
        artifact: &Path,
        target: &UpdateTarget,
    ) -> Result<(), String>;
}

/// Complete trust input for one local candidate verification.
#[derive(Clone, Copy)]
pub struct CandidateVerification<'a> {
    pub metadata: &'a SignedMetadata,
    pub trusted_key: &'a VerifyingKey,
    pub trusted_metadata_version: u64,
    pub installed_release: &'a Version,
    pub now: DateTime<Utc>,
    pub config: &'a GuardianConfig,
    pub artifact_path: &'a Path,
    pub platform_verifier: &'a dyn PlatformArtifactVerifier,
}

impl std::fmt::Debug for CandidateVerification<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CandidateVerification")
            .field("trusted_metadata_version", &self.trusted_metadata_version)
            .field("installed_release", &self.installed_release)
            .field("now", &self.now)
            .field("config", &self.config)
            .field("artifact_path", &self.artifact_path)
            .finish_non_exhaustive()
    }
}

/// Verifies a local artifact without copying or executing it.
///
/// # Errors
///
/// Returns a fail-closed [`GuardianError`] when metadata trust, target
/// selection, file bounds, digest, or platform signature validation fails.
pub fn verify_candidate(
    request: CandidateVerification<'_>,
) -> Result<VerifiedCandidate, GuardianError> {
    request.config.validate()?;
    request.metadata.verify(
        request.trusted_key,
        request.trusted_metadata_version,
        request.installed_release,
        request.now,
    )?;
    if request.metadata.signed.role != MetadataRole::Targets {
        return Err(GuardianError::WrongMetadataRole);
    }
    if request.metadata.signed.version <= request.trusted_metadata_version {
        return Err(GuardianError::MetadataNotNewer {
            trusted: request.trusted_metadata_version,
            received: request.metadata.signed.version,
        });
    }
    if request.metadata.signed.release <= *request.installed_release {
        return Err(GuardianError::ReleaseNotNewer {
            installed: request.installed_release.clone(),
            received: request.metadata.signed.release.clone(),
        });
    }

    let mut matching = request.metadata.signed.targets.iter().filter(|target| {
        target.platform == request.config.platform
            && target.architecture == request.config.architecture
    });
    let target = matching.next().ok_or(GuardianError::TargetNotFound)?;
    if matching.next().is_some() {
        return Err(GuardianError::AmbiguousTarget);
    }
    if target.length == 0 || target.length > request.config.max_artifact_bytes {
        return Err(GuardianError::ArtifactLengthOutOfBounds {
            length: target.length,
            maximum: request.config.max_artifact_bytes,
        });
    }

    let file_metadata = fs::symlink_metadata(request.artifact_path)?;
    if file_metadata.file_type().is_symlink() || !file_metadata.is_file() {
        return Err(GuardianError::ArtifactNotRegularFile);
    }
    if file_metadata.len() != target.length {
        return Err(GuardianError::ArtifactLengthMismatch {
            expected: target.length,
            actual: file_metadata.len(),
        });
    }
    let digest = sha256_file(request.artifact_path, request.config.max_artifact_bytes)?;
    if digest != target.sha256 {
        return Err(GuardianError::ArtifactDigestMismatch);
    }
    request
        .platform_verifier
        .verify_platform_signature(request.artifact_path, target)
        .map_err(GuardianError::PlatformSignature)?;

    Ok(VerifiedCandidate {
        release: request.metadata.signed.release.clone(),
        metadata_version: request.metadata.signed.version,
        artifact_path: request.artifact_path.to_path_buf(),
        artifact_sha256: digest,
        artifact_length: target.length,
    })
}

fn sha256_file(path: &Path, maximum: u64) -> Result<String, GuardianError> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = vec![0_u8; HASH_BUFFER_BYTES].into_boxed_slice();
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(u64::try_from(read).map_err(|_| GuardianError::ArtifactTooLarge)?)
            .ok_or(GuardianError::ArtifactTooLarge)?;
        if total > maximum {
            return Err(GuardianError::ArtifactTooLarge);
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct UpdateCommand {
    pub command_id: String,
    pub requested_at_unix_ms: i64,
    pub candidate: VerifiedCandidate,
}

impl UpdateCommand {
    fn validate(&self) -> Result<(), GuardianError> {
        if self.command_id.trim().is_empty() || self.command_id.len() > 128 {
            return Err(GuardianError::InvalidCommandId);
        }
        Ok(())
    }
}

/// One immutable, platform-managed runtime directory or package.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuntimeSlot {
    pub version: Version,
    pub locator: String,
}

/// Staging resolves both sides before activation, so recovery never has to
/// guess which prior runtime should be restored.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuntimePlan {
    pub staged: RuntimeSlot,
    pub previous: RuntimeSlot,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActivationObservation {
    NotActivated,
    Activated,
    Unknown,
}

/// Implemented by a signed native package. Methods must be idempotent because
/// the guardian may call them after process restart.
///
/// [`RuntimeManager::stage`] must call
/// [`VerifiedCandidate::revalidate_local_artifact`] and repeat the platform
/// signature check immediately before materializing bytes.
pub trait RuntimeManager {
    /// Materializes a verified candidate and resolves the prior runtime.
    ///
    /// # Errors
    ///
    /// Returns a redacted platform staging error.
    fn stage(&mut self, candidate: &VerifiedCandidate) -> Result<RuntimePlan, String>;
    /// Atomically selects the staged runtime where the platform permits it.
    ///
    /// # Errors
    ///
    /// Returns a redacted activation error.
    fn activate(&mut self, plan: &RuntimePlan) -> Result<(), String>;
    /// Reconciles a persisted activation intent after process restart.
    ///
    /// # Errors
    ///
    /// Returns a redacted platform observation error.
    fn observe_activation(&mut self, plan: &RuntimePlan) -> Result<ActivationObservation, String>;
    /// Restores the exact prior runtime resolved during staging.
    ///
    /// # Errors
    ///
    /// Returns a redacted restoration error.
    fn restore_previous(&mut self, plan: &RuntimePlan) -> Result<(), String>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HealthObservation {
    Pending,
    Healthy,
    Unhealthy(String),
}

pub trait RuntimeHealth {
    /// Observes local runtime health without blocking.
    ///
    /// # Errors
    ///
    /// Returns a redacted observation failure.
    fn observe(&mut self, expected: &RuntimeSlot) -> Result<HealthObservation, String>;
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum GuardianPhase {
    Idle,
    AwaitingIdle {
        blocked_by: Option<AdmissionBlock>,
    },
    Staging,
    Staged {
        plan: RuntimePlan,
    },
    Activating {
        plan: RuntimePlan,
    },
    HealthChecking {
        plan: RuntimePlan,
        started_at_unix_ms: i64,
        deadline_unix_ms: i64,
        attempts: u32,
    },
    RollingBack {
        plan: RuntimePlan,
        cause: String,
    },
    Completed {
        installed: RuntimeSlot,
        rollback: RuntimeSlot,
    },
    RolledBack {
        restored: RuntimeSlot,
        failed_release: Version,
        cause: String,
    },
    Failed {
        code: String,
        detail: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GuardianState {
    pub schema_version: u16,
    pub sequence: u64,
    pub installed_release: Version,
    pub trusted_metadata_version: u64,
    pub command: Option<UpdateCommand>,
    pub phase: GuardianPhase,
    pub updated_at_unix_ms: i64,
}

impl GuardianState {
    #[must_use]
    pub const fn initial(installed_release: Version, now_unix_ms: i64) -> Self {
        Self {
            schema_version: STATE_SCHEMA_VERSION,
            sequence: 0,
            installed_release,
            trusted_metadata_version: 0,
            command: None,
            phase: GuardianPhase::Idle,
            updated_at_unix_ms: now_unix_ms,
        }
    }
}

pub trait GuardianStore {
    /// Loads the last complete validated record.
    ///
    /// # Errors
    ///
    /// Returns an error for I/O, schema, ordering, or checksum failure.
    fn load(&self) -> Result<Option<GuardianState>, GuardianError>;
    /// Durably appends a state newer than the current record.
    ///
    /// # Errors
    ///
    /// Returns an error when the state is invalid or cannot be synchronized.
    fn append(&mut self, state: &GuardianState) -> Result<(), GuardianError>;
}

/// Append-only, checksummed persistence. A torn final write is ignored; any
/// corruption in a completed record fails closed.
#[derive(Debug)]
pub struct JournalStore {
    path: PathBuf,
}

impl JournalStore {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

#[derive(Deserialize, Serialize)]
struct JournalRecord {
    payload: String,
    sha256: String,
}

impl GuardianStore for JournalStore {
    fn load(&self) -> Result<Option<GuardianState>, GuardianError> {
        read_journal(&self.path).map(|journal| journal.latest)
    }

    fn append(&mut self, state: &GuardianState) -> Result<(), GuardianError> {
        let journal = read_journal(&self.path)?;
        validate_loaded_state(state, journal.latest.as_ref())?;
        let payload = serde_json::to_string(state)?;
        let record = JournalRecord {
            sha256: format!("{:x}", Sha256::digest(payload.as_bytes())),
            payload,
        };
        let mut encoded = serde_json::to_vec(&record)?;
        encoded.push(b'\n');
        if encoded.len() > MAX_JOURNAL_RECORD_BYTES {
            return Err(GuardianError::JournalRecordTooLarge);
        }
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        if journal.complete_bytes < journal.file_bytes {
            let file = OpenOptions::new().write(true).open(&self.path)?;
            file.set_len(journal.complete_bytes)?;
            file.sync_data()?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        let current = file.metadata()?.len();
        let added =
            u64::try_from(encoded.len()).map_err(|_| GuardianError::JournalRecordTooLarge)?;
        if current.saturating_add(added) > MAX_JOURNAL_BYTES {
            return Err(GuardianError::JournalTooLarge);
        }
        file.write_all(&encoded)?;
        file.sync_data()?;
        Ok(())
    }
}

struct JournalRead {
    latest: Option<GuardianState>,
    complete_bytes: u64,
    file_bytes: u64,
}

fn read_journal(path: &Path) -> Result<JournalRead, GuardianError> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(JournalRead {
                latest: None,
                complete_bytes: 0,
                file_bytes: 0,
            });
        }
        Err(error) => return Err(error.into()),
    };
    let file_bytes = file.metadata()?.len();
    if file_bytes > MAX_JOURNAL_BYTES {
        return Err(GuardianError::JournalTooLarge);
    }
    let mut latest: Option<GuardianState> = None;
    let mut complete_bytes = 0_u64;
    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
    loop {
        line.clear();
        let read = reader.read_until(b'\n', &mut line)?;
        if read == 0 {
            break;
        }
        if line.last() != Some(&b'\n') {
            break;
        }
        complete_bytes = complete_bytes
            .checked_add(u64::try_from(read).map_err(|_| GuardianError::JournalRecordTooLarge)?)
            .ok_or(GuardianError::JournalTooLarge)?;
        line.pop();
        if line.len() > MAX_JOURNAL_RECORD_BYTES {
            return Err(GuardianError::JournalRecordTooLarge);
        }
        let record: JournalRecord = serde_json::from_slice(&line)?;
        if format!("{:x}", Sha256::digest(record.payload.as_bytes())) != record.sha256 {
            return Err(GuardianError::JournalChecksum);
        }
        let state: GuardianState = serde_json::from_str(&record.payload)?;
        validate_loaded_state(&state, latest.as_ref())?;
        latest = Some(state);
    }
    Ok(JournalRead {
        latest,
        complete_bytes,
        file_bytes,
    })
}

const fn validate_loaded_state(
    state: &GuardianState,
    prior: Option<&GuardianState>,
) -> Result<(), GuardianError> {
    if state.schema_version != STATE_SCHEMA_VERSION {
        return Err(GuardianError::UnsupportedStateSchema(state.schema_version));
    }
    if let Some(prior) = prior {
        if state.sequence <= prior.sequence {
            return Err(GuardianError::StateSequence {
                prior: prior.sequence,
                received: state.sequence,
            });
        }
        if state.trusted_metadata_version < prior.trusted_metadata_version {
            return Err(GuardianError::PersistedMetadataRollback);
        }
    }
    Ok(())
}

/// Deterministic coordinator. Callers supply time and observations; it never
/// sleeps, downloads, executes, or touches a printer.
#[derive(Debug)]
pub struct UpdateGuardian<S> {
    config: GuardianConfig,
    store: S,
    state: GuardianState,
}

impl<S: GuardianStore> UpdateGuardian<S> {
    /// Opens the last durable state or creates an in-memory initial state.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid policy or unreadable/corrupt persistence.
    pub fn open(
        config: GuardianConfig,
        store: S,
        installed_release: Version,
        now_unix_ms: i64,
    ) -> Result<Self, GuardianError> {
        config.validate()?;
        let state = store
            .load()?
            .unwrap_or_else(|| GuardianState::initial(installed_release, now_unix_ms));
        Ok(Self {
            config,
            store,
            state,
        })
    }

    #[must_use]
    pub const fn state(&self) -> &GuardianState {
        &self.state
    }

    /// Persists a verified update request before any runtime side effect.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid/replayed commands, an active update, or a
    /// durable journal failure.
    pub fn request(
        &mut self,
        command: UpdateCommand,
        activity: RuntimeActivity,
        now_unix_ms: i64,
    ) -> Result<(), GuardianError> {
        command.validate()?;
        if command.candidate.release <= self.state.installed_release {
            return Err(GuardianError::ReleaseNotNewer {
                installed: self.state.installed_release.clone(),
                received: command.candidate.release,
            });
        }
        if command.candidate.metadata_version <= self.state.trusted_metadata_version {
            return Err(GuardianError::MetadataNotNewer {
                trusted: self.state.trusted_metadata_version,
                received: command.candidate.metadata_version,
            });
        }
        if !matches!(
            self.state.phase,
            GuardianPhase::Idle
                | GuardianPhase::Completed { .. }
                | GuardianPhase::RolledBack { .. }
                | GuardianPhase::Failed { .. }
        ) {
            return Err(GuardianError::UpdateAlreadyInProgress);
        }
        self.state.command = Some(command);
        self.transition(
            GuardianPhase::AwaitingIdle {
                blocked_by: match activity.admission() {
                    Admission::Ready => None,
                    Admission::Blocked(reason) => Some(reason),
                },
            },
            now_unix_ms,
        )
    }

    /// Advances at most one bounded state-machine step.
    ///
    /// # Errors
    ///
    /// Returns an error when persistence, runtime reconciliation, or health
    /// observation fails. Platform install errors are persisted as state.
    #[allow(clippy::too_many_lines)]
    pub fn advance(
        &mut self,
        activity: RuntimeActivity,
        now_unix_ms: i64,
        runtime: &mut dyn RuntimeManager,
        health: &mut dyn RuntimeHealth,
    ) -> Result<(), GuardianError> {
        match self.state.phase.clone() {
            GuardianPhase::AwaitingIdle { .. } => match activity.admission() {
                Admission::Blocked(reason) => self.transition(
                    GuardianPhase::AwaitingIdle {
                        blocked_by: Some(reason),
                    },
                    now_unix_ms,
                ),
                Admission::Ready => {
                    self.transition(GuardianPhase::Staging, now_unix_ms)?;
                    let candidate = self
                        .state
                        .command
                        .as_ref()
                        .ok_or(GuardianError::MissingCommand)?
                        .candidate
                        .clone();
                    match runtime.stage(&candidate) {
                        Ok(plan) if plan.staged.version == candidate.release => {
                            self.transition(GuardianPhase::Staged { plan }, now_unix_ms)
                        }
                        Ok(_) => self.fail(
                            "staged_version_mismatch",
                            "platform staged a runtime with the wrong version",
                            now_unix_ms,
                        ),
                        Err(detail) => self.fail("stage_failed", &detail, now_unix_ms),
                    }
                }
            },
            GuardianPhase::Staging => self.fail(
                "stage_interrupted",
                "process restarted while staging; artifact was not activated",
                now_unix_ms,
            ),
            GuardianPhase::Staged { plan } => {
                if !matches!(activity.admission(), Admission::Ready) {
                    return Ok(());
                }
                self.transition(
                    GuardianPhase::Activating { plan: plan.clone() },
                    now_unix_ms,
                )?;
                match runtime.activate(&plan) {
                    Ok(()) => self.begin_health(plan, now_unix_ms),
                    Err(detail) => {
                        self.begin_rollback(plan, &format!("activate_failed:{detail}"), now_unix_ms)
                    }
                }
            }
            GuardianPhase::Activating { plan } => match runtime
                .observe_activation(&plan)
                .map_err(GuardianError::Runtime)?
            {
                ActivationObservation::Activated => self.begin_health(plan, now_unix_ms),
                ActivationObservation::NotActivated => {
                    if !matches!(activity.admission(), Admission::Ready) {
                        return Ok(());
                    }
                    match runtime.activate(&plan) {
                        Ok(()) => self.begin_health(plan, now_unix_ms),
                        Err(detail) => self.begin_rollback(
                            plan,
                            &format!("activation_recovery_failed:{detail}"),
                            now_unix_ms,
                        ),
                    }
                }
                ActivationObservation::Unknown => {
                    self.begin_rollback(plan, "activation_state_unknown", now_unix_ms)
                }
            },
            GuardianPhase::HealthChecking {
                plan,
                started_at_unix_ms,
                deadline_unix_ms,
                attempts,
            } => {
                let next_attempts = attempts.saturating_add(1);
                match health
                    .observe(&plan.staged)
                    .map_err(GuardianError::Health)?
                {
                    HealthObservation::Healthy => {
                        let metadata_version = self
                            .state
                            .command
                            .as_ref()
                            .ok_or(GuardianError::MissingCommand)?
                            .candidate
                            .metadata_version;
                        self.complete(plan, metadata_version, now_unix_ms)
                    }
                    HealthObservation::Unhealthy(reason) => {
                        self.begin_rollback(plan, &format!("unhealthy:{reason}"), now_unix_ms)
                    }
                    HealthObservation::Pending
                        if now_unix_ms >= deadline_unix_ms
                            || next_attempts >= self.config.max_health_attempts =>
                    {
                        self.begin_rollback(plan, "health_check_timeout", now_unix_ms)
                    }
                    HealthObservation::Pending => self.transition(
                        GuardianPhase::HealthChecking {
                            plan,
                            started_at_unix_ms,
                            deadline_unix_ms,
                            attempts: next_attempts,
                        },
                        now_unix_ms,
                    ),
                }
            }
            GuardianPhase::RollingBack { plan, cause } => match runtime.restore_previous(&plan) {
                Ok(()) => self.transition(
                    GuardianPhase::RolledBack {
                        restored: plan.previous,
                        failed_release: plan.staged.version,
                        cause,
                    },
                    now_unix_ms,
                ),
                Err(detail) => self.fail(
                    "rollback_failed",
                    &format!("{cause}; restore error: {detail}"),
                    now_unix_ms,
                ),
            },
            GuardianPhase::Idle
            | GuardianPhase::Completed { .. }
            | GuardianPhase::RolledBack { .. }
            | GuardianPhase::Failed { .. } => Ok(()),
        }
    }

    fn begin_health(&mut self, plan: RuntimePlan, now_unix_ms: i64) -> Result<(), GuardianError> {
        let deadline = now_unix_ms
            .checked_add(self.config.health_timeout_ms)
            .ok_or(GuardianError::TimeOverflow)?;
        self.transition(
            GuardianPhase::HealthChecking {
                plan,
                started_at_unix_ms: now_unix_ms,
                deadline_unix_ms: deadline,
                attempts: 0,
            },
            now_unix_ms,
        )
    }

    fn complete(
        &mut self,
        plan: RuntimePlan,
        metadata_version: u64,
        now_unix_ms: i64,
    ) -> Result<(), GuardianError> {
        let prior = self.state.clone();
        self.state.installed_release = plan.staged.version.clone();
        self.state.trusted_metadata_version = metadata_version;
        let result = self.transition(
            GuardianPhase::Completed {
                installed: plan.staged,
                rollback: plan.previous,
            },
            now_unix_ms,
        );
        if result.is_err() {
            self.state = prior;
        }
        result
    }

    fn begin_rollback(
        &mut self,
        plan: RuntimePlan,
        cause: &str,
        now_unix_ms: i64,
    ) -> Result<(), GuardianError> {
        self.transition(
            GuardianPhase::RollingBack {
                plan,
                cause: bounded_detail(cause),
            },
            now_unix_ms,
        )
    }

    fn fail(&mut self, code: &str, detail: &str, now_unix_ms: i64) -> Result<(), GuardianError> {
        self.transition(
            GuardianPhase::Failed {
                code: code.into(),
                detail: bounded_detail(detail),
            },
            now_unix_ms,
        )
    }

    fn transition(&mut self, phase: GuardianPhase, now_unix_ms: i64) -> Result<(), GuardianError> {
        let prior = self.state.clone();
        self.state.sequence = self
            .state
            .sequence
            .checked_add(1)
            .ok_or(GuardianError::StateSequenceOverflow)?;
        self.state.updated_at_unix_ms = now_unix_ms;
        self.state.phase = phase;
        if let Err(error) = self.store.append(&self.state) {
            self.state = prior;
            return Err(error);
        }
        Ok(())
    }
}

fn bounded_detail(detail: &str) -> String {
    detail.chars().take(512).collect()
}

#[derive(Debug, Error)]
pub enum GuardianError {
    #[error("invalid guardian configuration: {0}")]
    InvalidConfig(String),
    #[error("signed metadata verification failed: {0}")]
    Metadata(#[from] VerificationError),
    #[error("expected targets metadata")]
    WrongMetadataRole,
    #[error("metadata version {received} is not newer than trusted version {trusted}")]
    MetadataNotNewer { trusted: u64, received: u64 },
    #[error("release {received} is not newer than installed release {installed}")]
    ReleaseNotNewer {
        installed: Version,
        received: Version,
    },
    #[error("no update target matched this platform and architecture")]
    TargetNotFound,
    #[error("more than one update target matched this platform and architecture")]
    AmbiguousTarget,
    #[error("artifact length {length} is outside the maximum {maximum}")]
    ArtifactLengthOutOfBounds { length: u64, maximum: u64 },
    #[error("artifact must be a regular non-symlink file")]
    ArtifactNotRegularFile,
    #[error("artifact length mismatch: expected {expected}, got {actual}")]
    ArtifactLengthMismatch { expected: u64, actual: u64 },
    #[error("artifact exceeded its configured bound while hashing")]
    ArtifactTooLarge,
    #[error("artifact digest does not match signed metadata")]
    ArtifactDigestMismatch,
    #[error("platform signature verification failed: {0}")]
    PlatformSignature(String),
    #[error("invalid update command id")]
    InvalidCommandId,
    #[error("another update is already in progress")]
    UpdateAlreadyInProgress,
    #[error("durable update command is missing")]
    MissingCommand,
    #[error("runtime operation failed: {0}")]
    Runtime(String),
    #[error("runtime health observation failed: {0}")]
    Health(String),
    #[error("guardian state schema {0} is unsupported")]
    UnsupportedStateSchema(u16),
    #[error("guardian state sequence did not increase: previous {prior}, received {received}")]
    StateSequence { prior: u64, received: u64 },
    #[error("guardian state sequence overflowed")]
    StateSequenceOverflow,
    #[error("persisted trusted metadata version moved backwards")]
    PersistedMetadataRollback,
    #[error("guardian journal exceeded its size limit")]
    JournalTooLarge,
    #[error("guardian journal record exceeded its size limit")]
    JournalRecordTooLarge,
    #[error("guardian journal checksum is invalid")]
    JournalChecksum,
    #[error("health-check deadline overflowed")]
    TimeOverflow,
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}
