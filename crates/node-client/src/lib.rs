//! Typed application client for the authenticated local node broker.
//!
//! The client owns no queue, connector credential or retry state. Applications
//! either attach to the installed broker or use the disposition returned by
//! `resolve_runtime` to construct an app-scoped embedded runtime.

#![allow(
    clippy::missing_errors_doc,
    reason = "all public operations return the documented NodeClientError transport/protocol taxonomy"
)]

use async_trait::async_trait;
use piqae_local_ipc::{
    BROKER_PROTOCOL_MIN_VERSION, BROKER_PROTOCOL_VERSION, BrokerAuthorizationHandle,
    BrokerAuthorizationState, BrokerCapability, BrokerCredential, BrokerOperation, BrokerRequest,
    BrokerResponse, BrokerResult, LocalOperation, LocalPrinter, LocalResult, LocalStatus,
    broker_proof_key, broker_request_proof, broker_response_proof, constant_time_proof_eq,
};
use piqae_node_runtime::{
    AttachPolicy, BrokerEndpoint, RuntimeDisposition, RuntimeSelectionError, select_runtime,
};
#[cfg(unix)]
use std::path::Path;
use std::path::PathBuf;
use thiserror::Error;
use uuid::Uuid;

#[cfg(not(test))]
const BROKER_IO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
#[cfg(test)]
const BROKER_IO_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(100);

#[derive(Debug, Error)]
pub enum NodeClientError {
    #[error("local broker transport failed: {0}")]
    Transport(String),
    #[error("local broker request timed out")]
    Timeout,
    #[error("local broker returned a response for a different request")]
    ResponseIdMismatch,
    #[error("local broker protocol range {minimum}..={maximum} is incompatible")]
    UnsupportedProtocol { minimum: u16, maximum: u16 },
    #[error("local broker rejected the request: {code}: {message}")]
    Rejected {
        code: String,
        message: String,
        retryable: bool,
    },
    #[error("local broker response did not match the requested operation")]
    UnexpectedResponse,
    #[error("runtime selection failed: {0}")]
    Selection(#[from] RuntimeSelectionError),
}

#[derive(Debug, Clone)]
pub struct NodeAuthorizationClient<T> {
    transport: T,
}

impl<T: BrokerTransport> NodeAuthorizationClient<T> {
    #[must_use]
    pub const fn new(transport: T) -> Self {
        Self { transport }
    }

    pub async fn request(
        &self,
        requested_capabilities: Vec<BrokerCapability>,
    ) -> Result<BrokerAuthorizationHandle, NodeClientError> {
        match request_transport(
            &self.transport,
            BrokerOperation::RequestAuthorization {
                application: None,
                requested_capabilities,
            },
        )
        .await?
        {
            BrokerResult::AuthorizationRequested(handle) => Ok(handle),
            _ => Err(NodeClientError::UnexpectedResponse),
        }
    }

    pub async fn status(
        &self,
        handle: BrokerAuthorizationHandle,
    ) -> Result<BrokerAuthorizationState, NodeClientError> {
        match request_transport(
            &self.transport,
            BrokerOperation::AuthorizationStatus { handle },
        )
        .await?
        {
            BrokerResult::AuthorizationStatus { state } => Ok(state),
            _ => Err(NodeClientError::UnexpectedResponse),
        }
    }

