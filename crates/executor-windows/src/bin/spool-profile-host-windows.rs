use spool_executor_windows::native_profile::{
    NativeProfileError, ProfileHostOperation, ProfileHostRequest, ProfileHostResponse,
    ProfileHostResult, WINDOWS_PROFILE_HOST_PROTOCOL_VERSION,
};
use std::io::{Read as _, Write as _};

const MAX_REQUEST_BYTES: u64 = 24 * 1024 * 1024;

fn main() {
    let response = run().unwrap_or_else(|error| ProfileHostResponse {
        protocol_version: WINDOWS_PROFILE_HOST_PROTOCOL_VERSION,
        request_id: String::new(),
        result: Err(error),
    });
    let encoded = match serde_json::to_vec(&response) {
        Ok(encoded) => encoded,
        Err(error) => {
            eprintln!("Windows profile host could not encode its response: {error}");
            std::process::exit(1);
        }
    };
    if let Err(error) = std::io::stdout().lock().write_all(&encoded) {
        eprintln!("Windows profile host could not write its response: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<ProfileHostResponse, NativeProfileError> {
    let mut encoded = Vec::new();
    std::io::stdin()
        .lock()
        .take(MAX_REQUEST_BYTES + 1)
        .read_to_end(&mut encoded)
        .map_err(|error| NativeProfileError::new("profile_host_read_failed", error.to_string()))?;
    if encoded.len() as u64 > MAX_REQUEST_BYTES {
        return Err(NativeProfileError::new(
            "profile_host_request_too_large",
            "profile host request exceeds 24 MiB",
        ));
    }
    let request: ProfileHostRequest = serde_json::from_slice(&encoded).map_err(|error| {
        NativeProfileError::new("profile_host_request_invalid", error.to_string())
    })?;
    let request_id = request.request_id.clone();
    let result = authorize(&request).and_then(|()| execute(request.operation));
    Ok(ProfileHostResponse {
        protocol_version: WINDOWS_PROFILE_HOST_PROTOCOL_VERSION,
        request_id,
        result,
    })
}

fn authorize(request: &ProfileHostRequest) -> Result<(), NativeProfileError> {
    if request.protocol_version != WINDOWS_PROFILE_HOST_PROTOCOL_VERSION {
        return Err(NativeProfileError::new(
            "profile_host_protocol_unsupported",
            format!(
                "unsupported profile host protocol {}; expected {}",
                request.protocol_version, WINDOWS_PROFILE_HOST_PROTOCOL_VERSION
            ),
        ));
    }
    if request.capture_token.len() < 32 {
        return Err(NativeProfileError::new(
            "profile_capture_token_invalid",
            "capture token must contain at least 32 bytes",
        ));
    }
    let expected = std::env::var("SPOOL_PROFILE_CAPTURE_TOKEN").map_err(|_| {
        NativeProfileError::new(
            "profile_capture_token_unconfigured",
            "profile host was not launched with a one-time capture token",
        )
    })?;
    if !constant_time_equal(expected.as_bytes(), request.capture_token.as_bytes()) {
        return Err(NativeProfileError::new(
            "profile_capture_token_invalid",
            "capture token was not accepted",
        ));
    }
    // This is a single-request process. Removing the inherited value reduces
    // accidental exposure to any child process loaded by a vendor UI.
    unsafe {
        std::env::remove_var("SPOOL_PROFILE_CAPTURE_TOKEN");
    }
    Ok(())
}

#[cfg(windows)]
fn execute(operation: ProfileHostOperation) -> Result<ProfileHostResult, NativeProfileError> {
    use spool_executor_windows::windows_native::{
        CaptureOutcome, capture_profile, validate_profile,
    };

    match operation {
        ProfileHostOperation::Capture {
            native_printer_id,
            owner_window,
            existing,
        } => match capture_profile(&native_printer_id, owner_window, existing.as_ref())? {
            CaptureOutcome::Captured(capture) => {
                Ok(ProfileHostResult::Captured { capture: *capture })
            }
            CaptureOutcome::Cancelled => Ok(ProfileHostResult::Cancelled),
        },
        ProfileHostOperation::Validate {
            native_printer_id,
            capture,
        } => Ok(ProfileHostResult::Validated {
            capture: validate_profile(&native_printer_id, &capture)?,
        }),
    }
}

#[cfg(not(windows))]
fn execute(_operation: ProfileHostOperation) -> Result<ProfileHostResult, NativeProfileError> {
    Err(NativeProfileError::new(
        "winspool_unavailable",
        "Windows native profile host is available only on Windows",
    ))
}

fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}
