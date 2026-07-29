//! Stable domain types and state transitions shared by every Spool component.

mod id;
mod job;
mod printer;

pub use id::{AgentId, EnvironmentId, EventId, JobId, ParseTypedIdError, PrinterId, WorkspaceId};
pub use job::{
    ContentKind, ContentSource, Duplex, Job, JobEvent, JobFailureReason, JobOptions, JobState,
    Rotation, StateTransitionError, UriAuthentication, validate_transition,
};
pub use printer::{
    NativePrinterChoice, NativePrinterOption, PRINTER_PROFILE_SCHEMA_VERSION, PrinterCapabilities,
    PrinterCapabilityProfile, PrinterProfileError, PrinterState,
};