    pub async fn exchange(
        &self,
        handle: BrokerAuthorizationHandle,
    ) -> Result<BrokerCredential, NodeClientError> {
        match request_transport(
            &self.transport,
            BrokerOperation::ExchangeAuthorization { handle },
        )
        .await?
        {
            BrokerResult::AuthorizationExchanged(credential) => Ok(credential),
            _ => Err(NodeClientError::UnexpectedResponse),
        }
    }
}

async fn request_transport<T: BrokerTransport>(
    transport: &T,
    operation: BrokerOperation,
) -> Result<BrokerResult, NodeClientError> {
    let request_id = Uuid::new_v4();
    let response = transport
        .request(BrokerRequest {
            protocol: BROKER_PROTOCOL_VERSION,
            request_id,
            operation,
        })
        .await?;
    if response.request_id != request_id {
        return Err(NodeClientError::ResponseIdMismatch);
    }
    if !(BROKER_PROTOCOL_MIN_VERSION..=BROKER_PROTOCOL_VERSION).contains(&response.protocol) {
        return Err(NodeClientError::UnsupportedProtocol {
            minimum: response.protocol,
            maximum: response.protocol,
        });
    }
    let result = response
        .result
        .map_err(|failure| NodeClientError::Rejected {
            code: failure.code,
            message: failure.message,
            retryable: failure.retryable,
        })?;
    if let BrokerResult::Presence(presence) = &result
        && (presence.protocol_min > presence.protocol_max
            || presence.protocol_max < BROKER_PROTOCOL_MIN_VERSION
            || presence.protocol_min > BROKER_PROTOCOL_VERSION)
    {
        return Err(NodeClientError::UnsupportedProtocol {
            minimum: presence.protocol_min,
            maximum: presence.protocol_max,
        });
    }
    Ok(result)
}

#[async_trait]
pub trait BrokerTransport: std::fmt::Debug + Send + Sync {
    async fn request(&self, request: BrokerRequest) -> Result<BrokerResponse, NodeClientError>;
}

#[derive(Clone)]
pub struct NodeClient<T> {
    transport: T,
    credential: BrokerCredential,
}

impl<T: std::fmt::Debug> std::fmt::Debug for NodeClient<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NodeClient")
            .field("transport", &self.transport)
            .field("credential", &self.credential)
            .finish()
    }
}

impl<T: BrokerTransport> NodeClient<T> {
    #[must_use]
    pub fn new(transport: T, application_id: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            transport,
            credential: BrokerCredential {
                application_id: application_id.into(),
                token: token.into(),
                granted_capabilities: Vec::new(),
            },
        }
    }

    #[must_use]
    pub const fn from_credential(transport: T, credential: BrokerCredential) -> Self {
        Self {
            transport,
            credential,
        }
    }

    #[must_use]
    pub fn granted_capabilities(&self) -> &[BrokerCapability] {
        &self.credential.granted_capabilities
    }

    pub async fn presence(&self) -> Result<piqae_local_ipc::BrokerPresence, NodeClientError> {
        match self.request(BrokerOperation::Presence).await? {
            BrokerResult::Presence(presence) => Ok(presence),
            _ => Err(NodeClientError::UnexpectedResponse),
        }
    }

    pub async fn status(&self) -> Result<LocalStatus, NodeClientError> {
        match self
            .execute(BrokerCapability::ObserveStatus, LocalOperation::Status)
            .await?
        {
            LocalResult::Status(status) => Ok(status),
            _ => Err(NodeClientError::UnexpectedResponse),
        }
    }

    pub async fn printers(&self) -> Result<Vec<LocalPrinter>, NodeClientError> {
        match self
            .execute(BrokerCapability::ObservePrinters, LocalOperation::Printers)
            .await?
        {
            LocalResult::Printers { printers } => Ok(printers),
            _ => Err(NodeClientError::UnexpectedResponse),
        }
    }

    pub async fn pause(&self) -> Result<(), NodeClientError> {
        self.accepted(BrokerCapability::ManageConnectors, LocalOperation::Pause)
            .await
    }

    pub async fn resume(&self) -> Result<(), NodeClientError> {
        self.accepted(BrokerCapability::ManageConnectors, LocalOperation::Resume)
            .await
    }

    async fn accepted(
        &self,
        capability: BrokerCapability,
        operation: LocalOperation,
    ) -> Result<(), NodeClientError> {
        match self.execute(capability, operation).await? {
            LocalResult::Accepted => Ok(()),
            _ => Err(NodeClientError::UnexpectedResponse),
        }
    }

    /// Executes one capability-bound operation using protocol-v4 request and
    /// response authentication. The bearer credential never crosses IPC.
    pub async fn execute_operation(
        &self,
        capability: BrokerCapability,
        operation: LocalOperation,
    ) -> Result<LocalResult, NodeClientError> {
        let request_id = Uuid::new_v4();
        let nonce = Uuid::new_v4().simple().to_string();
        let issued_unix_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| NodeClientError::UnexpectedResponse)?
            .as_millis()
            .try_into()
            .map_err(|_| NodeClientError::UnexpectedResponse)?;
        let key = broker_proof_key(&self.credential.token);
        let proof = broker_request_proof(
            &key,
            request_id,
            &self.credential.application_id,
            capability,
            &operation,
            &nonce,
            issued_unix_ms,
        )
        .map_err(|_| NodeClientError::UnexpectedResponse)?;
        let response = self
            .transport
            .request(BrokerRequest {
                protocol: BROKER_PROTOCOL_VERSION,
                request_id,
                operation: BrokerOperation::ExecuteAuthenticated {
                    application_id: self.credential.application_id.clone(),
                    capability,
                    operation,
                    nonce: nonce.clone(),
                    issued_unix_ms,
                    proof,
                },
            })
            .await?;
        if response.request_id != request_id || response.protocol != BROKER_PROTOCOL_VERSION {
            return Err(NodeClientError::ResponseIdMismatch);
        }
        let expected = broker_response_proof(&key, request_id, &nonce, &response.result)
            .map_err(|_| NodeClientError::UnexpectedResponse)?;
        if !response
            .proof
            .as_deref()
            .is_some_and(|proof| constant_time_proof_eq(proof, &expected))
        {
            return Err(NodeClientError::UnexpectedResponse);
        }
        match response
            .result
            .map_err(|failure| NodeClientError::Rejected {
                code: failure.code,
                message: failure.message,
                retryable: failure.retryable,
            })? {
            BrokerResult::Local { result } => Ok(result),
            _ => Err(NodeClientError::UnexpectedResponse),
        }
    }

    async fn execute(
        &self,
        capability: BrokerCapability,
        operation: LocalOperation,
    ) -> Result<LocalResult, NodeClientError> {
        self.execute_operation(capability, operation).await
    }

    async fn request(&self, operation: BrokerOperation) -> Result<BrokerResult, NodeClientError> {
        request_transport(&self.transport, operation).await
    }
}

