use base64::{Engine as _, engine::general_purpose::STANDARD};
use chrono::{Duration, Utc};
use ed25519_dalek::{Signer, SigningKey};
use piqae_update_guardian::{
    ActivationObservation, Admission, AdmissionBlock, CandidateVerification, GuardianConfig,
    GuardianError, GuardianPhase, GuardianState, GuardianStore, HealthObservation, JournalStore,
    PlatformArtifactVerifier, RuntimeActivity, RuntimeHealth, RuntimeManager, RuntimePlan,
    RuntimeSlot, UpdateCommand, UpdateGuardian, verify_candidate,
};
use piqae_update_metadata::{
    MetadataRole, ReleaseMetadata, SignedMetadata, UpdateChannel, UpdateTarget, key_id,
};
use semver::Version;
use sha2::{Digest as _, Sha256};
use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::Path;
use tempfile::tempdir;

#[derive(Default)]
struct AcceptSignature;

impl PlatformArtifactVerifier for AcceptSignature {
    fn verify_platform_signature(
        &self,
        _artifact: &Path,
        _target: &UpdateTarget,
    ) -> Result<(), String> {
        Ok(())
    }
}

fn config() -> GuardianConfig {
    GuardianConfig {
        platform: "macos".into(),
        architecture: "aarch64".into(),
        max_artifact_bytes: 1024,
        health_timeout_ms: 60_000,
        max_health_attempts: 3,
    }
}

fn signed_metadata(bytes: &[u8]) -> (SignedMetadata, SigningKey) {
    let signing_key = SigningKey::from_bytes(&[9_u8; 32]);
    let metadata = ReleaseMetadata {
        role: MetadataRole::Targets,
        version: 2,
        expires_at: Utc::now() + Duration::hours(1),
        channel: UpdateChannel::Stable,
        release: Version::new(2, 0, 0),
        rollout_percent: 100,
        targets: vec![UpdateTarget {
            platform: "macos".into(),
            architecture: "aarch64".into(),
            url: "https://downloads.piqae.com/releases/stable/piqae.zip".into(),
            sha256: format!("{:x}", Sha256::digest(bytes)),
            length: bytes.len() as u64,
        }],
    };
    let signature = signing_key.sign(&serde_json::to_vec(&metadata).unwrap_or_default());
    (
        SignedMetadata {
            key_id: key_id(&signing_key.verifying_key()),
            signed: metadata,
            signature: STANDARD.encode(signature.to_bytes()),
        },
        signing_key,
    )
}

fn candidate(path: &Path) -> piqae_update_guardian::VerifiedCandidate {
    let bytes = b"signed package";
    std::fs::write(path, bytes).unwrap_or_default();
    let (metadata, key) = signed_metadata(bytes);
    let verifying_key = key.verifying_key();
    let installed = Version::new(1, 0, 0);
    let guardian_config = config();
    verify_candidate(CandidateVerification {
        metadata: &metadata,
        trusted_key: &verifying_key,
        trusted_metadata_version: 1,
        installed_release: &installed,
        now: Utc::now(),
        config: &guardian_config,
        artifact_path: path,
        platform_verifier: &AcceptSignature,
    })
    .unwrap_or_else(|error| panic!("candidate should verify: {error}"))
}

fn idle() -> RuntimeActivity {
    RuntimeActivity {
        paused: true,
        ..RuntimeActivity::default()
    }
}

#[derive(Default)]
struct FakeRuntime {
    calls: Vec<&'static str>,
    observation: Option<ActivationObservation>,
    fail_restore: bool,
}

impl RuntimeManager for FakeRuntime {
    fn stage(
        &mut self,
        candidate: &piqae_update_guardian::VerifiedCandidate,
    ) -> Result<RuntimePlan, String> {
        self.calls.push("stage");
        Ok(RuntimePlan {
            staged: RuntimeSlot {
                version: candidate.release.clone(),
                locator: "staged-v2".into(),
            },
            previous: RuntimeSlot {
                version: Version::new(1, 0, 0),
                locator: "runtime-v1".into(),
            },
        })
    }

    fn activate(&mut self, _plan: &RuntimePlan) -> Result<(), String> {
        self.calls.push("activate");
        Ok(())
    }

