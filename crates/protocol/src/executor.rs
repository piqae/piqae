use serde::{Deserialize, Serialize};
use spool_domain::{ContentKind, JobId, JobOptions, PrinterCapabilities, PrinterState};
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExecutorRequest {
    pub request_id: Uuid,
    pub deadline_unix_ms: i64,
    pub operation: ExecutorOperation,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExecutorOperation {
    DiscoverPrinters,
    GetPrinterState {
        native_printer_id: String,
    },
    GetPrinterCapabilities {
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
    Printers { printers: Vec<DiscoveredPrinter> },
    State { state: PrinterState },
    Capabilities { capabilities: PrinterCapabilities },
    Submitted { native_job_id: Option<String> },
    Cancelled,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DiscoveredPrinter {
    pub native_id: String,
    pub name: String,
    pub is_default: bool,
    pub state: PrinterState,
    pub capabilities: PrinterCapabilities,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExecutorError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    pub handoff_may_have_succeeded: bool,
}