#[derive(Debug, Clone)]
pub struct NodeConfiguration {
    pub attach_policy: AttachPolicy,
    pub broker_endpoint: Option<BrokerEndpoint>,
    pub embedded_data_directory: Option<PathBuf>,
}

impl NodeConfiguration {
    pub fn resolve_runtime(&self) -> Result<RuntimeDisposition, NodeClientError> {
        Ok(select_runtime(
            self.attach_policy,
            self.broker_endpoint.clone(),
            self.embedded_data_directory.as_deref(),
        )?)
    }
}

#[cfg(unix)]
#[derive(Debug, Clone)]
pub struct UnixBrokerTransport {
    path: PathBuf,
}

#[cfg(unix)]
impl UnixBrokerTransport {
    #[must_use]
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }
}

#[cfg(unix)]
#[async_trait]
impl BrokerTransport for UnixBrokerTransport {
    async fn request(&self, request: BrokerRequest) -> Result<BrokerResponse, NodeClientError> {
        tokio::time::timeout(BROKER_IO_TIMEOUT, async {
            let mut stream = tokio::net::UnixStream::connect(&self.path)
                .await
                .map_err(|error| NodeClientError::Transport(error.to_string()))?;
            piqae_local_ipc::write_message(&mut stream, &request)
                .await
                .map_err(|error| NodeClientError::Transport(error.to_string()))?;
            piqae_local_ipc::read_message(&mut stream)
                .await
                .map_err(|error| NodeClientError::Transport(error.to_string()))
        })
        .await
        .map_err(|_| NodeClientError::Timeout)?
    }
}

#[cfg(windows)]
#[derive(Debug, Clone)]
pub struct WindowsBrokerTransport {
    pipe_name: String,
}

#[cfg(windows)]
impl WindowsBrokerTransport {
    #[must_use]
    pub fn new(pipe_name: impl Into<String>) -> Self {
        Self {
            pipe_name: pipe_name.into(),
        }
    }
}

