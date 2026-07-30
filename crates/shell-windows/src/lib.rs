//! Windows tray-shell client and native profile capture orchestration.
//!
//! The shell deliberately talks only to the authenticated loopback API. It
//! never opens the agent database or receives hosted credentials.

pub mod updater;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest as _, Sha256};
use spool_domain::{
    DriverFingerprint, JobOptions, NativeProfileKind, ProfileCaptureOperation, ProfileDependency,
    ProfileSummary, SafeProfileOverride,
};
use spool_executor_windows::native_profile::{
    ProfileHostOperation, ProfileHostRequest, ProfileHostResponse, ProfileHostResult,
    WINDOWS_PROFILE_HOST_PROTOCOL_VERSION, WindowsNativeProfileCapture,
};
use spool_local_ipc::{
    LocalPrinter, LocalPrinterProfile, LocalStatus, NativeProfileCapturePayload,
    ProfileCaptureAuthorized,
};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    io::{Read as _, Write as _},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};
use thiserror::Error;
use uuid::Uuid;

const DEFAULT_LOCAL_API_URL: &str = "http://127.0.0.1:39100";
const MAX_API_RESPONSE_BYTES: usize = 24 * 1024 * 1024;
const PROFILE_HOST_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const PROFILE_HOST_OUTPUT_LIMIT: usize = 24 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalApiConfiguration {
    pub base_url: String,
    pub token_file: PathBuf,
    address: SocketAddr,
}

impl LocalApiConfiguration {
    /// Loads configuration from the process environment and rejects any
    /// non-loopback API address.
    ///
    /// # Errors
    ///
    /// Returns an error when the URL could allow credentials or profile
    /// captures to leave this Windows node.
    pub fn from_environment() -> Result<Self, ShellError> {
        Self::from_values(std::env::vars_os())
    }

    fn from_values(
        environment: impl IntoIterator<Item = (OsString, OsString)>,
    ) -> Result<Self, ShellError> {
        let environment = environment.into_iter().collect::<BTreeMap<_, _>>();
        let base_url = environment
            .get(&OsString::from("SPOOL_LOCAL_API_URL"))
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty())
            .unwrap_or(DEFAULT_LOCAL_API_URL);
        let parsed = url::Url::parse(base_url)
            .map_err(|_| ShellError::Configuration("SPOOL_LOCAL_API_URL is invalid".into()))?;
        let host = parsed.host_str().map(str::to_ascii_lowercase);
        let address = match host.as_deref() {
            Some("127.0.0.1" | "localhost") => {
                SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), parsed.port().unwrap_or(80))
            }
            Some("::1") => {
                SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), parsed.port().unwrap_or(80))
            }
            _ => {
                return Err(ShellError::Configuration(
                    "SPOOL_LOCAL_API_URL must be an HTTP loopback origin".into(),
                ));
            }
        };
        if parsed.scheme() != "http"
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
            || !matches!(parsed.path(), "" | "/")
        {
            return Err(ShellError::Configuration(
                "SPOOL_LOCAL_API_URL must be an HTTP loopback origin".into(),
            ));
        }

        let token_file = environment
            .get(&OsString::from("SPOOL_LOCAL_TOKEN_FILE"))
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                environment
                    .get(&OsString::from("SPOOL_DATA_DIR"))
                    .filter(|value| !value.is_empty())
                    .map(PathBuf::from)
                    .map(|directory| directory.join("local.token"))
            })
            .unwrap_or_else(|| {
                environment
                    .get(&OsString::from("ProgramData"))
                    .filter(|value| !value.is_empty())
                    .map_or_else(
                        || PathBuf::from(".spool"),
                        |directory| PathBuf::from(directory).join("Spool"),
                    )
                    .join("local.token")
            });
        Ok(Self {
            base_url: parsed.as_str().trim_end_matches('/').to_owned(),
            token_file,
            address,
        })
    }
}

#[derive(Debug)]
pub struct LocalAgentClient {
    configuration: LocalApiConfiguration,
}

impl LocalAgentClient {
    /// Creates the bounded HTTP client used by the interactive shell.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP client cannot be configured.
    pub fn new(configuration: LocalApiConfiguration) -> Result<Self, ShellError> {
        Ok(Self { configuration })
    }

