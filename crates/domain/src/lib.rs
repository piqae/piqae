//! Stable domain types and state transitions shared by every Spool component.

mod id;
mod job;
mod printer;

pub use id::{AgentId, EnvironmentId, EventId, JobId, PrinterId, WorkspaceId};
pub use job::{
    ContentKind, ContentSource, Job, JobEvent, JobFailureReason, JobOptions, JobState,
    StateTransitionError, validate_transition,
};
pub use printer::{PrinterCapabilities, PrinterState};