    fn observe_activation(&mut self, _plan: &RuntimePlan) -> Result<ActivationObservation, String> {
        self.calls.push("observe_activation");
        Ok(self.observation.unwrap_or(ActivationObservation::Activated))
    }

    fn restore_previous(&mut self, _plan: &RuntimePlan) -> Result<(), String> {
        self.calls.push("restore");
        if self.fail_restore {
            Err("restore unavailable".into())
        } else {
            Ok(())
        }
    }
}

struct FakeHealth(Vec<HealthObservation>);

impl RuntimeHealth for FakeHealth {
    fn observe(&mut self, _expected: &RuntimeSlot) -> Result<HealthObservation, String> {
        if self.0.is_empty() {
            Ok(HealthObservation::Pending)
        } else {
            Ok(self.0.remove(0))
        }
    }
}

#[derive(Default)]
struct FailCompletionStore {
    current: Option<GuardianState>,
}

impl GuardianStore for FailCompletionStore {
    fn load(&self) -> Result<Option<GuardianState>, GuardianError> {
        Ok(self.current.clone())
    }

    fn append(&mut self, state: &GuardianState) -> Result<(), GuardianError> {
        if matches!(state.phase, GuardianPhase::Completed { .. }) {
            return Err(std::io::Error::other("simulated durable write failure").into());
        }
        self.current = Some(state.clone());
        Ok(())
    }
}

#[test]
fn candidate_verification_rejects_tampered_bytes_and_metadata_replay() {
    let directory = tempdir().unwrap_or_else(|error| panic!("tempdir: {error}"));
    let artifact = directory.path().join("piqae.zip");
    let bytes = b"signed package";
    std::fs::write(&artifact, bytes).unwrap_or_default();
    let (metadata, key) = signed_metadata(bytes);
    let verifying_key = key.verifying_key();
    let installed = Version::new(1, 0, 0);
    let guardian_config = config();

    std::fs::write(&artifact, b"changed package").unwrap_or_default();
    assert!(
        verify_candidate(CandidateVerification {
            metadata: &metadata,
            trusted_key: &verifying_key,
            trusted_metadata_version: 1,
            installed_release: &installed,
            now: Utc::now(),
            config: &guardian_config,
            artifact_path: &artifact,
            platform_verifier: &AcceptSignature,
        })
        .is_err()
    );

    std::fs::write(&artifact, bytes).unwrap_or_default();
    assert!(
        verify_candidate(CandidateVerification {
            metadata: &metadata,
            trusted_key: &verifying_key,
            trusted_metadata_version: 2,
            installed_release: &installed,
            now: Utc::now(),
            config: &guardian_config,
            artifact_path: &artifact,
            platform_verifier: &AcceptSignature,
        })
        .is_err()
    );
}

#[test]
fn deferred_candidate_detects_bytes_replaced_after_initial_verification() {
    let directory = tempdir().unwrap_or_else(|error| panic!("tempdir: {error}"));
    let artifact = directory.path().join("piqae.zip");
    let verified = candidate(&artifact);
    assert!(verified.revalidate_local_artifact(1024).is_ok());
    std::fs::write(&artifact, b"replaced bytes").unwrap_or_default();
    assert!(verified.revalidate_local_artifact(1024).is_err());
}

#[test]
fn busy_node_persists_command_and_defers_every_runtime_side_effect() {
    let directory = tempdir().unwrap_or_else(|error| panic!("tempdir: {error}"));
    let journal = directory.path().join("guardian.jsonl");
    let artifact = directory.path().join("piqae.zip");
    let mut guardian = UpdateGuardian::open(
        config(),
        JournalStore::new(&journal),
        Version::new(1, 0, 0),
        1,
    )
    .unwrap_or_else(|error| panic!("open: {error}"));
    let activity = RuntimeActivity {
        paused: false,
        queued_jobs: 2,
        ..RuntimeActivity::default()
    };
    guardian
        .request(
            UpdateCommand {
                command_id: "command-1".into(),
                requested_at_unix_ms: 1,
                candidate: candidate(&artifact),
            },
            activity,
            2,
        )
        .unwrap_or_else(|error| panic!("request: {error}"));

    let mut runtime = FakeRuntime::default();
    let mut health = FakeHealth(vec![]);
    guardian
        .advance(activity, 3, &mut runtime, &mut health)
        .unwrap_or_else(|error| panic!("advance: {error}"));
    assert!(runtime.calls.is_empty());
    assert!(matches!(
        guardian.state().phase,
        GuardianPhase::AwaitingIdle {
            blocked_by: Some(AdmissionBlock::NotPaused)
        }
    ));

    let reopened = UpdateGuardian::open(
        config(),
        JournalStore::new(journal),
        Version::new(1, 0, 0),
        4,
    )
    .unwrap_or_else(|error| panic!("reopen: {error}"));
    assert_eq!(
        reopened
            .state()
            .command
            .as_ref()
            .map(|command| command.command_id.as_str()),
        Some("command-1")
    );
}