    /// Returns current local-agent status.
    ///
    /// # Errors
    ///
    /// Returns an error when authentication, transport, or decoding fails.
    pub fn status(&self) -> Result<LocalStatus, ShellError> {
        self.get("/v1/local/status")
    }

    /// Returns locally discovered printers and their profiles.
    ///
    /// # Errors
    ///
    /// Returns an error when authentication, transport, or decoding fails.
    pub fn printers(&self) -> Result<Vec<LocalPrinter>, ShellError> {
        self.get("/v1/local/printers")
    }

    /// Opens a short-lived native profile capture session.
    ///
    /// # Errors
    ///
    /// Returns an error if the requested profile revision is not current or
    /// the local agent rejects the operation.
    pub fn begin_profile_capture(
        &self,
        printer_id: &str,
        operation: ProfileCaptureOperation,
        profile_id: Option<&str>,
        expected_revision: Option<u64>,
    ) -> Result<ProfileCaptureAuthorized, ShellError> {
        #[derive(Serialize)]
        struct Request<'a> {
            operation: ProfileCaptureOperation,
            profile_id: Option<&'a str>,
            expected_revision: Option<u64>,
        }
        self.post(
            &format!(
                "/v1/local/printers/{}/profile-capture-sessions",
                path_segment(printer_id)?
            ),
            &Request {
                operation,
                profile_id,
                expected_revision,
            },
            None,
        )
    }

    /// Commits the captured immutable native envelope.
    ///
    /// # Errors
    ///
    /// Returns an error if the one-time token has expired or validation fails.
    pub fn complete_profile_capture(
        &self,
        session: &ProfileCaptureAuthorized,
        capture: &NativeProfileCapturePayload,
    ) -> Result<LocalPrinterProfile, ShellError> {
        self.post(
            &format!(
                "/v1/local/profile-capture-sessions/{}/complete",
                path_segment(&session.session_id)?
            ),
            capture,
            Some(&session.capture_token),
        )
    }

    /// Cancels an unused capture token. Cancellation is idempotent from the
    /// shell's perspective.
    pub fn cancel_profile_capture(&self, session: &ProfileCaptureAuthorized) {
        let Ok(path) = path_segment(&session.session_id) else {
            return;
        };
        let _response = self.send::<serde_json::Value>(
            "DELETE",
            &format!("/v1/local/profile-capture-sessions/{path}"),
            None,
            Some(&session.capture_token),
            true,
        );
    }

    fn get<ResponseBody: DeserializeOwned>(&self, path: &str) -> Result<ResponseBody, ShellError> {
        self.send("GET", path, None, None, false)
    }

    fn post<RequestBody: Serialize, ResponseBody: DeserializeOwned>(
        &self,
        path: &str,
        body: &RequestBody,
        capture_token: Option<&str>,
    ) -> Result<ResponseBody, ShellError> {
        let body = serde_json::to_vec(body).map_err(ShellError::Json)?;
        self.send("POST", path, Some(&body), capture_token, false)
    }

    fn send<ResponseBody: DeserializeOwned>(
        &self,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
        capture_token: Option<&str>,
        allow_empty: bool,
    ) -> Result<ResponseBody, ShellError> {
        if !path.starts_with('/') || path.contains(['\r', '\n']) {
            return Err(ShellError::Protocol("invalid local API path".into()));
        }
        let token = self.read_token()?;
        ensure_header_value(&token)?;
        if let Some(capture_token) = capture_token {
            ensure_header_value(capture_token)?;
        }
        let mut stream =
            TcpStream::connect_timeout(&self.configuration.address, Duration::from_secs(1))
                .map_err(ShellError::Io)?;
        stream
            .set_read_timeout(Some(Duration::from_secs(8)))
            .map_err(ShellError::Io)?;
        stream
            .set_write_timeout(Some(Duration::from_secs(8)))
            .map_err(ShellError::Io)?;
        let body = body.unwrap_or_default();
        write!(
            stream,
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {token}\r\nAccept: application/json\r\nConnection: close\r\nContent-Length: {}\r\n",
            self.configuration.address,
            body.len()
        )
        .map_err(ShellError::Io)?;
        if !body.is_empty() {
            stream
                .write_all(b"Content-Type: application/json\r\n")
                .map_err(ShellError::Io)?;
        }
        if let Some(capture_token) = capture_token {
            write!(stream, "X-Spool-Capture-Token: {capture_token}\r\n").map_err(ShellError::Io)?;
        }
        stream.write_all(b"\r\n").map_err(ShellError::Io)?;
        stream.write_all(body).map_err(ShellError::Io)?;
        stream.flush().map_err(ShellError::Io)?;

        let mut response = Vec::new();
        stream
            .take(MAX_API_RESPONSE_BYTES as u64 + 64 * 1024 + 1)
            .read_to_end(&mut response)
            .map_err(ShellError::Io)?;
        let (status, body) = decode_http_response(&response)?;
        if !(200..300).contains(&status) {
            #[derive(serde::Deserialize)]
            struct Failure {
                message: String,
            }
            let message = serde_json::from_slice::<Failure>(&body)
                .map_or_else(|_| format!("HTTP {status}"), |failure| failure.message);
            return Err(ShellError::Agent { status, message });
        }
        if allow_empty && body.is_empty() {
            return serde_json::from_slice(b"null").map_err(ShellError::Json);
        }
        serde_json::from_slice(&body).map_err(ShellError::Json)
    }

    fn read_token(&self) -> Result<String, ShellError> {
        let metadata = fs::metadata(&self.configuration.token_file).map_err(|error| {
            ShellError::Token(format!(
                "cannot read {}: {error}",
                self.configuration.token_file.display()
            ))
        })?;
        if metadata.len() > 1024 {
            return Err(ShellError::Token("local token is oversized".into()));
        }
        let token = fs::read_to_string(&self.configuration.token_file)
            .map_err(|error| ShellError::Token(error.to_string()))?;
        let token = token.trim();
        if token.is_empty() || token.len() > 1024 {
            return Err(ShellError::Token(
                "local token is empty or oversized".into(),
            ));
        }
        Ok(token.to_owned())
    }
}

