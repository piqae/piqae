use crate::{AgentId, EnvironmentId, EventId, JobId, PrinterId, WorkspaceId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
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
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Duplex {
    OneSided,
    LongEdge,
    ShortEdge,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum Rotation {
    #[serde(rename = "0")]
    Deg0,
    #[serde(rename = "90")]
    Deg90,
    #[serde(rename = "180")]
    Deg180,
    #[serde(rename = "270")]
    Deg270,
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