#[test]
fn restart_after_activation_intent_reconciles_and_completes() {
    let directory = tempdir().unwrap_or_else(|error| panic!("tempdir: {error}"));
    let journal = directory.path().join("guardian.jsonl");
    let artifact = directory.path().join("piqae.zip");
    let mut guardian = UpdateGuardian::open(
        config(),
        JournalStore::new(&journal),
        Version::new(1, 0, 0),
        1,
    )
    .unwrap_or_else(|error| panic!("open: {error}"));
    guardian
        .request(
            UpdateCommand {
                command_id: "command-2".into(),
                requested_at_unix_ms: 1,
                candidate: candidate(&artifact),
            },
            idle(),
            2,
        )
        .unwrap_or_else(|error| panic!("request: {error}"));
    let mut runtime = FakeRuntime::default();
    let mut health = FakeHealth(vec![]);
    guardian
        .advance(idle(), 3, &mut runtime, &mut health)
        .unwrap_or_else(|error| panic!("stage: {error}"));
    guardian
        .advance(idle(), 4, &mut runtime, &mut health)
        .unwrap_or_else(|error| panic!("activate: {error}"));
    assert!(matches!(
        guardian.state().phase,
        GuardianPhase::HealthChecking { .. }
    ));

    // Simulate a process restart from the durable post-activation state.
    let mut reopened = UpdateGuardian::open(
        config(),
        JournalStore::new(journal),
        Version::new(1, 0, 0),
        5,
    )
    .unwrap_or_else(|error| panic!("reopen: {error}"));
    let mut health = FakeHealth(vec![HealthObservation::Healthy]);
    reopened
        .advance(idle(), 6, &mut runtime, &mut health)
        .unwrap_or_else(|error| panic!("health: {error}"));
    assert!(matches!(
        reopened.state().phase,
        GuardianPhase::Completed { .. }
    ));
    assert_eq!(reopened.state().installed_release, Version::new(2, 0, 0));
    assert_eq!(reopened.state().trusted_metadata_version, 2);
}

#[test]
fn interrupted_activation_is_reconciled_before_health_check() {
    let directory = tempdir().unwrap_or_else(|error| panic!("tempdir: {error}"));
    let journal = directory.path().join("guardian.jsonl");
    let artifact = directory.path().join("piqae.zip");
    let mut guardian = UpdateGuardian::open(
        config(),
        JournalStore::new(&journal),
        Version::new(1, 0, 0),
        1,
    )
    .unwrap_or_else(|error| panic!("open: {error}"));
    guardian
        .request(
            UpdateCommand {
                command_id: "command-3".into(),
                requested_at_unix_ms: 1,
                candidate: candidate(&artifact),
            },
            idle(),
            2,
        )
        .unwrap_or_else(|error| panic!("request: {error}"));
    let mut runtime = FakeRuntime::default();
    let mut health = FakeHealth(vec![]);
    guardian
        .advance(idle(), 3, &mut runtime, &mut health)
        .unwrap_or_else(|error| panic!("stage: {error}"));

    let plan = match guardian.state().phase.clone() {
        GuardianPhase::Staged { plan } => plan,
        phase => panic!("expected staged, got {phase:?}"),
    };
    // Persist the same intent that precedes activation, without performing the
    // fake side effect, then reopen as though the process exited.
    let mut raw_store = JournalStore::new(&journal);
    let mut state = guardian.state().clone();
    state.sequence += 1;
    state.phase = GuardianPhase::Activating { plan };
    raw_store
        .append(&state)
        .unwrap_or_else(|error| panic!("append intent: {error}"));

    let mut reopened = UpdateGuardian::open(
        config(),
        JournalStore::new(journal),
        Version::new(1, 0, 0),
        4,
    )
    .unwrap_or_else(|error| panic!("reopen: {error}"));
    runtime.observation = Some(ActivationObservation::Activated);
    reopened
        .advance(idle(), 5, &mut runtime, &mut health)
        .unwrap_or_else(|error| panic!("reconcile: {error}"));
    assert!(matches!(
        reopened.state().phase,
        GuardianPhase::HealthChecking { .. }
    ));
    assert!(runtime.calls.contains(&"observe_activation"));
}

