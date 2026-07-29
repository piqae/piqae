use spool_executor_protocol::{read_frame, write_frame};
use spool_protocol::executor::{ExecutorRequest, ExecutorResponse};

fn main() {
    if let Err(error) = run() {
        eprintln!("CUPS executor failed: {error}");
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

#[cfg(unix)]
mod platform {
    use spool_domain::{PrinterCapabilities, PrinterState};
    use spool_protocol::executor::{
        DiscoveredPrinter, ExecutorError, ExecutorOperation, ExecutorResult, NativeJobObservation,
        NativeJobState,
    };
    use std::{
        ffi::{CStr, CString, c_char, c_int, c_long},
        ptr,
    };

    #[repr(C)]
    struct CupsOption {
        name: *mut c_char,
        value: *mut c_char,
    }

    #[repr(C)]
    struct CupsDest {
        name: *mut c_char,
        instance: *mut c_char,
        is_default: c_int,
        num_options: c_int,
        options: *mut CupsOption,
    }

    #[repr(C)]
    struct CupsJob {
        id: c_int,
        dest: *mut c_char,
        title: *mut c_char,
        user: *mut c_char,
        format: *mut c_char,
        state: c_int,
        size: c_int,
        priority: c_int,
        completed_time: c_long,
        creation_time: c_long,
        processing_time: c_long,
    }

    #[link(name = "cups")]
    unsafe extern "C" {
        #[link_name = "cupsGetDests"]
        fn cups_get_dests(destinations: *mut *mut CupsDest) -> c_int;
        #[link_name = "cupsFreeDests"]
        fn cups_free_dests(count: c_int, destinations: *mut CupsDest);
        #[link_name = "cupsPrintFile"]
        fn cups_print_file(
            printer: *const c_char,
            filename: *const c_char,
            title: *const c_char,
            option_count: c_int,
            options: *mut CupsOption,
        ) -> c_int;
        #[link_name = "cupsAddOption"]
        fn cups_add_option(
            name: *const c_char,
            value: *const c_char,
            option_count: c_int,
            options: *mut *mut CupsOption,
        ) -> c_int;
        #[link_name = "cupsFreeOptions"]
        fn cups_free_options(option_count: c_int, options: *mut CupsOption);
        #[link_name = "cupsCancelJob"]
        fn cups_cancel_job(printer: *const c_char, job_id: c_int) -> c_int;
        #[link_name = "cupsGetJobs"]
        fn cups_get_jobs(
            jobs: *mut *mut CupsJob,
            printer: *const c_char,
            my_jobs: c_int,
            which_jobs: c_int,
        ) -> c_int;
        #[link_name = "cupsFreeJobs"]
        fn cups_free_jobs(count: c_int, jobs: *mut CupsJob);
        #[link_name = "cupsLastErrorString"]
        fn cups_last_error_string() -> *const c_char;
    }

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
                })
            }
            ExecutorOperation::Submit {
                native_printer_id,
                title,
                content_kind,
                content_path,
                options,
                ..
            } => submit(
                &native_printer_id,
                &title,
                &content_path,
                content_kind == spool_domain::ContentKind::Raw,
                &options,
            ),
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

    fn observe(printer: &str, native_job_id: &str) -> Result<ExecutorResult, ExecutorError> {
        let job_id = native_job_id.parse::<i32>().map_err(|_| ExecutorError {
            code: "invalid_native_job_id".into(),
            message: "CUPS job ID must be an integer".into(),
            retryable: false,
            handoff_may_have_succeeded: false,
        })?;
        let printer = c_string(printer)?;
        // SAFETY: CUPS owns the returned array. Its count and pointer are
        // validated, records are inspected only while the allocation is live,
        // and cupsFreeJobs releases it exactly once.
        let observation = unsafe {
            let mut jobs: *mut CupsJob = ptr::null_mut();
            let count = cups_get_jobs(&mut jobs, printer.as_ptr(), 0, -1);
            if count < 0 || (count > 0 && jobs.is_null()) {
                return Err(last_error("cups_observation_failed", false));
            }
            let length =
                usize::try_from(count).map_err(|_| last_error("cups_observation_failed", false))?;
            let records = std::slice::from_raw_parts(jobs, length);
            let state = records
                .iter()
                .find(|job| {
                    job.id == job_id
                        && !job.dest.is_null()
                        && CStr::from_ptr(job.dest).to_bytes() == printer.as_bytes()
                })
                .map(|job| job.state);
            cups_free_jobs(count, jobs);
            state.map_or_else(missing_observation, cups_observation)
        };
        Ok(ExecutorResult::Observation { observation })
    }

    fn cups_observation(state: i32) -> NativeJobObservation {
        let mapped = match state {
            3 => NativeJobState::Queued,
            4 | 6 => NativeJobState::Blocked,
            5 => NativeJobState::Printing,
            7 => NativeJobState::Cancelled,
            8 => NativeJobState::Failed,
            9 => NativeJobState::Completed,
            _ => NativeJobState::Unknown,
        };
        NativeJobObservation {
            state: mapped,
            native_code: Some(format!("ipp-job-state-{state}")),
            message: Some("CUPS reported IPP job state".into()),
        }
    }

    fn missing_observation() -> NativeJobObservation {
        NativeJobObservation {
            state: NativeJobState::Missing,
            native_code: Some("cups-job-not-found".into()),
            message: Some("CUPS no longer exposes this job in retained history".into()),
        }
    }

    fn discover() -> Result<ExecutorResult, ExecutorError> {
        // SAFETY: CUPS allocates the destination array, the returned count is
        // checked before making a slice, strings are copied before the array is
        // released exactly once with cupsFreeDests.
        unsafe {
            let mut destinations: *mut CupsDest = ptr::null_mut();
            let count = cups_get_dests(&mut destinations);
            if count < 0 || (count > 0 && destinations.is_null()) {
                return Err(last_error("cups_discovery_failed", false));
            }
            let length =
                usize::try_from(count).map_err(|_| last_error("cups_discovery_failed", false))?;
            let slice = std::slice::from_raw_parts(destinations, length);
            let printers = slice
                .iter()
                .filter_map(|destination| {
                    if destination.name.is_null() {
                        return None;
                    }
                    Some(DiscoveredPrinter {
                        native_id: CStr::from_ptr(destination.name)
                            .to_string_lossy()
                            .into_owned(),
                        name: CStr::from_ptr(destination.name)
                            .to_string_lossy()
                            .into_owned(),
                        is_default: destination.is_default != 0,
                        state: PrinterState::Unknown,
                        capabilities: PrinterCapabilities::default(),
                    })
                })
                .collect();
            cups_free_dests(count, destinations);
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
                message: format!("CUPS destination {native_id} was not found"),
                retryable: false,
                handoff_may_have_succeeded: false,
            })
    }

    fn submit(
        printer: &str,
        title: &str,
        content_path: &str,
        raw: bool,
        options: &spool_domain::JobOptions,
    ) -> Result<ExecutorResult, ExecutorError> {
        ensure_printer(printer)?;
        let printer = c_string(printer)?;
        let title = c_string(title)?;
        let path = c_string(content_path)?;
        let mut cups_options: *mut CupsOption = ptr::null_mut();
        let mut option_count = 0_i32;
        add_option(
            &mut option_count,
            &mut cups_options,
            "copies",
            &options.copies.unwrap_or(1).to_string(),
        )?;
        if let Some(paper) = &options.paper {
            add_option(&mut option_count, &mut cups_options, "media", paper)?;
        }
        if raw {
            add_option(&mut option_count, &mut cups_options, "raw", "true")?;
        }

        // SAFETY: All C strings and the option array remain alive for the
        // duration of the synchronous CUPS call. cupsFreeOptions releases the
        // array exactly once afterwards.
        let job_id = unsafe {
            let id = cups_print_file(
                printer.as_ptr(),
                path.as_ptr(),
                title.as_ptr(),
                option_count,
                cups_options,
            );
            cups_free_options(option_count, cups_options);
            id
        };
        if job_id <= 0 {
            return Err(last_error("cups_submission_failed", true));
        }
        Ok(ExecutorResult::Submitted {
            native_job_id: Some(job_id.to_string()),
        })
    }

    fn add_option(
        count: &mut i32,
        options: &mut *mut CupsOption,
        name: &str,
        value: &str,
    ) -> Result<(), ExecutorError> {
        let name = c_string(name)?;
        let value = c_string(value)?;
        // SAFETY: CUPS copies the provided option name and value into the
        // managed option array referenced by `options`.
        unsafe {
            *count = cups_add_option(name.as_ptr(), value.as_ptr(), *count, options);
        }
        Ok(())
    }

    fn cancel(printer: &str, native_job_id: &str) -> Result<ExecutorResult, ExecutorError> {
        let printer = c_string(printer)?;
        let job_id = native_job_id.parse::<i32>().map_err(|_| ExecutorError {
            code: "invalid_native_job_id".into(),
            message: "CUPS job ID must be an integer".into(),
            retryable: false,
            handoff_may_have_succeeded: false,
        })?;
        // SAFETY: `printer` is a valid NUL-terminated string and CUPS does not
        // retain the pointer after cupsCancelJob returns.
        let result = unsafe { cups_cancel_job(printer.as_ptr(), job_id) };
        if result == 0 {
            return Err(last_error("cups_cancel_failed", false));
        }
        Ok(ExecutorResult::Cancelled)
    }

    fn c_string(value: &str) -> Result<CString, ExecutorError> {
        CString::new(value).map_err(|_| ExecutorError {
            code: "invalid_native_string".into(),
            message: "native CUPS values cannot contain NUL bytes".into(),
            retryable: false,
            handoff_may_have_succeeded: false,
        })
    }

    fn last_error(code: &str, handoff: bool) -> ExecutorError {
        // SAFETY: cupsLastErrorString returns either null or a CUPS-owned
        // NUL-terminated string which is copied immediately.
        let message = unsafe {
            let pointer = cups_last_error_string();
            if pointer.is_null() {
                "unknown CUPS error".into()
            } else {
                CStr::from_ptr(pointer).to_string_lossy().into_owned()
            }
        };
        ExecutorError {
            code: code.into(),
            message,
            retryable: false,
            handoff_may_have_succeeded: handoff,
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn ipp_job_states_map_conservatively() {
            assert_eq!(cups_observation(3).state, NativeJobState::Queued);
            assert_eq!(cups_observation(5).state, NativeJobState::Printing);
            assert_eq!(cups_observation(9).state, NativeJobState::Completed);
            assert_eq!(cups_observation(8).state, NativeJobState::Failed);
            assert_eq!(cups_observation(42).state, NativeJobState::Unknown);
            assert_eq!(missing_observation().state, NativeJobState::Missing);
        }
    }
}

#[cfg(not(unix))]
mod platform {
    use spool_protocol::executor::{ExecutorError, ExecutorOperation, ExecutorResult};

    pub fn execute(_operation: ExecutorOperation) -> Result<ExecutorResult, ExecutorError> {
        Err(ExecutorError {
            code: "cups_unavailable".into(),
            message: "CUPS executor is available only on Unix systems".into(),
            retryable: false,
            handoff_may_have_succeeded: false,
        })
    }
}
