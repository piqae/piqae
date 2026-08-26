//! Revocable application-scoped authorization for the local node broker.

use anyhow::{Context as _, Result, bail};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::RngCore as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::{collections::BTreeMap, fs::OpenOptions, io::Write as _, path::PathBuf};
use tokio::sync::{Mutex, mpsc, oneshot};

use crate::command::{CommandFailure, RuntimeCommand};
use piqae_local_ipc::{
    BROKER_PROTOCOL_VERSION, BrokerCapability, BrokerOperation, BrokerPresence, BrokerRequest,
    BrokerResponse, BrokerResult, LocalFailure, LocalOperation, LocalResult, read_message,
    write_message,
};

const DOCUMENT_VERSION: u16 = 1;
const MAX_APPLICATIONS: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationIdentity {
    /// Reverse-DNS application identifier or an equivalently stable package ID.
    pub application_id: String,
    /// Human-readable label shown in the local consent UI only.
    pub display_name: String,
    /// Optional platform signing identity digest. This is supporting evidence,
    /// never a replacement for a user-approved broker capability.
    pub signing_identity_sha256: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "serialized least-privilege capabilities are independent grants"
)]
pub struct ApplicationCapabilities {
    pub observe_status: bool,
    pub observe_printers: bool,
    pub manage_profiles: bool,
    pub submit_local_jobs: bool,
    pub manage_connectors: bool,
}

impl ApplicationCapabilities {
    pub const OBSERVE_ONLY: Self = Self {
        observe_status: true,
        observe_printers: true,
        manage_profiles: false,
        submit_local_jobs: false,
        manage_connectors: false,
    };

