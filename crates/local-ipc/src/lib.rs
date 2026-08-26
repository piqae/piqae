//! Versioned local control contract for disposable native tray/menu shells.
//!
//! The shell has no database or cloud credential access. It can only request
//! bounded operational actions through this contract.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use piqae_domain::{
    DriverFingerprint, JobOptions, NativePrinterOption, NativeProfileKind, PrinterCapabilities,
    ProfileCaptureOperation, ProfileDependency, ProfileStatus, ProfileSummary, SafeProfileOverride,
};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::PathBuf;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use uuid::Uuid;

#[cfg(windows)]
mod windows_pipe;

#[cfg(windows)]
pub use windows_pipe::create_current_user_server as create_current_user_pipe_server;

#[must_use]
pub fn broker_endpoint_for_data_directory(data_directory: &std::path::Path) -> String {
    #[cfg(unix)]
    {
        data_directory
            .join("runtime")
            .join("node.sock")
            .to_string_lossy()
            .into_owned()
    }
    #[cfg(windows)]
    {
        let digest = Sha256::digest(data_directory.as_os_str().to_string_lossy().as_bytes());
        format!(r"\\.\pipe\piqae-node-{}", hex::encode(&digest[..12]))
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = data_directory;
        "piqae-node".to_owned()
    }
}

pub const LOCAL_PROTOCOL_VERSION: u16 = 2;
pub const BROKER_PROTOCOL_MIN_VERSION: u16 = 1;
pub const BROKER_PROTOCOL_VERSION: u16 = 3;
pub const MAX_MESSAGE_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_NATIVE_CAPTURE_BYTES: usize = 1024 * 1024;

