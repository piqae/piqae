//! Operating-system verified identity for local broker peers.
//!
//! The wire request contains an application identity only as a mismatch check.
//! Authorization is always keyed to evidence collected from the accepted IPC
//! connection by the server. Callers cannot construct verified evidence.

use crate::BrokerApplicationIdentity;

#[cfg(target_os = "macos")]
mod apple;
#[cfg(windows)]
mod windows;

/// A principal proved by the operating system for one accepted connection.
///
/// The principal digest deliberately excludes the PID. It identifies the
/// signed application across launches, while `process_instance_sha256` binds
/// this evidence to the kernel process instance used for this connection.
#[derive(Clone, PartialEq, Eq)]
pub struct PeerApplicationEvidence {
    application: BrokerApplicationIdentity,
    principal_sha256: String,
    process_instance_sha256: String,
    platform: &'static str,
    process_id: u32,
}

impl std::fmt::Debug for PeerApplicationEvidence {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PeerApplicationEvidence")
            .field("application", &self.application)
            .field("principal_sha256", &self.principal_sha256)
            .field("process_instance_sha256", &"<redacted>")
            .field("platform", &self.platform)
            .field("process_id", &self.process_id)
            .finish()
    }
}

impl PeerApplicationEvidence {
    #[must_use]
    pub const fn application(&self) -> &BrokerApplicationIdentity {
        &self.application
    }

    #[must_use]
    pub fn principal_sha256(&self) -> &str {
        &self.principal_sha256
    }

    #[must_use]
    pub fn process_instance_sha256(&self) -> &str {
        &self.process_instance_sha256
    }

    #[must_use]
    pub const fn platform(&self) -> &'static str {
        self.platform
    }

    #[must_use]
    pub const fn process_id(&self) -> u32 {
        self.process_id
    }

    #[cfg(any(test, feature = "test-peer-identity"))]
    #[doc(hidden)]
    #[must_use]
    pub fn deterministic_test_identity(
        application_id: &str,
        display_name: &str,
        signer: &str,
        process_instance: &str,
    ) -> Self {
        use sha2::{Digest as _, Sha256};

        let signing_identity_sha256 = hex::encode(Sha256::digest(signer.as_bytes()));
        Self {
            application: BrokerApplicationIdentity {
                application_id: application_id.to_owned(),
                display_name: display_name.to_owned(),
                signing_identity_sha256: Some(signing_identity_sha256),
            },
            principal_sha256: hex::encode(Sha256::digest(
                [b"piqae-test-principal-v1\0".as_slice(), signer.as_bytes()].concat(),
            )),
            process_instance_sha256: hex::encode(Sha256::digest(process_instance.as_bytes())),
            platform: "test",
            process_id: 7,
        }
    }

    #[cfg(any(target_os = "macos", windows))]
    const fn verified(
        application: BrokerApplicationIdentity,
        principal_sha256: String,
        process_instance_sha256: String,
        platform: &'static str,
        process_id: u32,
    ) -> Self {
        Self {
            application,
            principal_sha256,
            process_instance_sha256,
            platform,
            process_id,
        }
    }
}

/// Keeps any OS process handle used to verify the peer alive while the broker
/// handles the request. This closes the PID-reuse window on Windows.
#[derive(Debug)]
pub struct VerifiedPeerConnection {
    evidence: PeerApplicationEvidence,
    #[cfg(windows)]
    _process: windows::OwnedProcess,
}

impl VerifiedPeerConnection {
    #[must_use]
    pub const fn evidence(&self) -> &PeerApplicationEvidence {
        &self.evidence
    }
}

#[cfg(target_os = "macos")]
/// Verifies the signed process attached to an accepted macOS Unix socket.
///
/// # Errors
///
/// Fails closed when peer credentials, audit-token identity, code validity or
/// signing information cannot be obtained and matched.
pub fn verify_unix_peer(
    stream: &tokio::net::UnixStream,
) -> std::io::Result<VerifiedPeerConnection> {
    apple::verify(stream).map(|evidence| VerifiedPeerConnection { evidence })
}

#[cfg(all(unix, not(target_os = "macos")))]
/// Rejects broker authorization on Unix platforms without a signed-peer
/// verifier. Presence remains available through the broker transport.
///
/// # Errors
///
/// Always returns `Unsupported` on this platform.
pub fn verify_unix_peer(
    _stream: &tokio::net::UnixStream,
) -> std::io::Result<VerifiedPeerConnection> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "signed local broker peer verification is not available on this Unix platform",
    ))
}

#[cfg(windows)]
/// Verifies the process attached to an accepted Windows named-pipe instance.
///
/// # Errors
///
/// Fails closed when the client PID, held process, user/session, package family
/// or Authenticode identity cannot be verified.
pub fn verify_windows_peer(
    pipe: &tokio::net::windows::named_pipe::NamedPipeServer,
) -> std::io::Result<VerifiedPeerConnection> {
    let (evidence, process) = windows::verify(pipe)?;
    Ok(VerifiedPeerConnection {
        evidence,
        _process: process,
    })
}

#[cfg(any(test, feature = "test-peer-identity"))]
#[doc(hidden)]
#[must_use]
pub fn deterministic_test_connection(
    application_id: &str,
    signer: &str,
    process_instance: &str,
) -> VerifiedPeerConnection {
    VerifiedPeerConnection {
        evidence: PeerApplicationEvidence::deterministic_test_identity(
            application_id,
            application_id,
            signer,
            process_instance,
        ),
        #[cfg(windows)]
        _process: windows::OwnedProcess::test_sentinel(),
    }
}

#[cfg(all(test, target_os = "macos"))]
#[allow(clippy::unwrap_used)]
mod tests {
    #[tokio::test]
    async fn accepted_socket_identity_is_derived_from_the_kernel_peer() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("peer.sock");
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let client = tokio::spawn(async move { tokio::net::UnixStream::connect(path).await });
        let (server, _) = listener.accept().await.unwrap();
        let verified = super::verify_unix_peer(&server).unwrap();
        let client = client.await.unwrap().unwrap();

        assert_eq!(verified.evidence().platform(), "macos");
        assert_eq!(verified.evidence().process_id(), std::process::id());
        assert_eq!(
            client.peer_cred().unwrap().uid(),
            server.peer_cred().unwrap().uid()
        );
        assert_eq!(verified.evidence().principal_sha256().len(), 64);
        assert_eq!(verified.evidence().process_instance_sha256().len(), 64);
    }
}