    #[must_use]
    pub const fn allows(&self, requested: Self) -> bool {
        (!requested.observe_status || self.observe_status)
            && (!requested.observe_printers || self.observe_printers)
            && (!requested.manage_profiles || self.manage_profiles)
            && (!requested.submit_local_jobs || self.submit_local_jobs)
            && (!requested.manage_connectors || self.manage_connectors)
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct BrokerToken(String);

impl BrokerToken {
    #[must_use]
    pub fn expose_for_client(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for BrokerToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("BrokerToken([REDACTED])")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationAuthorization {
    pub identity: ApplicationIdentity,
    pub capabilities: ApplicationCapabilities,
    pub token: BrokerToken,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DurableApplicationAuthorization {
    identity: ApplicationIdentity,
    capabilities: ApplicationCapabilities,
    token_sha256: String,
    revoked: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BrokerDocument {
    version: u16,
    applications: Vec<DurableApplicationAuthorization>,
}

#[derive(Debug)]
pub struct BrokerRegistry {
    root: PathBuf,
    applications: BTreeMap<String, DurableApplicationAuthorization>,
}

impl BrokerRegistry {
    /// Loads the bounded application authorization registry.
    ///
    /// # Errors
    ///
    /// Fails closed for malformed, unsupported or unbounded state.
    pub fn load(root: impl AsRef<std::path::Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        let path = root.join("broker-applications.json");
        let document = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice::<BrokerDocument>(&bytes)
                .with_context(|| format!("parse {}", path.display()))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => BrokerDocument {
                version: DOCUMENT_VERSION,
                applications: Vec::new(),
            },
            Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
        };
        if document.version != DOCUMENT_VERSION {
            bail!("unsupported broker registry version {}", document.version);
        }
        if document.applications.len() > MAX_APPLICATIONS {
            bail!("broker application registry exceeds supported bounds");
        }
        let mut applications = BTreeMap::new();
        for authorization in document.applications {
            validate_identity(&authorization.identity)?;
            if applications
                .insert(authorization.identity.application_id.clone(), authorization)
                .is_some()
            {
                bail!("broker application registry contains a duplicate application id");
            }
        }
        Ok(Self { root, applications })
    }

    /// Creates or rotates one app's capability. The plaintext token is
    /// returned once and never persisted.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid identities, capacity, or durable writes.
    pub fn authorize(
        &mut self,
        identity: ApplicationIdentity,
        capabilities: ApplicationCapabilities,
    ) -> Result<ApplicationAuthorization> {
        validate_identity(&identity)?;
        if !self.applications.contains_key(&identity.application_id)
            && self.applications.len() >= MAX_APPLICATIONS
        {
            bail!("broker application limit reached");
        }
        let token = generate_token();
        self.applications.insert(
            identity.application_id.clone(),
            DurableApplicationAuthorization {
                identity: identity.clone(),
                capabilities,
                token_sha256: token_digest(token.expose_for_client()),
                revoked: false,
            },
        );
        self.persist()?;
        Ok(ApplicationAuthorization {
            identity,
            capabilities,
            token,
        })
    }

    #[must_use]
    pub fn authenticate(
        &self,
        application_id: &str,
        token: &str,
        requested: ApplicationCapabilities,
    ) -> bool {
        self.applications.get(application_id).is_some_and(|entry| {
            !entry.revoked
                && entry.capabilities.allows(requested)
                && constant_time_eq(
                    entry.token_sha256.as_bytes(),
                    token_digest(token).as_bytes(),
                )
        })
    }

    /// Revokes a capability durably before returning success.
    ///
    /// # Errors
    ///
    /// Returns an error when durable registry replacement fails.
    pub fn revoke(&mut self, application_id: &str) -> Result<bool> {
        let Some(entry) = self.applications.get_mut(application_id) else {
            return Ok(false);
        };
        if entry.revoked {
            return Ok(false);
        }
        entry.revoked = true;
        self.persist()?;
        Ok(true)
    }

    fn persist(&self) -> Result<()> {
        std::fs::create_dir_all(&self.root)?;
        let path = self.root.join("broker-applications.json");
        let staged = self.root.join("broker-applications.json.replacing");
        let _ = std::fs::remove_file(&staged);
        let document = BrokerDocument {
            version: DOCUMENT_VERSION,
            applications: self.applications.values().cloned().collect(),
        };
        let bytes = serde_json::to_vec_pretty(&document)?;
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options.open(&staged)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&staged, &path)?;
        Ok(())
    }
}

impl ApplicationCapabilities {
    const fn requiring(capability: BrokerCapability) -> Self {
        let mut requested = Self {
            observe_status: false,
            observe_printers: false,
            manage_profiles: false,
            submit_local_jobs: false,
            manage_connectors: false,
        };
        match capability {
            BrokerCapability::ObserveStatus => requested.observe_status = true,
            BrokerCapability::ObservePrinters => requested.observe_printers = true,
            BrokerCapability::ManageProfiles => requested.manage_profiles = true,
            BrokerCapability::SubmitLocalJobs => requested.submit_local_jobs = true,
            BrokerCapability::ManageConnectors => requested.manage_connectors = true,
        }
        requested
    }
}

#[derive(Clone)]
pub struct BrokerServerState {
    registry: std::sync::Arc<Mutex<BrokerRegistry>>,
    commands: mpsc::Sender<RuntimeCommand>,
}

impl std::fmt::Debug for BrokerServerState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BrokerServerState")
            .field("registry", &"<redacted>")
            .field("commands", &self.commands)
            .finish()
    }
}

impl BrokerServerState {
    #[must_use]
    pub fn new(registry: BrokerRegistry, commands: mpsc::Sender<RuntimeCommand>) -> Self {
        Self {
            registry: std::sync::Arc::new(Mutex::new(registry)),
            commands,
        }
    }

    async fn handle(&self, request: BrokerRequest) -> BrokerResponse {
        let result = if request.protocol == BROKER_PROTOCOL_VERSION {
            match request.operation {
                BrokerOperation::Presence => Ok(BrokerResult::Presence(BrokerPresence {
                    protocol_min: BROKER_PROTOCOL_VERSION,
                    protocol_max: BROKER_PROTOCOL_VERSION,
                })),
                BrokerOperation::Execute {
                    credential,
                    capability,
                    operation,
                } => {
                    let required = required_capability(&operation);
                    if required != Some(capability) {
                        Err(local_failure(
                            "capability_mismatch",
                            "the declared capability does not authorize this operation",
                            false,
                        ))
                    } else if !self.registry.lock().await.authenticate(
                        &credential.application_id,
                        &credential.token,
                        ApplicationCapabilities::requiring(capability),
                    ) {
                        Err(local_failure(
                            "application_unauthorized",
                            "the application capability is invalid or revoked",
                            false,
                        ))
                    } else {
                        dispatch_operation(&self.commands, operation)
                            .await
                            .map(BrokerResult::Local)
                    }
                }
            }
        } else {
            Err(local_failure(
                "unsupported_broker_protocol",
                "the application and node broker protocol versions do not overlap",
                false,
            ))
        };
        BrokerResponse {
            protocol: BROKER_PROTOCOL_VERSION,
            request_id: request.request_id,
            result,
        }
    }
}

#[cfg(unix)]
/// Serves the application broker on a private Unix-domain socket.
///
/// # Errors
///
/// Returns an error if the endpoint cannot be safely bound or accepted.
pub async fn serve_unix_broker(
    path: impl Into<PathBuf>,
    state: BrokerServerState,
) -> Result<(), piqae_local_ipc::LocalIpcError> {
    let endpoint = piqae_local_ipc::LocalEndpoint::bind(path)?;
    loop {
        let mut stream = endpoint.accept().await?;
        let state = state.clone();
        tokio::spawn(async move {
            let response = match read_message::<BrokerRequest>(&mut stream).await {
                Ok(request) => state.handle(request).await,
                Err(_) => return,
            };
            let _ = write_message(&mut stream, &response).await;
        });
    }
}

const fn required_capability(operation: &LocalOperation) -> Option<BrokerCapability> {
    match operation {
        LocalOperation::Status => Some(BrokerCapability::ObserveStatus),
        LocalOperation::Printers => Some(BrokerCapability::ObservePrinters),
        LocalOperation::BeginProfileCapture(_)
        | LocalOperation::CommitProfileCapture(_)
        | LocalOperation::CancelProfileCapture(_)
        | LocalOperation::ValidateProfile(_)
        | LocalOperation::ConfirmLoadedMedia(_) => Some(BrokerCapability::ManageProfiles),
        LocalOperation::Pause | LocalOperation::Resume => Some(BrokerCapability::ManageConnectors),
        LocalOperation::RestartAgent
        | LocalOperation::ExportSupportBundle { .. }
        | LocalOperation::Reenrol { .. } => None,
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "exhaustive protocol-to-command mapping stays in one auditable boundary"
)]
async fn dispatch_operation(
    commands: &mpsc::Sender<RuntimeCommand>,
    operation: LocalOperation,
) -> Result<LocalResult, LocalFailure> {
    match operation {
        LocalOperation::Status => {
            let (send, receive) = oneshot::channel();
            send_command(commands, RuntimeCommand::Status { respond_to: send }).await?;
            receive
                .await
                .map(LocalResult::Status)
                .map_err(|_| unavailable())
        }
        LocalOperation::Printers => {
            let (send, receive) = oneshot::channel();
            send_command(commands, RuntimeCommand::Printers { respond_to: send }).await?;
            receive
                .await
                .map(|printers| LocalResult::Printers { printers })
                .map_err(|_| unavailable())
        }
        LocalOperation::Pause | LocalOperation::Resume => {
            let (send, receive) = oneshot::channel();
            let command = if matches!(operation, LocalOperation::Pause) {
                RuntimeCommand::Pause { respond_to: send }
            } else {
                RuntimeCommand::Resume { respond_to: send }
            };
            send_command(commands, command).await?;
            receive
                .await
                .map_err(|_| unavailable())?
                .map(|()| LocalResult::Accepted)
                .map_err(command_failure)
        }
        LocalOperation::BeginProfileCapture(request) => {
            let (send, receive) = oneshot::channel();
            send_command(
                commands,
                RuntimeCommand::BeginProfileCapture {
                    printer_id: request.printer_id.clone(),
                    request: crate::command::ProfileCaptureBeginRequest {
                        operation: request.operation,
                        profile_id: request.profile_id,
                        expected_revision: request.expected_revision,
                    },
                    respond_to: send,
                },
            )
            .await?;
            receive
                .await
                .map_err(|_| unavailable())?
                .map(|authorization| LocalResult::ProfileCaptureAuthorized(Box::new(authorization)))
                .map_err(command_failure)
        }
        LocalOperation::CommitProfileCapture(request) => {
            let (send, receive) = oneshot::channel();
            send_command(
                commands,
                RuntimeCommand::CommitProfileCapture {
                    session_id: request.session_id,
                    capture_token: request.capture_token,
                    capture: Box::new(request.capture),
                    respond_to: send,
                },
            )
            .await?;
            receive
                .await
                .map_err(|_| unavailable())?
                .map(|profile| LocalResult::ProfileCaptured {
                    profile: Box::new(profile),
                })
                .map_err(command_failure)
        }
        LocalOperation::CancelProfileCapture(request) => {
            let (send, receive) = oneshot::channel();
            send_command(
                commands,
                RuntimeCommand::CancelProfileCapture {
                    session_id: request.session_id,
                    capture_token: request.capture_token,
                    respond_to: send,
                },
            )
            .await?;
            receive
                .await
                .map_err(|_| unavailable())?
                .map(|()| LocalResult::Accepted)
                .map_err(command_failure)
        }
        LocalOperation::ValidateProfile(request) => {
            let (send, receive) = oneshot::channel();
            send_command(
                commands,
                RuntimeCommand::ValidateProfile {
                    profile_id: request.profile_id,
                    revision: request.revision,
                    respond_to: send,
                },
            )
            .await?;
            receive
                .await
                .map_err(|_| unavailable())?
                .map(LocalResult::ProfileValidation)
                .map_err(command_failure)
        }
        LocalOperation::ConfirmLoadedMedia(request) => {
            let (send, receive) = oneshot::channel();
            send_command(
                commands,
                RuntimeCommand::ConfirmLoadedMedia {
                    request,
                    respond_to: send,
                },
            )
            .await?;
            receive
                .await
                .map_err(|_| unavailable())?
                .map(|()| LocalResult::Accepted)
                .map_err(command_failure)
        }
        LocalOperation::RestartAgent
        | LocalOperation::ExportSupportBundle { .. }
        | LocalOperation::Reenrol { .. } => Err(local_failure(
            "operation_requires_native_shell",
            "this privileged operation is not available to application clients",
            false,
        )),
    }
}

async fn send_command(
    commands: &mpsc::Sender<RuntimeCommand>,
    command: RuntimeCommand,
) -> Result<(), LocalFailure> {
    commands.send(command).await.map_err(|_| unavailable())
}

fn unavailable() -> LocalFailure {
    local_failure(
        "node_runtime_unavailable",
        "the durable node runtime is unavailable",
        true,
    )
}

fn command_failure(CommandFailure { code, message }: CommandFailure) -> LocalFailure {
    local_failure(&code, &message, false)
}

fn local_failure(code: &str, message: &str, retryable: bool) -> LocalFailure {
    LocalFailure {
        code: code.to_owned(),
        message: message.to_owned(),
        retryable,
    }
}

fn validate_identity(identity: &ApplicationIdentity) -> Result<()> {
    if identity.application_id.is_empty()
        || identity.application_id.len() > 255
        || !identity
            .application_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        || identity.display_name.is_empty()
        || identity.display_name.len() > 128
        || identity
            .signing_identity_sha256
            .as_ref()
            .is_some_and(|value| {
                value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
    {
        bail!("invalid broker application identity");
    }
    Ok(())
}

fn generate_token() -> BrokerToken {
    let mut bytes = [0_u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    BrokerToken(URL_SAFE_NO_PAD.encode(bytes))
}

fn token_digest(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (a, b)| difference | (a ^ b))
        == 0
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use piqae_local_ipc::{BrokerCredential, BrokerOperation, BrokerRequest, ConnectionState};
    use uuid::Uuid;

    fn identity() -> ApplicationIdentity {
        ApplicationIdentity {
            application_id: "com.example.pos".into(),
            display_name: "Example POS".into(),
            signing_identity_sha256: None,
        }
    }

    #[test]
    fn token_is_returned_once_and_registry_survives_restart() {
        let directory = tempfile::tempdir().unwrap();
        let mut registry = BrokerRegistry::load(directory.path()).unwrap();
        let issued = registry
            .authorize(identity(), ApplicationCapabilities::OBSERVE_ONLY)
            .unwrap();
        assert!(!format!("{issued:?}").contains(issued.token.expose_for_client()));
        drop(registry);

        let registry = BrokerRegistry::load(directory.path()).unwrap();
        assert!(registry.authenticate(
            "com.example.pos",
            issued.token.expose_for_client(),
            ApplicationCapabilities::OBSERVE_ONLY
        ));
    }

    #[test]
    fn least_privilege_and_revocation_are_enforced() {
        let directory = tempfile::tempdir().unwrap();
        let mut registry = BrokerRegistry::load(directory.path()).unwrap();
        let issued = registry
            .authorize(identity(), ApplicationCapabilities::OBSERVE_ONLY)
            .unwrap();
        assert!(!registry.authenticate(
            "com.example.pos",
            issued.token.expose_for_client(),
            ApplicationCapabilities {
                submit_local_jobs: true,
                ..ApplicationCapabilities::OBSERVE_ONLY
            }
        ));
        assert!(registry.revoke("com.example.pos").unwrap());
        assert!(!registry.authenticate(
            "com.example.pos",
            issued.token.expose_for_client(),
            ApplicationCapabilities::OBSERVE_ONLY
        ));
    }

    #[test]
    fn rotating_one_app_does_not_authorize_the_previous_token() {
        let directory = tempfile::tempdir().unwrap();
        let mut registry = BrokerRegistry::load(directory.path()).unwrap();
        let old = registry
            .authorize(identity(), ApplicationCapabilities::OBSERVE_ONLY)
            .unwrap();
        let current = registry
            .authorize(identity(), ApplicationCapabilities::OBSERVE_ONLY)
            .unwrap();
        assert!(!registry.authenticate(
            "com.example.pos",
            old.token.expose_for_client(),
            ApplicationCapabilities::OBSERVE_ONLY
        ));
        assert!(registry.authenticate(
            "com.example.pos",
            current.token.expose_for_client(),
            ApplicationCapabilities::OBSERVE_ONLY
        ));
    }

    #[tokio::test]
    async fn broker_dispatches_only_an_authorized_capability() {
        let directory = tempfile::tempdir().unwrap();
        let mut registry = BrokerRegistry::load(directory.path()).unwrap();
        let issued = registry
            .authorize(identity(), ApplicationCapabilities::OBSERVE_ONLY)
            .unwrap();
        let (commands, mut receive) = mpsc::channel(1);
        let state = BrokerServerState::new(registry, commands);
        tokio::spawn(async move {
            if let Some(RuntimeCommand::Status { respond_to }) = receive.recv().await {
                let _ = respond_to.send(piqae_local_ipc::LocalStatus {
                    agent_id: None,
                    workspace_name: None,
                    version: "test".into(),
                    connection: ConnectionState::LocalOnly,
                    queued_jobs: 0,
                    active_jobs: 0,
                    printer_warnings: 0,
                    paused: false,
                });
            }
        });
        let response = state
            .handle(BrokerRequest {
                protocol: BROKER_PROTOCOL_VERSION,
                request_id: Uuid::new_v4(),
                operation: BrokerOperation::Execute {
                    credential: BrokerCredential {
                        application_id: "com.example.pos".into(),
                        token: issued.token.expose_for_client().into(),
                    },
                    capability: BrokerCapability::ObserveStatus,
                    operation: LocalOperation::Status,
                },
            })
            .await;
        assert!(matches!(
            response.result,
            Ok(BrokerResult::Local(LocalResult::Status(_)))
        ));
    }
}
