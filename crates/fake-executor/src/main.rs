use spool_domain::{PrinterCapabilities, PrinterState};
use spool_executor_protocol::{read_frame, write_frame};
use spool_protocol::executor::{
    DiscoveredPrinter, ExecutorError, ExecutorOperation, ExecutorRequest, ExecutorResponse,
    ExecutorResult, NativeJobObservation, NativeJobState,
};

fn main() {
    if let Err(error) = run() {
        eprintln!("fake executor failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let request: ExecutorRequest = read_frame(std::io::stdin().lock())?;
    let result = execute(request.operation);
    write_frame(
        std::io::stdout().lock(),
        &ExecutorResponse {
            request_id: request.request_id,
            result,
        },
    )?;
    Ok(())
}

fn execute(operation: ExecutorOperation) -> Result<ExecutorResult, ExecutorError> {
    match operation {
        ExecutorOperation::DiscoverPrinters => Ok(ExecutorResult::Printers {
            printers: vec![fake_printer()],
        }),
        ExecutorOperation::GetPrinterState { native_printer_id }
        | ExecutorOperation::GetPrinterCapabilities { native_printer_id }
        | ExecutorOperation::ListJobs { native_printer_id }
            if native_printer_id != "fake-printer" =>
        {
            Err(not_found())
        }
        ExecutorOperation::GetPrinterState { .. } => Ok(ExecutorResult::State {
            state: PrinterState::Online,
        }),
        ExecutorOperation::GetPrinterCapabilities { .. } => Ok(ExecutorResult::Capabilities {
            capabilities: PrinterCapabilities::default(),
            native_options: std::collections::BTreeMap::new(),
        }),
        ExecutorOperation::ListJobs { .. } => Ok(ExecutorResult::Jobs { jobs: Vec::new() }),
        ExecutorOperation::Submit {
            native_printer_id, ..
        } if native_printer_id != "fake-printer" => Err(not_found()),
        ExecutorOperation::Submit { job_id, .. } => Ok(ExecutorResult::Submitted {
            native_job_id: Some(format!("fake-{}", job_id.as_ulid())),
        }),
        ExecutorOperation::Observe {
            native_printer_id, ..
        } if native_printer_id != "fake-printer" => Err(not_found()),
        ExecutorOperation::Observe { native_job_id, .. } => Ok(ExecutorResult::Observation {
            observation: fake_observation(&native_job_id),
        }),
        ExecutorOperation::Cancel {
            native_printer_id, ..
        } if native_printer_id != "fake-printer" => Err(not_found()),
        ExecutorOperation::Cancel { .. } => Ok(ExecutorResult::Cancelled),
    }
}

fn fake_observation(native_job_id: &str) -> NativeJobObservation {
    let state = if native_job_id.ends_with("-queued") {
        NativeJobState::Queued
    } else if native_job_id.ends_with("-printing") {
        NativeJobState::Printing
    } else if native_job_id.ends_with("-blocked") {
        NativeJobState::Blocked
    } else if native_job_id.ends_with("-failed") {
        NativeJobState::Failed
    } else if native_job_id.ends_with("-cancelled") {
        NativeJobState::Cancelled
    } else if native_job_id.ends_with("-missing") {
        NativeJobState::Missing
    } else if native_job_id.ends_with("-unknown") {
        NativeJobState::Unknown
    } else {
        NativeJobState::Completed
    };
    NativeJobObservation {
        state,
        native_code: Some("fake".into()),
        message: Some("Deterministic fake spooler observation".into()),
    }
}

fn fake_printer() -> DiscoveredPrinter {
    DiscoveredPrinter {
        native_id: "fake-printer".into(),
        name: "Spool deterministic fake printer".into(),
        is_default: true,
        state: PrinterState::Online,
        capabilities: PrinterCapabilities::default(),
        native_options: std::collections::BTreeMap::new(),
    }
}

fn not_found() -> ExecutorError {
    ExecutorError {
        code: "printer_not_found".into(),
        message: "fake printer was not found".into(),
        retryable: false,
        handoff_may_have_succeeded: false,
    }
}
