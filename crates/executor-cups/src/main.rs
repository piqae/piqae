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
    use spool_domain::{
        Duplex, NativePrinterChoice, NativePrinterOption, PrinterCapabilities, PrinterState,
        Rotation,
    };
    use spool_protocol::executor::{
        DiscoveredPrinter, ExecutorError, ExecutorOperation, ExecutorResult, NativeJobObservation,
        NativeJobState, NativeQueueJob,
    };
    use std::{
        collections::BTreeMap,
        ffi::{CStr, CString, c_char, c_int, c_long},
        path::Path,
        process::{Command, Stdio},
        ptr,
        time::{Duration, Instant},
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
                let (capabilities, native_options) = capability_profile(&native_printer_id);
                Ok(ExecutorResult::Capabilities {
                    capabilities,
                    native_options,
                })
            }
            ExecutorOperation::ListJobs { native_printer_id } => list_jobs(&native_printer_id),
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

    fn list_jobs(printer: &str) -> Result<ExecutorResult, ExecutorError> {
        ensure_printer(printer)?;
        let printer_string = printer.to_owned();
        let printer = c_string(printer)?;
        // SAFETY: CUPS owns the array; all strings are copied while it is
        // alive and the allocation is released exactly once.
        let jobs = unsafe {
            let mut jobs: *mut CupsJob = ptr::null_mut();
            let count = cups_get_jobs(&mut jobs, printer.as_ptr(), 0, -1);
            if count < 0 {
                return Err(last_error("cups_queue_failed", false));
            }
            let length =
                usize::try_from(count).map_err(|_| last_error("cups_queue_failed", false))?;
            let records = records_or_empty(jobs, length)
                .ok_or_else(|| last_error("cups_queue_failed", false))?;
            let result = records
                .iter()
                .map(|job| NativeQueueJob {
                    native_job_id: job.id.to_string(),
                    native_printer_id: printer_string.clone(),
                    title: c_lossy(job.title).unwrap_or_else(|| "Untitled".into()),
                    user: c_lossy(job.user),
                    state: cups_job_state(job.state),
                    native_code: Some(format!("ipp-job-state-{}", job.state)),
                    size_kib: u64::try_from(job.size).ok(),
                    created_unix_ms: cups_time_ms(job.creation_time),
                    processing_unix_ms: cups_time_ms(job.processing_time),
                    completed_unix_ms: cups_time_ms(job.completed_time),
                })
                .collect();
            if !jobs.is_null() {
                cups_free_jobs(count, jobs);
            }
            result
        };
        Ok(ExecutorResult::Jobs { jobs })
    }

    fn cups_time_ms(value: c_long) -> Option<i64> {
        (value > 0).then(|| value.saturating_mul(1_000))
    }

    unsafe fn c_lossy(value: *const c_char) -> Option<String> {
        if value.is_null() {
            None
        } else {
            // SAFETY: caller provides a CUPS-owned NUL-terminated string.
            Some(
                unsafe { CStr::from_ptr(value) }
                    .to_string_lossy()
                    .into_owned(),
            )
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
            if count < 0 {
                return Err(last_error("cups_observation_failed", false));
            }
            let length =
                usize::try_from(count).map_err(|_| last_error("cups_observation_failed", false))?;
            let records = records_or_empty(jobs, length)
                .ok_or_else(|| last_error("cups_observation_failed", false))?;
            let state = records
                .iter()
                .find(|job| {
                    job.id == job_id
                        && !job.dest.is_null()
                        && CStr::from_ptr(job.dest).to_bytes() == printer.as_bytes()
                })
                .map(|job| job.state);
            if !jobs.is_null() {
                cups_free_jobs(count, jobs);
            }
            state.map_or_else(missing_observation, cups_observation)
        };
        Ok(ExecutorResult::Observation { observation })
    }

    fn cups_observation(state: i32) -> NativeJobObservation {
        let mapped = cups_job_state(state);
        NativeJobObservation {
            state: mapped,
            native_code: Some(format!("ipp-job-state-{state}")),
            message: Some("CUPS reported IPP job state".into()),
        }
    }

    fn cups_job_state(state: i32) -> NativeJobState {
        match state {
            3 => NativeJobState::Queued,
            4 | 6 => NativeJobState::Blocked,
            5 => NativeJobState::Printing,
            7 => NativeJobState::Cancelled,
            8 => NativeJobState::Failed,
            9 => NativeJobState::Completed,
            _ => NativeJobState::Unknown,
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
            if count < 0 {
                return Err(last_error("cups_discovery_failed", false));
            }
            let length =
                usize::try_from(count).map_err(|_| last_error("cups_discovery_failed", false))?;
            let slice = records_or_empty(destinations, length)
                .ok_or_else(|| last_error("cups_discovery_failed", false))?;
            let printer_stubs: Vec<_> = slice
                .iter()
                .filter_map(|destination| {
                    if destination.name.is_null() {
                        return None;
                    }
                    let native_id = CStr::from_ptr(destination.name)
                        .to_string_lossy()
                        .into_owned();
                    let state = destination_option(destination, "printer-state")
                        .and_then(|value| value.parse::<u8>().ok())
                        .map_or(PrinterState::Unknown, cups_printer_state);
                    Some((native_id, destination.is_default != 0, state))
                })
                .collect();
            if !destinations.is_null() {
                cups_free_dests(count, destinations);
            }
            let printers = printer_stubs
                .into_iter()
                .map(|(native_id, is_default, state)| {
                    let (capabilities, native_options) = capability_profile(&native_id);
                    DiscoveredPrinter {
                        name: native_id.clone(),
                        native_id,
                        is_default,
                        state,
                        capabilities,
                        native_options,
                    }
                })
                .collect();
            Ok(ExecutorResult::Printers { printers })
        }
    }

    unsafe fn destination_option(destination: &CupsDest, key: &str) -> Option<String> {
        let length = usize::try_from(destination.num_options).ok()?;
        // SAFETY: destination options are owned by the live destination array.
        let options = unsafe { records_or_empty(destination.options, length) }?;
        options.iter().find_map(|option| {
            if option.name.is_null() || option.value.is_null() {
                return None;
            }
            // SAFETY: option pointers are valid C strings for the destination
            // allocation lifetime.
            let name = unsafe { CStr::from_ptr(option.name) }.to_bytes();
            if name == key.as_bytes() {
                // SAFETY: null was checked above.
                Some(
                    unsafe { CStr::from_ptr(option.value) }
                        .to_string_lossy()
                        .into_owned(),
                )
            } else {
                None
            }
        })
    }

    fn cups_printer_state(state: u8) -> PrinterState {
        match state {
            3 => PrinterState::Online,
            4 => PrinterState::Busy,
            5 => PrinterState::Paused,
            _ => PrinterState::Unknown,
        }
    }

    fn capability_profile(
        printer: &str,
    ) -> (PrinterCapabilities, BTreeMap<String, NativePrinterOption>) {
        let Some(path) = lpoptions_path() else {
            return (PrinterCapabilities::default(), BTreeMap::new());
        };
        let Some(output) = command_output_bounded(path, printer, Duration::from_millis(500)) else {
            return (PrinterCapabilities::default(), BTreeMap::new());
        };
        if !output.status.success() {
            return (PrinterCapabilities::default(), BTreeMap::new());
        }
        parse_lpoptions(&String::from_utf8_lossy(&output.stdout))
    }

    fn command_output_bounded(
        path: &Path,
        printer: &str,
        timeout: Duration,
    ) -> Option<std::process::Output> {
        let mut child = Command::new(path)
            .args(["-p", printer, "-l"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .ok()?;
        let mut stdout = child.stdout.take()?;
        let mut stderr = child.stderr.take()?;
        let stdout_reader = std::thread::spawn(move || drain_bounded(&mut stdout, 1024 * 1024));
        let stderr_reader = std::thread::spawn(move || drain_bounded(&mut stderr, 64 * 1024));
        let started = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if started.elapsed() < timeout => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Ok(None) | Err(_) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = stdout_reader.join();
                    let _ = stderr_reader.join();
                    return None;
                }
            }
        };
        Some(std::process::Output {
            status,
            stdout: stdout_reader.join().ok()?,
            stderr: stderr_reader.join().ok()?,
        })
    }

    fn drain_bounded(reader: &mut impl std::io::Read, retained_limit: usize) -> Vec<u8> {
        let mut retained = Vec::new();
        let mut buffer = [0_u8; 8 * 1024];
        while let Ok(count) = reader.read(&mut buffer) {
            if count == 0 {
                break;
            }
            let available = retained_limit.saturating_sub(retained.len()).min(count);
            retained.extend_from_slice(&buffer[..available]);
        }
        retained
    }

    fn lpoptions_path() -> Option<&'static Path> {
        ["/usr/bin/lpoptions", "/usr/sbin/lpoptions"]
            .into_iter()
            .map(Path::new)
            .find(|path| path.is_file())
    }

    fn parse_lpoptions(
        output: &str,
    ) -> (PrinterCapabilities, BTreeMap<String, NativePrinterOption>) {
        let mut capabilities = PrinterCapabilities::default();
        let mut native_options = BTreeMap::new();
        for line in output
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
        {
            let Some((heading, encoded_choices)) = line.split_once(':') else {
                continue;
            };
            let (key, display_name) = heading
                .trim()
                .split_once('/')
                .map_or((heading.trim(), heading.trim()), |(key, name)| {
                    (key.trim(), name.trim())
                });
            if key.is_empty() {
                continue;
            }
            let (choices, default_choice) = parse_choices(encoded_choices);
            if choices.is_empty() {
                continue;
            }
            apply_portable_capability(&mut capabilities, key, &choices);
            native_options.insert(
                key.to_owned(),
                NativePrinterOption {
                    display_name: display_name.to_owned(),
                    default_choice: default_choice.clone(),
                    selected_choice: default_choice,
                    choices,
                },
            );
        }
        (capabilities, native_options)
    }

    fn parse_choices(encoded: &str) -> (Vec<NativePrinterChoice>, Option<String>) {
        let tokens: Vec<_> = encoded.split_whitespace().collect();
        let labelled = tokens.iter().any(|token| token.contains('/'));
        let mut choices: Vec<NativePrinterChoice> = Vec::new();
        let mut default_choice = None;
        for token in tokens {
            let (is_default, token) = token
                .strip_prefix('*')
                .map_or((false, token), |choice| (true, choice));
            if labelled && !token.contains('/') {
                if let Some(previous) = choices.last_mut() {
                    if previous.display_name.ends_with('\\') {
                        previous.display_name.pop();
                    }
                    previous.display_name.push(' ');
                    previous.display_name.push_str(&token.replace("\\ ", " "));
                }
                continue;
            }
            let (value, label) = token
                .split_once('/')
                .map_or((token, token), |(value, label)| (value, label));
            if value.is_empty() {
                continue;
            }
            if is_default {
                default_choice = Some(value.to_owned());
            }
            choices.push(NativePrinterChoice {
                value: value.to_owned(),
                display_name: label.replace("\\ ", " "),
            });
        }
        (choices, default_choice)
    }

    fn apply_portable_capability(
        capabilities: &mut PrinterCapabilities,
        key: &str,
        choices: &[NativePrinterChoice],
    ) {
        let normalized = key.to_ascii_lowercase();
        let values = || choices.iter().map(|choice| choice.value.clone());
        match normalized.as_str() {
            "pagesize" | "pageregion" | "media" => {
                for choice in choices {
                    if let Some(size) = paper_size(&choice.value) {
                        capabilities
                            .papers
                            .insert(choice.value.clone(), [Some(size.0), Some(size.1)]);
                    } else if choice.value.to_ascii_lowercase().contains("custom") {
                        capabilities.supports_custom_paper_size = true;
                    } else {
                        capabilities
                            .papers
                            .entry(choice.value.clone())
                            .or_insert([None, None]);
                    }
                }
            }
            "resolution" | "printer-resolution" => capabilities.dpis.extend(values()),
            "inputslot" | "outputbin" | "media-source" => capabilities.bins.extend(values()),
            "mediatype" | "media-type" => capabilities.medias.extend(values()),
            "duplex" | "sides" => {
                capabilities.duplex = choices.iter().any(|choice| {
                    let value = choice.value.to_ascii_lowercase();
                    value.contains("two-sided")
                        || value.contains("duplex")
                        || value.contains("tumble")
                });
            }
            "colormodel" | "colormode" | "print-color-mode" => {
                capabilities.color = choices.iter().any(|choice| {
                    let value = choice.value.to_ascii_lowercase();
                    value.contains("color") || value.contains("rgb") || value.contains("cmy")
                });
            }
            "collate" => capabilities.collate = choices.len() > 1,
            "number-up" | "numberup" => {
                capabilities.nup.extend(
                    choices
                        .iter()
                        .filter_map(|choice| choice.value.parse::<u16>().ok()),
                );
            }
            _ => {}
        }
        capabilities.bins.sort();
        capabilities.bins.dedup();
        capabilities.dpis.sort();
        capabilities.dpis.dedup();
        capabilities.medias.sort();
        capabilities.medias.dedup();
        capabilities.nup.sort_unstable();
        capabilities.nup.dedup();
    }

    fn paper_size(value: &str) -> Option<(u32, u32)> {
        let normalized = value.to_ascii_lowercase();
        if normalized.contains("a4") || normalized.contains("210x297") {
            Some((2_100, 2_970))
        } else if normalized.contains("letter") || normalized.contains("8.5x11") {
            Some((2_159, 2_794))
        } else if normalized.contains("legal") || normalized.contains("8.5x14") {
            Some((2_159, 3_556))
        } else if normalized.contains("a3") || normalized.contains("297x420") {
            Some((2_970, 4_200))
        } else if normalized.contains("a5") || normalized.contains("148x210") {
            Some((1_480, 2_100))
        } else {
            None
        }
    }

    /// Converts a C-owned record array without constructing a zero-length
    /// Rust slice from a null pointer. CUPS uses `(0, NULL)` for an empty
    /// result set on clean machines.
    ///
    /// # Safety
    ///
    /// For a non-zero `length`, `records` must point to `length` initialized,
    /// contiguous values that remain alive for the returned lifetime.
    unsafe fn records_or_empty<'a, T>(records: *const T, length: usize) -> Option<&'a [T]> {
        if length == 0 {
            return Some(&[]);
        }
        if records.is_null() {
            return None;
        }
        // SAFETY: the caller owns the non-null pointer/length validity
        // contract documented above.
        Some(unsafe { std::slice::from_raw_parts(records, length) })
    }

    fn ensure_printer(native_id: &str) -> Result<(), ExecutorError> {
        // SAFETY: only destination names are inspected while the CUPS-owned
        // array is live, then it is released exactly once.
        let found = unsafe {
            let mut destinations: *mut CupsDest = ptr::null_mut();
            let count = cups_get_dests(&mut destinations);
            if count < 0 {
                return Err(last_error("cups_discovery_failed", false));
            }
            let length =
                usize::try_from(count).map_err(|_| last_error("cups_discovery_failed", false))?;
            let records = records_or_empty(destinations, length)
                .ok_or_else(|| last_error("cups_discovery_failed", false))?;
            let found = records.iter().any(|destination| {
                !destination.name.is_null()
                    && CStr::from_ptr(destination.name).to_bytes() == native_id.as_bytes()
            });
            if !destinations.is_null() {
                cups_free_dests(count, destinations);
            }
            found
        };
        found.then_some(()).ok_or_else(|| ExecutorError {
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
        let mapped_options = cups_job_options(raw, options);
        for (name, value) in &mapped_options {
            // Validate every value before CUPS allocates the first option, so
            // an embedded NUL cannot leave a partially built array behind.
            c_string(name)?;
            c_string(value)?;
        }
        for (name, value) in &mapped_options {
            add_option(&mut option_count, &mut cups_options, name, value)?;
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

    fn cups_job_options(raw: bool, options: &spool_domain::JobOptions) -> Vec<(String, String)> {
        if raw {
            return vec![("raw".into(), "true".into())];
        }

        let mut mapped: Vec<(String, String)> = Vec::new();
        if let Some(value) = &options.bin {
            mapped.push(("media-source".into(), value.clone()));
        }
        if let Some(value) = options.collate {
            mapped.push((
                "multiple-document-handling".into(),
                if value {
                    "separate-documents-collated-copies"
                } else {
                    "separate-documents-uncollated-copies"
                }
                .into(),
            ));
        }
        if let Some(value) = options.color {
            mapped.push((
                "print-color-mode".into(),
                if value { "color" } else { "monochrome" }.into(),
            ));
        }
        if let Some(value) = options.copies {
            mapped.push(("copies".into(), value.to_string()));
        }
        if let Some(value) = &options.dpi {
            mapped.push(("printer-resolution".into(), value.clone()));
        }
        if let Some(value) = options.duplex {
            mapped.push((
                "sides".into(),
                match value {
                    Duplex::OneSided => "one-sided",
                    Duplex::LongEdge => "two-sided-long-edge",
                    Duplex::ShortEdge => "two-sided-short-edge",
                }
                .into(),
            ));
        }
        if let Some(value) = options.fit_to_page {
            mapped.push(("fit-to-page".into(), value.to_string()));
        }
        if let Some(value) = &options.media {
            mapped.push(("media-type".into(), value.clone()));
        }
        if let Some(value) = options.nup {
            mapped.push(("number-up".into(), value.to_string()));
        }
        if let Some(value) = &options.pages {
            mapped.push(("page-ranges".into(), value.clone()));
        }
        if let Some(value) = &options.paper {
            mapped.push(("media".into(), value.clone()));
        }
        if let Some(value) = options.rotate {
            mapped.push((
                "orientation-requested".into(),
                match value {
                    Rotation::Deg0 => "3",
                    Rotation::Deg90 => "4",
                    Rotation::Deg180 => "6",
                    Rotation::Deg270 => "5",
                }
                .into(),
            ));
        }
        for (name, value) in &options.native_options {
            // Portable fields remain authoritative for their IPP keys. A
            // named profile cannot silently override a caller-visible option.
            if !mapped
                .iter()
                .any(|(existing, _)| existing.eq_ignore_ascii_case(name))
            {
                mapped.push((name.clone(), value.clone()));
            }
        }
        mapped
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
        fn null_zero_length_c_arrays_are_empty() {
            // SAFETY: a zero-length result never dereferences the pointer.
            let records =
                unsafe { records_or_empty::<u8>(ptr::null(), 0) }.expect("zero-length C array");
            assert!(records.is_empty());
        }

        #[test]
        fn null_non_empty_c_arrays_are_rejected() {
            // SAFETY: the helper rejects the pointer before dereferencing it.
            assert!(unsafe { records_or_empty::<u8>(ptr::null(), 1) }.is_none());
        }

        #[test]
        fn ipp_job_states_map_conservatively() {
            assert_eq!(cups_observation(3).state, NativeJobState::Queued);
            assert_eq!(cups_observation(5).state, NativeJobState::Printing);
            assert_eq!(cups_observation(9).state, NativeJobState::Completed);
            assert_eq!(cups_observation(8).state, NativeJobState::Failed);
            assert_eq!(cups_observation(42).state, NativeJobState::Unknown);
            assert_eq!(missing_observation().state, NativeJobState::Missing);
        }

        #[test]
        fn pdf_options_map_to_standard_ipp_names() {
            let options = spool_domain::JobOptions {
                bin: Some("tray-1".into()),
                collate: Some(true),
                color: Some(false),
                copies: Some(2),
                dpi: Some("300dpi".into()),
                duplex: Some(Duplex::LongEdge),
                fit_to_page: Some(true),
                media: Some("labels".into()),
                nup: Some(2),
                pages: Some("1,3-5".into()),
                paper: Some("iso_a4_210x297mm".into()),
                rotate: Some(Rotation::Deg90),
                native_options: BTreeMap::new(),
            };
            assert_eq!(
                cups_job_options(false, &options),
                vec![
                    ("media-source".into(), "tray-1".into()),
                    (
                        "multiple-document-handling".into(),
                        "separate-documents-collated-copies".into()
                    ),
                    ("print-color-mode".into(), "monochrome".into()),
                    ("copies".into(), "2".into()),
                    ("printer-resolution".into(), "300dpi".into()),
                    ("sides".into(), "two-sided-long-edge".into()),
                    ("fit-to-page".into(), "true".into()),
                    ("media-type".into(), "labels".into()),
                    ("number-up".into(), "2".into()),
                    ("page-ranges".into(), "1,3-5".into()),
                    ("media".into(), "iso_a4_210x297mm".into()),
                    ("orientation-requested".into(), "4".into()),
                ]
            );
        }

        #[test]
        fn raw_jobs_ignore_rendering_options() {
            let options = spool_domain::JobOptions {
                copies: Some(99),
                paper: Some("A4".into()),
                duplex: Some(Duplex::LongEdge),
                ..Default::default()
            };
            assert_eq!(
                cups_job_options(true, &options),
                vec![("raw".into(), "true".into())]
            );
        }

        #[test]
        fn lpoptions_preserves_vendor_choices_and_maps_portable_capabilities() {
            let (portable, native) = parse_lpoptions(
                "PageSize/Page Size: *A4/A4 Letter/US Letter\n\
                 Duplex/Two-Sided: *None/Off DuplexNoTumble/Long Edge\n\
                 ColorModel/Colour: Gray/Mono *RGB/Colour\n\
                 OKIBlackMark/Black Mark Sensor: *Off/Disabled On/Enabled\n\
                 Resolution/Resolution: 300dpi/300 600dpi/600\n",
            );
            assert_eq!(portable.papers.get("A4"), Some(&[Some(2_100), Some(2_970)]));
            assert!(portable.duplex);
            assert!(portable.color);
            assert_eq!(portable.dpis, vec!["300dpi", "600dpi"]);
            let vendor = native.get("OKIBlackMark").expect("vendor option");
            assert_eq!(vendor.display_name, "Black Mark Sensor");
            assert_eq!(vendor.default_choice.as_deref(), Some("Off"));
            assert_eq!(vendor.choices[1].value, "On");
            assert_eq!(vendor.choices[1].display_name, "Enabled");
        }

        #[test]
        fn parses_captured_macos_lpoptions_without_inventing_label_choices() {
            let (portable, native) = parse_lpoptions(
                "Collate/Collate: True *False\n\
                 ColorModel/Color Mode: Gray *RGB\n\
                 cupsPrintQuality/Quality: Draft *Normal High\n\
                 Duplex/2-Sided Printing: None *DuplexNoTumble DuplexTumble\n\
                 PageSize/Media Size: 100x150mm A3 *A4 A5 Letter Custom.WIDTHxHEIGHT\n\
                 InputSlot/Media Source: auto tray-2 *tray-1\n",
            );
            assert!(portable.color);
            assert!(portable.duplex);
            assert!(portable.collate);
            assert!(portable.papers.contains_key("A4"));
            assert!(portable.supports_custom_paper_size);
            assert_eq!(
                native["cupsPrintQuality"]
                    .choices
                    .iter()
                    .map(|choice| choice.value.as_str())
                    .collect::<Vec<_>>(),
                vec!["Draft", "Normal", "High"]
            );
            let (labelled, default) = parse_choices("*Letter/US Letter Legal/US Legal");
            assert_eq!(default.as_deref(), Some("Letter"));
            assert_eq!(labelled.len(), 2);
            assert_eq!(labelled[0].display_name, "US Letter");
            assert_eq!(labelled[1].display_name, "US Legal");
            let (escaped, _) = parse_choices("*Letter/US\\ Letter Legal/US\\ Legal");
            assert_eq!(escaped[0].display_name, "US Letter");
            assert_eq!(escaped[1].display_name, "US Legal");
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
