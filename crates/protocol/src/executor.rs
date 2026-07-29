use serde::{Deserialize, Serialize};
use spool_domain::{
    ContentKind, JobId, JobOptions, NativePrinterOption, PrinterCapabilities, PrinterState,
};
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExecutorRequest {
    pub request_id: Uuid,
    pub deadline_unix_ms: i64,
    pub operation: ExecutorOperation,
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
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExecutorError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    pub handoff_may_have_succeeded: bool,
}
