use crate::{AgentId, EnvironmentId, EventId, JobId, PrinterId, WorkspaceId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

pub const MAX_JOB_COPIES: u32 = 32_767;
pub const MAX_JOB_NUP: u16 = 64;
pub const MAX_JOB_OPTION_TEXT_BYTES: usize = 4_096;
pub const MAX_JOB_NATIVE_OPTIONS: usize = 256;
pub const MAX_JOB_NATIVE_OPTION_NAME_BYTES: usize = 255;

/// Exact encrypted-job envelope identifiers defined by the public `OpenAPI` contract.
pub const ENCRYPTED_JOB_V3_VERSION: &str = "piqae-encrypted-job-v3";
pub const ENCRYPTED_JOB_V3_SUITE: &str = "ECDH-ES-P256+HKDF-SHA256+A256GCMKW+A256GCM";
pub const ENCRYPTED_JOB_V3_RECIPIENT_ALGORITHM: &str = "ECDH-ES-P256+HKDF-SHA256+A256GCMKW";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentKind {
    Pdf,
    Raw,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentSource {
    Upload {
        upload_id: String,
    },
    Base64 {
        data: String,
    },
    Uri {
        uri: String,
        authentication: Option<UriAuthentication>,
    },
    EncryptedUpload {
        upload_id: String,
        manifest: Box<EncryptedContentManifest>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EncryptedContentManifest {
    pub version: String,
    pub suite: String,
    pub binding: EncryptedContentBinding,
    pub ciphertext_sha256: String,
    pub iv: String,
    pub recipients: Vec<EncryptedContentRecipient>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EncryptedContentBinding {
    pub envelope_id: String,
    pub workspace_id: String,
    pub environment_id: String,
    pub content_type: ContentKind,
    pub printer_id: String,
    pub target_id: String,
    pub profile_revision: String,
    pub options: JobOptions,
    pub deliveries: u16,
    pub expires_at: String,
    pub raw_authorized: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EncryptedContentRecipient {
    pub key_id: String,
    pub algorithm: String,
    pub ephemeral_public_key: String,
    pub hkdf_salt: String,
    pub key_wrap_iv: String,
    pub encrypted_content_key: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UriAuthentication {
    Basic { username: String, password: String },
    Digest { username: String, password: String },
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct JobOptions {
    pub bin: Option<String>,
    pub collate: Option<bool>,
    pub color: Option<bool>,
    pub copies: Option<u32>,
    pub dpi: Option<String>,
    pub duplex: Option<Duplex>,
    pub fit_to_page: Option<bool>,
    pub media: Option<String>,
    pub nup: Option<u16>,
    pub pages: Option<String>,
    pub paper: Option<String>,
    pub rotate: Option<Rotation>,
    /// Driver-specific selections validated against the current native
    /// capability snapshot before local submission.
    pub native_options: BTreeMap<String, String>,
}

impl JobOptions {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bin.is_none()
            && self.collate.is_none()
            && self.color.is_none()
            && self.copies.is_none()
            && self.dpi.is_none()
            && self.duplex.is_none()
            && self.fit_to_page.is_none()
            && self.media.is_none()
            && self.nup.is_none()
            && self.pages.is_none()
            && self.paper.is_none()
            && self.rotate.is_none()
            && self.native_options.is_empty()
    }

    /// Validates transport-safe bounds that every executor must enforce before
    /// consulting a driver. This deliberately does not infer support: live
    /// capability/profile validation remains a separate, fail-closed step.
    ///
    /// # Errors
    ///
    /// Returns a typed error for an out-of-range numeric value or unsafe,
    /// empty, oversized option text.
    pub fn validate_bounds(&self) -> Result<(), JobOptionsError> {
        if self
            .copies
            .is_some_and(|value| value == 0 || value > MAX_JOB_COPIES)
        {
            return Err(JobOptionsError::CopiesOutOfRange);
        }
        if self
            .nup
            .is_some_and(|value| value == 0 || value > MAX_JOB_NUP)
        {
            return Err(JobOptionsError::NupOutOfRange);
        }
        for (name, value) in [
            ("bin", self.bin.as_deref()),
            ("dpi", self.dpi.as_deref()),
            ("media", self.media.as_deref()),
            ("pages", self.pages.as_deref()),
            ("paper", self.paper.as_deref()),
        ] {
            if value.is_some_and(invalid_option_text) {
                return Err(JobOptionsError::InvalidText(name));
            }
        }
        if self.native_options.len() > MAX_JOB_NATIVE_OPTIONS {
            return Err(JobOptionsError::TooManyNativeOptions);
        }
        for (name, value) in &self.native_options {
            if name.is_empty()
                || name.len() > MAX_JOB_NATIVE_OPTION_NAME_BYTES
                || name.bytes().any(|byte| byte.is_ascii_control())
                || invalid_option_text(value)
            {
                return Err(JobOptionsError::InvalidNativeOption(name.clone()));
            }
        }
        Ok(())
    }
}

fn invalid_option_text(value: &str) -> bool {
    value.is_empty()
        || value.len() > MAX_JOB_OPTION_TEXT_BYTES
        || value
            .bytes()
            .any(|byte| byte == 0 || byte.is_ascii_control())
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum JobOptionsError {
    #[error("copies must be between 1 and {MAX_JOB_COPIES}")]
    CopiesOutOfRange,
    #[error("n-up must be between 1 and {MAX_JOB_NUP}")]
    NupOutOfRange,
    #[error("{0} contains empty, oversized, or control-character data")]
    InvalidText(&'static str),
    #[error("a job may contain at most {MAX_JOB_NATIVE_OPTIONS} native options")]
    TooManyNativeOptions,
    #[error("native option {0:?} has an invalid name or value")]
    InvalidNativeOption(String),
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Duplex {
    OneSided,
    LongEdge,
    ShortEdge,
}

#[cfg(test)]
mod option_tests {
    use super::*;

    #[test]
    fn bounded_options_reject_values_that_must_not_reach_a_driver() {
        assert_eq!(
            JobOptions {
                copies: Some(0),
                ..Default::default()
            }
            .validate_bounds(),
            Err(JobOptionsError::CopiesOutOfRange)
        );
        assert_eq!(
            JobOptions {
                nup: Some(MAX_JOB_NUP + 1),
                ..Default::default()
            }
            .validate_bounds(),
            Err(JobOptionsError::NupOutOfRange)
        );
        let mut options = JobOptions::default();
        options
            .native_options
            .insert("Vendor\nKey".into(), "On".into());
        assert!(matches!(
            options.validate_bounds(),
            Err(JobOptionsError::InvalidNativeOption(_))
        ));
    }

    #[test]
    fn empty_is_explicit_for_raw_job_policy() {
        assert!(JobOptions::default().is_empty());
        assert!(
            !JobOptions {
                fit_to_page: Some(false),
                ..Default::default()
            }
            .is_empty()
        );
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Rotation {
    Deg0,
    Deg90,
    Deg180,
    Deg270,
}

impl Serialize for Rotation {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u16(match self {
            Self::Deg0 => 0,
            Self::Deg90 => 90,
            Self::Deg180 => 180,
            Self::Deg270 => 270,
        })
    }
}

impl<'de> Deserialize<'de> for Rotation {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match u16::deserialize(deserializer)? {
            0 => Ok(Self::Deg0),
            90 => Ok(Self::Deg90),
            180 => Ok(Self::Deg180),
            270 => Ok(Self::Deg270),
            value => Err(serde::de::Error::custom(format!(
                "unsupported rotation {value}"
            ))),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Registered,
    ContentPending,
    WaitingForAgent,
    AgentDownloading,
    AgentAccepted,
    QueuedLocal,
    Preparing,
    Rendering,
    SpoolIntent,
    AcceptedBySpooler,
    Spooling,
    Printing,
    Blocked,
    CompletedReported,
    DeliveryUncertain,
    CancelRequested,
    Cancelled,
    Expired,
    FailedRetryable,
    FailedTerminal,
}

impl JobState {
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::CompletedReported
                | Self::DeliveryUncertain
                | Self::Cancelled
                | Self::Expired
                | Self::FailedTerminal
        )
    }

    #[must_use]
    pub const fn can_transition_to(self, next: Self) -> bool {
        use JobState::{
            AcceptedBySpooler, AgentAccepted, AgentDownloading, Blocked, CancelRequested,
            Cancelled, CompletedReported, ContentPending, DeliveryUncertain, Expired,
            FailedRetryable, FailedTerminal, Preparing, Printing, QueuedLocal, Registered,
            Rendering, SpoolIntent, Spooling, WaitingForAgent,
        };

        matches!(
            (self, next),
            (
                Registered,
                ContentPending | WaitingForAgent | CancelRequested | Expired
            ) | (
                ContentPending,
                WaitingForAgent | CancelRequested | Expired | FailedTerminal
            ) | (
                WaitingForAgent,
                AgentDownloading | CancelRequested | Expired | FailedRetryable
            ) | (
                AgentDownloading,
                AgentAccepted | WaitingForAgent | CancelRequested | Expired | FailedRetryable
            ) | (
                AgentAccepted,
                QueuedLocal | CancelRequested | FailedRetryable
            ) | (
                QueuedLocal,
                Preparing | CancelRequested | Blocked | FailedRetryable
            ) | (
                Preparing,
                Rendering | SpoolIntent | CancelRequested | FailedRetryable | FailedTerminal
            ) | (
                Rendering,
                SpoolIntent | CancelRequested | FailedRetryable | FailedTerminal
            ) | (
                SpoolIntent,
                AcceptedBySpooler | DeliveryUncertain | FailedRetryable | FailedTerminal
            ) | (
                AcceptedBySpooler,
                Spooling
                    | Printing
                    | CompletedReported
                    | Blocked
                    | DeliveryUncertain
                    | Cancelled
                    | FailedTerminal
            ) | (
                Spooling,
                Printing
                    | CompletedReported
                    | Blocked
                    | DeliveryUncertain
                    | Cancelled
                    | FailedTerminal
            ) | (
                Printing,
                CompletedReported | Blocked | DeliveryUncertain | Cancelled | FailedTerminal
            ) | (
                Blocked,
                Spooling
                    | Printing
                    | CompletedReported
                    | CancelRequested
                    | Cancelled
                    | FailedRetryable
                    | FailedTerminal
            ) | (
                CancelRequested,
                Cancelled | DeliveryUncertain | CompletedReported
            ) | (
                FailedRetryable,
                WaitingForAgent | QueuedLocal | CancelRequested | Expired
            )
        )
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobFailureReason {
    AgentUnavailable,
    ContentUnavailable,
    ContentChecksumMismatch,
    DownloadTimedOut,
    InvalidPdf,
    UnsupportedOption,
    PrinterOffline,
    PrinterPaused,
    PaperOut,
    AccessDenied,
    DriverError,
    ExecutorCrashed,
    ExecutorTimedOut,
    AmbiguousHandoff,
    CancelledByUser,
    Expired,
    Internal,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Job {
    pub id: JobId,
    pub workspace_id: WorkspaceId,
    pub environment_id: EnvironmentId,
    pub printer_id: PrinterId,
    pub title: String,
    pub source: Option<String>,
    pub content_kind: ContentKind,
    pub content: ContentSource,
    pub options: JobOptions,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
    pub deliveries: u16,
    pub state: JobState,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    /// When the job entered `DeliveryUncertain`, on the server clock.
    ///
    /// Present only for jobs in that state. Without it a caller can see that
    /// delivery is unproven but not for how long, and the age of the job is a
    /// poor substitute: a job may sit queued for hours before the handoff that
    /// could not be confirmed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_uncertain_since: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct JobEvent {
    pub id: EventId,
    pub job_id: JobId,
    pub sequence: u64,
    pub state: JobState,
    pub reason: Option<JobFailureReason>,
    pub message: Option<String>,
    pub agent_id: Option<AgentId>,
    pub native_job_id: Option<String>,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
#[error("invalid job state transition from {from:?} to {to:?}")]
pub struct StateTransitionError {
    pub from: JobState,
    pub to: JobState,
}

/// Confirms that a state change is permitted by the canonical state machine.
///
/// # Errors
///
/// Returns [`StateTransitionError`] when the transition is not permitted.
pub const fn validate_transition(from: JobState, to: JobState) -> Result<(), StateTransitionError> {
    if from.can_transition_to(to) {
        Ok(())
    } else {
        Err(StateTransitionError { from, to })
    }
}

#[cfg(test)]
mod tests {
    use super::{JobState, validate_transition};

    #[test]
    fn accepted_path_is_valid() {
        let states = [
            JobState::Registered,
            JobState::WaitingForAgent,
            JobState::AgentDownloading,
            JobState::AgentAccepted,
            JobState::QueuedLocal,
            JobState::Preparing,
            JobState::Rendering,
            JobState::SpoolIntent,
            JobState::AcceptedBySpooler,
            JobState::CompletedReported,
        ];

        for pair in states.windows(2) {
            assert!(validate_transition(pair[0], pair[1]).is_ok());
        }
    }

    #[test]
    fn terminal_states_do_not_restart_implicitly() {
        for state in [
            JobState::CompletedReported,
            JobState::DeliveryUncertain,
            JobState::Cancelled,
            JobState::Expired,
            JobState::FailedTerminal,
        ] {
            assert!(state.is_terminal());
            assert!(validate_transition(state, JobState::WaitingForAgent).is_err());
        }
    }
}