#[cfg(windows)]
#[async_trait]
impl BrokerTransport for WindowsBrokerTransport {
    async fn request(&self, request: BrokerRequest) -> Result<BrokerResponse, NodeClientError> {
        tokio::time::timeout(BROKER_IO_TIMEOUT, async {
            let mut stream = loop {
                match tokio::net::windows::named_pipe::ClientOptions::new().open(&self.pipe_name) {
                    Ok(stream) => break stream,
                    Err(error) if error.raw_os_error() == Some(231) => {
                        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                    }
                    Err(error) => return Err(NodeClientError::Transport(error.to_string())),
                }
            };
            piqae_local_ipc::write_message(&mut stream, &request)
                .await
                .map_err(|error| NodeClientError::Transport(error.to_string()))?;
            piqae_local_ipc::read_message(&mut stream)
                .await
                .map_err(|error| NodeClientError::Transport(error.to_string()))
        })
        .await
        .map_err(|_| NodeClientError::Timeout)?
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use piqae_local_ipc::{
        BrokerAuthorizationDecision, BrokerPresence, ConnectionState, LocalFailure, LocalPrinter,
        LocalPrinterQueueCounts,
    };
    #[cfg(unix)]
    use piqae_node_runtime::{
        BrokerConsentHandle, BrokerRegistry, BrokerServerState, RuntimeCommand,
        broker::serve_unix_broker_with_test_peer,
    };
    #[cfg(unix)]
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[derive(Debug, Clone)]
    struct FakeTransport {
        requests: Arc<Mutex<Vec<BrokerRequest>>>,
    }

    #[derive(Debug, Clone, Copy)]
    enum ResponseFault {
        MismatchedId,
        UnsupportedProtocol,
        InvalidPresenceRange,
    }

    #[derive(Debug, Clone, Copy)]
    struct FaultTransport(ResponseFault);

    #[async_trait]
    impl BrokerTransport for FaultTransport {
        async fn request(&self, request: BrokerRequest) -> Result<BrokerResponse, NodeClientError> {
            let (request_id, protocol, result) = match self.0 {
                ResponseFault::MismatchedId => (
                    Uuid::new_v4(),
                    BROKER_PROTOCOL_VERSION,
                    Ok(BrokerResult::Presence(BrokerPresence {
                        protocol_min: 1,
                        protocol_max: BROKER_PROTOCOL_VERSION,
                    })),
                ),
                ResponseFault::UnsupportedProtocol => (
                    request.request_id,
                    BROKER_PROTOCOL_VERSION.saturating_add(1),
                    Ok(BrokerResult::Presence(BrokerPresence {
                        protocol_min: 1,
                        protocol_max: BROKER_PROTOCOL_VERSION,
                    })),
                ),
                ResponseFault::InvalidPresenceRange => (
                    request.request_id,
                    BROKER_PROTOCOL_VERSION,
                    Ok(BrokerResult::Presence(BrokerPresence {
                        protocol_min: BROKER_PROTOCOL_VERSION.saturating_add(1),
                        protocol_max: BROKER_PROTOCOL_VERSION.saturating_add(2),
                    })),
                ),
            };
            Ok(BrokerResponse {
                protocol,
                request_id,
                result,
                proof: None,
            })
        }
    }

    #[async_trait]
    impl BrokerTransport for FakeTransport {
        async fn request(&self, request: BrokerRequest) -> Result<BrokerResponse, NodeClientError> {
            self.requests.lock().await.push(request.clone());
            let response_nonce = match &request.operation {
                BrokerOperation::ExecuteAuthenticated { nonce, .. } => Some(nonce.clone()),
                _ => None,
            };
            let result = match request.operation {
                BrokerOperation::Presence => Ok(BrokerResult::Presence(BrokerPresence {
                    protocol_min: 1,
                    protocol_max: 1,
                })),
                BrokerOperation::ExecuteAuthenticated {
                    operation: LocalOperation::Status,
                    ..
                } => Ok(BrokerResult::Local {
                    result: LocalResult::Status(LocalStatus {
                        agent_id: None,
                        workspace_name: None,
                        version: "test".into(),
                        connection: ConnectionState::LocalOnly,
                        queued_jobs: 0,
                        active_jobs: 0,
                        printer_warnings: 0,
                        paused: false,
                    }),
                }),
                BrokerOperation::Execute { .. }
                | BrokerOperation::ExecuteAuthenticated { .. }
                | BrokerOperation::RequestAuthorization { .. }
                | BrokerOperation::AuthorizationStatus { .. }
                | BrokerOperation::ExchangeAuthorization { .. } => Err(LocalFailure {
                    code: "unsupported".into(),
                    message: "unsupported".into(),
                    retryable: false,
                }),
            };
            let proof = response_nonce.and_then(|nonce| {
                broker_response_proof(
                    &broker_proof_key("secret-token"),
                    request.request_id,
                    &nonce,
                    &result,
                )
                .ok()
            });
            Ok(BrokerResponse {
                protocol: BROKER_PROTOCOL_VERSION,
                request_id: request.request_id,
                result,
                proof,
            })
        }
    }

    #[tokio::test]
    async fn client_uses_app_scoped_credential_without_exposing_it_in_debug() {
        let token = "secret-token";
        let requests = Arc::new(Mutex::new(Vec::new()));
        let client = NodeClient::new(
            FakeTransport {
                requests: Arc::clone(&requests),
            },
            "com.example.pos",
            token,
        );
        assert!(!format!("{client:?}").contains(token));
        assert_eq!(client.status().await.unwrap().version, "test");
        let wire = serde_json::to_string(&requests.lock().await[0]).unwrap();
        assert!(!wire.contains(token));
        assert!(wire.contains("execute_authenticated"));
    }

    #[test]
    fn automatic_mode_prefers_attach_and_has_explicit_embedded_fallback() {
        let configuration = NodeConfiguration {
            attach_policy: AttachPolicy::Automatic,
            broker_endpoint: Some(BrokerEndpoint {
                address: "/tmp/piqae.sock".into(),
                protocol_min: 1,
                protocol_max: 1,
            }),
            embedded_data_directory: Some("app-state".into()),
        };
        assert!(matches!(
            configuration.resolve_runtime().unwrap(),
            RuntimeDisposition::Attached(_)
        ));
    }

    #[tokio::test]
    async fn response_id_and_protocol_are_validated_before_data_is_accepted() {
        for (fault, expected) in [
            (ResponseFault::MismatchedId, "response_id"),
            (ResponseFault::UnsupportedProtocol, "protocol"),
            (ResponseFault::InvalidPresenceRange, "protocol"),
        ] {
            let client = NodeClient::new(FaultTransport(fault), "com.example.pos", "token");
            let error = client.presence().await.unwrap_err();
            match (expected, error) {
                ("response_id", NodeClientError::ResponseIdMismatch)
                | ("protocol", NodeClientError::UnsupportedProtocol { .. }) => {}
                (_, other) => panic!("unexpected error: {other}"),
            }
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unix_transport_bounds_a_peer_that_never_replies() {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("broker.sock");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let server = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.unwrap();
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        });
        let client = NodeClient::new(
            UnixBrokerTransport::new(&socket),
            "com.example.pos",
            "token",
        );
        assert!(matches!(
            client.presence().await,
            Err(NodeClientError::Timeout)
        ));
        server.abort();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn real_unix_broker_preserves_consent_and_credential_across_restart() {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("runtime/node.sock");
        let (consent, server) = spawn_test_broker(directory.path(), &socket).await;
        let transport = UnixBrokerTransport::new(&socket);
        let authorization = NodeAuthorizationClient::new(transport.clone());
        let capabilities = vec![
            BrokerCapability::ObserveStatus,
            BrokerCapability::ObservePrinters,
        ];
        let handle = authorization.request(capabilities.clone()).await.unwrap();
        assert_eq!(consent.pending().await.len(), 1);
        consent
            .decide(
                handle.authorization_id,
                BrokerAuthorizationDecision {
                    approved: true,
                    granted_capabilities: capabilities,
                },
            )
            .await
            .unwrap();
        assert_eq!(
            authorization.status(handle.clone()).await.unwrap(),
            BrokerAuthorizationState::Approved
        );
        let credential = authorization.exchange(handle).await.unwrap();
        let client = NodeClient::from_credential(transport, credential.clone());
        assert_eq!(client.status().await.unwrap().version, "fake-macos-node");
        assert_eq!(
            client.printers().await.unwrap()[0].native_id,
            "virtual-printer"
        );

        server.abort();
        let _ = server.await;
        let (restarted_consent, restarted_server) =
            spawn_test_broker(directory.path(), &socket).await;
        let restarted = NodeClient::from_credential(UnixBrokerTransport::new(&socket), credential);
        assert_eq!(restarted.status().await.unwrap().version, "fake-macos-node");

        let restarted_authorization =
            NodeAuthorizationClient::new(UnixBrokerTransport::new(&socket));
        let denied = restarted_authorization
            .request(vec![BrokerCapability::ObservePrinters])
            .await
            .unwrap();
        let pending = restarted_consent.pending().await;
        let denied_id = pending
            .iter()
            .find(|item| item.application.application_id == "com.example.pos")
            .unwrap()
            .authorization_id;
        restarted_consent
            .decide(
                denied_id,
                BrokerAuthorizationDecision {
                    approved: false,
                    granted_capabilities: Vec::new(),
                },
            )
            .await
            .unwrap();
        assert_eq!(
            restarted_authorization
                .status(denied.clone())
                .await
                .unwrap(),
            BrokerAuthorizationState::Denied
        );
        assert!(matches!(
            restarted_authorization.exchange(denied).await,
            Err(NodeClientError::Rejected { code, .. }) if code == "authorization_denied"
        ));

        prove_partial_grant_fails_closed(&restarted_authorization, &restarted_consent, &socket)
            .await;

        let expiring = restarted_authorization
            .request(vec![BrokerCapability::ObserveStatus])
            .await
            .unwrap();
        let expired = BrokerAuthorizationHandle {
            expires_unix_ms: 0,
            ..expiring
        };
        assert_eq!(
            restarted_authorization.status(expired).await.unwrap(),
            BrokerAuthorizationState::Expired
        );
        restarted_server.abort();
    }

    #[cfg(unix)]
    async fn prove_partial_grant_fails_closed(
        authorization: &NodeAuthorizationClient<UnixBrokerTransport>,
        consent: &BrokerConsentHandle,
        socket: &std::path::Path,
    ) {
        let partial = authorization
            .request(vec![
                BrokerCapability::ObserveStatus,
                BrokerCapability::ObservePrinters,
            ])
            .await
            .unwrap();
        consent
            .decide(
                partial.authorization_id,
                BrokerAuthorizationDecision {
                    approved: true,
                    granted_capabilities: vec![BrokerCapability::ObserveStatus],
                },
            )
            .await
            .unwrap();
        let credential = authorization.exchange(partial).await.unwrap();
        let client = NodeClient::from_credential(UnixBrokerTransport::new(socket), credential);
        assert_eq!(client.status().await.unwrap().version, "fake-macos-node");
        assert!(matches!(
            client.printers().await,
            // A request outside the durable grant is intentionally answered
            // without a proof, so the client treats it like any forged reply.
            Err(NodeClientError::UnexpectedResponse)
        ));
    }

    #[cfg(unix)]
    async fn spawn_test_broker(
        root: &std::path::Path,
        socket: &std::path::Path,
    ) -> (BrokerConsentHandle, tokio::task::JoinHandle<()>) {
        let (commands, mut receiver) = tokio::sync::mpsc::channel(8);
        tokio::spawn(async move {
            while let Some(command) = receiver.recv().await {
                match command {
                    RuntimeCommand::Status { respond_to } => {
                        let _ = respond_to.send(LocalStatus {
                            agent_id: Some("agt_fake".into()),
                            workspace_name: Some("Fake workspace".into()),
                            version: "fake-macos-node".into(),
                            connection: ConnectionState::LocalOnly,
                            queued_jobs: 0,
                            active_jobs: 0,
                            printer_warnings: 0,
                            paused: false,
                        });
                    }
                    RuntimeCommand::Printers { respond_to } => {
                        let _ = respond_to.send(vec![LocalPrinter {
                            printer_id: "prn_fake".into(),
                            native_id: "virtual-printer".into(),
                            name: "Virtual printer".into(),
                            state: "idle".into(),
                            is_default: true,
                            exposed: true,
                            capability_revision: 1,
                            capabilities: serde_json::from_str("{}").unwrap(),
                            native_options: BTreeMap::new(),
                            profiles: Vec::new(),
                            queue_counts: LocalPrinterQueueCounts::default(),
                        }]);
                    }
                    _ => {}
                }
            }
        });
        let state = BrokerServerState::new(BrokerRegistry::load(root).unwrap(), commands);
        let consent = state.consent_handle();
        let socket = socket.to_path_buf();
        let server_socket = socket.clone();
        let server = tokio::spawn(async move {
            let peer = piqae_local_ipc::deterministic_test_connection(
                "com.example.pos",
                "example-signer",
                "node-client-test-process",
            )
            .evidence()
            .clone();
            serve_unix_broker_with_test_peer(server_socket, state, peer)
                .await
                .unwrap();
        });
        for _ in 0..100 {
            if tokio::net::UnixStream::connect(&socket).await.is_ok() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
        assert!(tokio::net::UnixStream::connect(&socket).await.is_ok());
        (consent, server)
    }
}