/// Non-sensitive broker discovery result. Presence never reveals an installed
/// node's tenants, connectors, printers or installation identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrokerPresence {
    pub protocol_min: u16,
    pub protocol_max: u16,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrokerCredential {
    pub application_id: String,
    pub token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerApplicationIdentity {
    pub application_id: String,
    pub display_name: String,
    /// Evidence shown to the operator. A claimed digest never grants access.
    pub signing_identity_sha256: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrokerAuthorizationHandle {
    pub authorization_id: Uuid,
    pub nonce: String,
    pub expires_unix_ms: i64,
}

impl std::fmt::Debug for BrokerAuthorizationHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BrokerAuthorizationHandle")
            .field("authorization_id", &self.authorization_id)
            .field("nonce", &"[REDACTED]")
            .field("expires_unix_ms", &self.expires_unix_ms)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrokerAuthorizationState {
    Pending,
    Approved,
    Denied,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingBrokerAuthorization {
    pub authorization_id: Uuid,
    pub application: BrokerApplicationIdentity,
    pub requested_capabilities: Vec<BrokerCapability>,
    pub requested_unix_ms: i64,
    pub expires_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerAuthorizationDecision {
    pub approved: bool,
    #[serde(default)]
    pub granted_capabilities: Vec<BrokerCapability>,
}

impl std::fmt::Debug for BrokerCredential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BrokerCredential")
            .field("application_id", &self.application_id)
            .field("token", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrokerCapability {
    ObserveStatus,
    ObservePrinters,
    ManageProfiles,
    SubmitLocalJobs,
    ManageConnectors,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrokerRequest {
    pub protocol: u16,
    pub request_id: Uuid,
    pub operation: BrokerOperation,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(
    clippy::large_enum_variant,
    reason = "wire-compatible operation variants are bounded by MAX_MESSAGE_BYTES and boxing would complicate generated SDK schemas"
)]
pub enum BrokerOperation {
    Presence,
    RequestAuthorization {
        application: BrokerApplicationIdentity,
        requested_capabilities: Vec<BrokerCapability>,
    },
    AuthorizationStatus {
        handle: BrokerAuthorizationHandle,
    },
    ExchangeAuthorization {
        handle: BrokerAuthorizationHandle,
    },
    Execute {
        credential: BrokerCredential,
        capability: BrokerCapability,
        operation: LocalOperation,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrokerResponse {
    pub protocol: u16,
    pub request_id: Uuid,
    pub result: Result<BrokerResult, LocalFailure>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BrokerResult {
    Presence(BrokerPresence),
    AuthorizationRequested(BrokerAuthorizationHandle),
    AuthorizationStatus { state: BrokerAuthorizationState },
    AuthorizationExchanged(BrokerCredential),
    Local(LocalResult),
}

/// Generates a one-time 256-bit bearer token and the digest that may be
/// persisted by the agent. The plaintext token is returned only to the
/// authorized native profile host.
#[must_use]
pub fn generate_capture_token() -> (String, String) {
    let mut token = [0_u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut token);
    let token = URL_SAFE_NO_PAD.encode(token);
    let digest = capture_token_digest(&token);
    (token, digest)
}

/// Produces the stable storage representation of a profile capture token.
#[must_use]
pub fn capture_token_digest(token: &str) -> String {
    format!("sha256:{:x}", Sha256::digest(token.as_bytes()))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalRequest {
    pub protocol: u16,
    pub request_id: Uuid,
    pub challenge: String,
    pub operation: LocalOperation,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LocalOperation {
    Status,
    Printers,
    Pause,
    Resume,
    RestartAgent,
    ExportSupportBundle { destination: PathBuf },
    Reenrol { confirmation: String },
    BeginProfileCapture(BeginProfileCapture),
    CommitProfileCapture(Box<CommitProfileCapture>),
    CancelProfileCapture(CancelProfileCapture),
    ValidateProfile(ValidateProfile),
    ConfirmLoadedMedia(ConfirmLoadedMedia),
    Sdk { operation: SdkBrokerOperation },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(
    clippy::large_enum_variant,
    reason = "bounded SDK wire operation keeps the generated schema direct"
)]
pub enum SdkBrokerOperation {
    SubmitLocalJob {
        printer_id: String,
        title: String,
        content_kind: piqae_domain::ContentKind,
        content_base64: String,
        #[serde(default)]
        options: JobOptions,
        expires_unix_ms: Option<i64>,
    },
    Profiles {
        printer_id: String,
    },
    JobHistory {
        offset: usize,
        limit: usize,
    },
    ConnectorSnapshots,
    RevokeConnector {
        connector_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalResponse {
    pub protocol: u16,
    pub request_id: Uuid,
    pub result: Result<LocalResult, LocalFailure>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LocalResult {
    Status(LocalStatus),
    Printers {
        printers: Vec<LocalPrinter>,
    },
    Accepted,
    SupportBundle {
        path: PathBuf,
    },
    ProfileCaptureAuthorized(Box<ProfileCaptureAuthorized>),
    ProfileCaptured {
        profile: Box<LocalPrinterProfile>,
    },
    ProfileValidation(ProfileValidationResult),
    /// Additive SDK result whose inner schema is selected by the matching
    /// `SdkBrokerOperation`. Secrets remain forbidden from this projection.
    Sdk {
        data: serde_json::Value,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalStatus {
    pub agent_id: Option<String>,
    pub workspace_name: Option<String>,
    pub version: String,
    pub connection: ConnectionState,
    pub queued_jobs: u32,
    pub active_jobs: u32,
    pub printer_warnings: u32,
    pub paused: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    LocalOnly,
    Connected,
    Connecting,
    Offline,
    Degraded,
    /// The control plane rejected this node's identity.
    ///
    /// Distinct from `Offline` because retrying does not help: the node has
    /// been revoked, or its key no longer matches the enrolled one, and an
    /// operator must re-pair it. Reporting this as `Offline` sends people
    /// looking for a network fault that does not exist.
    Unauthorized,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalPrinter {
    pub printer_id: String,
    pub native_id: String,
    pub name: String,
    pub state: String,
    pub is_default: bool,
    pub exposed: bool,
    pub capability_revision: u64,
    pub capabilities: PrinterCapabilities,
    pub native_options: BTreeMap<String, NativePrinterOption>,
    pub profiles: Vec<LocalPrinterProfile>,
    pub queue_counts: LocalPrinterQueueCounts,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalPrinterProfile {
    pub profile_id: String,
    pub revision: u64,
    pub name: String,
    pub is_default: bool,
    pub options: JobOptions,
    #[serde(default)]
    pub status: ProfileStatus,
    #[serde(default)]
    pub native_kind: Option<NativeProfileKind>,
    #[serde(default)]
    pub native_digest: Option<String>,
    #[serde(default)]
    pub driver_fingerprint: DriverFingerprint,
    #[serde(default)]
    pub summary: ProfileSummary,
    #[serde(default)]
    pub stock_id: Option<String>,
    #[serde(default)]
    pub dependencies: Vec<ProfileDependency>,
    #[serde(default)]
    pub safe_overrides: Vec<SafeProfileOverride>,
    #[serde(default)]
    pub last_validated_unix_ms: Option<i64>,
    #[serde(default)]
    pub last_test_job_id: Option<String>,
    #[serde(default)]
    pub published: bool,
    /// True only when this profile deliberately follows the operating
    /// system driver's current defaults instead of replaying a saved native
    /// configuration.
    #[serde(default)]
    pub uses_current_printer_defaults: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BeginProfileCapture {
    pub printer_id: String,
    pub operation: ProfileCaptureOperation,
    pub profile_id: Option<String>,
    pub expected_revision: Option<u64>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileCaptureAuthorized {
    pub session_id: String,
    pub capture_token: String,
    pub expires_unix_ms: i64,
    pub operation: ProfileCaptureOperation,
    pub printer_id: String,
    pub native_id: String,
    pub printer_name: String,
    pub profile_id: Option<String>,
    pub profile_name: Option<String>,
    pub stock_id: Option<String>,
    #[serde(default)]
    pub safe_overrides: Vec<SafeProfileOverride>,
    pub expected_revision: Option<u64>,
    /// The prior immutable revision for edit/clone. This response is available
    /// only through the authenticated, loopback-only local API.
    pub native_configuration: Option<NativeProfileSeed>,
}

impl std::fmt::Debug for ProfileCaptureAuthorized {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProfileCaptureAuthorized")
            .field("session_id", &self.session_id)
            .field("capture_token", &"[REDACTED]")
            .field("expires_unix_ms", &self.expires_unix_ms)
            .field("operation", &self.operation)
            .field("printer_id", &self.printer_id)
            .field("native_id", &self.native_id)
            .field("printer_name", &self.printer_name)
            .field("profile_id", &self.profile_id)
            .field("profile_name", &self.profile_name)
            .field("stock_id", &self.stock_id)
            .field("safe_overrides", &self.safe_overrides)
            .field("expected_revision", &self.expected_revision)
            .field("native_configuration", &self.native_configuration)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeProfileSeed {
    pub kind: NativeProfileKind,
    pub schema_version: u16,
    pub digest: String,
    /// Standard Base64. Kept inside the short-lived authenticated response so
    /// the native profile host can restore the exact prior driver state.
    pub native_blob_base64: String,
}

impl std::fmt::Debug for NativeProfileSeed {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NativeProfileSeed")
            .field("kind", &self.kind)
            .field("schema_version", &self.schema_version)
            .field("digest", &self.digest)
            .field(
                "native_blob_bytes_estimate",
                &(self.native_blob_base64.len().saturating_mul(3) / 4),
            )
            .finish()
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct CommitProfileCapture {
    pub session_id: String,
    pub capture_token: String,
    pub capture: NativeProfileCapturePayload,
}

impl std::fmt::Debug for CommitProfileCapture {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CommitProfileCapture")
            .field("session_id", &self.session_id)
            .field("capture_token", &"[REDACTED]")
            .field("capture", &self.capture.redacted())
            .finish()
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct NativeProfileCapturePayload {
    pub name: String,
    #[serde(default)]
    pub is_default: bool,
    #[serde(default)]
    pub options: JobOptions,
    pub native_kind: NativeProfileKind,
    pub native_schema_version: u16,
    pub native_digest: String,
    /// Standard Base64. Decoding and the one-MiB ceiling are enforced before
    /// the capture reaches durable storage.
    pub native_blob_base64: String,
    pub driver_fingerprint: DriverFingerprint,
    pub summary: ProfileSummary,
    pub stock_id: Option<String>,
    #[serde(default)]
    pub dependencies: Vec<ProfileDependency>,
    #[serde(default)]
    pub safe_overrides: Vec<SafeProfileOverride>,
    #[serde(default)]
    pub published: bool,
}

impl NativeProfileCapturePayload {
    fn redacted(&self) -> NativeProfileCaptureDebug<'_> {
        NativeProfileCaptureDebug {
            name: &self.name,
            native_kind: self.native_kind,
            native_schema_version: self.native_schema_version,
            native_digest: &self.native_digest,
            native_blob_bytes_estimate: self.native_blob_base64.len().saturating_mul(3) / 4,
        }
    }
}

impl std::fmt::Debug for NativeProfileCapturePayload {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.redacted().fmt(formatter)
    }
}

struct NativeProfileCaptureDebug<'a> {
    name: &'a str,
    native_kind: NativeProfileKind,
    native_schema_version: u16,
    native_digest: &'a str,
    native_blob_bytes_estimate: usize,
}

impl std::fmt::Debug for NativeProfileCaptureDebug<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NativeProfileCapture")
            .field("name", &self.name)
            .field("native_kind", &self.native_kind)
            .field("native_schema_version", &self.native_schema_version)
            .field("native_digest", &self.native_digest)
            .field(
                "native_blob_bytes_estimate",
                &self.native_blob_bytes_estimate,
            )
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelProfileCapture {
    pub session_id: String,
    pub capture_token: String,
}

impl std::fmt::Debug for CancelProfileCapture {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CancelProfileCapture")
            .field("session_id", &self.session_id)
            .field("capture_token", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidateProfile {
    pub profile_id: String,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileValidationResult {
    pub profile_id: String,
    pub revision: u64,
    pub status: ProfileStatus,
    pub code: Option<String>,
    pub message: Option<String>,
    pub validated_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfirmLoadedMedia {
    pub device_id: String,
    pub source: String,
    pub stock_id: Option<String>,
    pub confidence: piqae_domain::LoadedMediaConfidence,
    pub confirmed_by: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalPrinterQueueCounts {
    pub queued: u32,
    pub active: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalPrinterQueue {
    pub printer_id: String,
    pub local_jobs: Vec<LocalQueueJob>,
    pub native_jobs: Vec<LocalNativeQueueJob>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalQueueJob {
    pub job_id: String,
    pub sequence: i64,
    pub title: String,
    pub state: String,
    pub native_job_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalNativeQueueJob {
    pub native_job_id: String,
    pub title: String,
    pub user: Option<String>,
    pub state: String,
    pub native_code: Option<String>,
    pub size_kib: Option<u64>,
    pub created_unix_ms: Option<i64>,
    pub processing_unix_ms: Option<i64>,
    pub completed_unix_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalFailure {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

#[derive(Debug, Error)]
pub enum LocalIpcError {
    #[error("message size {0} exceeds the {MAX_MESSAGE_BYTES} byte limit")]
    MessageTooLarge(usize),
    #[error("local IPC stream ended before a complete message")]
    Truncated,
    #[error("local IPC I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid local IPC JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("local IPC endpoint path exists but is not a socket: {0}")]
    UnsafeExistingPath(PathBuf),
}

#[derive(Debug)]
pub struct SessionAuthenticator {
    challenge_digest: [u8; 32],
}

impl SessionAuthenticator {
    #[must_use]
    pub fn generate() -> (Self, String) {
        let mut challenge = [0_u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut challenge);
        let encoded = URL_SAFE_NO_PAD.encode(challenge);
        (Self::from_challenge(&encoded), encoded)
    }

    #[must_use]
    pub fn from_challenge(challenge: &str) -> Self {
        Self {
            challenge_digest: Sha256::digest(challenge.as_bytes()).into(),
        }
    }

    #[must_use]
    pub fn authenticate(&self, candidate: &str) -> bool {
        let candidate: [u8; 32] = Sha256::digest(candidate.as_bytes()).into();
        constant_time_eq(&candidate, &self.challenge_digest)
    }
}

fn constant_time_eq(left: &[u8; 32], right: &[u8; 32]) -> bool {
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (a, b)| difference | (a ^ b))
        == 0
}

/// Writes one bounded local IPC message.
///
/// # Errors
///
/// Returns an error if serialization fails, the body exceeds the protocol
/// limit, or the stream cannot be written and flushed.
pub async fn write_message<T: Serialize + Sync>(
    writer: &mut (impl AsyncWrite + Unpin + Send),
    value: &T,
) -> Result<(), LocalIpcError> {
    let body = serde_json::to_vec(value)?;
    if body.len() > MAX_MESSAGE_BYTES {
        return Err(LocalIpcError::MessageTooLarge(body.len()));
    }
    let size = u32::try_from(body.len()).map_err(|_| LocalIpcError::MessageTooLarge(body.len()))?;
    writer.write_all(&size.to_be_bytes()).await?;
    writer.write_all(&body).await?;
    writer.flush().await?;
    Ok(())
}

/// Reads one bounded local IPC message.
///
/// # Errors
///
/// Returns an error if the declared body exceeds the protocol limit, the
/// stream ends early, or the body is not valid JSON for `T`.
pub async fn read_message<T: serde::de::DeserializeOwned>(
    reader: &mut (impl AsyncRead + Unpin + Send),
) -> Result<T, LocalIpcError> {
    let size = reader.read_u32().await?;
    let size = usize::try_from(size).map_err(|_| LocalIpcError::MessageTooLarge(usize::MAX))?;
    if size > MAX_MESSAGE_BYTES {
        return Err(LocalIpcError::MessageTooLarge(size));
    }
    let mut body = vec![0_u8; size];
    reader.read_exact(&mut body).await?;
    Ok(serde_json::from_slice(&body)?)
}

#[cfg(unix)]
#[derive(Debug)]
pub struct LocalEndpoint {
    listener: tokio::net::UnixListener,
    path: PathBuf,
}

#[cfg(unix)]
impl LocalEndpoint {
    /// Binds a Unix socket after creating its private parent directory. A
    /// pre-existing non-socket path is never removed.
    ///
    /// # Errors
    ///
    /// Returns an error when the private directory or socket cannot be
    /// created, or when the requested path contains a non-socket entry.
    pub fn bind(path: impl Into<PathBuf>) -> Result<Self, LocalIpcError> {
        use std::os::unix::fs::{FileTypeExt, PermissionsExt};

        let path = path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
            std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
        }
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_socket() => std::fs::remove_file(&path)?,
            Ok(_) => return Err(LocalIpcError::UnsafeExistingPath(path)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let listener = tokio::net::UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        Ok(Self { listener, path })
    }

    /// Accepts one local IPC client.
    ///
    /// # Errors
    ///
    /// Returns an error if the operating system cannot accept the connection.
    pub async fn accept(&self) -> Result<tokio::net::UnixStream, LocalIpcError> {
        let (stream, _) = self.listener.accept().await?;
        Ok(stream)
    }

    #[must_use]
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    fn request(challenge: String) -> LocalRequest {
        LocalRequest {
            protocol: LOCAL_PROTOCOL_VERSION,
            request_id: Uuid::nil(),
            challenge,
            operation: LocalOperation::Status,
        }
    }

    #[test]
    fn broker_credentials_are_redacted_from_debug_output() {
        let credential = BrokerCredential {
            application_id: "com.example.pos".into(),
            token: "sensitive-token".into(),
        };
        let output = format!("{credential:?}");
        assert!(output.contains("com.example.pos"));
        assert!(!output.contains("sensitive-token"));
    }

    #[test]
    fn checked_in_broker_presence_fixtures_remain_compatible() {
        let request: BrokerRequest = serde_json::from_slice(include_bytes!(
            "../../../contracts/node-sdk/v1/broker-presence-request.json"
        ))
        .unwrap();
        assert!(matches!(request.operation, BrokerOperation::Presence));
        let response: BrokerResponse = serde_json::from_slice(include_bytes!(
            "../../../contracts/node-sdk/v1/broker-presence-response.json"
        ))
        .unwrap();
        assert!(matches!(
            response.result,
            Ok(BrokerResult::Presence(BrokerPresence {
                protocol_min: 1,
                protocol_max: 1
            }))
        ));
    }

    #[test]
    fn checked_in_broker_consent_fixtures_pin_protocol_two_without_trusting_claims() {
        let request: BrokerRequest = serde_json::from_slice(include_bytes!(
            "../../../contracts/node-sdk/v1/broker-authorization-request.json"
        ))
        .unwrap();
        assert_eq!(request.protocol, 2);
        assert!(matches!(
            request.operation,
            BrokerOperation::RequestAuthorization {
                application: BrokerApplicationIdentity { ref application_id, .. },
                ..
            } if application_id == "com.example.pos"
        ));
        let response: BrokerResponse = serde_json::from_slice(include_bytes!(
            "../../../contracts/node-sdk/v1/broker-authorization-requested-response.json"
        ))
        .unwrap();
        assert!(matches!(
            response.result,
            Ok(BrokerResult::AuthorizationRequested(_))
        ));
        for fixture in [
            include_bytes!(
                "../../../contracts/node-sdk/v1/broker-authorization-status-request.json"
            )
            .as_slice(),
            include_bytes!(
                "../../../contracts/node-sdk/v1/broker-authorization-exchange-request.json"
            )
            .as_slice(),
        ] {
            let request: BrokerRequest = serde_json::from_slice(fixture).unwrap();
            assert_eq!(request.protocol, 2);
        }
    }

    #[tokio::test]
    async fn codec_round_trips_over_split_stream() {
        let (mut client, mut server) = tokio::io::duplex(1024);
        let original = request("secret".into());
        let send = tokio::spawn(async move { write_message(&mut client, &original).await });
        let received: LocalRequest = read_message(&mut server).await.expect("read");
        send.await.expect("task").expect("write");
        assert_eq!(received, request("secret".into()));
    }

    #[test]
    fn session_authentication_rejects_wrong_challenge() {
        let (authenticator, challenge) = SessionAuthenticator::generate();
        assert!(authenticator.authenticate(&challenge));
        assert!(!authenticator.authenticate("wrong"));
    }

    #[test]
    fn capture_tokens_are_random_and_only_their_digest_needs_persistence() {
        let (first, first_digest) = generate_capture_token();
        let (second, second_digest) = generate_capture_token();
        assert_ne!(first, second);
        assert_ne!(first_digest, second_digest);
        assert_eq!(first_digest, capture_token_digest(&first));
        assert!(!first_digest.contains(&first));
    }

    #[test]
    fn native_profile_seed_debug_never_exposes_the_blob() {
        let seed = NativeProfileSeed {
            kind: NativeProfileKind::MacosPrintcore,
            schema_version: 1,
            digest: "sha256:test".into(),
            native_blob_base64: "sensitive-native-driver-state".into(),
        };
        let debug = format!("{seed:?}");
        assert!(!debug.contains("sensitive-native-driver-state"));
        assert!(debug.contains("native_blob_bytes_estimate"));
    }

    #[cfg(unix)]
    #[test]
    fn endpoint_never_removes_an_existing_regular_file() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("agent.sock");
        std::fs::write(&path, "do not remove").expect("write");
        assert!(matches!(
            LocalEndpoint::bind(&path),
            Err(LocalIpcError::UnsafeExistingPath(_))
        ));
        assert_eq!(
            std::fs::read_to_string(path).expect("read"),
            "do not remove"
        );
    }
}