fn ensure_header_value(value: &str) -> Result<(), ShellError> {
    if value.is_empty() || value.contains(['\r', '\n']) {
        return Err(ShellError::Protocol(
            "local authentication token is invalid".into(),
        ));
    }
    Ok(())
}

fn decode_http_response(response: &[u8]) -> Result<(u16, Vec<u8>), ShellError> {
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| ShellError::Protocol("local API returned incomplete HTTP headers".into()))?;
    if header_end > 64 * 1024 {
        return Err(ShellError::ResponseTooLarge);
    }
    let headers = std::str::from_utf8(&response[..header_end])
        .map_err(|_| ShellError::Protocol("local API returned invalid HTTP headers".into()))?;
    let mut lines = headers.split("\r\n");
    let status = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|status| status.parse::<u16>().ok())
        .ok_or_else(|| ShellError::Protocol("local API returned invalid HTTP status".into()))?;
    let mut content_length = None;
    let mut chunked = false;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            return Err(ShellError::Protocol(
                "local API returned a malformed HTTP header".into(),
            ));
        };
        if name.eq_ignore_ascii_case("content-length") {
            content_length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| ShellError::Protocol("invalid Content-Length".into()))?,
            );
        } else if name.eq_ignore_ascii_case("transfer-encoding")
            && value
                .split(',')
                .any(|encoding| encoding.trim().eq_ignore_ascii_case("chunked"))
        {
            chunked = true;
        }
    }
    let raw_body = &response[header_end + 4..];
    let body = if chunked {
        decode_chunked_body(raw_body)?
    } else if let Some(length) = content_length {
        if length > MAX_API_RESPONSE_BYTES || raw_body.len() < length {
            return Err(if length > MAX_API_RESPONSE_BYTES {
                ShellError::ResponseTooLarge
            } else {
                ShellError::Protocol("local API returned a truncated response".into())
            });
        }
        raw_body[..length].to_vec()
    } else {
        raw_body.to_vec()
    };
    if body.len() > MAX_API_RESPONSE_BYTES {
        return Err(ShellError::ResponseTooLarge);
    }
    Ok((status, body))
}