#[test]
fn unhealthy_runtime_rolls_back_and_preserves_installed_version() {
    let directory = tempdir().unwrap_or_else(|error| panic!("tempdir: {error}"));
    let journal = directory.path().join("guardian.jsonl");
    let artifact = directory.path().join("piqae.zip");
    let mut guardian = UpdateGuardian::open(
        config(),
        JournalStore::new(journal),
        Version::new(1, 0, 0),
        1,
    )
    .unwrap_or_else(|error| panic!("open: {error}"));
    guardian
        .request(
            UpdateCommand {
                command_id: "command-4".into(),
                requested_at_unix_ms: 1,
                candidate: candidate(&artifact),
            },
            idle(),
            2,
        )
        .unwrap_or_else(|error| panic!("request: {error}"));
    let mut runtime = FakeRuntime::default();
    let mut health = FakeHealth(vec![HealthObservation::Unhealthy("IPC unavailable".into())]);
    guardian
        .advance(idle(), 3, &mut runtime, &mut health)
        .unwrap_or_default();
    guardian
        .advance(idle(), 4, &mut runtime, &mut health)
        .unwrap_or_default();
    guardian
        .advance(idle(), 5, &mut runtime, &mut health)
        .unwrap_or_default();
    assert!(matches!(
        guardian.state().phase,
        GuardianPhase::RollingBack { .. }
    ));
    guardian
        .advance(idle(), 6, &mut runtime, &mut health)
        .unwrap_or_default();
    assert!(matches!(
        guardian.state().phase,
        GuardianPhase::RolledBack { .. }
    ));
    assert_eq!(guardian.state().installed_release, Version::new(1, 0, 0));
    assert!(runtime.calls.contains(&"restore"));
}

