use spool_executor_protocol::{read_frame, write_frame};
use spool_protocol::executor::ExecutorError;
use spool_protocol::executor::{ExecutorRequest, ExecutorResponse};

fn main() {
    if let Err(error) = run() {
        eprintln!("Windows executor failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let request: ExecutorRequest = read_frame(std::io::stdin().lock())?;
    let result = platform::execute(request.operation);
    write_frame(
        std::io::stdout().lock(),
        &ExecutorResponse {
            request_id: request.request_id,
            result,
        },
    )?;
    Ok(())
}

#[cfg(any(windows, test))]
fn sumatra_settings(options: &spool_domain::JobOptions) -> Result<Vec<String>, ExecutorError> {
    use spool_domain::{Duplex, Rotation};

    let mut unsupported = Vec::new();
    if options.dpi.is_some() {
        unsupported.push("dpi");
    }
    if options.media.is_some() {
        unsupported.push("media");
    }
    if options.nup.is_some() {
        unsupported.push("nup");
    }
    if !unsupported.is_empty() {
        return Err(ExecutorError {
            code: "windows_pdf_option_unsupported".into(),
            message: format!(
                "SumatraPDF does not support requested option(s): {}",
                unsupported.join(", ")
            ),
            retryable: false,
            handoff_may_have_succeeded: false,
        });
    }

    let mut settings = Vec::new();
    if let Some(pages) = &options.pages {
        settings.push(pages.clone());
    }
    if let Some(copies) = options.copies {
        settings.push(format!("{copies}x"));
    }
    if let Some(color) = options.color {
        settings.push(if color { "color" } else { "monochrome" }.into());
    }
    if let Some(collate) = options.collate {
        settings.push(if collate { "collate" } else { "nocollate" }.into());
    }
    if let Some(duplex) = options.duplex {
        settings.push(
            match duplex {
                Duplex::OneSided => "simplex",
                Duplex::LongEdge => "duplexlong",
                Duplex::ShortEdge => "duplexshort",
            }
            .into(),
        );
    }
    if let Some(bin) = &options.bin {
        settings.push(format!("bin={bin}"));
    }
    if let Some(paper) = &options.paper {
        settings.push(format!("paper={paper}"));
    }
    if let Some(fit_to_page) = options.fit_to_page {
        settings.push(if fit_to_page { "fit" } else { "noscale" }.into());
    }
    if let Some(rotation) = options.rotate {
        match rotation {
            Rotation::Deg0 => {}
            Rotation::Deg90 => settings.push("rotate=90".into()),
            Rotation::Deg180 => settings.push("rotate=180".into()),
            Rotation::Deg270 => settings.push("rotate=270".into()),
        }
    }
    Ok(settings)
}

#[cfg(windows)]
mod platform {
    use spool_domain::{ContentKind, PrinterCapabilities, PrinterState};
    use spool_protocol::executor::{
        DiscoveredPrinter, ExecutorError, ExecutorOperation, ExecutorResult, NativeJobObservation,
        NativeJobState,
    };
    use std::{ffi::c_void, ptr};
    use windows_sys::Win32::{
        Foundation::{
            ERROR_FILE_NOT_FOUND, ERROR_INSUFFICIENT_BUFFER, ERROR_INVALID_PARAMETER, GetLastError,
            HANDLE,
        },
        Graphics::Printing::{
            ClosePrinter, DOC_INFO_1W, EndDocPrinter, EndPagePrinter, EnumPrintersW, GetJobW,
            JOB_CONTROL_CANCEL, JOB_INFO_1W, JOB_STATUS_BLOCKED_DEVQ, JOB_STATUS_COMPLETE,
            JOB_STATUS_DELETED, JOB_STATUS_DELETING, JOB_STATUS_ERROR, JOB_STATUS_OFFLINE,
            JOB_STATUS_PAPEROUT, JOB_STATUS_PAUSED, JOB_STATUS_PRINTED, JOB_STATUS_PRINTING,
            JOB_STATUS_RENDERING_LOCALLY, JOB_STATUS_SPOOLING, JOB_STATUS_USER_INTERVENTION,
            OpenPrinterW, PRINTER_ENUM_CONNECTIONS, PRINTER_ENUM_LOCAL, PRINTER_INFO_4W, SetJobW,
            StartDocPrinterW, StartPagePrinter, WritePrinter,
        },
    };

    pub fn execute(operation: ExecutorOperation) -> Result<ExecutorResult, ExecutorError> {
        match operation {
            ExecutorOperation::DiscoverPrinters => discover(),
            ExecutorOperation::GetPrinterState { native_printer_id } => {
                ensure_printer(&native_printer_id)?;
                Ok(ExecutorResult::State {
                    state: PrinterState::Unknown,
                })
            }
            ExecutorOperation::GetPrinterCapabilities { native_printer_id } => {
                ensure_printer(&native_printer_id)?;
                Ok(ExecutorResult::Capabilities {
                    capabilities: PrinterCapabilities::default(),
                    native_options: std::collections::BTreeMap::new(),
                })
            }
            ExecutorOperation::ListJobs { native_printer_id } => {
                ensure_printer(&native_printer_id)?;
                Ok(ExecutorResult::Jobs { jobs: Vec::new() })
            }
            ExecutorOperation::Submit {
                native_printer_id,
                title,
                content_kind: ContentKind::Raw,
                content_path,
                native_profile,
                ..
            } => {
                if native_profile.is_some() {
                    Err(native_profile_backend_unavailable(
                        "RAW jobs cannot replay a Windows driver profile",
                    ))
                } else {
                    submit_raw(&native_printer_id, &title, &content_path)
                }
            }
            ExecutorOperation::Submit {
                job_id,
                native_printer_id,
                title: _,
                content_kind: ContentKind::Pdf,
                content_path,
                options,
                native_profile,
            } => {
                if native_profile.is_some() {
                    Err(native_profile_backend_unavailable(
                        "Windows native profile replay requires the PDFium/GDI backend; the Sumatra fallback cannot apply DEVMODE profiles",
                    ))
                } else {
                    submit_pdf_helper(job_id, &native_printer_id, &content_path, &options)
                }
            }
            ExecutorOperation::Observe {
                native_printer_id,
                native_job_id,
            } => observe(&native_printer_id, &native_job_id),
            ExecutorOperation::Cancel {
                native_printer_id,
                native_job_id,
            } => cancel(&native_printer_id, &native_job_id),
        }
    }

    fn native_profile_backend_unavailable(message: &str) -> ExecutorError {
        ExecutorError {
            code: "native_profile_backend_unavailable".into(),
            message: message.into(),
            retryable: false,
            handoff_may_have_succeeded: false,
        }
    }

    fn observe(printer: &str, native_job_id: &str) -> Result<ExecutorResult, ExecutorError> {
        if native_job_id.starts_with("sumatra-") {
            return Ok(ExecutorResult::Observation {
                observation: NativeJobObservation {
                    state: NativeJobState::Unknown,
                    native_code: Some("sumatra-job-id-unavailable".into()),
                    message: Some(
                        "The external PDF helper did not expose a Winspool job identifier".into(),
                    ),
                },
            });
        }
        let job_id = native_job_id.parse::<u32>().map_err(|_| ExecutorError {
            code: "invalid_native_job_id".into(),
            message: "Winspool job ID must be an integer".into(),
            retryable: false,
            handoff_may_have_succeeded: false,
        })?;
        let printer = wide(printer);
        let mut handle: HANDLE = ptr::null_mut();
        // SAFETY: The printer handle is checked and closed exactly once. The
        // first GetJobW call obtains the required size and the second receives
        // an allocation of that exact size; JOB_INFO_1W is copied before the
        // buffer is dropped.
        unsafe {
            if OpenPrinterW(printer.as_ptr(), &mut handle, ptr::null()) == 0 {
                return Err(win_error("winspool_open_failed", false));
            }
            let mut needed = 0_u32;
            GetJobW(handle, job_id, 1, ptr::null_mut(), 0, &mut needed);
            let first_error = GetLastError();
            if first_error == ERROR_FILE_NOT_FOUND || first_error == ERROR_INVALID_PARAMETER {
                ClosePrinter(handle);
                return Ok(ExecutorResult::Observation {
                    observation: missing_observation(first_error),
                });
            }
            if first_error != ERROR_INSUFFICIENT_BUFFER || needed == 0 {
                ClosePrinter(handle);
                return Err(win_error_code(
                    "winspool_observation_failed",
                    first_error,
                    false,
                ));
            }
            let mut buffer = vec![0_u8; usize::try_from(needed).unwrap_or(0)];
            if GetJobW(handle, job_id, 1, buffer.as_mut_ptr(), needed, &mut needed) == 0 {
                let error = GetLastError();
                ClosePrinter(handle);
                if error == ERROR_FILE_NOT_FOUND || error == ERROR_INVALID_PARAMETER {
                    return Ok(ExecutorResult::Observation {
                        observation: missing_observation(error),
                    });
                }
                return Err(win_error_code("winspool_observation_failed", error, false));
            }
            let status = buffer.as_ptr().cast::<JOB_INFO_1W>().read().Status;
            ClosePrinter(handle);
            Ok(ExecutorResult::Observation {
                observation: winspool_observation(status),
            })
        }
    }

    fn winspool_observation(status: u32) -> NativeJobObservation {
        let state = if status & (JOB_STATUS_COMPLETE | JOB_STATUS_PRINTED) != 0 {
            NativeJobState::Completed
        } else if status & JOB_STATUS_DELETED != 0 {
            NativeJobState::Cancelled
        } else if status & JOB_STATUS_ERROR != 0 {
            NativeJobState::Failed
        } else if status
            & (JOB_STATUS_PAUSED
                | JOB_STATUS_OFFLINE
                | JOB_STATUS_PAPEROUT
                | JOB_STATUS_BLOCKED_DEVQ
                | JOB_STATUS_USER_INTERVENTION)
            != 0
        {
            NativeJobState::Blocked
        } else if status & JOB_STATUS_PRINTING != 0 {
            NativeJobState::Printing
        } else if status
            & (JOB_STATUS_SPOOLING | JOB_STATUS_RENDERING_LOCALLY | JOB_STATUS_DELETING)
            != 0
        {
            NativeJobState::Queued
        } else {
            NativeJobState::Unknown
        };
        NativeJobObservation {
            state,
            native_code: Some(format!("winspool-status-0x{status:08x}")),
            message: Some("Winspool reported job status flags".into()),
        }
    }

    fn missing_observation(error: u32) -> NativeJobObservation {
        NativeJobObservation {
            state: NativeJobState::Missing,
            native_code: Some(format!("win32-error-{error}")),
            message: Some("Winspool no longer exposes this job".into()),
        }
    }

    fn discover() -> Result<ExecutorResult, ExecutorError> {
        // SAFETY: EnumPrintersW is first called for its required size; the
        // second call receives an exactly sized writable buffer. The returned
        // PRINTER_INFO_4W records and strings remain inside that buffer while
        // copied into Rust-owned strings.
        unsafe {
            let flags = PRINTER_ENUM_LOCAL | PRINTER_ENUM_CONNECTIONS;
            let mut needed = 0_u32;
            let mut returned = 0_u32;
            EnumPrintersW(
                flags,
                ptr::null(),
                4,
                ptr::null_mut(),
                0,
                &mut needed,
                &mut returned,
            );
            if needed == 0 && GetLastError() != ERROR_INSUFFICIENT_BUFFER {
                return Err(win_error("winspool_discovery_failed", false));
            }
            let mut buffer = vec![0_u8; usize::try_from(needed).unwrap_or(0)];
            if EnumPrintersW(
                flags,
                ptr::null(),
                4,
                buffer.as_mut_ptr(),
                needed,
                &mut needed,
                &mut returned,
            ) == 0
            {
                return Err(win_error("winspool_discovery_failed", false));
            }
            let records = std::slice::from_raw_parts(
                buffer.as_ptr().cast::<PRINTER_INFO_4W>(),
                usize::try_from(returned).unwrap_or(0),
            );
            let printers = records
                .iter()
                .filter_map(|record| wide_pointer(record.pPrinterName, &buffer))
                .map(|name| DiscoveredPrinter {
                    native_id: name.clone(),
                    name,
                    is_default: false,
                    state: PrinterState::Unknown,
                    capabilities: PrinterCapabilities::default(),
                    native_options: std::collections::BTreeMap::new(),
                })
                .collect();
            Ok(ExecutorResult::Printers { printers })
        }
    }

    fn ensure_printer(native_id: &str) -> Result<(), ExecutorError> {
        let printers = match discover()? {
            ExecutorResult::Printers { printers } => printers,
            _ => Vec::new(),
        };
        printers
            .iter()
            .any(|printer| printer.native_id == native_id)
            .then_some(())
            .ok_or_else(|| ExecutorError {
                code: "printer_not_found".into(),
                message: format!("Windows printer {native_id} was not found"),
                retryable: false,
                handoff_may_have_succeeded: false,
            })
    }

    fn submit_raw(
        printer: &str,
        title: &str,
        content_path: &str,
    ) -> Result<ExecutorResult, ExecutorError> {
        let content = std::fs::read(content_path).map_err(|error| ExecutorError {
            code: "content_unavailable".into(),
            message: error.to_string(),
            retryable: false,
            handoff_may_have_succeeded: false,
        })?;
        let printer = wide(printer);
        let title = wide(title);
        let raw = wide("RAW");
        let mut handle: HANDLE = ptr::null_mut();
        // SAFETY: All passed buffers live through the synchronous Winspool
        // calls. Every successful OpenPrinterW is paired with ClosePrinter;
        // the content pointer is valid for the exact byte length supplied.
        unsafe {
            if OpenPrinterW(printer.as_ptr(), &mut handle, ptr::null()) == 0 {
                return Err(win_error("winspool_open_failed", false));
            }
            let document = DOC_INFO_1W {
                pDocName: title.as_ptr().cast_mut(),
                pOutputFile: ptr::null_mut(),
                pDatatype: raw.as_ptr().cast_mut(),
            };
            let job_id = StartDocPrinterW(handle, 1, &document);
            if job_id == 0 {
                ClosePrinter(handle);
                return Err(win_error("winspool_start_failed", false));
            }
            if StartPagePrinter(handle) == 0 {
                EndDocPrinter(handle);
                ClosePrinter(handle);
                return Err(win_error("winspool_page_failed", true));
            }
            let mut written = 0_u32;
            let length = u32::try_from(content.len()).map_err(|_| ExecutorError {
                code: "content_too_large".into(),
                message: "RAW content exceeds the Winspool call limit".into(),
                retryable: false,
                handoff_may_have_succeeded: true,
            })?;
            let wrote = WritePrinter(
                handle,
                content.as_ptr().cast::<c_void>(),
                length,
                &mut written,
            );
            EndPagePrinter(handle);
            EndDocPrinter(handle);
            ClosePrinter(handle);
            if wrote == 0 || written != length {
                return Err(win_error("winspool_write_failed", true));
            }
            Ok(ExecutorResult::Submitted {
                native_job_id: Some(job_id.to_string()),
            })
        }
    }

    fn submit_pdf_helper(
        job_id: spool_domain::JobId,
        printer: &str,
        content_path: &str,
        options: &spool_domain::JobOptions,
    ) -> Result<ExecutorResult, ExecutorError> {
        let settings = super::sumatra_settings(options)?;
        ensure_printer(printer)?;
        let helper = std::env::var_os("SPOOL_WINDOWS_PDF_HELPER").ok_or_else(|| ExecutorError {
            code: "windows_pdf_helper_unconfigured".into(),
            message: "set SPOOL_WINDOWS_PDF_HELPER to an approved SumatraPDF executable".into(),
            retryable: false,
            handoff_may_have_succeeded: false,
        })?;

        let mut command = std::process::Command::new(helper);
        command.arg("-print-to").arg(printer);
        if !settings.is_empty() {
            command.arg("-print-settings").arg(settings.join(","));
        }
        let status = command
            .arg(content_path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map_err(|error| ExecutorError {
                code: "windows_pdf_helper_start_failed".into(),
                message: error.to_string(),
                retryable: false,
                handoff_may_have_succeeded: false,
            })?;
        if !status.success() {
            return Err(ExecutorError {
                code: "windows_pdf_helper_failed".into(),
                message: format!(
                    "PDF helper exited with code {}",
                    status.code().unwrap_or(-1)
                ),
                retryable: false,
                handoff_may_have_succeeded: false,
            });
        }
        Ok(ExecutorResult::Submitted {
            // Sumatra does not expose the Winspool ID. This marker remains
            // explicitly backend-scoped and must not be queried through
            // SetJobW as though it were a native integer.
            native_job_id: Some(format!("sumatra-{job_id}")),
        })
    }

    fn cancel(printer: &str, native_job_id: &str) -> Result<ExecutorResult, ExecutorError> {
        let job_id = native_job_id.parse::<u32>().map_err(|_| ExecutorError {
            code: "invalid_native_job_id".into(),
            message: "Winspool job ID must be an integer".into(),
            retryable: false,
            handoff_may_have_succeeded: false,
        })?;
        let printer = wide(printer);
        let mut handle: HANDLE = ptr::null_mut();
        // SAFETY: The printer name is a valid NUL-terminated UTF-16 buffer and
        // the acquired handle is closed exactly once.
        unsafe {
            if OpenPrinterW(printer.as_ptr(), &mut handle, ptr::null()) == 0 {
                return Err(win_error("winspool_open_failed", false));
            }
            let result = SetJobW(handle, job_id, 0, ptr::null_mut(), JOB_CONTROL_CANCEL);
            ClosePrinter(handle);
            if result == 0 {
                return Err(win_error("winspool_cancel_failed", false));
            }
            Ok(ExecutorResult::Cancelled)
        }
    }

    fn wide_pointer(pointer: *mut u16, buffer: &[u8]) -> Option<String> {
        if pointer.is_null() {
            return None;
        }
        let start = buffer.as_ptr() as usize;
        let end = start.checked_add(buffer.len())?;
        let address = pointer as usize;
        if address < start || address >= end || !address.is_multiple_of(std::mem::size_of::<u16>())
        {
            return None;
        }
        let maximum_units = (end - address) / std::mem::size_of::<u16>();
        // SAFETY: The pointer was returned for this exact EnumPrintersW buffer,
        // is checked for alignment and containment above, and the slice is
        // bounded by the remaining bytes in that live buffer.
        let units = unsafe { std::slice::from_raw_parts(pointer, maximum_units) };
        let length = units.iter().position(|unit| *unit == 0)?;
        Some(String::from_utf16_lossy(&units[..length]))
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn win_error(code: &str, handoff: bool) -> ExecutorError {
        // SAFETY: GetLastError has no preconditions.
        let native = unsafe { GetLastError() };
        win_error_code(code, native, handoff)
    }

    fn win_error_code(code: &str, native: u32, handoff: bool) -> ExecutorError {
        ExecutorError {
            code: code.into(),
            message: format!("Win32 error {native}"),
            retryable: false,
            handoff_may_have_succeeded: handoff,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spool_domain::{Duplex, JobOptions, Rotation};

    #[test]
    fn documented_sumatra_options_are_mapped() {
        let options = JobOptions {
            bin: Some("Tray 1".into()),
            collate: Some(true),
            color: Some(false),
            copies: Some(2),
            duplex: Some(Duplex::LongEdge),
            fit_to_page: Some(true),
            pages: Some("1,3-5".into()),
            paper: Some("A4".into()),
            rotate: Some(Rotation::Deg180),
            ..Default::default()
        };
        assert_eq!(
            sumatra_settings(&options).expect("settings"),
            [
                "1,3-5",
                "2x",
                "monochrome",
                "collate",
                "duplexlong",
                "bin=Tray 1",
                "paper=A4",
                "fit",
                "rotate=180",
            ]
        );
    }

    #[test]
    fn unsupported_sumatra_options_fail_before_handoff() {
        let options = JobOptions {
            dpi: Some("300x300".into()),
            media: Some("Labels".into()),
            nup: Some(2),
            ..Default::default()
        };
        let error = sumatra_settings(&options).expect_err("unsupported");
        assert_eq!(error.code, "windows_pdf_option_unsupported");
        assert!(error.message.contains("dpi, media, nup"));
        assert!(!error.handoff_may_have_succeeded);
    }
}

#[cfg(not(windows))]
mod platform {
    use super::ExecutorError;
    use spool_protocol::executor::{ExecutorOperation, ExecutorResult};

    pub fn execute(_operation: ExecutorOperation) -> Result<ExecutorResult, ExecutorError> {
        Err(ExecutorError {
            code: "winspool_unavailable".into(),
            message: "Winspool executor is available only on Windows".into(),
            retryable: false,
            handoff_may_have_succeeded: false,
        })
    }
}
