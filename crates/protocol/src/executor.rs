use piqae_domain::{
    ContentKind, DriverFingerprint, JobId, JobOptions, NativePrinterOption, NativeProfileKind,
    PrinterCapabilities, PrinterState, SafeProfileOverride,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExecutorRequest {
    pub request_id: Uuid,
    pub deadline_unix_ms: i64,
    pub operation: ExecutorOperation,
}

/// An immutable, locally-held native profile revision pinned to one job.
///
/// Native configuration never travels through the control plane. The agent
/// loads the exact revision selected by routing and sends it only to the
/// sandboxed local executor that performs the operating-system handoff.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NativeProfilePayload {
    pub profile_id: String,
    pub revision: u64,
    pub kind: NativeProfileKind,
    pub schema_version: u16,
    pub digest: String,
    pub blob: Vec<u8>,
    #[serde(default)]
    pub safe_overrides: Vec<SafeProfileOverride>,
    pub driver_fingerprint: DriverFingerprint,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(
    clippy::large_enum_variant,
    reason = "the framed executor contract owns one operation and avoids an extra allocation"
)]
pub enum ExecutorOperation {
    DiscoverPrinters,
    GetPrinterState {
        native_printer_id: String,
    },
    GetPrinterCapabilities {
        native_printer_id: String,
    },
    ListJobs {
        native_printer_id: String,
    },
    Submit {
        job_id: JobId,
        native_printer_id: String,
        title: String,
        content_kind: ContentKind,
        content_path: String,
        options: JobOptions,
        /// Exact native profile revision selected before local execution.
        /// `None` preserves direct printer submissions without a profile.
        #[serde(default)]
        native_profile: Option<NativeProfilePayload>,
    },
    Observe {
        native_printer_id: String,
        native_job_id: String,
    },
    Cancel {
        native_printer_id: String,
        native_job_id: String,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExecutorResponse {
    pub request_id: Uuid,
    pub result: Result<ExecutorResult, ExecutorError>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExecutorResult {
    Printers {
        printers: Vec<DiscoveredPrinter>,
    },
    State {
        state: PrinterState,
    },
    Capabilities {
        capabilities: PrinterCapabilities,
        #[serde(default)]
        native_options: BTreeMap<String, NativePrinterOption>,
    },
    Jobs {
        jobs: Vec<NativeQueueJob>,
    },
    Submitted {
        native_job_id: Option<String>,
    },
    Observation {
        observation: NativeJobObservation,
    },
    Cancelled,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NativeJobObservation {
    pub state: NativeJobState,
    pub native_code: Option<String>,
    pub message: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NativeQueueJob {
    pub native_job_id: String,
    pub native_printer_id: String,
    pub title: String,
    pub user: Option<String>,
    pub state: NativeJobState,
    pub native_code: Option<String>,
    pub size_kib: Option<u64>,
    pub created_unix_ms: Option<i64>,
    pub processing_unix_ms: Option<i64>,
    pub completed_unix_ms: Option<i64>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeJobState {
    Queued,
    Printing,
    Blocked,
    Completed,
    Failed,
    Cancelled,
    Missing,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DiscoveredPrinter {
    pub native_id: String,
    pub name: String,
    pub is_default: bool,
    pub state: PrinterState,
    pub capabilities: PrinterCapabilities,
    #[serde(default)]
    pub native_options: BTreeMap<String, NativePrinterOption>,
    /// Exact, locally discovered identity. Absent or incomplete fingerprints
    /// must never activate vendor support-pack mappings.
    #[serde(default)]
    pub driver_fingerprint: Option<DriverFingerprint>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExecutorError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    pub handoff_may_have_succeeded: bool,
}