fn decode_chunked_body(mut input: &[u8]) -> Result<Vec<u8>, ShellError> {
    let mut output = Vec::new();
    loop {
        let line_end = input
            .windows(2)
            .position(|window| window == b"\r\n")
            .ok_or_else(|| ShellError::Protocol("truncated HTTP chunk".into()))?;
        let size_text = std::str::from_utf8(&input[..line_end])
            .map_err(|_| ShellError::Protocol("invalid HTTP chunk size".into()))?
            .split(';')
            .next()
            .unwrap_or_default();
        let size = usize::from_str_radix(size_text.trim(), 16)
            .map_err(|_| ShellError::Protocol("invalid HTTP chunk size".into()))?;
        input = &input[line_end + 2..];
        if size == 0 {
            return Ok(output);
        }
        if size > MAX_API_RESPONSE_BYTES.saturating_sub(output.len())
            || input.len() < size.saturating_add(2)
            || &input[size..size + 2] != b"\r\n"
        {
            return Err(ShellError::ResponseTooLarge);
        }
        output.extend_from_slice(&input[..size]);
        input = &input[size + 2..];
    }
}

/// Launches the single-use Windows driver property-sheet host and returns its
/// captured DEVMODE envelope.
///
/// # Errors
///
/// Returns an error for host discovery, timeout, malformed output, or native
/// driver failure.
pub fn run_profile_host(
    executable: &Path,
    session: &ProfileCaptureAuthorized,
    owner_window: Option<isize>,
) -> Result<Option<WindowsNativeProfileCapture>, ShellError> {
    let existing = session
        .native_configuration
        .as_ref()
        .map(decode_native_seed)
        .transpose()?;
    let request_id = Uuid::new_v4().to_string();
    let request = ProfileHostRequest {
        protocol_version: WINDOWS_PROFILE_HOST_PROTOCOL_VERSION,
        request_id: request_id.clone(),
        capture_token: session.capture_token.clone(),
        operation: ProfileHostOperation::Capture {
            native_printer_id: session.native_id.clone(),
            owner_window,
            existing,
        },
    };
    let encoded = serde_json::to_vec(&request).map_err(ShellError::Json)?;
    let mut child = Command::new(executable)
        .env("SPOOL_PROFILE_CAPTURE_TOKEN", &session.capture_token)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| {
            ShellError::ProfileHost(format!("cannot launch {}: {error}", executable.display()))
        })?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| ShellError::ProfileHost("profile host stdout is unavailable".into()))?;
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut output = Vec::new();
        let result = stdout
            .take(PROFILE_HOST_OUTPUT_LIMIT as u64 + 1)
            .read_to_end(&mut output)
            .map(|_| output);
        let _ignored = sender.send(result);
    });
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| ShellError::ProfileHost("profile host stdin is unavailable".into()))?;
    stdin.write_all(&encoded).map_err(ShellError::Io)?;
    drop(stdin);

    let deadline = Instant::now() + PROFILE_HOST_TIMEOUT;
    let exit_status = loop {
        if let Some(status) = child.try_wait().map_err(ShellError::Io)? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ignored = child.kill();
            let _ignored = child.wait();
            return Err(ShellError::ProfileHost(
                "printer settings dialog exceeded the 10-minute limit".into(),
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let output = receiver
        .recv_timeout(Duration::from_secs(2))
        .map_err(|_| ShellError::ProfileHost("profile host output was not closed".into()))?
        .map_err(ShellError::Io)?;
    if output.len() > PROFILE_HOST_OUTPUT_LIMIT {
        return Err(ShellError::ResponseTooLarge);
    }
    if !exit_status.success() && output.is_empty() {
        return Err(ShellError::ProfileHost(format!(
            "profile host exited with {exit_status}"
        )));
    }
    let response: ProfileHostResponse =
        serde_json::from_slice(&output).map_err(ShellError::Json)?;
    if response.protocol_version != WINDOWS_PROFILE_HOST_PROTOCOL_VERSION
        || response.request_id != request_id
    {
        return Err(ShellError::Protocol(
            "profile host returned a mismatched response".into(),
        ));
    }
    match response
        .result
        .map_err(|error| ShellError::ProfileHost(error.to_string()))?
    {
        ProfileHostResult::Captured { capture } => Ok(Some(capture)),
        ProfileHostResult::Cancelled => Ok(None),
        ProfileHostResult::Validated { .. } => Err(ShellError::Protocol(
            "profile host validated instead of capturing".into(),
        )),
    }
}

/// Builds the local-agent capture payload without exposing the native DEVMODE
/// as portable job options.
///
/// # Errors
///
/// Returns an error if the immutable envelope cannot be encoded.
pub fn capture_payload(
    session: &ProfileCaptureAuthorized,
    capture: &WindowsNativeProfileCapture,
    is_default: bool,
) -> Result<NativeProfileCapturePayload, ShellError> {
    let native_blob = serde_json::to_vec(capture).map_err(ShellError::Json)?;
    let summary = &capture.summary;
    let mut native = BTreeMap::new();
    add_native_value(&mut native, "paper_size", summary.paper_size);
    add_native_value(&mut native, "source", summary.source);
    add_native_value(&mut native, "print_quality", summary.print_quality);
    add_native_value(&mut native, "color", summary.color);
    add_native_value(&mut native, "duplex", summary.duplex);
    add_native_value(&mut native, "y_resolution", summary.y_resolution);
    add_native_value(&mut native, "collate", summary.collate);
    let profile_summary = ProfileSummary {
        paper: summary.form_name.clone(),
        dimensions_mm: summary
            .paper_width_tenths_mm
            .zip(summary.paper_length_tenths_mm)
            .map(|(width, length)| [f64::from(width) / 10.0, f64::from(length) / 10.0]),
        source: summary.source.map(|source| source.to_string()),
        color: summary.color.map(|color| color.to_string()),
        duplex: summary.duplex.map(|duplex| duplex.to_string()),
        resolution: summary
            .y_resolution
            .or(summary.print_quality)
            .map(|resolution| resolution.to_string()),
        copies: summary.copies.and_then(|copies| u32::try_from(copies).ok()),
        native,
        details: BTreeMap::from([
            (
                "driver_environment".into(),
                serde_json::Value::String(capture.fingerprint.driver_environment.clone()),
            ),
            (
                "devmode_public_bytes".into(),
                serde_json::Value::from(summary.public_bytes),
            ),
            (
                "devmode_private_bytes".into(),
                serde_json::Value::from(summary.private_bytes),
            ),
        ]),
        ..ProfileSummary::default()
    };
    let name = suggested_profile_name(session);
    let fingerprint = &capture.fingerprint;
    Ok(NativeProfileCapturePayload {
        name,
        is_default,
        options: JobOptions::default(),
        native_kind: NativeProfileKind::WindowsDevmode,
        native_schema_version: capture.schema_version,
        native_digest: format!("sha256:{}", hex::encode(Sha256::digest(&native_blob))),
        native_blob_base64: STANDARD.encode(native_blob),
        driver_fingerprint: DriverFingerprint {
            platform: fingerprint.platform.clone(),
            driver_name: fingerprint.driver_name.clone(),
            driver_version: Some(fingerprint.driver_version.clone()),
            architecture: Some(fingerprint.architecture.clone()),
            native_queue_id: fingerprint.native_queue_id.clone(),
            device_fingerprint: Some(fingerprint.device_fingerprint.clone()),
            driver_package_fingerprint: None,
        },
        summary: profile_summary,
        stock_id: session.stock_id.clone(),
        dependencies: vec![ProfileDependency {
            kind: "windows_driver".into(),
            value: format!(
                "{} {} ({})",
                fingerprint.driver_name, fingerprint.driver_version, fingerprint.driver_environment
            ),
        }],
        safe_overrides: if session.safe_overrides.is_empty() {
            vec![SafeProfileOverride::Copies, SafeProfileOverride::Pages]
        } else {
            session.safe_overrides.clone()
        },
        published: false,
    })
}

fn decode_native_seed(
    seed: &spool_local_ipc::NativeProfileSeed,
) -> Result<WindowsNativeProfileCapture, ShellError> {
    if seed.kind != NativeProfileKind::WindowsDevmode {
        return Err(ShellError::Protocol(format!(
            "profile uses {:?}, not a Windows DEVMODE",
            seed.kind
        )));
    }
    let bytes = STANDARD
        .decode(&seed.native_blob_base64)
        .map_err(|_| ShellError::Protocol("stored native profile is not valid Base64".into()))?;
    let digest = format!("sha256:{}", hex::encode(Sha256::digest(&bytes)));
    if digest != seed.digest {
        return Err(ShellError::Protocol(
            "stored native profile digest does not match its content".into(),
        ));
    }
    serde_json::from_slice(&bytes).map_err(ShellError::Json)
}

fn suggested_profile_name(session: &ProfileCaptureAuthorized) -> String {
    let current = session.profile_name.as_deref().unwrap_or("").trim();
    match session.operation {
        ProfileCaptureOperation::Create => {
            if current.is_empty() {
                format!("{} profile", session.printer_name)
            } else {
                current.to_owned()
            }
        }
        ProfileCaptureOperation::Edit => current.to_owned(),
        ProfileCaptureOperation::Clone => {
            if current.is_empty() {
                format!("{} profile copy", session.printer_name)
            } else {
                format!("{current} copy")
            }
        }
    }
}

fn add_native_value<T: ToString>(
    destination: &mut BTreeMap<String, String>,
    name: &str,
    value: Option<T>,
) {
    if let Some(value) = value {
        destination.insert(name.into(), value.to_string());
    }
}

fn path_segment(value: &str) -> Result<&str, ShellError> {
    if value.is_empty() || matches!(value, "." | "..") || value.contains(['/', '\\']) {
        return Err(ShellError::Protocol(
            "local agent returned an invalid resource identifier".into(),
        ));
    }
    Ok(value)
}

#[derive(Debug, Error)]
pub enum ShellError {
    #[error("{0}")]
    Configuration(String),
    #[error("{0}")]
    Token(String),
    #[error("local API returned {status}: {message}")]
    Agent { status: u16, message: String },
    #[error("local API response exceeded its safety limit")]
    ResponseTooLarge,
    #[error("local I/O failed: {0}")]
    Io(std::io::Error),
    #[error("invalid JSON response: {0}")]
    Json(serde_json::Error),
    #[error("{0}")]
    Protocol(String),
    #[error("{0}")]
    ProfileHost(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use spool_domain::{DriverFingerprint, ProfileCaptureOperation};
    use std::net::TcpListener;

    #[test]
    fn configuration_rejects_non_loopback_origins() {
        for url in [
            "https://127.0.0.1:39100",
            "http://192.168.1.20:39100",
            "http://localhost.evil:39100",
            "http://user:pass@localhost:39100",
            "http://localhost:39100/path",
        ] {
            let result = LocalApiConfiguration::from_values([(
                OsString::from("SPOOL_LOCAL_API_URL"),
                OsString::from(url),
            )]);
            assert!(result.is_err(), "{url} should be rejected");
        }
    }

    #[test]
    fn configuration_uses_program_data_by_default() {
        let configuration = LocalApiConfiguration::from_values([(
            OsString::from("ProgramData"),
            OsString::from(r"C:\ProgramData"),
        )])
        .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            configuration.token_file,
            PathBuf::from(r"C:\ProgramData").join("Spool/local.token")
        );
    }

    #[test]
    fn capture_payload_wraps_the_exact_envelope_and_maps_summary() {
        let mut bytes = vec![0_u8; 220];
        bytes[68..70].copy_from_slice(&220_u16.to_le_bytes());
        bytes[70..72].copy_from_slice(&0_u16.to_le_bytes());
        bytes[78..80].copy_from_slice(&9_i16.to_le_bytes());
        bytes[80..82].copy_from_slice(&2970_i16.to_le_bytes());
        bytes[82..84].copy_from_slice(&2100_i16.to_le_bytes());
        bytes[86..88].copy_from_slice(&2_i16.to_le_bytes());
        let native = WindowsNativeProfileCapture::new(
            spool_executor_windows::native_profile::WindowsDriverFingerprint {
                platform: "windows".into(),
                driver_name: "OKI PostScript".into(),
                driver_version: "1.2.3".into(),
                driver_environment: "Windows x64".into(),
                architecture: "x86_64".into(),
                native_queue_id: "OKI C9500".into(),
                device_fingerprint: "sha256:device".into(),
                driver_date_filetime: Some(42),
            },
            &bytes,
        )
        .unwrap_or_else(|error| panic!("{error}"));
        let session = ProfileCaptureAuthorized {
            session_id: "pcs_1".into(),
            capture_token: "secret".into(),
            expires_unix_ms: 1,
            operation: ProfileCaptureOperation::Create,
            printer_id: "ptr_1".into(),
            native_id: "OKI C9500".into(),
            printer_name: "OKI C9500".into(),
            profile_id: None,
            profile_name: None,
            stock_id: Some("stock_label".into()),
            safe_overrides: vec![SafeProfileOverride::Copies],
            expected_revision: None,
            native_configuration: None,
        };

        let payload =
            capture_payload(&session, &native, true).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(payload.name, "OKI C9500 profile");
        assert!(payload.is_default);
        assert_eq!(payload.native_kind, NativeProfileKind::WindowsDevmode);
        assert_eq!(payload.summary.dimensions_mm, Some([210.0, 297.0]));
        assert_eq!(payload.summary.copies, Some(2));
        assert_eq!(payload.stock_id.as_deref(), Some("stock_label"));
        assert_eq!(payload.safe_overrides, vec![SafeProfileOverride::Copies]);
        let decoded = STANDARD
            .decode(&payload.native_blob_base64)
            .unwrap_or_else(|error| panic!("{error}"));
        let restored: WindowsNativeProfileCapture =
            serde_json::from_slice(&decoded).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(restored, native);
        assert_eq!(
            payload.native_digest,
            format!("sha256:{}", hex::encode(Sha256::digest(decoded)))
        );
    }

    #[test]
    fn shell_error_does_not_format_capture_tokens() {
        let session = ProfileCaptureAuthorized {
            session_id: "pcs_1".into(),
            capture_token: "capture-secret".into(),
            expires_unix_ms: 1,
            operation: ProfileCaptureOperation::Create,
            printer_id: "ptr_1".into(),
            native_id: "Printer".into(),
            printer_name: "Printer".into(),
            profile_id: None,
            profile_name: None,
            stock_id: None,
            safe_overrides: Vec::new(),
            expected_revision: None,
            native_configuration: Some(spool_local_ipc::NativeProfileSeed {
                kind: NativeProfileKind::MacosPrintcore,
                schema_version: 1,
                digest: "sha256:bad".into(),
                native_blob_base64: String::new(),
            }),
        };
        let error = decode_native_seed(
            session
                .native_configuration
                .as_ref()
                .unwrap_or_else(|| panic!("missing seed")),
        )
        .unwrap_err();
        assert!(!format!("{error:?}").contains("capture-secret"));
        let _fingerprint = DriverFingerprint::default();
    }

    #[test]
    fn client_authenticates_a_bounded_loopback_status_request() {
        let listener =
            TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap_or_else(|error| panic!("{error}"));
        let address = listener
            .local_addr()
            .unwrap_or_else(|error| panic!("{error}"));
        let token_file = std::env::temp_dir().join(format!("spool-shell-token-{}", Uuid::new_v4()));
        fs::write(&token_file, "local-secret\n").unwrap_or_else(|error| panic!("{error}"));
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap_or_else(|error| panic!("{error}"));
            let mut request = Vec::new();
            loop {
                let mut byte = [0_u8; 1];
                stream
                    .read_exact(&mut byte)
                    .unwrap_or_else(|error| panic!("{error}"));
                request.push(byte[0]);
                if request.ends_with(b"\r\n\r\n") {
                    break;
                }
                assert!(request.len() < 16 * 1024);
            }
            let request = String::from_utf8(request).unwrap_or_else(|error| panic!("{error}"));
            assert!(request.starts_with("GET /v1/local/status HTTP/1.1\r\n"));
            assert!(request.contains("\r\nAuthorization: Bearer local-secret\r\n"));
            let body = br#"{"agent_id":"agt_1","workspace_name":"Test","version":"0.1.0","connection":"connected","queued_jobs":0,"active_jobs":0,"printer_warnings":0,"paused":false}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap_or_else(|error| panic!("{error}"));
            stream
                .write_all(body)
                .unwrap_or_else(|error| panic!("{error}"));
        });
        let client = LocalAgentClient::new(LocalApiConfiguration {
            base_url: format!("http://{address}"),
            token_file: token_file.clone(),
            address,
        })
        .unwrap_or_else(|error| panic!("{error}"));

        let status = client.status().unwrap_or_else(|error| panic!("{error}"));

        assert_eq!(status.agent_id.as_deref(), Some("agt_1"));
        assert_eq!(
            status.connection,
            spool_local_ipc::ConnectionState::Connected
        );
        server.join().unwrap_or_else(|error| panic!("{error:?}"));
        fs::remove_file(token_file).unwrap_or_else(|error| panic!("{error}"));
    }

    #[test]
    fn chunked_http_responses_are_decoded_without_trailers_entering_json() {
        let response =
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\n{\"ok\r\n5\r\n\":1}\n\r\n0\r\n\r\n";
        let (status, body) =
            decode_http_response(response).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(status, 200);
        assert_eq!(body, b"{\"ok\":1}\n");
    }
}
