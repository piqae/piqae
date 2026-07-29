//! Stable domain types and state transitions shared by every Spool component.

mod id;
mod job;
mod printer;
mod profile;

pub use id::{
    AgentId, EnvironmentId, EventId, JobId, NativeProfileBlobId, ParseTypedIdError,
    PhysicalDeviceId, PrinterId, ProfileCaptureSessionId, ProfileId, StockId, TargetBindingId,
    TargetId, WorkspaceId,
};
pub use job::{
    ContentKind, ContentSource, Duplex, Job, JobEvent, JobFailureReason, JobOptions, JobState,
    Rotation, StateTransitionError, UriAuthentication, validate_transition,
};
pub use printer::{
    NativePrinterChoice, NativePrinterOption, PRINTER_PROFILE_SCHEMA_VERSION, PrinterCapabilities,
    PrinterCapabilityProfile, PrinterProfileError, PrinterState,
};
pub use profile::{
    BindingRole, DriverFingerprint, JobProfilePin, LoadedMedia, LoadedMediaConfidence,
    MediaDimensionsMm, NATIVE_PROFILE_SCHEMA_VERSION, NativeConfigurationRef, NativeProfileKind,
    NativeProfileRevision, PrintTarget, ProfileCaptureOperation, ProfileCaptureSession,
    ProfileCaptureStatus, ProfileDependency, ProfileStatus, ProfileSummary, SafeProfileOverride,
    Stock, StockKind, TargetBinding, TargetReadiness, TargetRoutingPolicy,
};
