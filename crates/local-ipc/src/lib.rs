//! Versioned local control contract for disposable native tray/menu shells.
//!
//! The shell has no database or cloud credential access. It can only request
//! bounded operational actions through this contract.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use uuid::Uuid;

pub const LOCAL_PROTOCOL_VERSION: u16 = 1;
pub const MAX_MESSAGE_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalRequest {
    pub protocol: u16,
    pub request_id: Uuid,
    pub challenge: String,
    pub operation: LocalOperation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LocalOperation {
    Status,
    Printers,
    Pause,
    Resume,
    RestartAgent,
    ExportSupportBundle { destination: PathBuf },
    Reenrol { confirmation: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalResponse {
    pub protocol: u16,
    pub request_id: Uuid,
    pub result: Result<LocalResult, LocalFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LocalResult {
    Status(LocalStatus),
    Printers { printers: Vec<LocalPrinter> },
    Accepted,
    SupportBundle { path: PathBuf },
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalPrinter {
    pub printer_id: String,
    pub name: String,
    pub state: String,
    pub is_default: bool,
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
    pub fn path(&self) -> &Path {
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
