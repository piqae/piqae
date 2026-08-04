use crate::{AgentId, EnvironmentId, EventId, JobId, PrinterId, WorkspaceId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

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

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Duplex {
    OneSided,
    LongEdge,
    ShortEdge,
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