#[test]
fn journal_repairs_torn_final_record_before_next_durable_transition() {
    let directory = tempdir().unwrap_or_else(|error| panic!("tempdir: {error}"));
    let journal = directory.path().join("guardian.jsonl");
    let mut guardian = UpdateGuardian::open(
        config(),
        JournalStore::new(&journal),
        Version::new(1, 0, 0),
        1,
    )
    .unwrap_or_else(|error| panic!("open: {error}"));
    let artifact = directory.path().join("piqae.zip");
    guardian
        .request(
            UpdateCommand {
                command_id: "durable".into(),
                requested_at_unix_ms: 1,
                candidate: candidate(&artifact),
            },
            RuntimeActivity::default(),
            2,
        )
        .unwrap_or_else(|error| panic!("request: {error}"));
    OpenOptions::new()
        .append(true)
        .open(&journal)
        .and_then(|mut file| file.write_all(br#"{"payload":"torn"#))
        .unwrap_or_else(|error| panic!("append torn record: {error}"));
    let recovered = JournalStore::new(&journal)
        .load()
        .unwrap_or_else(|error| panic!("recover: {error}"));
    assert_eq!(
        recovered
            .and_then(|state| state.command)
            .map(|command| command.command_id),
        Some("durable".into())
    );

    let mut recovered_guardian = UpdateGuardian::open(
        config(),
        JournalStore::new(&journal),
        Version::new(1, 0, 0),
        3,
    )
    .unwrap_or_else(|error| panic!("reopen: {error}"));
    let mut runtime = FakeRuntime::default();
    let mut health = FakeHealth(vec![]);
    recovered_guardian
        .advance(idle(), 4, &mut runtime, &mut health)
        .unwrap_or_else(|error| panic!("advance after torn record: {error}"));
    assert!(JournalStore::new(&journal).load().is_ok());
}

#[test]
fn journal_rejects_completed_corruption() {
    let directory = tempdir().unwrap_or_else(|error| panic!("tempdir: {error}"));
    let journal = directory.path().join("guardian.jsonl");
    let mut guardian = UpdateGuardian::open(
        config(),
        JournalStore::new(&journal),
        Version::new(1, 0, 0),
        1,
    )
    .unwrap_or_else(|error| panic!("open: {error}"));
    let artifact = directory.path().join("piqae.zip");
    guardian
        .request(
            UpdateCommand {
                command_id: "durable".into(),
                requested_at_unix_ms: 1,
                candidate: candidate(&artifact),
            },
            RuntimeActivity::default(),
            2,
        )
        .unwrap_or_else(|error| panic!("request: {error}"));
    OpenOptions::new()
        .append(true)
        .open(&journal)
        .and_then(|mut file| file.write_all(b"{\"payload\":\"bad\",\"sha256\":\"wrong\"}\n"))
        .unwrap_or_default();
    assert!(JournalStore::new(journal).load().is_err());
}

#[test]
fn activity_admission_is_fail_closed() {
    assert_eq!(
        RuntimeActivity::default().admission(),
        Admission::Blocked(AdmissionBlock::NotPaused)
    );
    assert_eq!(idle().admission(), Admission::Ready);
}

#[test]
fn health_attempt_limit_enters_rollback_without_sleeping() {
    let directory = tempdir().unwrap_or_else(|error| panic!("tempdir: {error}"));
    let journal = directory.path().join("guardian.jsonl");
    let artifact = directory.path().join("piqae.zip");
    let mut guardian = UpdateGuardian::open(
        config(),
        JournalStore::new(journal),
        Version::new(1, 0, 0),
        1,
    )
    .unwrap_or_else(|error| panic!("open: {error}"));
    guardian
        .request(
            UpdateCommand {
                command_id: "bounded-health".into(),
                requested_at_unix_ms: 1,
                candidate: candidate(&artifact),
            },
            idle(),
            2,
        )
        .unwrap_or_default();
    let mut runtime = FakeRuntime::default();
    let mut health = FakeHealth(vec![]);
    guardian
        .advance(idle(), 3, &mut runtime, &mut health)
        .unwrap_or_default();
    guardian
        .advance(idle(), 4, &mut runtime, &mut health)
        .unwrap_or_default();
    guardian
        .advance(idle(), 5, &mut runtime, &mut health)
        .unwrap_or_default();
    guardian
        .advance(idle(), 6, &mut runtime, &mut health)
        .unwrap_or_default();
    guardian
        .advance(idle(), 7, &mut runtime, &mut health)
        .unwrap_or_default();
    assert!(matches!(
        guardian.state().phase,
        GuardianPhase::RollingBack { .. }
    ));
}

#[test]
fn failed_completion_persistence_does_not_advance_in_memory_trust() {
    let directory = tempdir().unwrap_or_else(|error| panic!("tempdir: {error}"));
    let artifact = directory.path().join("piqae.zip");
    let mut guardian = UpdateGuardian::open(
        config(),
        FailCompletionStore::default(),
        Version::new(1, 0, 0),
        1,
    )
    .unwrap_or_else(|error| panic!("open: {error}"));
    guardian
        .request(
            UpdateCommand {
                command_id: "completion-persistence".into(),
                requested_at_unix_ms: 1,
                candidate: candidate(&artifact),
            },
            idle(),
            2,
        )
        .unwrap_or_default();
    let mut runtime = FakeRuntime::default();
    let mut health = FakeHealth(vec![HealthObservation::Healthy]);
    guardian
        .advance(idle(), 3, &mut runtime, &mut health)
        .unwrap_or_default();
    guardian
        .advance(idle(), 4, &mut runtime, &mut health)
        .unwrap_or_default();
    assert!(
        guardian
            .advance(idle(), 5, &mut runtime, &mut health)
            .is_err()
    );
    assert_eq!(guardian.state().installed_release, Version::new(1, 0, 0));
    assert_eq!(guardian.state().trusted_metadata_version, 0);
    assert!(matches!(
        guardian.state().phase,
        GuardianPhase::HealthChecking { .. }
    ));
}
