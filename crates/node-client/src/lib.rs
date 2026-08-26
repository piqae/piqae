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
    BROKER_PROTOCOL_VERSION, BrokerCapability, BrokerCredential, BrokerOperation, BrokerRequest,
    BrokerResponse, BrokerResult, LocalOperation, LocalPrinter, LocalResult, LocalStatus,
};
use piqae_node_runtime::{
    AttachPolicy, BrokerEndpoint, RuntimeDisposition, RuntimeSelectionError, select_runtime,
};
use std::path::{Path, PathBuf};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum NodeClientError {
    #[error("local broker transport failed: {0}")]
    Transport(String),
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
            },
        }
    }

    pub async fn presence(&self) -> Result<piqae_local_ipc::BrokerPresence, NodeClientError> {
        match self.request(BrokerOperation::Presence).await? {
            BrokerResult::Presence(presence) => Ok(presence),
            BrokerResult::Local(_) => Err(NodeClientError::UnexpectedResponse),
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

    async fn execute(
        &self,
        capability: BrokerCapability,
        operation: LocalOperation,
    ) -> Result<LocalResult, NodeClientError> {
        match self
            .request(BrokerOperation::Execute {
                credential: self.credential.clone(),
                capability,
                operation,
            })
            .await?
        {
            BrokerResult::Local(result) => Ok(result),
            BrokerResult::Presence(_) => Err(NodeClientError::UnexpectedResponse),
        }
    }

    async fn request(&self, operation: BrokerOperation) -> Result<BrokerResult, NodeClientError> {
        let response = self
            .transport
            .request(BrokerRequest {
                protocol: BROKER_PROTOCOL_VERSION,
                request_id: Uuid::new_v4(),
                operation,
            })
            .await?;
        response
            .result
            .map_err(|failure| NodeClientError::Rejected {
                code: failure.code,
                message: failure.message,
                retryable: failure.retryable,
            })
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
        let mut stream = tokio::net::UnixStream::connect(&self.path)
            .await
            .map_err(|error| NodeClientError::Transport(error.to_string()))?;
        piqae_local_ipc::write_message(&mut stream, &request)
            .await
            .map_err(|error| NodeClientError::Transport(error.to_string()))?;
        piqae_local_ipc::read_message(&mut stream)
            .await
            .map_err(|error| NodeClientError::Transport(error.to_string()))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use piqae_local_ipc::{BrokerPresence, ConnectionState, LocalFailure};
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[derive(Debug, Clone)]
    struct FakeTransport {
        requests: Arc<Mutex<Vec<BrokerRequest>>>,
    }

    #[async_trait]
    impl BrokerTransport for FakeTransport {
        async fn request(&self, request: BrokerRequest) -> Result<BrokerResponse, NodeClientError> {
            self.requests.lock().await.push(request.clone());
            let result = match request.operation {
                BrokerOperation::Presence => Ok(BrokerResult::Presence(BrokerPresence {
                    protocol_min: 1,
                    protocol_max: 1,
                })),
                BrokerOperation::Execute {
                    operation: LocalOperation::Status,
                    ..
                } => Ok(BrokerResult::Local(LocalResult::Status(LocalStatus {
                    agent_id: None,
                    workspace_name: None,
                    version: "test".into(),
                    connection: ConnectionState::LocalOnly,
                    queued_jobs: 0,
                    active_jobs: 0,
                    printer_warnings: 0,
                    paused: false,
                }))),
                BrokerOperation::Execute { .. } => Err(LocalFailure {
                    code: "unsupported".into(),
                    message: "unsupported".into(),
                    retryable: false,
                }),
            };
            Ok(BrokerResponse {
                protocol: 1,
                request_id: request.request_id,
                result,
            })
        }
    }

    #[tokio::test]
    async fn client_uses_app_scoped_credential_without_exposing_it_in_debug() {
        let token = "secret-token";
        let client = NodeClient::new(
            FakeTransport {
                requests: Arc::new(Mutex::new(Vec::new())),
            },
            "com.example.pos",
            token,
        );
        assert!(!format!("{client:?}").contains(token));
        assert_eq!(client.status().await.unwrap().version, "test");
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
}
