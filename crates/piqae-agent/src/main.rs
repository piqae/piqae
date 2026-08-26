mod connector_runtime;
mod content_key_store;
mod route_coordinator;
mod uri_fetch;
#[cfg(windows)]
mod windows_acl;

use aes_gcm::{
    Aes256Gcm, KeyInit as _,
    aead::{Aead, Payload},
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use chrono::Utc;
use clap::{Parser, ValueEnum};
use futures::TryStreamExt;
use hkdf::Hkdf;
use p256::{PublicKey, SecretKey, ecdh::diffie_hellman, pkcs8::EncodePublicKey as _};
use piqae_agent_client::{AgentClient, ClientError, DeviceIdentity};
use piqae_agent_core::{
    AgentEngine, ContentStore, Executor, ExecutorFailure, FakeExecutor, LocalSubmission,
    NativeAcceptance, NativeJobReference, SystemClock,
    document_render::{
        NodeDocumentCapabilities, NodeRenderRequirement, NodeRenderResult, RENDERER_ABI,
        render_with_resources_or_fallback,
    },
    document_resources::{DocumentResourceCache, NodeResourceDescriptor, RESOURCE_ABI},
};
use piqae_agent_storage::{
    AcceptedJob, AgentStore, CloudAcceptIntent, NativeProfileCapture, PendingEvent, QueueCounts,
    StorageError, StoredLoadedMedia, StoredNamedProfile, StoredPrinter,
};
use piqae_domain::{
    AgentId, ContentKind, EventId, JobEvent, JobFailureReason, JobId, JobState, NativeProfileKind,
    ProfileCaptureOperation, ProfileStatus,
};
use piqae_executor_supervisor::{ExecutorSupervisor, SupervisedExecutor};
use piqae_local_api::{
    ControlFailure, ControlRequest, LocalApiState, LocalConnectorDetail, LocalContent,
    LocalCreateJob, LocalHistoryJob, LocalJobAccepted, LocalJobHistory, ProfileCreate,
    ProfileUpdate,
};
use piqae_local_ipc::{
    ConnectionState, LocalNativeQueueJob, LocalPrinter, LocalPrinterProfile, LocalPrinterQueue,
    LocalPrinterQueueCounts, LocalQueueJob, LocalStatus, NativeProfileCapturePayload,
    NativeProfileSeed, ProfileCaptureAuthorized, ProfileValidationResult, SessionAuthenticator,
    capture_token_digest, generate_capture_token,
};
use piqae_protocol::{
    CURRENT_PROTOCOL_VERSION,
    agent::{
        AgentAcceptJobRequest, AgentCommand, AgentHealth, AgentReleaseLeaseRequest,
        AgentRenewLeaseRequest, AgentSyncRequest, AgentSyncResponse, ContentDescriptor,
        CreateDeviceAuthorizationRequest, EnrolRequest, InstallationMode, JobOffer, PrinterGrant,
        PrinterProfileSnapshot, PrinterSnapshot, QueueSnapshot,
    },
    executor::{DiscoveredPrinter, ExecutorOperation, ExecutorResult, NativeJobObservation},
};
use piqae_support_packs::{RegistryConfig as SupportPackConfig, SupportPackRegistry};
use sha2::{Digest as _, Sha256};
use std::{
    future::Future,
    io::Read as _,
    io::Write as _,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Mutex, Notify, RwLock, mpsc, oneshot};
use tokio_util::io::StreamReader;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;
use uri_fetch::UriFetcher;
use url::Url;

const LEASE_RENEWAL_INTERVAL: Duration = Duration::from_secs(10);
const LEASE_RENEWAL_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const PROFILE_CAPTURE_LIFETIME: Duration = Duration::from_secs(5 * 60);
const LOCAL_PROFILE_HOST_ID: &str = "authenticated-loopback-profile-host";
const DOCUMENT_RESOURCE_CACHE_BYTES: u64 = 512 * 1024 * 1024;
const DOCUMENT_RESOURCE_MAX_BYTES: u64 = 4 * 1024 * 1024;
static DOCUMENT_RESOURCE_CACHE: OnceLock<Arc<DocumentResourceCache>> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum AgentMode {
    Local,
    Hosted,
    SelfHosted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum ExecutorMode {
    Disabled,
    Fake,
    Process,
}

#[derive(Debug, Parser)]
#[command(version, about = "Piqae headless print node")]
#[allow(
    clippy::struct_excessive_bools,
    reason = "CLI action flags are mutually constrained by clap"
)]
struct Arguments {
    /// Runtime mode. Hosted modes require enrolment before cloud sync begins.
    #[arg(long, env = "PIQAE_AGENT_MODE", default_value = "local")]
    mode: AgentMode,

    /// Durable application-data directory.
    #[arg(long, env = "PIQAE_DATA_DIR", default_value = ".piqae")]
    data_dir: PathBuf,

    /// Loopback address for the local operational API.
    #[arg(long, env = "PIQAE_LOCAL_BIND", default_value = "127.0.0.1:39100")]
    local_bind: SocketAddr,

    /// Hosted or self-hosted Rust control-plane origin.
    #[arg(long, env = "PIQAE_CONTROL_PLANE_URL")]
    control_plane_url: Option<Url>,

    /// Enrolled agent ID. Required outside local mode.
    #[arg(long, env = "PIQAE_AGENT_ID")]
    agent_id: Option<String>,

    /// File containing the enrolled Ed25519 private key as 64 hex characters.
    #[arg(long, env = "PIQAE_DEVICE_KEY_FILE")]
    device_key_file: Option<PathBuf>,

    /// Native executor selection. Fake is only for development and tests.
    #[arg(long, env = "PIQAE_EXECUTOR", default_value = "disabled")]
    executor: ExecutorMode,

    /// Executor child-process path when --executor=process.
    #[arg(long, env = "PIQAE_EXECUTOR_PATH")]
    executor_path: Option<PathBuf>,

    /// Trusted declarative driver support-pack directory. Repeat for multiple packs.
    #[arg(long, env = "PIQAE_SUPPORT_PACK_DIRS", value_delimiter = ',')]
    support_pack_dirs: Vec<PathBuf>,

    /// Pinned canonical support-pack SHA-256. Repeat or comma-separate.
    #[arg(long, env = "PIQAE_SUPPORT_PACK_DIGESTS", value_delimiter = ',')]
    support_pack_digests: Vec<String>,

    /// Trusted Ed25519 publisher public key as 64 hexadecimal characters.
    #[arg(long, env = "PIQAE_SUPPORT_PACK_TRUST_KEYS", value_delimiter = ',')]
    support_pack_trust_keys: Vec<String>,

    /// Allow trusted private, loopback, and link-local URI content sources.
    /// Cloud metadata and unspecified/multicast destinations remain blocked.
    #[arg(long, env = "PIQAE_ALLOW_PRIVATE_URI_SOURCES", default_value_t = false)]
    allow_private_uri_sources: bool,

    /// Consume a one-time token, generate this installation's device key, and exit.
    #[arg(long, env = "PIQAE_ENROLMENT_TOKEN", hide_env_values = true)]
    enrolment_token: Option<String>,

    /// Read the one-time enrolment token from standard input and exit.
    ///
    /// Native launchers use this instead of argv or environment variables so
    /// the capability is not exposed by process inspection or shell history.
    #[arg(
        long,
        conflicts_with_all = ["enrolment_token", "pair", "rotate_key"]
    )]
    enrolment_token_stdin: bool,

    /// Preview an invitation read from standard input without consuming it.
    #[arg(long, conflicts_with_all = ["enrolment_token", "enrolment_token_stdin", "pair", "rotate_key", "add_connector_json_stdin"])]
    preview_connect_token_stdin: bool,

    /// Add a locally approved connector from a bounded JSON document on stdin.
    #[arg(long, conflicts_with_all = ["enrolment_token", "enrolment_token_stdin", "pair", "rotate_key", "preview_connect_token_stdin"])]
    add_connector_json_stdin: bool,

    /// Pair this installation through a browser approval flow, then exit.
    #[arg(long, conflicts_with = "enrolment_token")]
    pair: bool,

    /// Replace this node's device key through a browser approval flow, keeping
    /// its node ID, printers, and routing. Requires an already-paired node.
    #[arg(long, conflicts_with_all = ["enrolment_token", "pair"])]
    rotate_key: bool,

    /// Human-readable installation name used with --enrolment-token or --pair.
    #[arg(long)]
    enrolment_name: Option<String>,
}

#[derive(Debug)]
struct CloudConfiguration {
    client: AgentClient,
    identity: DeviceIdentity,
    agent_id: AgentId,
    content_encryption_keys: Arc<content_key_store::ContentKeyring>,
    allowed_printer_ids: Option<std::collections::BTreeSet<String>>,
    connector_id: String,
}

#[derive(Debug)]
enum RuntimeExecutor {
    Disabled,
    Fake(FakeExecutor),
    Process(SupervisedExecutor),
}

const ROUTE_OBSERVATION_FRESHNESS: Duration = Duration::from_secs(5);

#[derive(Clone, Debug)]
struct CachedNativeRouteObservation {
    cached_at: tokio::time::Instant,
    observed_at: chrono::DateTime<Utc>,
    state: piqae_domain::PrinterState,
    queue: Option<Vec<piqae_protocol::executor::NativeQueueJob>>,
}

/// Installation-wide native telemetry cache. Connector sync loops share this
/// cache so N connectors do not turn one native queue into N CUPS/Windows
/// polls. The raw native records never cross the process boundary; each
/// connector receives only aggregate counts calculated against its isolated
/// known native job IDs.
#[derive(Debug, Default)]
struct RouteObservationCache {
    entries: std::collections::BTreeMap<String, CachedNativeRouteObservation>,
}

impl RouteObservationCache {
    async fn get_or_collect<F, Fut>(
        &mut self,
        native_id: &str,
        collect: F,
    ) -> CachedNativeRouteObservation
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = CachedNativeRouteObservation>,
    {
        if let Some(cached) = self.entries.get(native_id)
            && cached.cached_at.elapsed() < ROUTE_OBSERVATION_FRESHNESS
        {
            return cached.clone();
        }
        let observation = collect().await;
        self.entries
            .insert(native_id.to_owned(), observation.clone());
        self.entries.retain(|_, cached| {
            cached.cached_at.elapsed() < ROUTE_OBSERVATION_FRESHNESS.saturating_mul(12)
        });
        observation
    }
}

/// One bounded native handoff boundary shared by every connector runtime.
/// Tokio's mutex queues waiters FIFO, preventing a connector from bypassing
/// already-waiting peers while also ensuring drivers never receive concurrent
/// operations from this process.
#[derive(Debug, Clone)]
struct SharedRuntimeExecutor {
    runtime: Arc<Mutex<RuntimeExecutor>>,
    coordinator: Arc<Mutex<route_coordinator::RouteCoordinator>>,
    observation_cache: Arc<Mutex<RouteObservationCache>>,
    connector_id: String,
}

#[derive(Debug, Clone)]
struct StopSignal {
    sender: tokio::sync::watch::Sender<bool>,
}

impl Default for StopSignal {
    fn default() -> Self {
        let (sender, _) = tokio::sync::watch::channel(false);
        Self { sender }
    }
}

impl StopSignal {
    fn stop(&self) {
        self.sender.send_replace(true);
    }
    fn is_stopped(&self) -> bool {
        *self.sender.borrow()
    }
    async fn cancelled(&self) {
        let mut receiver = self.sender.subscribe();
        if *receiver.borrow() {
            return;
        }
        let _ = receiver.changed().await;
    }
}

enum ConnectorSupervisorCommand {
    Reload {
        respond_to: oneshot::Sender<Result<(), ControlFailure>>,
    },
    Revoke {
        connector_id: String,
        respond_to: oneshot::Sender<Result<(), ControlFailure>>,
    },
    Details {
        respond_to: oneshot::Sender<Result<Vec<LocalConnectorDetail>, ControlFailure>>,
    },
    RefreshPrinters,
}

fn reject_connector_supervisor_command(
    error: mpsc::error::TrySendError<ConnectorSupervisorCommand>,
) {
    let (is_full, command) = match error {
        mpsc::error::TrySendError::Full(command) => (true, command),
        mpsc::error::TrySendError::Closed(command) => (false, command),
    };
    match command {
        ConnectorSupervisorCommand::Reload { respond_to } => {
            let failure = if is_full {
                control_failure(
                    "connector_reload_deferred",
                    "connector supervisor is busy; periodic recovery will retry",
                )
            } else {
                control_failure(
                    "connector_supervisor_unavailable",
                    "connector supervisor is unavailable",
                )
            };
            let _ = respond_to.send(Err(failure));
        }
        ConnectorSupervisorCommand::Revoke { respond_to, .. } => {
            let failure = if is_full {
                control_failure(
                    "connector_revoke_deferred",
                    "connector supervisor is busy; retry revocation",
                )
            } else {
                control_failure(
                    "connector_supervisor_unavailable",
                    "connector supervisor is unavailable",
                )
            };
            let _ = respond_to.send(Err(failure));
        }
        ConnectorSupervisorCommand::Details { respond_to } => {
            let _ = respond_to.send(Err(control_failure(
                "connector_supervisor_unavailable",
                if is_full {
                    "connector supervisor is busy"
                } else {
                    "connector supervisor is unavailable"
                },
            )));
        }
        ConnectorSupervisorCommand::RefreshPrinters => {
            warn!(
                busy = is_full,
                "connector printer refresh notification was deferred"
            );
        }
    }
}

struct ConnectorWorker {
    record: connector_runtime::ConnectorRecord,
    printer_inventory_dirty: Arc<AtomicBool>,
    wakeup: Arc<Notify>,
    last_sync_error_code: Arc<RwLock<Option<String>>>,
    sync_stop: StopSignal,
    scheduler_stop: StopSignal,
    connection_stop: StopSignal,
    sync: tokio::task::JoinHandle<()>,
    scheduler: tokio::task::JoinHandle<()>,
    connection_watch: tokio::task::JoinHandle<()>,
}

#[derive(Clone)]
struct ConnectorConnectionTracker {
    states: Arc<Mutex<std::collections::BTreeMap<String, ConnectionState>>>,
    aggregate: Arc<RwLock<ConnectionState>>,
}

impl ConnectorConnectionTracker {
    fn new(aggregate: Arc<RwLock<ConnectionState>>) -> Self {
        Self {
            states: Arc::new(Mutex::new(std::collections::BTreeMap::new())),
            aggregate,
        }
    }

    async fn update(&self, connector_id: &str, state: ConnectionState) {
        self.states
            .lock()
            .await
            .insert(connector_id.to_owned(), state);
        self.refresh().await;
    }

    async fn remove(&self, connector_id: &str) {
        self.states.lock().await.remove(connector_id);
        self.refresh().await;
    }

    async fn state(&self, connector_id: &str) -> ConnectionState {
        self.states
            .lock()
            .await
            .get(connector_id)
            .copied()
            .unwrap_or(ConnectionState::Offline)
    }

    async fn refresh(&self) {
        let state = {
            let states = self.states.lock().await;
            aggregate_connector_connection(states.values().copied())
        };
        *self.aggregate.write().await = state;
    }
}

fn aggregate_connector_connection(
    states: impl Iterator<Item = ConnectionState>,
) -> ConnectionState {
    let states = states.collect::<Vec<_>>();
    if states.is_empty() {
        return ConnectionState::LocalOnly;
    }
    if states.contains(&ConnectionState::Connected) {
        ConnectionState::Connected
    } else if states.contains(&ConnectionState::Connecting) {
        ConnectionState::Connecting
    } else if states
        .iter()
        .all(|state| *state == ConnectionState::Unauthorized)
    {
        ConnectionState::Unauthorized
    } else if states.contains(&ConnectionState::Degraded)
        || states.contains(&ConnectionState::Unauthorized)
    {
        ConnectionState::Degraded
    } else {
        ConnectionState::Offline
    }
}

struct LegacyCloudWorker {
    stop: StopSignal,
    task: tokio::task::JoinHandle<()>,
}

impl SharedRuntimeExecutor {
    fn for_connector(&self, connector_id: impl Into<String>) -> Self {
        Self {
            runtime: Arc::clone(&self.runtime),
            coordinator: Arc::clone(&self.coordinator),
            observation_cache: Arc::clone(&self.observation_cache),
            connector_id: connector_id.into(),
        }
    }

    async fn discover_printers(&self) -> Result<Vec<DiscoveredPrinter>, ControlFailure> {
        self.runtime.lock().await.discover_printers().await
    }

    async fn native_queue(
        &self,
        native_printer_id: &str,
    ) -> Result<Vec<piqae_protocol::executor::NativeQueueJob>, ControlFailure> {
        self.runtime
            .lock()
            .await
            .native_queue(native_printer_id)
            .await
    }
}

#[async_trait]
impl Executor for SharedRuntimeExecutor {
    async fn submit(
        &mut self,
        mut submission: LocalSubmission,
    ) -> Result<NativeAcceptance, ExecutorFailure> {
        let now = Utc::now();
        let reservation = self
            .coordinator
            .lock()
            .await
            .reserve(
                &self.connector_id,
                &submission.printer_native_id,
                &submission.job_id,
                now.timestamp_millis(),
            )
            .map_err(|error| ExecutorFailure {
                code: if error.to_string().contains("already crossed") {
                    "native_handoff_already_recorded"
                } else {
                    "route_reserved"
                }
                .into(),
                message: error.to_string(),
                retryable: !error.to_string().contains("already crossed"),
                handoff_may_have_succeeded: error.to_string().contains("already crossed"),
                native_code: None,
            })?;
        let validation = self.coordinator.lock().await.validate(&reservation);
        if let Err(error) = validation {
            return Err(ExecutorFailure {
                code: "stale_route_fence".into(),
                message: error.to_string(),
                retryable: true,
                handoff_may_have_succeeded: false,
                native_code: None,
            });
        }
        submission.route_fence = Some(piqae_protocol::executor::LocalRouteFence {
            route_id: reservation.local_route_key.clone(),
            reservation_id: reservation.reservation_id,
            generation: reservation.generation,
        });
        let job_id = submission.job_id.clone();
        let result = self.runtime.lock().await.submit(submission).await;
        let (outcome, native_job_id) = match &result {
            Ok(acceptance) => (
                piqae_protocol::agent::NativeHandoffOutcome::Accepted,
                Some(acceptance.native_job_id.clone()),
            ),
            Err(error) if error.handoff_may_have_succeeded => {
                (piqae_protocol::agent::NativeHandoffOutcome::Ambiguous, None)
            }
            Err(_) => (
                piqae_protocol::agent::NativeHandoffOutcome::RejectedBeforeHandoff,
                None,
            ),
        };
        let finish = self.coordinator.lock().await.finish(
            &self.connector_id,
            &job_id,
            &reservation,
            outcome,
            native_job_id,
            Utc::now(),
        );
        if let Err(error) = finish {
            return Err(ExecutorFailure {
                code: "route_fence_persistence_failed".into(),
                message: error.to_string(),
                retryable: false,
                handoff_may_have_succeeded: result.is_ok()
                    || result
                        .as_ref()
                        .is_err_and(|failure| failure.handoff_may_have_succeeded),
                native_code: None,
            });
        }
        result
    }

    async fn observe(
        &mut self,
        reference: NativeJobReference,
    ) -> Result<NativeJobObservation, ExecutorFailure> {
        self.runtime.lock().await.observe(reference).await
    }

    async fn cancel(&mut self, reference: NativeJobReference) -> Result<(), ExecutorFailure> {
        self.runtime.lock().await.cancel(reference).await
    }
}

#[derive(Debug, Clone)]
enum PrinterDiscovery {
    Disabled,
    Fake,
    Process(SupervisedExecutor),
}

impl RuntimeExecutor {
    async fn discover_printers(&self) -> Result<Vec<DiscoveredPrinter>, ControlFailure> {
        let printers = match self {
            Self::Disabled => Vec::new(),
            Self::Fake(_) => vec![DiscoveredPrinter {
                native_id: "fake-printer".into(),
                name: "Fake Printer".into(),
                is_default: true,
                state: piqae_domain::PrinterState::Online,
                capabilities: piqae_domain::PrinterCapabilities::default(),
                native_options: std::collections::BTreeMap::new(),
                driver_fingerprint: None,
                identity_evidence: Vec::new(),
            }],
            Self::Process(executor) => match executor
                .execute_operation(
                    ExecutorOperation::DiscoverPrinters,
                    Utc::now().timestamp_millis() + 30_000,
                )
                .await
                .map_err(|error| control_failure(&error.code, &error.message))?
            {
                ExecutorResult::Printers { printers } => printers,
                _ => {
                    return Err(control_failure(
                        "unexpected_executor_response",
                        "executor returned the wrong discovery result",
                    ));
                }
            },
        };
        Ok(printers)
    }

    async fn native_queue(
        &self,
        native_printer_id: &str,
    ) -> Result<Vec<piqae_protocol::executor::NativeQueueJob>, ControlFailure> {
        match self {
            Self::Disabled | Self::Fake(_) => Ok(Vec::new()),
            Self::Process(executor) => match executor
                .execute_operation(
                    ExecutorOperation::ListJobs {
                        native_printer_id: native_printer_id.to_owned(),
                    },
                    Utc::now().timestamp_millis() + 2_000,
                )
                .await
                .map_err(|error| control_failure(&error.code, &error.message))?
            {
                ExecutorResult::Jobs { jobs } => Ok(jobs),
                _ => Err(control_failure(
                    "unexpected_executor_response",
                    "executor returned the wrong queue result",
                )),
            },
        }
    }
}

impl PrinterDiscovery {
    async fn observe_state(&self, native_printer_id: &str) -> Result<piqae_domain::PrinterState> {
        match self {
            Self::Disabled => Ok(piqae_domain::PrinterState::Unknown),
            Self::Fake => Ok(piqae_domain::PrinterState::Online),
            Self::Process(executor) => match executor
                .execute_operation(
                    ExecutorOperation::GetPrinterState {
                        native_printer_id: native_printer_id.to_owned(),
                    },
                    Utc::now().timestamp_millis() + 2_000,
                )
                .await
                .map_err(|error| anyhow::anyhow!("{}: {}", error.code, error.message))?
            {
                ExecutorResult::State { state } => Ok(state),
                _ => anyhow::bail!("executor returned the wrong printer-state result"),
            },
        }
    }

    async fn observe_queue(
        &self,
        native_printer_id: &str,
    ) -> Result<Vec<piqae_protocol::executor::NativeQueueJob>> {
        match self {
            Self::Disabled | Self::Fake => Ok(Vec::new()),
            Self::Process(executor) => match executor
                .execute_operation(
                    ExecutorOperation::ListJobs {
                        native_printer_id: native_printer_id.to_owned(),
                    },
                    Utc::now().timestamp_millis() + 2_000,
                )
                .await
                .map_err(|error| anyhow::anyhow!("{}: {}", error.code, error.message))?
            {
                ExecutorResult::Jobs { jobs } => Ok(jobs),
                _ => anyhow::bail!("executor returned the wrong queue result"),
            },
        }
    }
}

#[async_trait]
impl Executor for RuntimeExecutor {
    async fn submit(
        &mut self,
        submission: LocalSubmission,
    ) -> Result<NativeAcceptance, ExecutorFailure> {
        match self {
            Self::Fake(executor) => executor.submit(submission).await,
            Self::Process(executor) => executor.submit(submission).await,
            Self::Disabled => Err(ExecutorFailure {
                code: "native_adapter_disabled".into(),
                message: "this build has no enabled native print adapter".into(),
                retryable: false,
                handoff_may_have_succeeded: false,
                native_code: None,
            }),
        }
    }

    async fn observe(
        &mut self,
        reference: NativeJobReference,
    ) -> Result<NativeJobObservation, ExecutorFailure> {
        match self {
            Self::Fake(executor) => executor.observe(reference).await,
            Self::Process(executor) => executor.observe(reference).await,
            Self::Disabled => Err(disabled_executor_failure()),
        }
    }

    async fn cancel(&mut self, reference: NativeJobReference) -> Result<(), ExecutorFailure> {
        match self {
            Self::Fake(executor) => executor.cancel(reference).await,
            Self::Process(executor) => executor.cancel(reference).await,
            Self::Disabled => Err(disabled_executor_failure()),
        }
    }
}

fn disabled_executor_failure() -> ExecutorFailure {
    ExecutorFailure {
        code: "native_adapter_disabled".into(),
        message: "this build has no enabled native print adapter".into(),
        retryable: false,
        handoff_may_have_succeeded: false,
        native_code: None,
    }
}

#[tokio::main]
#[allow(
    clippy::too_many_lines,
    reason = "startup keeps process-boundary ownership wiring explicit"
)]
async fn main() -> Result<()> {
    let arguments = Arguments::parse();
    if arguments.preview_connect_token_stdin {
        let token = read_enrolment_token_from_stdin()?;
        return preview_connector(&arguments, &token).await;
    }
    if arguments.add_connector_json_stdin {
        return add_connector(&arguments, read_connector_consent(std::io::stdin())?).await;
    }
    if arguments.enrolment_token_stdin {
        let token = read_enrolment_token_from_stdin()?;
        return enrol_installation(&arguments, &token).await;
    }
    if let Some(token) = arguments.enrolment_token.as_deref() {
        return enrol_installation(&arguments, token).await;
    }
    if arguments.pair {
        return pair_installation(&arguments, PairingIntent::FirstPairing).await;
    }
    if arguments.rotate_key {
        return pair_installation(&arguments, PairingIntent::KeyRotation).await;
    }

    initialize_logging()?;

    let outcome: Result<()> = async {
    anyhow::ensure!(
        arguments.local_bind.ip().is_loopback(),
        "PIQAE_LOCAL_BIND must use a loopback address; the authenticated local API must not be exposed to the network"
    );

    std::fs::create_dir_all(&arguments.data_dir)
        .with_context(|| format!("create {}", arguments.data_dir.display()))?;
    // Loading is intentionally fail-closed: a corrupt or unsupported
    // multi-connector registry must not silently fall back to another tenant's
    // legacy identity. An absent registry preserves single-connector behavior.
    let connector_registry = connector_runtime::ConnectorRegistry::load(&arguments.data_dir)?;
    let configured_connectors = connector_registry.enabled().count();
    let database_path = arguments.data_dir.join("agent.sqlite3");
    let store = AgentStore::open(&database_path)
        .with_context(|| format!("open {}", database_path.display()))?;
    if !store.integrity_check()? {
        anyhow::bail!("agent database integrity check failed");
    }
    let document_resource_cache = DocumentResourceCache::open(
        arguments.data_dir.join("document-resources"),
        &database_path,
        DOCUMENT_RESOURCE_CACHE_BYTES,
        DOCUMENT_RESOURCE_MAX_BYTES,
    )
    .context("open document resource cache")?;
    DOCUMENT_RESOURCE_CACHE
        .set(Arc::new(document_resource_cache))
        .map_err(|_| anyhow::anyhow!("document resource cache was initialized twice"))?;
    let initially_paused = store.setting("paused")?.as_deref() == Some("true");
    let support_packs = Arc::new(SupportPackRegistry::load(&SupportPackConfig {
        pack_directories: arguments.support_pack_dirs.clone(),
        pinned_digest_hex: arguments.support_pack_digests.clone(),
        ed25519_public_key_hex: arguments.support_pack_trust_keys.clone(),
    }).context("load trusted driver support packs")?);

    let challenge = load_or_create_private_token(&arguments.data_dir.join("local.token"))?;
    let content_store = ContentStore::open(arguments.data_dir.join("content")).await?;
    let cloud_content_store = content_store.clone();
    let uri_fetcher = UriFetcher::new(arguments.allow_private_uri_sources);
    let cloud_uri_fetcher = uri_fetcher.clone();
    let (executor, printer_discovery) = match arguments.executor {
        ExecutorMode::Disabled => (RuntimeExecutor::Disabled, PrinterDiscovery::Disabled),
        ExecutorMode::Fake => (
            RuntimeExecutor::Fake(FakeExecutor::default()),
            PrinterDiscovery::Fake,
        ),
        ExecutorMode::Process => {
            let supervised = SupervisedExecutor::new(ExecutorSupervisor::new(
                arguments
                    .executor_path
                    .clone()
                    .context("--executor-path is required with --executor=process")?,
                Duration::from_secs(120),
            ));
            (
                RuntimeExecutor::Process(supervised.clone()),
                PrinterDiscovery::Process(supervised),
            )
        }
    };
    let route_coordinator = route_coordinator::RouteCoordinator::open(&arguments.data_dir)
        .context("open installation route coordinator")?;
    let executor = SharedRuntimeExecutor {
        runtime: Arc::new(Mutex::new(executor)),
        coordinator: Arc::new(Mutex::new(route_coordinator)),
        observation_cache: Arc::new(Mutex::new(RouteObservationCache::default())),
        connector_id: "local".into(),
    };
    let engine = AgentEngine::new(store, executor.clone(), SystemClock);

    let connection = Arc::new(RwLock::new(if arguments.mode == AgentMode::Local {
        ConnectionState::LocalOnly
    } else {
        ConnectionState::Connecting
    }));
    let paused = Arc::new(AtomicBool::new(initially_paused));
    let cloud_sync_wakeup = Arc::new(Notify::new());
    let printer_inventory_dirty = Arc::new(AtomicBool::new(true));
    let (control_tx, control_rx) = mpsc::channel(32);
    let (connector_supervisor_tx, connector_supervisor_rx) = mpsc::channel(32);
    let mut control_task = tokio::spawn(control_loop(
        control_rx,
        engine,
        content_store,
        uri_fetcher,
        env!("CARGO_PKG_VERSION").to_owned(),
        arguments.agent_id.clone(),
        Arc::clone(&connection),
        Arc::clone(&paused),
        Arc::clone(&cloud_sync_wakeup),
        Arc::clone(&printer_inventory_dirty),
        connector_supervisor_tx,
    ));

    // A populated registry supersedes the legacy cloud identity. Running both
    // could lease the same tenant twice during migration. With no registry,
    // the original CLI/configuration path remains byte-for-byte compatible.
    let legacy_cloud_worker = if arguments.mode != AgentMode::Local && configured_connectors == 0 {
        let cloud = cloud_configuration(&arguments)?;
        let stop = StopSignal::default();
        let task = tokio::spawn(cloud_sync_loop(
            cloud,
            database_path.clone(),
            database_path.clone(),
            cloud_content_store,
            cloud_uri_fetcher.clone(),
            printer_discovery.clone(),
            Arc::clone(&executor.coordinator),
            Arc::clone(&executor.observation_cache),
            Arc::clone(&support_packs),
            Arc::clone(&connection),
            Arc::clone(&paused),
            Arc::clone(&cloud_sync_wakeup),
            Arc::clone(&printer_inventory_dirty),
            Arc::new(RwLock::new(None)),
            stop.clone(),
        ));
        Some(LegacyCloudWorker { stop, task })
    } else {
        None
    };

    // A locally installed node starts without a remote principal, but a
    // connector accepted through the native consent flow is an explicit,
    // printer-scoped authorization. Keep the supervisor available in every
    // mode so that approval can activate that isolated connector immediately.
    let connector_connections = ConnectorConnectionTracker::new(Arc::clone(&connection));
    let mut connector_supervisor_task = tokio::spawn(connector_supervisor_loop(
        arguments.data_dir.clone(),
        connector_supervisor_rx,
        executor,
        cloud_uri_fetcher,
        printer_discovery,
        support_packs,
        legacy_cloud_worker,
        connector_connections,
    ));

    info!(
        mode = ?arguments.mode,
        configured_connectors,
        database = %database_path.display(),
        bind = %arguments.local_bind,
        "Piqae node started"
    );
    let local_api = piqae_local_api::serve(
        arguments.local_bind,
        LocalApiState::new(&challenge, control_tx),
    );
    tokio::pin!(local_api);
    let result = tokio::select! {
        result = &mut local_api => result.context("serve local API"),
        result = &mut control_task => Err(unexpected_task_exit("local control loop", result)),
        result = &mut connector_supervisor_task => {
            Err(unexpected_task_exit("connector supervisor", result))
        }
    };
    control_task.abort();
    connector_supervisor_task.abort();
    result
    }
    .await;
    if let Err(error) = &outcome {
        error!(error = %error, "Piqae agent stopped unexpectedly");
    }
    outcome
}

fn unexpected_task_exit(
    name: &str,
    result: std::result::Result<(), tokio::task::JoinError>,
) -> anyhow::Error {
    match result {
        Ok(()) => anyhow::anyhow!("critical task `{name}` exited unexpectedly"),
        Err(error) if error.is_panic() => {
            anyhow::anyhow!("critical task `{name}` panicked: {error}")
        }
        Err(error) => anyhow::anyhow!("critical task `{name}` failed: {error}"),
    }
}

fn initialize_logging() -> Result<()> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    if let Some(path) = std::env::var_os("PIQAE_LOG_FILE").filter(|path| !path.is_empty()) {
        let writer = piqae_native_logging::BoundedLogWriter::open_with_defaults(path)
            .context("open bounded native agent log")?;
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(writer)
            .json()
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .json()
            .init();
    }
    let default_panic_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic| {
        if let Some(location) = panic.location() {
            error!(
                file = location.file(),
                line = location.line(),
                "Piqae agent panicked"
            );
        } else {
            error!("Piqae agent panicked");
        }
        // Packaged nodes have a bounded structured sink. Avoid also emitting
        // the raw panic payload to inherited stderr because panic messages can
        // contain document paths, driver data, or credentials.
        if std::env::var_os("PIQAE_LOG_FILE").is_none() {
            default_panic_hook(panic);
        }
    }));
    Ok(())
}

const MAX_ENROLMENT_TOKEN_INPUT_BYTES: u64 = 256;
const MAX_CONNECTOR_CONSENT_BYTES: u64 = 32 * 1024;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ConnectorConsentInput {
    token: String,
    #[serde(default)]
    printer_grant: PrinterGrant,
    printer_ids: Vec<String>,
}

fn read_enrolment_token_from_stdin() -> Result<String> {
    read_enrolment_token(std::io::stdin())
}

fn read_enrolment_token(reader: impl std::io::Read) -> Result<String> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_ENROLMENT_TOKEN_INPUT_BYTES + 1)
        .read_to_end(&mut bytes)
        .context("read enrolment token from standard input")?;
    if bytes.len() as u64 > MAX_ENROLMENT_TOKEN_INPUT_BYTES {
        anyhow::bail!("enrolment token input exceeds 256 bytes");
    }
    let token = std::str::from_utf8(&bytes)
        .context("enrolment token input must be UTF-8")?
        .trim();
    if token.is_empty() {
        anyhow::bail!("enrolment token input cannot be empty");
    }
    Ok(token.to_owned())
}

fn read_connector_consent(reader: impl std::io::Read) -> Result<ConnectorConsentInput> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_CONNECTOR_CONSENT_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_CONNECTOR_CONSENT_BYTES {
        anyhow::bail!("connector consent input exceeds 32 KiB");
    }
    let consent: ConnectorConsentInput =
        serde_json::from_slice(&bytes).context("connector consent input is invalid")?;
    if !consent.token.starts_with("piq_enr_")
        || consent.token.len() > 128
        || (consent.printer_grant == PrinterGrant::SelectedPrinters
            && consent.printer_ids.is_empty())
        || (consent.printer_grant == PrinterGrant::AllLocalPrinters
            && !consent.printer_ids.is_empty())
        || consent.printer_ids.len() > 128
        || consent
            .printer_ids
            .iter()
            .any(|id| id.is_empty() || id.len() > 128)
        || consent
            .printer_ids
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
            != consent.printer_ids.len()
    {
        anyhow::bail!("connector consent is outside supported limits");
    }
    Ok(consent)
}

fn installed_control_plane(arguments: &Arguments) -> Result<(Url, ExistingInstallation, PathBuf)> {
    let config_path = arguments.data_dir.join("agent-config.json");
    let base_url = arguments
        .control_plane_url
        .clone()
        .or_else(|| {
            std::fs::read(&config_path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
                .and_then(|body| {
                    body.get("control_plane_url")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned)
                })
                .and_then(|value| value.parse().ok())
        })
        .context("connection invitation has no control-plane URL")?;
    if !config_path.exists() {
        let agent_id = std::fs::read_to_string(arguments.data_dir.join("agent-id"))?
            .trim()
            .to_owned();
        anyhow::ensure!(
            !agent_id.is_empty(),
            "local installation has no durable identity"
        );
        return Ok((
            base_url,
            ExistingInstallation {
                agent_id: agent_id.clone(),
                installation_id: agent_id,
            },
            arguments.data_dir.join("device.key"),
        ));
    }
    let installation = existing_installation(&config_path)?;
    let body: serde_json::Value = serde_json::from_slice(&std::fs::read(&config_path)?)?;
    let key_path = body
        .get("device_key_file")
        .and_then(serde_json::Value::as_str)
        .map(PathBuf::from)
        .context("installed configuration has no device key path")?;
    Ok((base_url, installation, key_path))
}

fn installed_local_bind(arguments: &Arguments) -> Result<SocketAddr> {
    let config_path = arguments.data_dir.join("agent-config.json");
    let body: serde_json::Value = serde_json::from_slice(&std::fs::read(&config_path)?)?;
    let bind = body
        .get("local_bind")
        .and_then(serde_json::Value::as_str)
        .context("installed configuration has no local API bind address")?
        .parse::<SocketAddr>()
        .context("installed configuration has an invalid local API bind address")?;
    anyhow::ensure!(
        bind.ip().is_loopback(),
        "installed local API bind address is not loopback"
    );
    Ok(bind)
}

async fn preview_connector(arguments: &Arguments, token: &str) -> Result<()> {
    let base_url = arguments
        .control_plane_url
        .clone()
        .context("connection invitation has no control-plane URL")?;
    let preview = AgentClient::new(base_url)?
        .preview_connect_session(token)
        .await
        .context("preview Piqae connector invitation")?;
    println!("{}", serde_json::to_string(&preview)?);
    Ok(())
}

#[allow(
    clippy::too_many_lines,
    reason = "connector enrolment keeps preview, proof, durable identity, and reload ordering visible"
)]
async fn add_connector(arguments: &Arguments, consent: ConnectorConsentInput) -> Result<()> {
    let ConnectorConsentInput {
        token,
        printer_grant,
        printer_ids,
    } = consent;
    let mut allowed_printer_ids = printer_ids;
    allowed_printer_ids.sort();
    let (base_url, installation, installation_key_path) = installed_control_plane(arguments)?;
    let preview = AgentClient::new(base_url.clone())?
        .preview_connect_session(&token)
        .await
        .context("read Piqae connector identity")?;
    let fingerprint = hex::encode(Sha256::digest(token.as_bytes()));
    let local_first_connection = no_enabled_connectors(&arguments.data_dir)?;
    let relative_key = if local_first_connection {
        PathBuf::from("device.key")
    } else {
        PathBuf::from("connectors")
            .join("keys")
            .join(format!("{fingerprint}.key"))
    };
    let key_path = arguments.data_dir.join(&relative_key);
    let identity = if key_path.exists() {
        let encoded = std::fs::read_to_string(&key_path)?;
        let bytes = hex::decode(encoded.trim()).context("decode pending connector key")?;
        let secret: [u8; 32] = bytes
            .try_into()
            .map_err(|_| anyhow::anyhow!("pending connector key is invalid"))?;
        DeviceIdentity::from_secret_bytes(AgentId::new(), &secret)
    } else {
        let generated = DeviceIdentity::generate(AgentId::new());
        write_new_device_key(&key_path, &generated.secret_bytes())?;
        generated
    };
    let connector_public_key = identity.public_key_base64();
    let installation_secret = hex::decode(std::fs::read_to_string(&installation_key_path)?.trim())?;
    let installation_secret: [u8; 32] = installation_secret
        .try_into()
        .map_err(|_| anyhow::anyhow!("installed device key is invalid"))?;
    let installation_identity = DeviceIdentity::from_secret_bytes(
        installation
            .agent_id
            .parse()
            .context("installed agent id is invalid")?,
        &installation_secret,
    );
    let proof_message = connector_installation_proof_message(
        &token,
        &installation.installation_id,
        &connector_public_key,
        printer_grant,
        &allowed_printer_ids,
    );
    let installation_proof = installation_identity.sign_base64(&proof_message);
    let enrolled = AgentClient::new(base_url.clone())?
        .enrol(&EnrolRequest {
            token,
            public_key: connector_public_key,
            name: installation_hostname(),
            hostname: installation_hostname(),
            platform: std::env::consts::OS.to_owned(),
            architecture: std::env::consts::ARCH.to_owned(),
            installation_mode: InstallationMode::User,
            agent_version: env!("CARGO_PKG_VERSION").to_owned(),
            protocol_version: CURRENT_PROTOCOL_VERSION,
            installation_id: Some(installation.installation_id),
            printer_grant,
            allowed_printer_ids: allowed_printer_ids.clone(),
            installation_proof: Some(installation_proof),
        })
        .await
        .context("accept Piqae connector invitation")?;
    let connector_id = enrolled
        .connector_id
        .context("control plane omitted connector id")?;
    let mut registry = connector_runtime::ConnectorRegistry::load(&arguments.data_dir)?;
    let record = connector_runtime::ConnectorRecord {
        connector_id: connector_id.clone(),
        agent_id: enrolled.agent_id.to_string(),
        control_plane_url: base_url,
        display_name: preview
            .requesting_service_name
            .or_else(|| Some(preview.workspace_name.clone())),
        workspace_name: Some(preview.workspace_name),
        authorization_type: Some(preview.authorization_type),
        workspace_id: Some(preview.workspace_id),
        environment_id: Some(preview.environment_id),
        requesting_service_account_id: preview.requesting_service_account_id,
        manage_url: preview.return_url.and_then(|value| value.parse().ok()),
        device_key_file: relative_key,
        enabled: true,
        printer_grant,
        allowed_printer_ids,
    };
    if registry.contains(&connector_id) {
        registry.replace(record)?;
        if let Err(error) = signal_connector_reload(arguments).await {
            warn!(%error, "existing connector is durable but immediate reload was deferred");
        }
        print_connector_connected(&enrolled.agent_id);
        return Ok(());
    }
    registry.add(record)?;
    if let Err(error) = signal_connector_reload(arguments).await {
        warn!(%error, "connector is durable but the running node could not be notified; periodic recovery will retry");
    }
    print_connector_connected(&enrolled.agent_id);
    Ok(())
}

fn print_connector_connected(agent_id: &AgentId) {
    println!(
        "{}",
        serde_json::json!({"state":"connected","agent_id":agent_id})
    );
}

fn connector_installation_proof_message(
    token: &str,
    installation_id: &str,
    connector_public_key: &str,
    printer_grant: PrinterGrant,
    allowed_printer_ids: &[String],
) -> Vec<u8> {
    match printer_grant {
        PrinterGrant::SelectedPrinters => piqae_protocol::agent::connector_proof_message(
            token,
            installation_id,
            connector_public_key,
            allowed_printer_ids,
        ),
        PrinterGrant::AllLocalPrinters => piqae_protocol::agent::connector_grant_proof_message(
            token,
            installation_id,
            connector_public_key,
            printer_grant,
            allowed_printer_ids,
        ),
    }
}

fn no_enabled_connectors(data_dir: &Path) -> Result<bool> {
    Ok(connector_runtime::ConnectorRegistry::load(data_dir)?
        .enabled()
        .next()
        .is_none())
}

async fn signal_connector_reload(arguments: &Arguments) -> Result<()> {
    let token_path = arguments.data_dir.join("local.token");
    let token = std::fs::read_to_string(&token_path)
        .with_context(|| format!("read {}", token_path.display()))?;
    let local_bind = installed_local_bind(arguments)?;
    let response = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?
        .post(format!("http://{local_bind}/v1/local/connectors/reload"))
        .bearer_auth(token.trim())
        .send()
        .await
        .context("signal running node to reload connectors")?;
    if !response.status().is_success() {
        anyhow::bail!("running node rejected connector reload");
    }
    Ok(())
}

async fn enrol_installation(arguments: &Arguments, token: &str) -> Result<()> {
    let token = token.trim();
    if token.is_empty() {
        anyhow::bail!("--enrolment-token cannot be empty");
    }
    let base_url = arguments
        .control_plane_url
        .clone()
        .context("--control-plane-url is required for enrolment")?;
    std::fs::create_dir_all(&arguments.data_dir)
        .with_context(|| format!("create {}", arguments.data_dir.display()))?;
    let key_path = arguments
        .device_key_file
        .clone()
        .unwrap_or_else(|| arguments.data_dir.join("device.key"));
    if key_path.exists() {
        anyhow::bail!(
            "{} already exists; refusing to replace an enrolled device identity",
            key_path.display()
        );
    }
    let config_path = arguments.data_dir.join("agent-config.json");
    if config_path.exists() {
        anyhow::bail!(
            "{} already exists; refusing to replace an enrolled agent configuration",
            config_path.display()
        );
    }
    let hostname = installation_hostname();
    let name = arguments
        .enrolment_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(hostname.as_str());
    let provisional_id = AgentId::new();
    let identity = DeviceIdentity::generate(provisional_id);
    write_new_device_key(&key_path, &identity.secret_bytes())?;
    let client = AgentClient::new(base_url.clone())?;
    let enrolled = client
        .enrol(&EnrolRequest {
            token: token.to_owned(),
            public_key: identity.public_key_base64(),
            name: name.to_owned(),
            hostname,
            platform: std::env::consts::OS.to_owned(),
            architecture: std::env::consts::ARCH.to_owned(),
            installation_mode: InstallationMode::User,
            agent_version: env!("CARGO_PKG_VERSION").to_owned(),
            protocol_version: CURRENT_PROTOCOL_VERSION,
            installation_id: None,
            printer_grant: PrinterGrant::SelectedPrinters,
            allowed_printer_ids: Vec::new(),
            installation_proof: None,
        })
        .await;
    let enrolled = match enrolled {
        Ok(enrolled) => enrolled,
        Err(error) => {
            let _ = std::fs::remove_file(&key_path);
            return Err(error).context("enrol this Piqae installation");
        }
    };
    let config = serde_json::json!({
        "mode": "self-hosted",
        "control_plane_url": base_url,
        "agent_id": enrolled.agent_id,
        "environment": enrolled.environment,
        "device_key_file": key_path,
        "data_dir": arguments.data_dir,
        "local_bind": arguments.local_bind,
    });
    if let Err(error) = write_new_json(&config_path, &config) {
        let _ = std::fs::remove_file(&key_path);
        return Err(error).context("persist enrolled agent configuration");
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "agent_id": enrolled.agent_id,
            "environment": enrolled.environment,
            "config_file": config_path,
            "device_key_file": key_path,
        }))?
    );
    Ok(())
}

/// Why a node is running the browser-pairing flow.
///
/// Both cases exchange an operator approval for a device key, but they differ
/// in what already exists locally and what must survive the exchange.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PairingIntent {
    /// No identity exists yet. Refuse to run if one does, so an accidental
    /// re-pair cannot silently discard a working node.
    FirstPairing,
    /// An identity exists and its key is being replaced. The stored
    /// installation ID is reused so the control plane rebinds the existing
    /// node rather than admitting a second one, keeping the node ID, its
    /// printers, and any routing that points at them.
    KeyRotation,
}

#[allow(
    clippy::too_many_lines,
    reason = "keeps the secret-bearing browser-pairing lifecycle in one auditable flow"
)]
async fn pair_installation(arguments: &Arguments, intent: PairingIntent) -> Result<()> {
    let base_url = arguments
        .control_plane_url
        .clone()
        .context("--control-plane-url is required for browser pairing")?;
    std::fs::create_dir_all(&arguments.data_dir)
        .with_context(|| format!("create {}", arguments.data_dir.display()))?;
    let key_path = arguments
        .device_key_file
        .clone()
        .unwrap_or_else(|| arguments.data_dir.join("device.key"));
    let config_path = arguments.data_dir.join("agent-config.json");
    let existing = match intent {
        PairingIntent::FirstPairing => {
            if key_path.exists() || config_path.exists() {
                anyhow::bail!(
                    "this installation already has a device identity; run --rotate-key to \
                     replace its key, or revoke and migrate it explicitly"
                );
            }
            None
        }
        PairingIntent::KeyRotation => Some(existing_installation(&config_path)?),
    };
    let hostname = installation_hostname();
    let name = arguments
        .enrolment_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(hostname.as_str());
    let installation_id = existing.as_ref().map_or_else(
        || uuid::Uuid::now_v7().to_string(),
        |existing| existing.installation_id.clone(),
    );
    let identity = DeviceIdentity::generate(AgentId::new());
    let client = AgentClient::new(base_url.clone())?;
    let installation_id_for_config = installation_id.clone();
    let authorization = client
        .create_device_authorization(&CreateDeviceAuthorizationRequest {
            public_key: identity.public_key_base64(),
            installation_id,
            proposed_name: name.to_owned(),
            hostname,
            platform: std::env::consts::OS.to_owned(),
            architecture: std::env::consts::ARCH.to_owned(),
            installation_mode: InstallationMode::User,
            agent_version: env!("CARGO_PKG_VERSION").to_owned(),
            protocol_version: CURRENT_PROTOCOL_VERSION,
        })
        .await
        .context("start browser pairing")?;
    let mut verification_url = base_url
        .join(authorization.verification_uri.trim_start_matches('/'))
        .context("resolve pairing verification URL")?;
    verification_url
        .query_pairs_mut()
        .append_pair("authorization_id", &authorization.id);
    println!("Open {verification_url}");
    println!("Enter pairing code {}", authorization.user_code);
    open_verification_url(&verification_url);
    let lifetime = u64::try_from(authorization.expires_in).unwrap_or(600);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(lifetime);
    let interval = Duration::from_secs(u64::from(authorization.interval.max(1)));
    let exchange = loop {
        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!("browser pairing expired; start pairing again");
        }
        tokio::time::sleep(interval).await;
        let status = client
            .device_authorization_status(&authorization.device_code)
            .await
            .context("poll browser pairing")?;
        match status.state.as_str() {
            "approved" => {
                break client
                    .exchange_device_authorization(&authorization.device_code)
                    .await
                    .context("exchange approved browser pairing")?;
            }
            "pending" => {}
            "denied" => anyhow::bail!("browser pairing was denied"),
            "expired" => anyhow::bail!("browser pairing expired; start pairing again"),
            "consumed" => anyhow::bail!("browser pairing code was already consumed"),
            _ => anyhow::bail!("control plane returned an unknown pairing state"),
        }
    };
    if let Some(existing) = &existing {
        // The approving operator chose a workspace. If that is not the
        // workspace this node already belongs to, the exchange admitted a new
        // node instead of rebinding this one, and overwriting the local key
        // would strand the original. Stop before touching anything on disk.
        if exchange.node_id.to_string() != existing.agent_id {
            anyhow::bail!(
                "rotation was approved into a different node ({}) than this installation ({}); \
                 the existing device key is unchanged. Approve the rotation from the workspace \
                 that owns this node.",
                exchange.node_id,
                existing.agent_id
            );
        }
    }
    let mode = match arguments.mode {
        AgentMode::Hosted => "hosted",
        AgentMode::SelfHosted | AgentMode::Local => "self-hosted",
    };
    let config = serde_json::json!({
        "mode": mode,
        "control_plane_url": base_url,
        "agent_id": exchange.node_id,
        "workspace_id": exchange.workspace_id,
        "environment_id": exchange.environment_id,
        "installation_id": installation_id_for_config,
        "device_key_file": key_path,
        "data_dir": arguments.data_dir,
        "local_bind": arguments.local_bind,
    });
    match intent {
        PairingIntent::FirstPairing => {
            write_new_device_key(&key_path, &identity.secret_bytes())?;
            if let Err(error) = write_new_json(&config_path, &config) {
                let _ = std::fs::remove_file(&key_path);
                return Err(error).context("persist paired node configuration");
            }
        }
        PairingIntent::KeyRotation => {
            // The control plane already trusts the new key, so the old one is
            // dead either way. Replace it atomically: a crash between these
            // steps must leave a complete key file, never a truncated one.
            replace_device_key(&key_path, &identity.secret_bytes())
                .context("replace this node's device key")?;
            replace_json(&config_path, &config).context("persist rotated node configuration")?;
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "node_id": exchange.node_id,
            "workspace_id": exchange.workspace_id,
            "environment_id": exchange.environment_id,
            "config_file": config_path,
            "rotated": intent == PairingIntent::KeyRotation,
        }))?
    );
    Ok(())
}

/// One installation's durable identity, as recorded at pairing time.
#[derive(Debug)]
struct ExistingInstallation {
    agent_id: String,
    installation_id: String,
}

/// Reads the identity a rotation must preserve.
///
/// Nodes paired before installation IDs were recorded cannot be rotated in
/// place: without one the control plane would admit a second node and leave
/// the original stranded. Say so plainly instead of silently doing that.
fn existing_installation(config_path: &Path) -> Result<ExistingInstallation> {
    let body = std::fs::read_to_string(config_path).with_context(|| {
        format!(
            "read {}; --rotate-key requires an already-paired node",
            config_path.display()
        )
    })?;
    let config: serde_json::Value =
        serde_json::from_str(&body).with_context(|| format!("parse {}", config_path.display()))?;
    let text = |key: &str| {
        config
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    };
    Ok(ExistingInstallation {
        agent_id: text("agent_id")
            .with_context(|| format!("{} does not record this node's ID", config_path.display()))?,
        installation_id: text("installation_id").with_context(|| {
            format!(
                "{} predates in-place key rotation and does not record an installation ID. \
                 Revoke this node in the control plane and pair it again with --pair.",
                config_path.display()
            )
        })?,
    })
}

fn open_verification_url(url: &Url) {
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(url.as_str()).spawn();
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("cmd")
        .args(["/C", "start", "", url.as_str()])
        .spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = std::process::Command::new("xdg-open")
        .arg(url.as_str())
        .spawn();
    if let Err(error) = result {
        eprintln!("Could not open the browser automatically: {error}");
    }
}

fn installation_hostname() -> String {
    ["COMPUTERNAME", "HOSTNAME"]
        .into_iter()
        .find_map(|name| std::env::var(name).ok())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "Piqae node".into())
}

fn write_new_device_key(path: &Path, secret: &[u8; 32]) -> Result<()> {
    let parent = path
        .parent()
        .context("device key path has no parent directory")?;
    std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    write_device_key_file(path, secret, true)
}

/// Replaces an existing device key without ever leaving a partial key on disk.
///
/// The key is written to a sibling temporary file with the same restricted
/// permissions, flushed, and then renamed over the original. A crash at any
/// point leaves either the old key or the new one, and both are complete.
fn replace_device_key(path: &Path, secret: &[u8; 32]) -> Result<()> {
    let parent = path
        .parent()
        .context("device key path has no parent directory")?;
    std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    let staged = path.with_extension("key.rotating");
    let _ = std::fs::remove_file(&staged);
    write_device_key_file(&staged, secret, true)?;
    std::fs::rename(&staged, path)
        .with_context(|| format!("replace {} with {}", path.display(), staged.display()))
}

fn write_device_key_file(path: &Path, secret: &[u8; 32], create_new: bool) -> Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(create_new);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .with_context(|| format!("create {}", path.display()))?;
    file.write_all(hex::encode(secret).as_bytes())
        .with_context(|| format!("write {}", path.display()))?;
    file.sync_all()
        .with_context(|| format!("sync {}", path.display()))?;
    drop(file);
    restrict_secret_to_owner(path)
}

/// Restricts a secret file to its owner on platforms without POSIX modes.
///
/// On Unix the mode is applied at creation. On Windows a newly created file
/// inherits the directory's ACL, which for a machine-mode install under
/// `ProgramData` grants every authenticated user read access — so the device
/// key must be given an explicit owner-only ACL after it is written.
#[cfg(not(windows))]
#[allow(
    clippy::unnecessary_wraps,
    reason = "matches the fallible Windows implementation of the same operation"
)]
const fn restrict_secret_to_owner(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(windows)]
fn restrict_secret_to_owner(path: &Path) -> Result<()> {
    windows_acl::restrict_to_owner(path)
}

fn write_new_json(path: &Path, value: &serde_json::Value) -> Result<()> {
    let body = serde_json::to_vec_pretty(value)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("create {}", path.display()))?;
    file.write_all(&body)
        .with_context(|| format!("write {}", path.display()))?;
    file.sync_all()
        .with_context(|| format!("sync {}", path.display()))
}

/// Rewrites a configuration file atomically, preserving the previous contents
/// if any step fails.
fn replace_json(path: &Path, value: &serde_json::Value) -> Result<()> {
    let body = serde_json::to_vec_pretty(value)?;
    let staged = path.with_extension("json.replacing");
    let _ = std::fs::remove_file(&staged);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staged)
        .with_context(|| format!("create {}", staged.display()))?;
    file.write_all(&body)
        .with_context(|| format!("write {}", staged.display()))?;
    file.sync_all()
        .with_context(|| format!("sync {}", staged.display()))?;
    drop(file);
    std::fs::rename(&staged, path).with_context(|| format!("replace {}", path.display()))
}

#[allow(
    clippy::cognitive_complexity,
    clippy::too_many_arguments,
    reason = "the single owner loop coordinates queue, control, and wakeup state without shared mutable engines"
)]
async fn control_loop(
    mut requests: mpsc::Receiver<ControlRequest>,
    mut engine: AgentEngine<SharedRuntimeExecutor>,
    content_store: ContentStore,
    uri_fetcher: UriFetcher,
    version: String,
    agent_id: Option<String>,
    connection: Arc<RwLock<ConnectionState>>,
    paused: Arc<AtomicBool>,
    cloud_sync_wakeup: Arc<Notify>,
    printer_inventory_dirty: Arc<AtomicBool>,
    connector_supervisor: mpsc::Sender<ConnectorSupervisorCommand>,
) {
    let mut scheduler = tokio::time::interval(Duration::from_millis(250));
    scheduler.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            request = requests.recv() => {
                let Some(request) = request else {
                    break;
                };
                let inventory_changed = matches!(
                    &request,
                    ControlRequest::SetPrinterExposure { .. }
                        | ControlRequest::CreateProfile { .. }
                        | ControlRequest::UpdateProfile { .. }
                        | ControlRequest::DeleteProfile { .. }
                        | ControlRequest::CommitProfileCapture { .. }
                        | ControlRequest::ConfirmLoadedMedia { .. }
                );
                let sync_relevant = inventory_changed
                    || matches!(&request, ControlRequest::Pause { .. } | ControlRequest::Resume { .. });
                if inventory_changed {
                    printer_inventory_dirty.store(true, Ordering::Release);
                }
                handle_control_request(
                    request,
                    &mut engine,
                    &content_store,
                    &uri_fetcher,
                    &version,
                    agent_id.as_deref(),
                    &connection,
                    &paused,
                    &connector_supervisor,
                ).await;
                if sync_relevant {
                    cloud_sync_wakeup.notify_one();
                }
                if inventory_changed
                    && let Err(error) = connector_supervisor
                        .try_send(ConnectorSupervisorCommand::RefreshPrinters)
                {
                    reject_connector_supervisor_command(error);
                }
            }
            _ = scheduler.tick(), if !paused.load(Ordering::Relaxed) => {
                let before = engine.store().latest_pending_cloud_event_sequence();
                if let Err(error) = engine.run_once().await {
                    error!(%error, "local print scheduler iteration failed");
                }
                let after = engine.store().latest_pending_cloud_event_sequence();
                if matches!((before, after), (Ok(before), Ok(after)) if after > before) {
                    cloud_sync_wakeup.notify_one();
                }
            }
        }
    }
    warn!("local control channel closed");
}

async fn connector_scheduler_loop(
    connector_id: String,
    mut engine: AgentEngine<SharedRuntimeExecutor>,
    paused: Arc<AtomicBool>,
    cloud_sync_wakeup: Arc<Notify>,
    stop: StopSignal,
) {
    let mut scheduler = tokio::time::interval(Duration::from_millis(250));
    scheduler.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = scheduler.tick() => {}
            () = stop.cancelled() => break,
        }
        if paused.load(Ordering::Relaxed) {
            continue;
        }
        let before = engine.store().latest_pending_cloud_event_sequence();
        if let Err(error) = engine.run_once().await {
            error!(%connector_id, %error, "connector print scheduler iteration failed");
        }
        let after = engine.store().latest_pending_cloud_event_sequence();
        if matches!((before, after), (Ok(before), Ok(after)) if after > before) {
            cloud_sync_wakeup.notify_one();
        }
    }
}

#[allow(
    clippy::cognitive_complexity,
    clippy::needless_collect,
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "supervisor owns command, recovery, liveness, and shutdown transitions in one audit point"
)]
async fn connector_supervisor_loop(
    data_dir: PathBuf,
    mut commands: mpsc::Receiver<ConnectorSupervisorCommand>,
    executor: SharedRuntimeExecutor,
    uri_fetcher: UriFetcher,
    printer_discovery: PrinterDiscovery,
    support_packs: Arc<SupportPackRegistry>,
    mut legacy_cloud_worker: Option<LegacyCloudWorker>,
    connections: ConnectorConnectionTracker,
) {
    let mut workers = std::collections::BTreeMap::<String, ConnectorWorker>::new();
    if let Err(error) =
        retire_legacy_cloud_worker_if_needed(&data_dir, &mut legacy_cloud_worker).await
    {
        error!(%error, "initial legacy cloud worker retirement failed");
    }
    if let Err(error) = reload_connector_workers(
        &data_dir,
        &mut workers,
        &executor,
        &uri_fetcher,
        &printer_discovery,
        &support_packs,
        &connections,
    )
    .await
    {
        error!(%error, "initial connector worker load failed");
    }
    let mut recovery = tokio::time::interval(Duration::from_secs(30));
    recovery.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut liveness = tokio::time::interval(Duration::from_secs(1));
    liveness.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        let command = tokio::select! {
            command = commands.recv() => command,
            _ = liveness.tick() => {
                if legacy_cloud_worker
                    .as_ref()
                    .is_some_and(|worker| worker.task.is_finished())
                {
                    error!("legacy cloud sync task exited unexpectedly; stopping node for supervised restart");
                    break;
                }
                if workers.values().any(connector_worker_has_exited)
                    && let Err(error) = reload_connector_workers(
                        &data_dir,
                        &mut workers,
                        &executor,
                        &uri_fetcher,
                        &printer_discovery,
                        &support_packs,
                        &connections,
                    ).await
                {
                    warn!(%error, "connector worker liveness recovery deferred");
                }
                continue;
            }
            _ = recovery.tick() => {
                if let Err(error) = retire_legacy_cloud_worker_if_needed(&data_dir, &mut legacy_cloud_worker).await {
                    warn!(%error, "periodic legacy cloud worker retirement deferred");
                    continue;
                }
                if let Err(error) = reload_connector_workers(
                    &data_dir,
                    &mut workers,
                    &executor,
                    &uri_fetcher,
                    &printer_discovery,
                    &support_packs,
                    &connections,
                ).await {
                    warn!(%error, "periodic connector recovery deferred");
                }
                continue;
            }
        };
        let Some(command) = command else {
            break;
        };
        match command {
            ConnectorSupervisorCommand::Reload { respond_to } => {
                let result =
                    match retire_legacy_cloud_worker_if_needed(&data_dir, &mut legacy_cloud_worker)
                        .await
                    {
                        Ok(()) => {
                            reload_connector_workers(
                                &data_dir,
                                &mut workers,
                                &executor,
                                &uri_fetcher,
                                &printer_discovery,
                                &support_packs,
                                &connections,
                            )
                            .await
                        }
                        Err(error) => Err(error),
                    }
                    .map_err(|error| {
                        control_failure("connector_reload_failed", &error.to_string())
                    });
                let _ = respond_to.send(result);
            }
            ConnectorSupervisorCommand::Revoke {
                connector_id,
                respond_to,
            } => {
                let result = async {
                    let mut registry = connector_runtime::ConnectorRegistry::load(&data_dir)?;
                    if !registry.revoke(&connector_id)? {
                        anyhow::bail!("connector was not active");
                    }
                    stop_connector_worker(&mut workers, &connector_id, &connections).await?;
                    Ok(())
                }
                .await
                .map_err(|error: anyhow::Error| {
                    control_failure("connector_revoke_failed", &error.to_string())
                });
                let _ = respond_to.send(result);
            }
            ConnectorSupervisorCommand::Details { respond_to } => {
                let result = connector_runtime::ConnectorRegistry::load(&data_dir)
                    .map(|registry| registry.enabled().cloned().collect::<Vec<_>>())
                    .map_err(|error| {
                        control_failure("connector_details_failed", &error.to_string())
                    });
                let details = match result {
                    Ok(records) => {
                        let local_printers = AgentStore::open(data_dir.join("agent.sqlite3"))
                            .and_then(|store| store.present_printers())
                            .map_err(|error| {
                                control_failure("connector_details_failed", &error.to_string())
                            });
                        let local_printers = match local_printers {
                            Ok(printers) => printers,
                            Err(failure) => {
                                let _ = respond_to.send(Err(failure));
                                continue;
                            }
                        };
                        let printer_groups = {
                            let coordinator = executor.coordinator.lock().await;
                            local_printers
                                .iter()
                                .filter_map(|printer| {
                                    coordinator
                                        .coordination_key(&printer.native_id)
                                        .map(|key| (printer.printer_id.clone(), key.to_owned()))
                                })
                                .collect::<Vec<_>>()
                        };
                        let cross_authority_connectors =
                            connector_runtime::cross_authority_connectors(
                                &records,
                                &printer_groups,
                            );
                        let mut details = Vec::with_capacity(records.len());
                        for record in records {
                            let connection =
                                enum_string(connections.state(&record.connector_id).await);
                            let permission = enum_string(record.printer_grant);
                            let selected_printer_count = record.allowed_printer_ids.len();
                            let allowed_printers = connector_allowed_printers(&record);
                            let eligible_printer_count = local_printers
                                .iter()
                                .filter(|printer| {
                                    printer_is_allowed(
                                        allowed_printers.as_ref(),
                                        &printer.printer_id,
                                    )
                                })
                                .count();
                            let (last_sync_error_code, inventory_refresh_pending) =
                                if let Some(worker) = workers.get(&record.connector_id) {
                                    (
                                        worker.last_sync_error_code.read().await.clone(),
                                        worker.printer_inventory_dirty.load(Ordering::Acquire),
                                    )
                                } else {
                                    (Some("connector_worker_unavailable".to_owned()), true)
                                };
                            let inventory_revision =
                                connector_runtime::ConnectorRegistry::load(&data_dir)
                                    .ok()
                                    .and_then(|registry| registry.paths(&record.connector_id).ok())
                                    .and_then(|paths| AgentStore::open(paths.database).ok())
                                    .and_then(|store| {
                                        store.setting("printer_inventory_revision").ok()
                                    })
                                    .flatten()
                                    .and_then(|revision| revision.parse::<u64>().ok())
                                    .unwrap_or(0);
                            let cross_authority_route_warning =
                                cross_authority_connectors.contains(&record.connector_id);
                            details.push(LocalConnectorDetail {
                                connector_id: record.connector_id,
                                display_name: record
                                    .display_name
                                    .unwrap_or_else(|| "Piqae connection".to_owned()),
                                workspace_name: record.workspace_name,
                                authorization_type: record.authorization_type,
                                workspace_id: record.workspace_id,
                                environment_id: record.environment_id,
                                requesting_service_account_id: record.requesting_service_account_id,
                                endpoint: record.control_plane_url.origin().ascii_serialization(),
                                connection,
                                permission,
                                allowed_printer_ids: record.allowed_printer_ids,
                                selected_printer_count,
                                last_sync_error_code,
                                local_printer_count: local_printers.len(),
                                eligible_printer_count,
                                inventory_revision,
                                inventory_refresh_pending,
                                cross_authority_route_warning,
                                manage_url: record.manage_url.map(|url| url.to_string()),
                            });
                        }
                        Ok(details)
                    }
                    Err(failure) => Err(failure),
                };
                let _ = respond_to.send(details);
            }
            ConnectorSupervisorCommand::RefreshPrinters => {
                for worker in workers.values() {
                    worker
                        .printer_inventory_dirty
                        .store(true, Ordering::Release);
                    worker.wakeup.notify_one();
                }
            }
        }
    }
    if let Some(worker) = legacy_cloud_worker.take() {
        stop_legacy_cloud_worker(worker).await;
    }
    for id in workers.keys().cloned().collect::<Vec<_>>() {
        if let Err(error) = stop_connector_worker(&mut workers, &id, &connections).await {
            warn!(connector_id = %id, %error, "connector worker shutdown was forced");
        }
    }
}

async fn retire_legacy_cloud_worker_if_needed(
    data_dir: &Path,
    worker: &mut Option<LegacyCloudWorker>,
) -> Result<()> {
    if worker.is_none()
        || connector_runtime::ConnectorRegistry::load(data_dir)?
            .enabled()
            .next()
            .is_none()
    {
        return Ok(());
    }
    if let Some(worker) = worker.take() {
        stop_legacy_cloud_worker(worker).await;
    }
    Ok(())
}

async fn stop_legacy_cloud_worker(worker: LegacyCloudWorker) {
    worker.stop.stop();
    let mut task = worker.task;
    match tokio::time::timeout(Duration::from_secs(10), &mut task).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => error!(%error, "legacy cloud worker task failed"),
        Err(_) => {
            warn!("legacy cloud worker exceeded the shutdown deadline; aborting it");
            task.abort();
            let _ = task.await;
        }
    }
}

#[allow(
    clippy::cognitive_complexity,
    clippy::needless_collect,
    reason = "reload independently reconciles removals, grant changes, paths, and starts while aggregating per-connector failures"
)]
async fn reload_connector_workers(
    data_dir: &Path,
    workers: &mut std::collections::BTreeMap<String, ConnectorWorker>,
    executor: &SharedRuntimeExecutor,
    uri_fetcher: &UriFetcher,
    printer_discovery: &PrinterDiscovery,
    support_packs: &Arc<SupportPackRegistry>,
    connections: &ConnectorConnectionTracker,
) -> Result<()> {
    let registry = connector_runtime::ConnectorRegistry::load(data_dir)?;
    let enabled = registry
        .enabled()
        .map(|r| (r.connector_id.clone(), r.clone()))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut failures = Vec::new();
    for id in workers
        .keys()
        .filter(|id| !enabled.contains_key(*id))
        .cloned()
        .collect::<Vec<_>>()
    {
        if let Err(error) = stop_connector_worker(workers, &id, connections).await {
            warn!(connector_id = %id, %error, "removed connector worker shutdown was forced");
            failures.push(format!("{id}: {error}"));
        }
    }
    for (id, record) in enabled {
        let worker_exited = workers.get(&id).is_some_and(connector_worker_has_exited);
        if worker_exited {
            connections.update(&id, ConnectionState::Degraded).await;
            error!(connector_id = %id, "connector worker task exited unexpectedly; restarting connector runtime");
        }
        if workers
            .get(&id)
            .is_some_and(|worker| connector_worker_matches(worker, &record))
        {
            continue;
        }
        if workers.contains_key(&id) {
            if let Err(error) = stop_connector_worker(workers, &id, connections).await {
                warn!(connector_id = %id, %error, "changed connector worker shutdown was forced");
                failures.push(format!("{id}: {error}"));
                continue;
            }
        }
        let paths = match registry.paths(&id) {
            Ok(paths) => paths,
            Err(error) => {
                warn!(connector_id = %id, %error, "connector runtime paths could not be resolved");
                failures.push(format!("{id}: {error}"));
                continue;
            }
        };
        match start_connector_worker(
            record,
            paths,
            data_dir.join("agent.sqlite3"),
            executor.clone(),
            uri_fetcher.clone(),
            printer_discovery.clone(),
            Arc::clone(support_packs),
            connections.clone(),
        )
        .await
        {
            Ok(worker) => {
                workers.insert(id, worker);
            }
            Err(error) => {
                warn!(connector_id = %id, %error, "connector worker could not be started");
                failures.push(format!("{id}: {error}"));
            }
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        anyhow::bail!("connector reload partially failed: {}", failures.join("; "))
    }
}

fn connector_worker_has_exited(worker: &ConnectorWorker) -> bool {
    worker.sync.is_finished()
        || worker.scheduler.is_finished()
        || worker.connection_watch.is_finished()
}

fn connector_worker_matches(
    worker: &ConnectorWorker,
    record: &connector_runtime::ConnectorRecord,
) -> bool {
    worker.record == *record && !connector_worker_has_exited(worker)
}

#[allow(
    clippy::too_many_arguments,
    reason = "connector workers receive explicit isolated runtime capabilities"
)]
async fn start_connector_worker(
    record: connector_runtime::ConnectorRecord,
    paths: connector_runtime::ConnectorRuntimePaths,
    inventory_database: PathBuf,
    executor: SharedRuntimeExecutor,
    uri_fetcher: UriFetcher,
    printer_discovery: PrinterDiscovery,
    support_packs: Arc<SupportPackRegistry>,
    connections: ConnectorConnectionTracker,
) -> Result<ConnectorWorker> {
    let parent = paths
        .database
        .parent()
        .context("connector database has no parent")?;
    std::fs::create_dir_all(parent)?;
    let store = AgentStore::open(&paths.database)?;
    if !store.integrity_check()? {
        anyhow::bail!("connector database integrity check failed");
    }
    let paused = Arc::new(AtomicBool::new(
        store.setting("paused")?.as_deref() == Some("true"),
    ));
    let wakeup = Arc::new(Notify::new());
    let printer_inventory_dirty = Arc::new(AtomicBool::new(true));
    let last_sync_error_code = Arc::new(RwLock::new(None));
    let sync_stop = StopSignal::default();
    let scheduler_stop = StopSignal::default();
    // Resolve every fallible runtime dependency before spawning either half;
    // a failed key/content setup must not leave an orphan scheduler behind.
    let cloud = cloud_configuration_from_connector(&record, &paths.device_key)?;
    let content = ContentStore::open(paths.content).await?;
    let connector_executor = executor.for_connector(record.connector_id.clone());
    let route_coordinator = Arc::clone(&executor.coordinator);
    let observation_cache = Arc::clone(&executor.observation_cache);
    let scheduler = tokio::spawn(connector_scheduler_loop(
        record.connector_id.clone(),
        AgentEngine::new(store, connector_executor, SystemClock),
        paused.clone(),
        wakeup.clone(),
        scheduler_stop.clone(),
    ));
    let connector_connection = Arc::new(RwLock::new(ConnectionState::Connecting));
    connections
        .update(&record.connector_id, ConnectionState::Connecting)
        .await;
    let sync = tokio::spawn(cloud_sync_loop(
        cloud,
        paths.database,
        inventory_database,
        content,
        uri_fetcher,
        printer_discovery,
        route_coordinator,
        observation_cache,
        support_packs,
        Arc::clone(&connector_connection),
        paused,
        Arc::clone(&wakeup),
        Arc::clone(&printer_inventory_dirty),
        Arc::clone(&last_sync_error_code),
        sync_stop.clone(),
    ));
    let connection_stop = StopSignal::default();
    let connection_watch = tokio::spawn(watch_connector_connection(
        record.connector_id.clone(),
        connector_connection,
        connections,
        connection_stop.clone(),
    ));
    Ok(ConnectorWorker {
        record,
        printer_inventory_dirty,
        wakeup,
        last_sync_error_code,
        sync_stop,
        scheduler_stop,
        connection_stop,
        sync,
        scheduler,
        connection_watch,
    })
}

async fn watch_connector_connection(
    connector_id: String,
    connection: Arc<RwLock<ConnectionState>>,
    connections: ConnectorConnectionTracker,
    stop: StopSignal,
) {
    let mut previous = ConnectionState::Connecting;
    let mut interval = tokio::time::interval(Duration::from_millis(250));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = interval.tick() => {
                let current = *connection.read().await;
                if current != previous {
                    connections.update(&connector_id, current).await;
                    previous = current;
                }
            }
            () = stop.cancelled() => break,
        }
    }
}

async fn stop_connector_worker(
    workers: &mut std::collections::BTreeMap<String, ConnectorWorker>,
    connector_id: &str,
    connections: &ConnectorConnectionTracker,
) -> Result<()> {
    let Some(worker) = workers.remove(connector_id) else {
        return Ok(());
    };
    worker.sync_stop.stop();
    worker.scheduler_stop.stop();
    worker.connection_stop.stop();
    let mut sync = worker.sync;
    let mut scheduler = worker.scheduler;
    let mut connection_watch = worker.connection_watch;
    if tokio::time::timeout(Duration::from_secs(10), async {
        log_connector_task_exit(connector_id, "cloud sync", (&mut sync).await);
        log_connector_task_exit(connector_id, "print scheduler", (&mut scheduler).await);
        log_connector_task_exit(
            connector_id,
            "connection watcher",
            (&mut connection_watch).await,
        );
    })
    .await
    .is_err()
    {
        warn!(%connector_id, "connector workers exceeded the shutdown deadline; aborting them");
        sync.abort();
        scheduler.abort();
        connection_watch.abort();
        let _ = sync.await;
        let _ = scheduler.await;
        let _ = connection_watch.await;
        connections.remove(connector_id).await;
        anyhow::bail!("connector workers exceeded the shutdown deadline and were aborted");
    }
    connections.remove(connector_id).await;
    Ok(())
}

fn log_connector_task_exit(
    connector_id: &str,
    task: &str,
    result: std::result::Result<(), tokio::task::JoinError>,
) {
    if let Err(error) = result {
        error!(%connector_id, %task, %error, "connector worker task failed");
    }
}

#[allow(
    clippy::too_many_arguments,
    clippy::cognitive_complexity,
    clippy::cognitive_complexity,
    clippy::too_many_lines,
    reason = "keeps the exhaustive authenticated control command dispatch in one audit point"
)]
async fn handle_control_request(
    request: ControlRequest,
    engine: &mut AgentEngine<SharedRuntimeExecutor>,
    content_store: &ContentStore,
    uri_fetcher: &UriFetcher,
    version: &str,
    agent_id: Option<&str>,
    connection: &RwLock<ConnectionState>,
    paused: &AtomicBool,
    connector_supervisor: &mpsc::Sender<ConnectorSupervisorCommand>,
) {
    match request {
        ControlRequest::Status { respond_to } => {
            let current_connection = *connection.read().await;
            let _ = respond_to.send(local_status(
                engine.store(),
                agent_id,
                version,
                current_connection,
                paused,
            ));
        }
        ControlRequest::Printers { respond_to } => match refresh_local_printers(engine).await {
            Ok(printers) => {
                if let Err(error) =
                    connector_supervisor.try_send(ConnectorSupervisorCommand::RefreshPrinters)
                {
                    reject_connector_supervisor_command(error);
                }
                let _ = respond_to.send(printers);
            }
            Err(error) => {
                warn!(code = %error.code, message = %error.message, "printer discovery failed");
                let _ = respond_to.send(Vec::new());
            }
        },
        ControlRequest::SetPrinterExposure {
            printer_id,
            exposed,
            respond_to,
        } => {
            let result = engine
                .store_mut()
                .set_printer_exposed(&printer_id, exposed, Utc::now().timestamp_millis())
                .map_err(storage_control_failure)
                .and_then(|()| local_printer(engine.store(), &printer_id));
            let _ = respond_to.send(result);
        }
        ControlRequest::Profiles {
            printer_id,
            respond_to,
        } => {
            let result = local_profiles(engine.store(), &printer_id);
            let _ = respond_to.send(result);
        }
        ControlRequest::CreateProfile {
            printer_id,
            request,
            respond_to,
        } => {
            let result = create_profile(engine.store_mut(), &printer_id, &request);
            let _ = respond_to.send(result);
        }
        ControlRequest::UpdateProfile {
            printer_id,
            profile_id,
            request,
            respond_to,
        } => {
            let result = update_profile(engine.store_mut(), &printer_id, &profile_id, &request);
            let _ = respond_to.send(result);
        }
        ControlRequest::DeleteProfile {
            printer_id,
            profile_id,
            expected_revision,
            respond_to,
        } => {
            let result = engine
                .store_mut()
                .delete_named_profile(
                    &printer_id,
                    &profile_id,
                    expected_revision,
                    Utc::now().timestamp_millis(),
                )
                .map_err(storage_control_failure);
            let _ = respond_to.send(result);
        }
        ControlRequest::BeginProfileCapture {
            printer_id,
            request,
            respond_to,
        } => {
            let result = begin_profile_capture(engine.store_mut(), &printer_id, &request);
            let _ = respond_to.send(result);
        }
        ControlRequest::CommitProfileCapture {
            session_id,
            capture_token,
            capture,
            respond_to,
        } => {
            let result =
                commit_profile_capture(engine.store_mut(), &session_id, &capture_token, *capture);
            let _ = respond_to.send(result);
        }
        ControlRequest::CancelProfileCapture {
            session_id,
            capture_token,
            respond_to,
        } => {
            let result = engine
                .store_mut()
                .cancel_profile_capture(
                    &session_id,
                    &capture_token_digest(&capture_token),
                    LOCAL_PROFILE_HOST_ID,
                    Utc::now().timestamp_millis(),
                )
                .map_err(storage_control_failure);
            let _ = respond_to.send(result);
        }
        ControlRequest::ValidateProfile {
            profile_id,
            revision,
            respond_to,
        } => {
            let result = validate_profile_revision(engine.store_mut(), &profile_id, revision);
            let _ = respond_to.send(result);
        }
        ControlRequest::ConfirmLoadedMedia {
            request,
            respond_to,
        } => {
            let result = confirm_loaded_media(engine.store_mut(), request);
            let _ = respond_to.send(result);
        }
        ControlRequest::PrinterQueue {
            printer_id,
            respond_to,
        } => {
            let result = printer_queue(engine, &printer_id).await;
            let _ = respond_to.send(result);
        }
        ControlRequest::JobHistory {
            offset,
            limit,
            respond_to,
        } => {
            let result = local_job_history(engine.store(), offset, limit);
            let _ = respond_to.send(result);
        }
        ControlRequest::ReprintJob {
            job_id,
            idempotency_key,
            confirmed,
            respond_to,
        } => {
            let result = reprint_local_job(
                engine,
                &job_id,
                &idempotency_key,
                confirmed,
                paused.load(Ordering::Relaxed),
            );
            let _ = respond_to.send(result);
        }
        ControlRequest::ConnectorDetails { respond_to } => {
            if let Err(error) =
                connector_supervisor.try_send(ConnectorSupervisorCommand::Details { respond_to })
            {
                reject_connector_supervisor_command(error);
            }
        }
        ControlRequest::TestPage {
            printer_id,
            profile_id,
            confirmed,
            respond_to,
        } => {
            let result = submit_test_page(
                engine,
                content_store,
                &printer_id,
                &profile_id,
                confirmed,
                paused.load(Ordering::Relaxed),
            )
            .await;
            let _ = respond_to.send(result);
        }
        ControlRequest::Pause { respond_to } => {
            let result = engine
                .store_mut()
                .set_setting("paused", "true")
                .map_err(|error| control_failure("pause_failed", &error.to_string()))
                .map(|()| paused.store(true, Ordering::Relaxed));
            let _ = respond_to.send(result);
        }
        ControlRequest::Resume { respond_to } => {
            let result = engine
                .store_mut()
                .set_setting("paused", "false")
                .map_err(|error| control_failure("resume_failed", &error.to_string()))
                .map(|()| paused.store(false, Ordering::Relaxed));
            let _ = respond_to.send(result);
        }
        ControlRequest::ReloadConnectors { respond_to } => {
            if let Err(error) =
                connector_supervisor.try_send(ConnectorSupervisorCommand::Reload { respond_to })
            {
                reject_connector_supervisor_command(error);
            }
        }
        ControlRequest::RevokeConnector {
            connector_id,
            respond_to,
        } => {
            if let Err(error) = connector_supervisor.try_send(ConnectorSupervisorCommand::Revoke {
                connector_id,
                respond_to,
            }) {
                reject_connector_supervisor_command(error);
            }
        }
        ControlRequest::SubmitJob {
            request,
            respond_to,
        } => {
            let result = submit_local_job(
                engine,
                content_store,
                uri_fetcher,
                *request,
                paused.load(Ordering::Relaxed),
            )
            .await;
            let _ = respond_to.send(result);
        }
    }
}

fn local_status(
    store: &AgentStore,
    agent_id: Option<&str>,
    version: &str,
    connection: ConnectionState,
    paused: &AtomicBool,
) -> LocalStatus {
    let counts = match store.queue_counts() {
        Ok(counts) => counts,
        Err(error) => {
            error!(%error, "failed to read local queue counts");
            QueueCounts::default()
        }
    };
    let printer_warnings = match store.present_printer_warning_count() {
        Ok(count) => count,
        Err(error) => {
            error!(%error, "failed to read printer warning count");
            0
        }
    };
    LocalStatus {
        agent_id: agent_id.map(ToOwned::to_owned),
        workspace_name: None,
        version: version.to_owned(),
        connection,
        queued_jobs: counts.queued,
        active_jobs: counts.active,
        printer_warnings,
        paused: paused.load(Ordering::Relaxed),
    }
}

async fn submit_local_job(
    engine: &mut AgentEngine<SharedRuntimeExecutor>,
    content_store: &ContentStore,
    uri_fetcher: &UriFetcher,
    request: LocalCreateJob,
    paused: bool,
) -> Result<LocalJobAccepted, ControlFailure> {
    if paused {
        return Err(control_failure(
            "agent_paused",
            "the agent is not accepting new local jobs",
        ));
    }
    let printer = resolve_exposed_printer(engine.store(), &request.printer_id)?;
    if let Some(claimed_native_id) = &request.printer_native_id
        && claimed_native_id != &printer.native_id
    {
        return Err(control_failure(
            "printer_native_id_mismatch",
            "native printer IDs are resolved by the agent and cannot be overridden",
        ));
    }
    validate_options(&printer, &request.options)?;
    let mut request = request;
    request.printer_native_id = Some(printer.native_id);
    let input: Box<dyn tokio::io::AsyncRead + Unpin + Send> = match &request.content {
        LocalContent::Base64 { data } => {
            let bytes = STANDARD
                .decode(data)
                .map_err(|_| control_failure("invalid_base64", "content is not valid base64"))?;
            Box::new(std::io::Cursor::new(bytes))
        }
        LocalContent::Uri { uri } => {
            let stored = uri_fetcher
                .fetch_to_store(content_store, uri, None, None)
                .await
                .map_err(|error| control_failure("content_unavailable", &error.to_string()))?;
            return accept_stored_local_job(engine, request, stored, None).await;
        }
    };
    let stored = content_store
        .put(input)
        .await
        .map_err(|error| control_failure("content_store_failed", &error.to_string()))?;
    accept_stored_local_job(engine, request, stored, None).await
}

async fn refresh_local_printers(
    engine: &mut AgentEngine<SharedRuntimeExecutor>,
) -> Result<Vec<LocalPrinter>, ControlFailure> {
    let discovered = engine.executor_mut().discover_printers().await?;
    let present_native_ids = discovered
        .iter()
        .map(|printer| printer.native_id.clone())
        .collect::<Vec<_>>();
    let observed_unix_ms = Utc::now().timestamp_millis();
    for printer in discovered {
        let state = enum_string(printer.state);
        let capabilities_json = serde_json::to_string(&printer.capabilities)
            .map_err(|error| control_failure("capabilities_invalid", &error.to_string()))?;
        let native_options_json = serde_json::to_string(&printer.native_options)
            .map_err(|error| control_failure("capabilities_invalid", &error.to_string()))?;
        let stored = engine
            .store_mut()
            .upsert_printer(
                &printer.native_id,
                &printer.name,
                &state,
                printer.is_default,
                &capabilities_json,
                observed_unix_ms,
            )
            .map_err(storage_control_failure)?;
        engine
            .store_mut()
            .store_printer_profile(
                &stored.printer_id,
                None,
                &capabilities_json,
                &native_options_json,
                observed_unix_ms,
            )
            .map_err(storage_control_failure)?;
        engine
            .store_mut()
            .ensure_current_printer_defaults_profile(&stored.printer_id, observed_unix_ms)
            .map_err(storage_control_failure)?;
    }
    engine
        .store_mut()
        .reconcile_printer_presence(&present_native_ids)
        .map_err(storage_control_failure)?;
    engine
        .store()
        .present_printers()
        .map_err(storage_control_failure)?
        .into_iter()
        .map(|printer| local_printer_from_stored(engine.store(), printer))
        .collect()
}

fn local_printer(store: &AgentStore, printer_id: &str) -> Result<LocalPrinter, ControlFailure> {
    let printer = store
        .printer(printer_id)
        .map_err(storage_control_failure)?
        .ok_or_else(|| control_failure("printer_not_found", "printer was not found"))?;
    local_printer_from_stored(store, printer)
}

fn local_printer_from_stored(
    store: &AgentStore,
    printer: StoredPrinter,
) -> Result<LocalPrinter, ControlFailure> {
    let counts = store
        .printer_queue_counts(&printer.printer_id)
        .map_err(storage_control_failure)?;
    let queue_counts = LocalPrinterQueueCounts {
        queued: counts.queued,
        active: counts.active,
    };
    Ok(LocalPrinter {
        printer_id: printer.printer_id.clone(),
        native_id: printer.native_id,
        name: printer.name,
        state: printer.state,
        is_default: printer.is_default,
        exposed: printer.exposed,
        capability_revision: printer.profile_revision,
        capabilities: serde_json::from_str(&printer.capabilities_json)
            .map_err(|error| control_failure("capabilities_invalid", &error.to_string()))?,
        native_options: serde_json::from_str(&printer.native_options_json)
            .map_err(|error| control_failure("capabilities_invalid", &error.to_string()))?,
        profiles: local_profiles(store, &printer.printer_id)?,
        queue_counts,
    })
}

fn local_profiles(
    store: &AgentStore,
    printer_id: &str,
) -> Result<Vec<LocalPrinterProfile>, ControlFailure> {
    store
        .named_profiles(printer_id)
        .map_err(storage_control_failure)?
        .into_iter()
        .map(|profile| local_profile(store, profile))
        .collect()
}

fn local_profile(
    store: &AgentStore,
    profile: StoredNamedProfile,
) -> Result<LocalPrinterProfile, ControlFailure> {
    let status = parse_stored_enum(&profile.status, "profile status")?;
    let native_kind = if profile.native_kind.is_empty() {
        None
    } else {
        Some(parse_stored_enum(
            &profile.native_kind,
            "native profile kind",
        )?)
    };
    let dependencies = store
        .profile_dependencies(&profile.profile_id, profile.revision)
        .map_err(storage_control_failure)?;
    Ok(LocalPrinterProfile {
        profile_id: profile.profile_id,
        revision: profile.revision,
        name: profile.name,
        is_default: profile.is_default,
        options: serde_json::from_str(&profile.options_json)
            .map_err(|error| control_failure("profile_invalid", &error.to_string()))?,
        status,
        native_kind,
        native_digest: profile.native_digest,
        driver_fingerprint: serde_json::from_str(&profile.driver_fingerprint_json)
            .map_err(|error| control_failure("profile_invalid", &error.to_string()))?,
        summary: serde_json::from_str(&profile.summary_json)
            .map_err(|error| control_failure("profile_invalid", &error.to_string()))?,
        stock_id: profile.stock_id,
        dependencies,
        safe_overrides: serde_json::from_str(&profile.safe_overrides_json)
            .map_err(|error| control_failure("profile_invalid", &error.to_string()))?,
        last_validated_unix_ms: profile.last_validated_unix_ms,
        last_test_job_id: profile.last_test_job_id,
        published: profile.published,
        uses_current_printer_defaults: profile.uses_current_printer_defaults,
    })
}

fn create_profile(
    store: &mut AgentStore,
    printer_id: &str,
    request: &ProfileCreate,
) -> Result<LocalPrinterProfile, ControlFailure> {
    let printer = store
        .printer(printer_id)
        .map_err(storage_control_failure)?
        .ok_or_else(|| control_failure("printer_not_found", "printer was not found"))?;
    validate_options(&printer, &request.options)?;
    let options_json = serde_json::to_string(&request.options)
        .map_err(|error| control_failure("profile_invalid", &error.to_string()))?;
    let profile = store
        .create_named_profile(
            printer_id,
            &request.name,
            request.is_default,
            &options_json,
            Utc::now().timestamp_millis(),
        )
        .map_err(storage_control_failure)?;
    local_profile(store, profile)
}

fn update_profile(
    store: &mut AgentStore,
    printer_id: &str,
    profile_id: &str,
    request: &ProfileUpdate,
) -> Result<LocalPrinterProfile, ControlFailure> {
    let printer = store
        .printer(printer_id)
        .map_err(storage_control_failure)?
        .ok_or_else(|| control_failure("printer_not_found", "printer was not found"))?;
    validate_options(&printer, &request.options)?;
    let options_json = serde_json::to_string(&request.options)
        .map_err(|error| control_failure("profile_invalid", &error.to_string()))?;
    let profile = store
        .update_named_profile(
            printer_id,
            profile_id,
            request.expected_revision,
            &request.name,
            request.is_default,
            &options_json,
            Utc::now().timestamp_millis(),
        )
        .map_err(storage_control_failure)?;
    local_profile(store, profile)
}

fn begin_profile_capture(
    store: &mut AgentStore,
    printer_id: &str,
    request: &piqae_local_api::ProfileCaptureBeginRequest,
) -> Result<ProfileCaptureAuthorized, ControlFailure> {
    match request.operation {
        ProfileCaptureOperation::Create
            if request.profile_id.is_some() || request.expected_revision.is_some() =>
        {
            return Err(control_failure(
                "profile_invalid",
                "create capture cannot reference an existing profile",
            ));
        }
        ProfileCaptureOperation::Edit | ProfileCaptureOperation::Clone
            if request.profile_id.is_none() || request.expected_revision.is_none() =>
        {
            return Err(control_failure(
                "profile_invalid",
                "edit and clone capture require a profile and exact revision",
            ));
        }
        _ => {}
    }

    let printer = store
        .printer(printer_id)
        .map_err(storage_control_failure)?
        .ok_or_else(|| control_failure("printer_not_found", "printer was not found"))?;
    if !printer.present {
        return Err(control_failure(
            "printer_not_present",
            "printer is not currently installed on this node",
        ));
    }
    let existing = match (request.profile_id.as_deref(), request.expected_revision) {
        (Some(profile_id), Some(revision)) => store
            .named_profile_revision(printer_id, profile_id, revision)
            .map_err(storage_control_failure)?,
        _ => None,
    };
    let native_configuration = if let Some(profile) = &existing {
        store
            .native_profile_blob(&profile.profile_id, profile.revision)
            .map_err(storage_control_failure)?
            .map(|blob| {
                Ok(NativeProfileSeed {
                    kind: parse_stored_enum(&blob.native_kind, "native profile kind")?,
                    schema_version: blob.schema_version,
                    digest: blob.digest,
                    native_blob_base64: STANDARD.encode(blob.native_blob),
                })
            })
            .transpose()?
    } else {
        None
    };
    let safe_overrides = existing
        .as_ref()
        .map(|profile| serde_json::from_str(&profile.safe_overrides_json))
        .transpose()
        .map_err(|error| control_failure("profile_invalid", &error.to_string()))?
        .unwrap_or_default();

    let session_id = piqae_domain::ProfileCaptureSessionId::new().to_string();
    let (capture_token, token_digest) = generate_capture_token();
    let created_unix_ms = Utc::now().timestamp_millis();
    let lifetime_ms = i64::try_from(PROFILE_CAPTURE_LIFETIME.as_millis())
        .map_err(|error| control_failure("profile_invalid", &error.to_string()))?;
    let expires_unix_ms = created_unix_ms.saturating_add(lifetime_ms);
    store
        .create_profile_capture_session(
            &session_id,
            &token_digest,
            printer_id,
            request.profile_id.as_deref(),
            request.expected_revision,
            &enum_string(request.operation),
            LOCAL_PROFILE_HOST_ID,
            expires_unix_ms,
            created_unix_ms,
        )
        .map_err(storage_control_failure)?;

    Ok(ProfileCaptureAuthorized {
        session_id,
        capture_token,
        expires_unix_ms,
        operation: request.operation,
        printer_id: printer.printer_id,
        native_id: printer.native_id,
        printer_name: printer.name,
        profile_id: existing.as_ref().map(|profile| profile.profile_id.clone()),
        profile_name: existing.as_ref().map(|profile| profile.name.clone()),
        stock_id: existing
            .as_ref()
            .and_then(|profile| profile.stock_id.clone()),
        safe_overrides,
        expected_revision: request.expected_revision,
        native_configuration,
    })
}

fn commit_profile_capture(
    store: &mut AgentStore,
    session_id: &str,
    capture_token: &str,
    capture: NativeProfileCapturePayload,
) -> Result<LocalPrinterProfile, ControlFailure> {
    let native_blob = STANDARD.decode(&capture.native_blob_base64).map_err(|_| {
        control_failure("profile_invalid", "native profile blob is not valid base64")
    })?;
    if native_blob.len() > piqae_local_ipc::MAX_NATIVE_CAPTURE_BYTES {
        return Err(control_failure(
            "profile_invalid",
            "native profile blob exceeds the one MiB limit",
        ));
    }
    let durable_capture = NativeProfileCapture {
        name: capture.name,
        is_default: capture.is_default,
        options_json: serde_json::to_string(&capture.options)
            .map_err(|error| control_failure("profile_invalid", &error.to_string()))?,
        status: enum_string(ProfileStatus::NeedsTest),
        native_kind: enum_string(capture.native_kind),
        native_schema_version: capture.native_schema_version,
        native_digest: capture.native_digest,
        native_blob,
        driver_fingerprint_json: serde_json::to_string(&capture.driver_fingerprint)
            .map_err(|error| control_failure("profile_invalid", &error.to_string()))?,
        summary_json: serde_json::to_string(&capture.summary)
            .map_err(|error| control_failure("profile_invalid", &error.to_string()))?,
        stock_id: capture.stock_id,
        dependencies_json: serde_json::to_string(&capture.dependencies)
            .map_err(|error| control_failure("profile_invalid", &error.to_string()))?,
        safe_overrides_json: serde_json::to_string(&capture.safe_overrides)
            .map_err(|error| control_failure("profile_invalid", &error.to_string()))?,
        // Captures must pass an actual driver test before publication.
        published: false,
    };
    let stored = store
        .commit_profile_capture(
            session_id,
            &capture_token_digest(capture_token),
            LOCAL_PROFILE_HOST_ID,
            &durable_capture,
            Utc::now().timestamp_millis(),
        )
        .map_err(storage_control_failure)?;
    local_profile(store, stored)
}

fn validate_profile_revision(
    store: &mut AgentStore,
    profile_id: &str,
    revision: u64,
) -> Result<ProfileValidationResult, ControlFailure> {
    let validated_unix_ms = Utc::now().timestamp_millis();
    let profile = store
        .profile_revision(profile_id, revision)
        .map_err(storage_control_failure)?
        .ok_or_else(|| control_failure("profile_not_found", "profile revision was not found"))?;
    let kind: NativeProfileKind = parse_stored_enum(&profile.native_kind, "native profile kind")?;
    if kind != NativeProfileKind::PortableOptions {
        let blob = store
            .native_profile_blob(profile_id, revision)
            .map_err(storage_control_failure)?
            .ok_or_else(|| {
                control_failure("profile_invalid", "native profile payload was not found")
            })?;
        let blob_kind: NativeProfileKind =
            parse_stored_enum(&blob.native_kind, "native profile kind")?;
        if blob_kind != kind || profile.native_digest.as_deref() != Some(blob.digest.as_str()) {
            return Err(control_failure(
                "profile_invalid",
                "native profile metadata does not match its immutable payload",
            ));
        }
    }
    let status = parse_stored_enum(&profile.status, "profile status")?;
    store
        .record_profile_validation(profile_id, revision, validated_unix_ms)
        .map_err(storage_control_failure)?;
    Ok(ProfileValidationResult {
        profile_id: profile_id.to_owned(),
        revision,
        status,
        code: (status == ProfileStatus::NeedsTest).then(|| "driver_test_required".into()),
        message: (status == ProfileStatus::NeedsTest).then(|| {
            "The immutable native settings are intact; run a driver test before publishing.".into()
        }),
        validated_unix_ms,
    })
}

fn confirm_loaded_media(
    store: &mut AgentStore,
    request: piqae_local_ipc::ConfirmLoadedMedia,
) -> Result<(), ControlFailure> {
    if request.device_id.trim().is_empty() || request.source.trim().is_empty() {
        return Err(control_failure(
            "loaded_media_invalid",
            "device and source are required",
        ));
    }
    store
        .confirm_loaded_media(&StoredLoadedMedia {
            device_id: request.device_id,
            source: request.source,
            stock_id: request.stock_id,
            confidence: enum_string(request.confidence),
            confirmed_unix_ms: Utc::now().timestamp_millis(),
            confirmed_by: request.confirmed_by,
        })
        .map_err(storage_control_failure)
}

fn resolve_exposed_printer(
    store: &AgentStore,
    printer_id: &str,
) -> Result<StoredPrinter, ControlFailure> {
    let printer = resolve_present_printer(store, printer_id)?;
    if !printer.exposed {
        return Err(control_failure(
            "printer_not_exposed",
            "printer exposure must be explicitly enabled before submission",
        ));
    }
    Ok(printer)
}

fn resolve_present_printer(
    store: &AgentStore,
    printer_id: &str,
) -> Result<StoredPrinter, ControlFailure> {
    let printer = store
        .printer(printer_id)
        .map_err(storage_control_failure)?
        .ok_or_else(|| control_failure("printer_not_found", "printer was not found"))?;
    if !printer.present {
        return Err(control_failure(
            "printer_not_present",
            "printer is not present in the latest successful native discovery",
        ));
    }
    Ok(printer)
}

fn require_local_driver_test_confirmation(confirmed: bool) -> Result<(), ControlFailure> {
    if !confirmed {
        return Err(control_failure(
            "local_test_not_confirmed",
            "confirm the local driver test before submitting it",
        ));
    }
    Ok(())
}

fn validate_options(
    printer: &StoredPrinter,
    options: &piqae_domain::JobOptions,
) -> Result<(), ControlFailure> {
    let capabilities: piqae_domain::PrinterCapabilities =
        serde_json::from_str(&printer.capabilities_json)
            .map_err(|error| control_failure("capabilities_invalid", &error.to_string()))?;
    let native_options: std::collections::BTreeMap<String, piqae_domain::NativePrinterOption> =
        serde_json::from_str(&printer.native_options_json)
            .map_err(|error| control_failure("capabilities_invalid", &error.to_string()))?;
    let supports = |values: &[String], value: &Option<String>| {
        value
            .as_ref()
            .is_none_or(|selected| values.is_empty() || values.contains(selected))
    };
    if options.copies == Some(0)
        || capabilities.copies > 0
            && options
                .copies
                .is_some_and(|copies| copies > capabilities.copies)
        || !supports(&capabilities.bins, &options.bin)
        || !supports(&capabilities.dpis, &options.dpi)
        || !supports(&capabilities.medias, &options.media)
        || options.paper.as_ref().is_some_and(|paper| {
            !capabilities.papers.is_empty() && !capabilities.papers.contains_key(paper)
        })
        || options
            .nup
            .is_some_and(|nup| !capabilities.nup.is_empty() && !capabilities.nup.contains(&nup))
        || options.duplex.is_some() && !capabilities.duplex
        || options.color == Some(true) && !capabilities.color
    {
        return Err(control_failure(
            "unsupported_profile_option",
            "one or more portable options are not supported by the current capability revision",
        ));
    }
    for (key, selected) in &options.native_options {
        let definition = native_options.get(key).ok_or_else(|| {
            control_failure(
                "unknown_native_option",
                &format!("native option {key} is not advertised by the driver"),
            )
        })?;
        if !definition.choices.is_empty()
            && !definition
                .choices
                .iter()
                .any(|choice| choice.value == *selected)
        {
            return Err(control_failure(
                "unsupported_native_value",
                &format!("native option {key} does not allow value {selected}"),
            ));
        }
    }
    Ok(())
}

async fn printer_queue(
    engine: &mut AgentEngine<SharedRuntimeExecutor>,
    printer_id: &str,
) -> Result<LocalPrinterQueue, ControlFailure> {
    let printer = engine
        .store()
        .printer(printer_id)
        .map_err(storage_control_failure)?
        .ok_or_else(|| control_failure("printer_not_found", "printer was not found"))?;
    let local_jobs = engine
        .store()
        .jobs_for_printer(printer_id, 200)
        .map_err(storage_control_failure)?
        .into_iter()
        .map(|job| LocalQueueJob {
            job_id: job.job_id,
            sequence: job.printer_sequence,
            title: job.title,
            state: job.state,
            native_job_id: job.native_job_id,
        })
        .collect();
    let native_jobs = engine
        .executor_mut()
        .native_queue(&printer.native_id)
        .await?
        .into_iter()
        .map(|job| LocalNativeQueueJob {
            native_job_id: job.native_job_id,
            title: job.title,
            user: job.user,
            state: enum_string(job.state),
            native_code: job.native_code,
            size_kib: job.size_kib,
            created_unix_ms: job.created_unix_ms,
            processing_unix_ms: job.processing_unix_ms,
            completed_unix_ms: job.completed_unix_ms,
        })
        .collect();
    Ok(LocalPrinterQueue {
        printer_id: printer_id.to_owned(),
        local_jobs,
        native_jobs,
    })
}

fn local_job_history(
    store: &AgentStore,
    offset: usize,
    limit: usize,
) -> Result<LocalJobHistory, ControlFailure> {
    let jobs = store
        .local_job_history(offset, limit)
        .map_err(storage_control_failure)?;
    let returned = jobs.len();
    let jobs = jobs
        .into_iter()
        .map(|job| {
            let created_unix_ms = job
                .job_id
                .strip_prefix("job_")
                .and_then(|value| value.parse::<ulid::Ulid>().ok())
                .and_then(|value| i64::try_from(value.timestamp_ms()).ok());
            let can_reprint = is_terminal_job_state(&job.state)
                && resolve_present_printer(store, &job.printer_id).is_ok()
                && std::fs::metadata(&job.content_path).is_ok_and(|metadata| metadata.is_file());
            LocalHistoryJob {
                job_id: job.job_id,
                printer_id: job.printer_id,
                title: job.title,
                state: job.state,
                native_job_id: job.native_job_id,
                can_reprint,
                created_unix_ms,
            }
        })
        .collect();
    Ok(LocalJobHistory {
        jobs,
        next_offset: (returned == limit).then_some(offset.saturating_add(returned)),
    })
}

fn is_terminal_job_state(state: &str) -> bool {
    matches!(
        state,
        "completed_reported"
            | "completed"
            | "failed_terminal"
            | "cancelled"
            | "cancelled_by_server"
            | "expired"
            | "expired_before_handoff"
            | "delivery_uncertain"
            | "ambiguous_handoff"
    )
}

fn reprint_local_job(
    engine: &mut AgentEngine<SharedRuntimeExecutor>,
    original_job_id: &str,
    idempotency_key: &str,
    confirmed: bool,
    paused: bool,
) -> Result<LocalJobAccepted, ControlFailure> {
    if paused {
        return Err(control_failure("agent_paused", "the agent is paused"));
    }
    if !confirmed {
        return Err(control_failure(
            "confirmation_required",
            "reprint requires explicit confirmation",
        ));
    }
    let original = engine
        .store()
        .get_job(original_job_id)
        .map_err(storage_control_failure)?
        .ok_or_else(|| control_failure("job_not_found", "the original job was not found"))?;
    if !is_terminal_job_state(&original.state) {
        return Err(control_failure(
            "job_not_terminal",
            "only a finished print attempt can be reprinted",
        ));
    }
    let _printer = resolve_present_printer(engine.store(), &original.printer_id)?;
    if !std::fs::metadata(&original.content_path).is_ok_and(|metadata| metadata.is_file()) {
        return Err(control_failure(
            "content_unavailable",
            "retained print content is no longer available",
        ));
    }
    let (job_id, digest) = reprint_job_identity(original_job_id, idempotency_key);
    let accepted = piqae_agent_storage::AcceptedJob {
        job_id,
        submission_id: format!("reprint:{original_job_id}:{digest}"),
        printer_id: original.printer_id,
        printer_native_id: original.printer_native_id,
        title: format!(
            "Reprint — {}",
            original.title.chars().take(240).collect::<String>()
        ),
        content_sha256: original.content_sha256,
        content_path: original.content_path,
        content_kind: original.content_kind,
        options_json: original.options_json,
        expires_unix_ms: None,
        accepted_unix_ms: Utc::now().timestamp_millis(),
        cloud_managed: false,
    };
    let job = engine
        .accept(&accepted)
        .map_err(|error| control_failure("reprint_failed", &error.to_string()))?;
    Ok(LocalJobAccepted {
        job_id: job.job_id,
        state: job.state,
    })
}

fn reprint_job_identity(original_job_id: &str, idempotency_key: &str) -> (String, String) {
    let digest = hex::encode(Sha256::digest(
        format!("local-reprint\0{original_job_id}\0{idempotency_key}").as_bytes(),
    ));
    (format!("job_reprint_{}", &digest[..32]), digest)
}

async fn submit_test_page(
    engine: &mut AgentEngine<SharedRuntimeExecutor>,
    content_store: &ContentStore,
    printer_id: &str,
    profile_id: &str,
    confirmed: bool,
    paused: bool,
) -> Result<LocalJobAccepted, ControlFailure> {
    if paused {
        return Err(control_failure("agent_paused", "the agent is paused"));
    }
    require_local_driver_test_confirmation(confirmed)?;
    let printer = resolve_present_printer(engine.store(), printer_id)?;
    let profile = engine
        .store()
        .named_profile(printer_id, profile_id)
        .map_err(storage_control_failure)?
        .ok_or_else(|| control_failure("profile_not_found", "print profile was not found"))?;
    let mut options: piqae_domain::JobOptions = serde_json::from_str(&profile.options_json)
        .map_err(|error| control_failure("profile_invalid", &error.to_string()))?;
    let capabilities: piqae_domain::PrinterCapabilities =
        serde_json::from_str(&printer.capabilities_json)
            .map_err(|error| control_failure("capabilities_invalid", &error.to_string()))?;
    let native_definitions: std::collections::BTreeMap<String, piqae_domain::NativePrinterOption> =
        serde_json::from_str(&printer.native_options_json)
            .map_err(|error| control_failure("capabilities_invalid", &error.to_string()))?;
    let a4 = capabilities
        .papers
        .keys()
        .find(|paper| {
            let normalized = paper.to_ascii_lowercase();
            normalized.contains("a4") || normalized.contains("210x297")
        })
        .cloned()
        .ok_or_else(|| {
            control_failure(
                "a4_not_supported",
                "the current printer capability revision does not advertise A4",
            )
        })?;
    if options.paper.as_ref().is_some_and(|paper| !is_a4(paper)) {
        return Err(control_failure(
            "test_profile_not_a4",
            "the selected profile has a non-A4 portable paper selection",
        ));
    }
    for (key, selected) in &options.native_options {
        if is_native_page_size_key(key) && !is_a4(selected) {
            return Err(control_failure(
                "test_profile_not_a4",
                &format!("native page-size option {key} selects non-A4 stock"),
            ));
        }
    }
    options.paper = Some(a4);
    if !options
        .native_options
        .keys()
        .any(|key| is_native_page_size_key(key))
        && let Some((key, choice)) = native_definitions.iter().find_map(|(key, definition)| {
            is_native_page_size_key(key).then(|| {
                definition
                    .choices
                    .iter()
                    .find(|choice| is_a4(&choice.value))
                    .map(|choice| (key.clone(), choice.value.clone()))
            })?
        })
    {
        options.native_options.insert(key, choice);
    }
    validate_options(&printer, &options)?;
    let stored = content_store
        .put(std::io::Cursor::new(a4_test_pdf()))
        .await
        .map_err(|error| control_failure("content_store_failed", &error.to_string()))?;
    let accepted = accept_stored_local_job(
        engine,
        LocalCreateJob {
            printer_id: printer_id.to_owned(),
            printer_native_id: Some(printer.native_id),
            title: "Piqae A4 diagnostic".into(),
            content_kind: ContentKind::Pdf,
            content: LocalContent::Base64 {
                data: String::new(),
            },
            options,
            expires_unix_ms: Some(Utc::now().timestamp_millis() + 300_000),
        },
        stored,
        Some((profile.profile_id.clone(), profile.revision)),
    )
    .await?;
    engine
        .store_mut()
        .record_profile_test_result(
            &profile.profile_id,
            profile.revision,
            &accepted.job_id,
            profile_test_passed(&accepted.state),
            Utc::now().timestamp_millis(),
        )
        .map_err(storage_control_failure)?;
    Ok(accepted)
}

fn profile_test_passed(state: &str) -> bool {
    matches!(
        state,
        "accepted_by_spooler" | "spooling" | "printing" | "completed_reported"
    )
}

fn is_a4(value: &str) -> bool {
    let normalized = value.to_ascii_lowercase();
    normalized.contains("a4") || normalized.contains("210x297")
}

fn is_native_page_size_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "pagesize" | "pageregion" | "media" | "media-size" | "media-size-name"
    )
}

fn a4_test_pdf() -> Vec<u8> {
    let content = b"BT /F1 22 Tf 72 760 Td (Piqae A4 diagnostic) Tj /F1 11 Tf 0 -30 Td (Local queue and driver test) Tj 0 -22 Td (No external content was used.) Tj ET";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        [format!("<< /Length {} >>\nstream\n", content.len()).into_bytes(), content.to_vec(), b"\nendstream".to_vec()].concat(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
    ];
    let mut pdf = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        pdf.extend_from_slice(object);
        pdf.extend_from_slice(b"\nendobj\n");
    }
    let xref = pdf.len();
    pdf.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    pdf
}

fn enum_string<T: serde::Serialize>(value: T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| "unknown".into())
}

fn parse_stored_enum<T: serde::de::DeserializeOwned>(
    value: &str,
    label: &str,
) -> Result<T, ControlFailure> {
    serde_json::from_value(serde_json::Value::String(value.to_owned()))
        .map_err(|error| control_failure("profile_invalid", &format!("invalid {label}: {error}")))
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "matches map_err's owned error signature throughout the agent boundary"
)]
fn storage_control_failure(error: StorageError) -> ControlFailure {
    match error {
        StorageError::PrinterNotFound(_) => {
            control_failure("printer_not_found", &error.to_string())
        }
        StorageError::ProfileRevisionConflict { .. } => {
            control_failure("profile_revision_conflict", &error.to_string())
        }
        StorageError::InvalidPrinterProfile(_) | StorageError::NativeBlobTooLarge(_) => {
            control_failure("profile_invalid", &error.to_string())
        }
        StorageError::CaptureSessionNotFound(_) => {
            control_failure("profile_capture_not_found", &error.to_string())
        }
        StorageError::CaptureSessionNotAuthorized(_) => {
            control_failure("profile_capture_not_authorized", &error.to_string())
        }
        StorageError::InvalidCaptureToken => {
            control_failure("profile_capture_token_invalid", &error.to_string())
        }
        _ => control_failure("local_storage_failed", &error.to_string()),
    }
}

async fn accept_stored_local_job(
    engine: &mut AgentEngine<SharedRuntimeExecutor>,
    request: LocalCreateJob,
    stored: piqae_agent_core::StoredContent,
    profile_pin: Option<(String, u64)>,
) -> Result<LocalJobAccepted, ControlFailure> {
    let job_id = JobId::new().to_string();
    let options_json = serde_json::to_string(&request.options)
        .map_err(|_| control_failure("invalid_options", "print options are invalid"))?;
    engine
        .accept(&AcceptedJob {
            job_id: job_id.clone(),
            submission_id: format!("sub_{}", uuid::Uuid::new_v4()),
            printer_id: request.printer_id,
            printer_native_id: request.printer_native_id.ok_or_else(|| {
                control_failure(
                    "printer_not_resolved",
                    "the logical printer has no resolved native queue",
                )
            })?,
            title: request.title,
            content_sha256: stored.sha256,
            content_path: stored.path.to_string_lossy().into_owned(),
            content_kind: match request.content_kind {
                ContentKind::Pdf => "pdf",
                ContentKind::Raw => "raw",
            }
            .into(),
            options_json,
            expires_unix_ms: request.expires_unix_ms,
            accepted_unix_ms: Utc::now().timestamp_millis(),
            cloud_managed: false,
        })
        .map_err(|error| control_failure("local_accept_failed", &error.to_string()))?;
    if let Some((profile_id, revision)) = &profile_pin {
        engine
            .store_mut()
            .pin_job_profile(&job_id, None, None, profile_id, *revision, None, None)
            .map_err(storage_control_failure)?;
    }
    engine
        .run_once()
        .await
        .map_err(|error| control_failure("local_execution_failed", &error.to_string()))?;
    let state = engine
        .store()
        .get_job(&job_id)
        .map_err(|error| control_failure("local_query_failed", &error.to_string()))?
        .map_or_else(|| "unknown".to_owned(), |job| job.state);
    Ok(LocalJobAccepted { job_id, state })
}

fn control_failure(code: &str, message: &str) -> ControlFailure {
    ControlFailure {
        code: code.to_owned(),
        message: message.to_owned(),
    }
}

fn cloud_configuration(arguments: &Arguments) -> Result<CloudConfiguration> {
    let base_url = arguments
        .control_plane_url
        .clone()
        .context("--control-plane-url is required outside local mode")?;
    let raw_agent_id = arguments
        .agent_id
        .as_deref()
        .context("--agent-id is required outside local mode")?
        .strip_prefix("agt_")
        .unwrap_or_else(|| arguments.agent_id.as_deref().unwrap_or_default());
    let agent_id: AgentId =
        serde_json::from_value(serde_json::Value::String(raw_agent_id.to_owned()))
            .context("parse --agent-id")?;
    let key_path = arguments
        .device_key_file
        .as_ref()
        .context("--device-key-file is required outside local mode")?;
    let encoded = std::fs::read_to_string(key_path)
        .with_context(|| format!("read {}", key_path.display()))?;
    let bytes = hex::decode(encoded.trim()).context("decode Ed25519 device key")?;
    let secret: [u8; 32] = bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("device key must contain exactly 32 bytes"))?;
    let content_encryption_keys = content_key_store::load_or_create(
        &arguments.data_dir.join("content-encryption.key"),
        &agent_id.to_string(),
    )?;
    Ok(CloudConfiguration {
        client: AgentClient::new(base_url)?,
        identity: DeviceIdentity::from_secret_bytes(agent_id, &secret),
        agent_id,
        content_encryption_keys: Arc::new(content_encryption_keys),
        allowed_printer_ids: None,
        connector_id: "legacy".into(),
    })
}

fn cloud_configuration_from_connector(
    record: &connector_runtime::ConnectorRecord,
    key_path: &Path,
) -> Result<CloudConfiguration> {
    let raw_agent_id = record
        .agent_id
        .strip_prefix("agt_")
        .unwrap_or(&record.agent_id);
    let agent_id: AgentId =
        serde_json::from_value(serde_json::Value::String(raw_agent_id.to_owned()))
            .with_context(|| format!("parse agent id for connector {}", record.connector_id))?;
    let encoded = std::fs::read_to_string(key_path)
        .with_context(|| format!("read connector {} device key", record.connector_id))?;
    let bytes = hex::decode(encoded.trim())
        .with_context(|| format!("decode connector {} device key", record.connector_id))?;
    let secret: [u8; 32] = bytes.try_into().map_err(|_| {
        anyhow::anyhow!(
            "connector {} device key must contain exactly 32 bytes",
            record.connector_id
        )
    })?;
    let encryption_path = key_path.with_extension("content-encryption.key");
    let content_encryption_keys =
        content_key_store::load_or_create(&encryption_path, &record.agent_id)?;
    Ok(CloudConfiguration {
        client: AgentClient::new(record.control_plane_url.clone())?,
        identity: DeviceIdentity::from_secret_bytes(agent_id, &secret),
        agent_id,
        content_encryption_keys: Arc::new(content_encryption_keys),
        allowed_printer_ids: connector_allowed_printers(record),
        connector_id: record.connector_id.clone(),
    })
}

fn connector_allowed_printers(
    record: &connector_runtime::ConnectorRecord,
) -> Option<std::collections::BTreeSet<String>> {
    match record.printer_grant {
        PrinterGrant::SelectedPrinters => {
            Some(record.allowed_printer_ids.iter().cloned().collect())
        }
        PrinterGrant::AllLocalPrinters => None,
    }
}

fn inventory_projection_confirmed(
    acknowledgement_supported: bool,
    acknowledgement: Option<&piqae_protocol::agent::InventoryProjectionAcknowledgement>,
    submitted_revision: u64,
) -> bool {
    acknowledgement.map_or(!acknowledgement_supported, |acknowledgement| {
        acknowledgement.revision == submitted_revision
    })
}

#[allow(
    clippy::too_many_arguments,
    reason = "cloud synchronization dependencies are explicit process-boundary capabilities"
)]
async fn cloud_sync_loop(
    cloud: CloudConfiguration,
    database_path: PathBuf,
    inventory_database_path: PathBuf,
    content_store: ContentStore,
    uri_fetcher: UriFetcher,
    printer_discovery: PrinterDiscovery,
    route_coordinator: Arc<Mutex<route_coordinator::RouteCoordinator>>,
    observation_cache: Arc<Mutex<RouteObservationCache>>,
    support_packs: Arc<SupportPackRegistry>,
    connection: Arc<RwLock<ConnectionState>>,
    paused: Arc<AtomicBool>,
    cloud_sync_wakeup: Arc<Notify>,
    printer_inventory_dirty: Arc<AtomicBool>,
    last_sync_error_code: Arc<RwLock<Option<String>>>,
    stop: StopSignal,
) {
    let store = match AgentStore::open(&database_path) {
        Ok(store) => store,
        Err(error) => {
            error!(%error, "cloud sync cannot open the agent database");
            *connection.write().await = ConnectionState::Degraded;
            return;
        }
    };
    // Connector job queues are deliberately isolated, but printer identity,
    // exposure and profiles belong to the physical node. Reading inventory
    // from a fresh connector database would generate unrelated printer IDs
    // and default every queue to unexposed, causing an approved connector to
    // publish an empty printer list.
    let inventory_store = match AgentStore::open(&inventory_database_path) {
        Ok(store) => store,
        Err(error) => {
            error!(%error, "cloud sync cannot open the node printer inventory");
            *connection.write().await = ConnectionState::Degraded;
            return;
        }
    };
    sweep_confidential_files(&store);
    Box::pin(run_cloud_sync_loop(
        cloud,
        store,
        inventory_store,
        content_store,
        uri_fetcher,
        printer_discovery,
        route_coordinator,
        observation_cache,
        support_packs,
        connection,
        paused,
        cloud_sync_wakeup,
        printer_inventory_dirty,
        last_sync_error_code,
        stop,
    ))
    .await;
}

#[allow(
    clippy::too_many_arguments,
    clippy::cognitive_complexity,
    clippy::too_many_lines,
    reason = "cloud synchronization dependencies are explicit process-boundary capabilities"
)]
async fn run_cloud_sync_loop(
    cloud: CloudConfiguration,
    mut store: AgentStore,
    mut inventory_store: AgentStore,
    content_store: ContentStore,
    uri_fetcher: UriFetcher,
    printer_discovery: PrinterDiscovery,
    route_coordinator: Arc<Mutex<route_coordinator::RouteCoordinator>>,
    observation_cache: Arc<Mutex<RouteObservationCache>>,
    support_packs: Arc<SupportPackRegistry>,
    connection: Arc<RwLock<ConnectionState>>,
    paused: Arc<AtomicBool>,
    cloud_sync_wakeup: Arc<Notify>,
    printer_inventory_dirty: Arc<AtomicBool>,
    last_sync_error_code: Arc<RwLock<Option<String>>>,
    stop: StopSignal,
) {
    let Some((active_content_key_id, active_content_key)) = cloud.content_encryption_keys.active()
    else {
        error!("content encryption keyring has no active key");
        *connection.write().await = ConnectionState::Degraded;
        return;
    };
    let public_key_spki = match active_content_key.public_key().to_public_key_der() {
        Ok(der) => URL_SAFE_NO_PAD.encode(der.as_bytes()),
        Err(error) => {
            error!(%error, "content encryption key cannot be encoded");
            *connection.write().await = ConnectionState::Degraded;
            return;
        }
    };
    loop {
        match cloud
            .client
            .register_content_encryption_key(
                &cloud.identity,
                active_content_key_id,
                &public_key_spki,
            )
            .await
        {
            Ok(_) => break,
            Err(error) => {
                *last_sync_error_code.write().await = redacted_sync_error_code(&error);
                warn!(%error, "content encryption key registration deferred");
                *connection.write().await = failure_state(&error);
                tokio::select! {
                    () = tokio::time::sleep(Duration::from_secs(5)) => {}
                    () = stop.cancelled() => return,
                }
            }
        }
    }
    let started_at = Utc::now();
    let mut failures = 0_u32;
    let mut last_printer_refresh: Option<tokio::time::Instant> = None;
    loop {
        if stop.is_stopped() {
            break;
        }
        sweep_confidential_files(&store);
        resume_pending_cloud_accepts(&cloud, &mut store, &route_coordinator).await;
        let refresh_printers = printer_inventory_dirty.swap(false, Ordering::AcqRel)
            || last_printer_refresh.is_none_or(|last| last.elapsed() >= Duration::from_secs(60));
        let request = match prepare_sync_request(
            &mut store,
            &mut inventory_store,
            &printer_discovery,
            &route_coordinator,
            &observation_cache,
            &support_packs,
            &cloud.connector_id,
            cloud.agent_id,
            started_at,
            paused.load(Ordering::Relaxed),
            refresh_printers,
            cloud.allowed_printer_ids.as_ref(),
        )
        .await
        {
            Ok(request) => request,
            Err(error) => {
                error!(%error, "cloud sync cannot read queue health");
                *connection.write().await = ConnectionState::Degraded;
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };
        let submitted_printer_inventory = request.printers.is_some();
        let submitted_printer_revision = request.printer_revision;
        if !submitted_printer_inventory && refresh_printers {
            printer_inventory_dirty.store(true, Ordering::Release);
        }
        let delay = match cloud.client.sync(&cloud.identity, &request).await {
            Ok(response) => {
                let projection_acknowledged = inventory_projection_confirmed(
                    response.inventory_projection_acknowledgement_supported,
                    response.inventory_projection.as_ref(),
                    submitted_printer_revision,
                );
                if submitted_printer_inventory && projection_acknowledged {
                    last_printer_refresh = Some(tokio::time::Instant::now());
                } else if submitted_printer_inventory {
                    // A normal heartbeat response does not prove that printer
                    // rows were committed. Keep retrying the exact connector
                    // projection until the server acknowledges its revision.
                    printer_inventory_dirty.store(true, Ordering::Release);
                }
                *last_sync_error_code.write().await = None;
                sync_succeeded(
                    response,
                    SyncContext {
                        cloud: &cloud,
                        store: &mut store,
                        inventory_store: &mut inventory_store,
                        content_store: &content_store,
                        uri_fetcher: &uri_fetcher,
                        paused: &paused,
                        failures: &mut failures,
                        connection: &connection,
                        stop: &stop,
                        route_coordinator: &route_coordinator,
                    },
                )
                .await
            }
            Err(error) => {
                if submitted_printer_inventory {
                    // A heartbeat succeeding after an inventory projection
                    // failure must not mask missing printers for fifteen
                    // minutes. Retry the same current inventory next cycle.
                    printer_inventory_dirty.store(true, Ordering::Release);
                }
                *last_sync_error_code.write().await = redacted_sync_error_code(&error);
                sync_failed(&error, &mut failures, &connection).await
            }
        };
        tokio::select! {
            () = tokio::time::sleep(delay) => {}
            () = cloud_sync_wakeup.notified() => {}
            () = stop.cancelled() => break,
        }
    }
}

#[allow(
    clippy::cognitive_complexity,
    reason = "bounded cleanup reports independent filesystem failures"
)]
fn sweep_confidential_files(store: &AgentStore) {
    let files = match store.confidential_files_due() {
        Ok(files) => files,
        Err(error) => {
            warn!(%error, "confidential file sweep query failed");
            return;
        }
    };
    for file in files {
        match std::fs::remove_file(&file.path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                warn!(%error, "confidential file cleanup failed");
                continue;
            }
        }
        if let Err(error) = store.mark_confidential_file_deleted(&file.job_id) {
            warn!(%error, "confidential file cleanup could not be recorded");
        }
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "queue and node-inventory stores are separate connector security boundaries"
)]
async fn prepare_sync_request(
    store: &mut AgentStore,
    inventory_store: &mut AgentStore,
    printer_discovery: &PrinterDiscovery,
    route_coordinator: &Arc<Mutex<route_coordinator::RouteCoordinator>>,
    observation_cache: &Arc<Mutex<RouteObservationCache>>,
    support_packs: &SupportPackRegistry,
    connector_id: &str,
    agent_id: AgentId,
    started_at: chrono::DateTime<Utc>,
    paused: bool,
    refresh_printers: bool,
    allowed_printer_ids: Option<&std::collections::BTreeSet<String>>,
) -> Result<AgentSyncRequest> {
    let mut inventory_revision = store
        .setting("printer_inventory_revision")?
        .and_then(|revision| revision.parse::<u64>().ok())
        .unwrap_or(0);
    let printers = if refresh_printers {
        let next = inventory_revision.saturating_add(1);
        match discover_cloud_printers(
            inventory_store,
            printer_discovery,
            route_coordinator,
            support_packs,
            next,
        )
        .await
        {
            Ok(mut printers) => {
                printers.retain(|printer| {
                    printer_is_allowed(allowed_printer_ids, &printer.id.to_string())
                });
                store.set_setting("printer_inventory_revision", &next.to_string())?;
                inventory_revision = next;
                Some(printers)
            }
            Err(error) => {
                warn!(%error, "native printer inventory refresh failed");
                None
            }
        }
    } else {
        None
    };
    let observation_inputs = route_observation_inputs(store, inventory_store);
    let route_observations = collect_route_observations(
        observation_inputs,
        printer_discovery,
        route_coordinator,
        observation_cache,
        inventory_revision,
    )
    .await;
    let (topology_changes, native_handoffs) = {
        let coordinator = route_coordinator.lock().await;
        let acknowledged_handoff = store
            .setting("acknowledged_handoff_sequence")?
            .and_then(|sequence| sequence.parse::<u64>().ok())
            .unwrap_or(0);
        (
            coordinator.topology_changes(),
            coordinator.handoffs_for_connector(connector_id, acknowledged_handoff),
        )
    };
    let mut request = sync_request(store, agent_id, started_at, paused, printers)?;
    request.route_observations = route_observations;
    request.topology_changes = topology_changes;
    request.native_handoffs = native_handoffs;
    Ok(request)
}

struct SyncContext<'a> {
    cloud: &'a CloudConfiguration,
    store: &'a mut AgentStore,
    inventory_store: &'a mut AgentStore,
    content_store: &'a ContentStore,
    uri_fetcher: &'a UriFetcher,
    paused: &'a AtomicBool,
    failures: &'a mut u32,
    connection: &'a RwLock<ConnectionState>,
    stop: &'a StopSignal,
    route_coordinator: &'a Arc<Mutex<route_coordinator::RouteCoordinator>>,
}

async fn sync_succeeded(response: AgentSyncResponse, context: SyncContext<'_>) -> Duration {
    let AgentSyncResponse {
        acknowledged_event_cursor,
        acknowledged_diagnostics,
        acknowledged_handoff_sequence,
        command_cursor,
        commands,
        candidate_jobs,
        next_poll_after_ms,
        ..
    } = response;
    *context.failures = 0;
    *context.connection.write().await = ConnectionState::Connected;
    apply_event_acknowledgement(context.store, acknowledged_event_cursor);
    acknowledge_diagnostics(context.store, &acknowledged_diagnostics);
    if let Some(sequence) = acknowledged_handoff_sequence
        && let Err(error) = context
            .store
            .set_setting("acknowledged_handoff_sequence", &sequence.to_string())
    {
        warn!(%error, "native handoff acknowledgement could not be persisted");
    }
    apply_commands(
        context.store,
        context.paused,
        context.route_coordinator,
        &context.cloud.connector_id,
        commands,
        command_cursor,
    )
    .await;
    for offer in candidate_jobs {
        if let Err(error) = accept_offer(
            context.cloud,
            context.store,
            context.inventory_store,
            context.content_store,
            context.uri_fetcher,
            context.route_coordinator,
            offer,
            context.stop,
        )
        .await
        {
            warn!(%error, "job offer could not be durably accepted");
        }
    }
    Duration::from_millis(next_poll_after_ms.clamp(250, 60_000))
}

fn apply_event_acknowledgement(store: &mut AgentStore, cursor: Option<EventId>) {
    let Some(cursor) = cursor else {
        return;
    };
    if let Err(error) =
        store.acknowledge_cloud_event(&cursor.to_string(), Utc::now().timestamp_millis())
    {
        warn!(%error, "server event acknowledgement could not be applied");
    }
}

async fn apply_commands(
    store: &mut AgentStore,
    paused: &AtomicBool,
    route_coordinator: &Arc<Mutex<route_coordinator::RouteCoordinator>>,
    connector_id: &str,
    commands: Vec<AgentCommand>,
    command_cursor: Option<String>,
) {
    for command in commands {
        let result = match command {
            AgentCommand::ResolveAmbiguousHandoff {
                job_id,
                local_route_key,
                reservation_id,
                generation,
                resolution,
            } => route_coordinator.lock().await.resolve_ambiguous_handoff(
                connector_id,
                &job_id.to_string(),
                &local_route_key,
                reservation_id,
                generation,
                resolution,
            ),
            command => apply_command(store, paused, command).map_err(anyhow::Error::from),
        };
        if let Err(error) = result {
            warn!(%error, "cloud command could not be applied durably");
            return;
        }
    }
    let Some(cursor) = command_cursor else {
        return;
    };
    if let Err(error) = store.set_setting("command_cursor", &cursor) {
        warn!(%error, "command cursor could not be persisted");
    }
}

fn apply_command(
    store: &mut AgentStore,
    paused: &AtomicBool,
    command: AgentCommand,
) -> Result<(), StorageError> {
    match command {
        AgentCommand::Pause => {
            store.set_setting("paused", "true")?;
            paused.store(true, Ordering::Relaxed);
        }
        AgentCommand::Resume => {
            store.set_setting("paused", "false")?;
            paused.store(false, Ordering::Relaxed);
        }
        AgentCommand::CancelJob { job_id } => {
            store.request_cancel(&job_id.to_string(), Utc::now().timestamp_millis())?;
        }
        AgentCommand::RefreshPrinters => {
            warn!("printer refresh requested but inventory watcher is not enabled");
        }
        AgentCommand::UpdateAvailable { version, .. } => {
            info!(%version, "signed update is available");
        }
        AgentCommand::CollectDiagnostics { request_id } => {
            collect_diagnostics(store, &request_id)?;
            info!(%request_id, "bounded redacted diagnostics collected");
        }
        AgentCommand::ResolveAmbiguousHandoff { .. } => {
            return Err(StorageError::InvalidLocalEvent(
                "route resolution command reached the queue-only handler".into(),
            ));
        }
    }
    Ok(())
}

const MAX_PENDING_DIAGNOSTICS: usize = 8;

fn pending_diagnostics(
    store: &AgentStore,
) -> Result<Vec<piqae_protocol::agent::DiagnosticReport>, StorageError> {
    let Some(encoded) = store.setting("pending_diagnostics")? else {
        return Ok(Vec::new());
    };
    serde_json::from_str(&encoded).map_err(StorageError::from)
}

fn collect_diagnostics(store: &mut AgentStore, request_id: &str) -> Result<(), StorageError> {
    let counts = store.queue_counts();
    let integrity = store.integrity_check();
    let health = store.failure_health();
    let collection_failed = counts.is_err() || integrity.is_err() || health.is_err();
    let (queued_jobs, active_jobs) = counts.map_or((0, 0), |value| (value.queued, value.active));
    let sqlite_integrity_ok = integrity.unwrap_or(false);
    let (executor_crashes, last_error_code) = health.unwrap_or((0, None));
    let report = piqae_protocol::agent::DiagnosticReport {
        request_id: request_id.to_owned(),
        observed_at: Utc::now(),
        state: if collection_failed {
            "failed"
        } else {
            "complete"
        }
        .into(),
        agent_version: env!("CARGO_PKG_VERSION").into(),
        platform: std::env::consts::OS.into(),
        architecture: std::env::consts::ARCH.into(),
        queued_jobs,
        active_jobs,
        sqlite_integrity_ok,
        executor_crashes,
        last_error_code,
        collection_error_code: collection_failed.then(|| "diagnostic_collection_failed".into()),
    };
    let mut reports = pending_diagnostics(store)?;
    reports.retain(|existing| existing.request_id != request_id);
    reports.push(report);
    if reports.len() > MAX_PENDING_DIAGNOSTICS {
        reports.drain(..reports.len() - MAX_PENDING_DIAGNOSTICS);
    }
    store.set_setting("pending_diagnostics", &serde_json::to_string(&reports)?)
}

fn acknowledge_diagnostics(store: &mut AgentStore, acknowledged: &[String]) {
    if acknowledged.is_empty() {
        return;
    }
    let Ok(mut reports) = pending_diagnostics(store) else {
        warn!("diagnostic acknowledgement could not read durable reports");
        return;
    };
    reports.retain(|report| !acknowledged.contains(&report.request_id));
    match serde_json::to_string(&reports) {
        Ok(encoded) => {
            if let Err(error) = store.set_setting("pending_diagnostics", &encoded) {
                warn!(%error, "diagnostic acknowledgement could not be persisted");
            }
        }
        Err(error) => warn!(%error, "diagnostic acknowledgement could not be encoded"),
    }
}

#[allow(
    clippy::needless_pass_by_ref_mut,
    reason = "exclusive inventory access keeps the spawned sync future Send across awaits"
)]
#[allow(
    clippy::too_many_arguments,
    reason = "offer acceptance crosses explicit queue, content, route, and shutdown boundaries"
)]
async fn accept_offer(
    cloud: &CloudConfiguration,
    store: &mut AgentStore,
    inventory_store: &mut AgentStore,
    content_store: &ContentStore,
    uri_fetcher: &UriFetcher,
    route_coordinator: &Arc<Mutex<route_coordinator::RouteCoordinator>>,
    offer: JobOffer,
    stop: &StopSignal,
) -> Result<()> {
    if !printer_is_allowed(
        cloud.allowed_printer_ids.as_ref(),
        &offer.job.printer_id.to_string(),
    ) {
        cloud
            .client
            .release_lease(
                &cloud.identity,
                offer.job.id,
                &AgentReleaseLeaseRequest {
                    lease_id: offer.lease_id,
                    lease_token: offer.lease_token.clone(),
                    reason: "printer_not_granted".into(),
                },
            )
            .await
            .context("release lease for printer outside connector grant")?;
        anyhow::bail!("cloud offered a job outside this connector's printer grant");
    }
    let lease_id = offer.lease_id;
    let lease_token = offer.lease_token.clone();
    let job_id = offer.job.id;
    let route_reservation = offer.route_reservation.clone();
    let result = tokio::select! {
      result = maintain_lease(
        offer.lease_expires_at,
        LEASE_RENEWAL_INTERVAL,
        accept_offer_under_lease(
            cloud,
            store,
            inventory_store,
            content_store,
            uri_fetcher,
            route_coordinator,
            offer,
        ),
        || async {
            tokio::time::timeout(
                LEASE_RENEWAL_REQUEST_TIMEOUT,
                cloud.client.renew_lease(
                    &cloud.identity,
                    job_id,
                    &AgentRenewLeaseRequest {
                        lease_id,
                        lease_token: lease_token.clone(),
                        route_reservation_id: route_reservation
                            .as_ref()
                            .map(|reservation| reservation.reservation_id),
                        route_generation: route_reservation
                            .as_ref()
                            .map(|reservation| reservation.generation),
                        route_fencing_token: route_reservation
                            .as_ref()
                            .map(|reservation| reservation.fencing_token.clone()),
                    },
                ),
            )
            .await
            .map_err(|_| anyhow::anyhow!("job lease renewal failed"))?
            .map(|response| response.lease_expires_at)
            .map_err(|_| anyhow::anyhow!("job lease renewal failed"))
        },
      ) => result,
      () = stop.cancelled() => Err(anyhow::anyhow!("connector revoked while job was leased")),
    };
    if let Err(error) = &result {
        let has_durable_intent = match store.pending_cloud_accepts() {
            Ok(intents) => intents
                .iter()
                .any(|intent| intent.job_id == job_id.to_string()),
            Err(read_error) => {
                warn!(
                    %read_error,
                    %job_id,
                    "cloud acceptance intent could not be read; retaining the lease to prevent duplicate handoff"
                );
                true
            }
        };
        if !has_durable_intent {
            let _ = cloud
                .client
                .release_lease(
                    &cloud.identity,
                    job_id,
                    &AgentReleaseLeaseRequest {
                        lease_id,
                        lease_token,
                        reason: error
                            .downcast_ref::<OfferRejection>()
                            .map_or("acceptance_failed", |rejection| rejection.reason)
                            .into(),
                    },
                )
                .await;
        }
    }
    result
}

fn printer_is_allowed(
    allowed: Option<&std::collections::BTreeSet<String>>,
    printer_id: &str,
) -> bool {
    allowed.is_none_or(|ids| ids.contains(printer_id))
}

#[allow(
    clippy::too_many_lines,
    clippy::needless_pass_by_ref_mut,
    reason = "keeps the lease-fenced acceptance and durable-intent boundary auditable in one flow"
)]
async fn accept_offer_under_lease(
    cloud: &CloudConfiguration,
    store: &mut AgentStore,
    inventory_store: &mut AgentStore,
    content_store: &ContentStore,
    uri_fetcher: &UriFetcher,
    route_coordinator: &Arc<Mutex<route_coordinator::RouteCoordinator>>,
    offer: JobOffer,
) -> Result<()> {
    let job_id = offer.job.id;
    let route_reservation = offer.route_reservation.clone();
    let logical_printer_id = offer.job.printer_id.to_string();
    let profile_pin = profile_pin_metadata(&offer.job.metadata)?;
    if matches!(&offer.content, ContentDescriptor::EncryptedDownload { .. })
        && profile_pin.is_none()
    {
        return Err(OfferRejection::new(
            "encrypted_profile_pin_required",
            "encrypted content requires an exact local target/profile pin".into(),
        )
        .into());
    }
    // Connector stores intentionally contain only that connector's queue and
    // cloud cursors. Printer identity, exposure, capabilities, and profiles
    // are authoritative in the node-owned inventory store.
    let printer = resolve_cloud_offer_printer(inventory_store, &logical_printer_id)?;
    if !printer.present {
        return Err(OfferRejection::new(
            "printer_not_present",
            format!("printer is not present: {logical_printer_id}"),
        )
        .into());
    }
    // Cloud access is authorized by the connector grant. `printer_exposure`
    // predates multi-connector consent and remains a local compatibility
    // preference; allowing it to veto a connector would make an approved
    // all-printer grant publish printers that it then cannot use.
    let mut uses_current_printer_defaults = false;
    if let Some(pin) = &profile_pin {
        let profile = inventory_store
            .named_profile_revision(&printer.printer_id, &pin.profile_id, pin.profile_revision)?
            .with_context(|| {
                format!(
                    "profile_not_found: {} revision {}",
                    pin.profile_id, pin.profile_revision
                )
            })?;
        let status: ProfileStatus =
            serde_json::from_value(serde_json::Value::String(profile.status.clone()))
                .context("profile status is invalid")?;
        if !status.permits_jobs() {
            return Err(OfferRejection::new(
                "profile_not_ready",
                format!(
                    "profile {} revision {} is {}",
                    pin.profile_id, pin.profile_revision, profile.status
                ),
            )
            .into());
        }
        uses_current_printer_defaults = profile.uses_current_printer_defaults;
        if let ContentDescriptor::EncryptedDownload { manifest, .. } = &offer.content {
            let expected = format!("{}:{}", pin.profile_id, pin.profile_revision);
            if manifest.binding.profile_revision != expected
                || pin.target_id.as_deref() != Some(manifest.binding.target_id.as_str())
                || manifest.binding.content_type != offer.job.content_kind
                || manifest.binding.printer_id != offer.job.printer_id.to_string()
                || manifest.binding.options != offer.job.options
                || manifest.binding.deliveries != offer.job.deliveries
                || manifest.binding.raw_authorized != (offer.job.content_kind == ContentKind::Raw)
                || manifest.version != piqae_domain::ENCRYPTED_JOB_V3_VERSION
                || manifest.suite != piqae_domain::ENCRYPTED_JOB_V3_SUITE
            {
                return Err(OfferRejection::new(
                    "encrypted_binding_mismatch",
                    "encrypted content is not bound to the locally pinned target/profile".into(),
                )
                .into());
            }
        }
    }
    validate_options(&printer, &offer.job.options).map_err(|failure| {
        if failure.code == "unsupported_profile_option" {
            anyhow::Error::new(OfferRejection::new(
                "unsupported_profile_option",
                failure.message,
            ))
        } else {
            anyhow::anyhow!("{}: {}", failure.code, failure.message)
        }
    })?;
    let stored = materialize_descriptor(
        cloud,
        content_store,
        uri_fetcher,
        job_id,
        offer.lease_id,
        &offer.lease_token,
        offer.content,
    )
    .await?;
    let accepted_unix_ms = Utc::now().timestamp_millis();
    let local = store.prepare_cloud_job(
        &AcceptedJob {
            job_id: job_id.to_string(),
            submission_id: format!("sub_{job_id}"),
            printer_id: logical_printer_id,
            printer_native_id: printer.native_id.clone(),
            title: offer.job.title,
            content_sha256: stored.sha256.clone(),
            content_path: stored.path.to_string_lossy().into_owned(),
            content_kind: match offer.job.content_kind {
                ContentKind::Pdf => "pdf",
                ContentKind::Raw => "raw",
            }
            .into(),
            options_json: serde_json::to_string(&offer.job.options)?,
            expires_unix_ms: Some(offer.job.expires_at.timestamp_millis()),
            accepted_unix_ms,
            cloud_managed: true,
        },
        &offer.lease_id.to_string(),
        &offer.lease_token,
        offer.lease_expires_at.timestamp_millis(),
    )?;
    if let Some(reservation) = &route_reservation {
        route_coordinator.lock().await.register_authoritative(
            &cloud.connector_id,
            &printer.native_id,
            &job_id.to_string(),
            reservation,
            Utc::now(),
        )?;
    }
    if let Some(pin) = profile_pin.filter(|_| !uses_current_printer_defaults) {
        store.pin_job_profile(
            &job_id.to_string(),
            pin.target_id.as_deref(),
            pin.binding_id.as_deref(),
            &pin.profile_id,
            pin.profile_revision,
            pin.stock_id.as_deref(),
            pin.loaded_media_snapshot_json.as_deref(),
        )?;
    }
    confirm_cloud_accept(
        cloud,
        store,
        route_coordinator,
        &CloudAcceptIntent {
            job_id: job_id.to_string(),
            lease_id: offer.lease_id.to_string(),
            lease_token: offer.lease_token,
            lease_expires_unix_ms: offer.lease_expires_at.timestamp_millis(),
            content_sha256: stored.sha256,
            local_sequence: u64::try_from(local.printer_sequence).unwrap_or(u64::MAX),
        },
    )
    .await
}

fn resolve_cloud_offer_printer(
    inventory_store: &AgentStore,
    logical_printer_id: &str,
) -> Result<StoredPrinter> {
    inventory_store
        .printer(logical_printer_id)?
        .with_context(|| format!("printer_not_found: {logical_printer_id}"))
}

#[derive(Debug)]
struct OfferRejection {
    reason: &'static str,
    message: String,
}

impl OfferRejection {
    const fn new(reason: &'static str, message: String) -> Self {
        Self { reason, message }
    }
}

impl std::fmt::Display for OfferRejection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for OfferRejection {}

#[derive(Debug)]
struct ProfilePinMetadata {
    target_id: Option<String>,
    binding_id: Option<String>,
    profile_id: String,
    profile_revision: u64,
    stock_id: Option<String>,
    loaded_media_snapshot_json: Option<String>,
}

fn profile_pin_metadata(
    metadata: &std::collections::BTreeMap<String, String>,
) -> Result<Option<ProfilePinMetadata>> {
    let value = |suffix: &str| {
        metadata
            .get(&format!("piqae.{suffix}"))
            .or_else(|| metadata.get(&format!("spool.{suffix}")))
    };
    let Some(profile_id) = value("profile_id") else {
        return Ok(None);
    };
    let revision = value("profile_revision")
        .context("piqae.profile_revision is required with piqae.profile_id")?
        .parse::<u64>()
        .context("piqae.profile_revision must be an unsigned integer")?;
    if profile_id.trim().is_empty() || revision == 0 {
        anyhow::bail!("profile pin metadata is invalid");
    }
    let loaded_media_snapshot_json = value("loaded_media_snapshot").cloned();
    if let Some(snapshot) = &loaded_media_snapshot_json {
        let _: serde_json::Value =
            serde_json::from_str(snapshot).context("loaded-media snapshot is invalid")?;
    }
    Ok(Some(ProfilePinMetadata {
        target_id: value("target_id").cloned(),
        binding_id: value("binding_id").cloned(),
        profile_id: profile_id.clone(),
        profile_revision: revision,
        stock_id: value("stock_id").cloned(),
        loaded_media_snapshot_json,
    }))
}

async fn resume_pending_cloud_accepts(
    cloud: &CloudConfiguration,
    store: &mut AgentStore,
    route_coordinator: &Arc<Mutex<route_coordinator::RouteCoordinator>>,
) {
    let intents = match store.pending_cloud_accepts() {
        Ok(intents) => intents,
        Err(error) => {
            warn!(%error, "pending cloud accept intents could not be read");
            return;
        }
    };
    for intent in intents {
        let job_id = intent.job_id.clone();
        let result = tokio::time::timeout(
            LEASE_RENEWAL_REQUEST_TIMEOUT,
            confirm_cloud_accept(cloud, store, route_coordinator, &intent),
        )
        .await;
        if !matches!(result, Ok(Ok(()))) {
            warn!(%job_id, "pending cloud acceptance retry deferred");
        }
    }
}

async fn confirm_cloud_accept(
    cloud: &CloudConfiguration,
    store: &mut AgentStore,
    route_coordinator: &Arc<Mutex<route_coordinator::RouteCoordinator>>,
    intent: &CloudAcceptIntent,
) -> Result<()> {
    let job_id = intent.job_id.parse::<JobId>()?;
    let route_proof = route_coordinator
        .lock()
        .await
        .cloud_proof_for_job(&cloud.connector_id, &intent.job_id);
    cloud
        .client
        .accept_job(
            &cloud.identity,
            job_id,
            &AgentAcceptJobRequest {
                lease_id: intent.lease_id.parse()?,
                lease_token: intent.lease_token.clone(),
                content_sha256: intent.content_sha256.clone(),
                local_sequence: intent.local_sequence,
                route_reservation_id: route_proof.as_ref().map(|proof| proof.reservation_id),
                route_generation: route_proof.as_ref().map(|proof| proof.generation),
                route_fencing_token: route_proof.map(|proof| proof.fencing_token),
            },
        )
        .await
        .map_err(|_| anyhow::anyhow!("remote cloud acceptance failed"))?;
    store.activate_cloud_job(&intent.job_id, Utc::now().timestamp_millis())?;
    Ok(())
}

async fn maintain_lease<T, Work, Renew, Renewal>(
    initial_expires_at: chrono::DateTime<Utc>,
    maximum_interval: Duration,
    work: Work,
    mut renew: Renew,
) -> Result<T>
where
    Work: Future<Output = Result<T>>,
    Renew: FnMut() -> Renewal,
    Renewal: Future<Output = Result<chrono::DateTime<Utc>>>,
{
    let mut expires_at = initial_expires_at;
    tokio::pin!(work);
    loop {
        let delay = lease_renewal_delay(expires_at, maximum_interval);
        tokio::select! {
            biased;
            () = tokio::time::sleep(delay) => {
                expires_at = renew().await?;
            }
            result = &mut work => return result,
        }
    }
}

fn lease_renewal_delay(expires_at: chrono::DateTime<Utc>, maximum_interval: Duration) -> Duration {
    const SAFETY_MARGIN: Duration = Duration::from_secs(5);
    const NORMAL_MINIMUM_DELAY: Duration = Duration::from_millis(250);
    let maximum_interval = maximum_interval.max(Duration::from_millis(1));
    let minimum_delay = maximum_interval.min(NORMAL_MINIMUM_DELAY);
    let remaining = (expires_at - Utc::now()).to_std().unwrap_or(Duration::ZERO);
    remaining
        .saturating_sub(SAFETY_MARGIN)
        .min(maximum_interval)
        .max(minimum_delay)
}

#[allow(
    clippy::too_many_lines,
    reason = "encrypted and plaintext materialization share one lease-fenced boundary"
)]
async fn materialize_descriptor(
    cloud: &CloudConfiguration,
    content_store: &ContentStore,
    uri_fetcher: &UriFetcher,
    job_id: JobId,
    lease_id: uuid::Uuid,
    lease_token: &str,
    descriptor: ContentDescriptor,
) -> Result<piqae_agent_core::StoredContent> {
    match descriptor {
        ContentDescriptor::InlineBase64 {
            data,
            sha256,
            bytes: _,
        } => {
            let decoded = STANDARD
                .decode(data)
                .context("decode offered base64 content")?;
            if let Some(expected) = sha256 {
                let path = content_store
                    .put_verified(&expected, std::io::Cursor::new(decoded))
                    .await?;
                Ok(piqae_agent_core::StoredContent {
                    bytes: tokio::fs::metadata(&path).await?.len(),
                    path,
                    sha256: expected,
                })
            } else {
                Ok(content_store.put(std::io::Cursor::new(decoded)).await?)
            }
        }
        ContentDescriptor::Download {
            url: _,
            sha256,
            bytes,
        } => {
            if bytes > ContentStore::MAX_CONTENT_BYTES {
                anyhow::bail!("offered content exceeds local limit");
            }
            let response = cloud
                .client
                .download_content(&cloud.identity, job_id, lease_id, lease_token)
                .await?;
            if response
                .content_length()
                .is_some_and(|length| length > ContentStore::MAX_CONTENT_BYTES)
            {
                anyhow::bail!("download response exceeds local limit");
            }
            let stream = response.bytes_stream().map_err(std::io::Error::other);
            let path = content_store
                .put_verified(&sha256, StreamReader::new(stream))
                .await?;
            Ok(piqae_agent_core::StoredContent {
                bytes,
                path,
                sha256,
            })
        }
        ContentDescriptor::EncryptedDownload {
            url: _,
            sha256,
            bytes,
            manifest,
        } => {
            let manifest_digest = URL_SAFE_NO_PAD
                .decode(&manifest.ciphertext_sha256)
                .context("decode encrypted manifest digest")?;
            if manifest_digest.len() != 32 || hex::encode(manifest_digest) != sha256 {
                anyhow::bail!("encrypted manifest digest does not match the leased ciphertext");
            }
            if bytes > MAX_CIPHERTEXT_BYTES {
                anyhow::bail!("offered ciphertext exceeds local limit");
            }
            if manifest
                .binding
                .expires_at
                .parse::<chrono::DateTime<Utc>>()
                .is_err()
                || manifest
                    .binding
                    .expires_at
                    .parse::<chrono::DateTime<Utc>>()?
                    <= Utc::now()
            {
                anyhow::bail!("encrypted job binding is expired or invalid");
            }
            let selected_key_id = cloud
                .content_encryption_keys
                .select_key_id(manifest.recipients.iter().filter_map(|recipient| {
                    (recipient.algorithm == piqae_domain::ENCRYPTED_JOB_V3_RECIPIENT_ALGORITHM)
                        .then_some(recipient.key_id.as_str())
                }))
                .context("encrypted job has no recipient for this node key")?;
            let recipient = manifest
                .recipients
                .iter()
                .find(|recipient| recipient.key_id == selected_key_id)
                .context("encrypted job has no recipient for this node key")?;
            let recipient_private_key = cloud
                .content_encryption_keys
                .key(&recipient.key_id)
                .context("encrypted job recipient key is unavailable")?;
            let response = cloud
                .client
                .download_content(&cloud.identity, job_id, lease_id, lease_token)
                .await?;
            if response
                .content_length()
                .is_some_and(|length| length > MAX_CIPHERTEXT_BYTES)
            {
                anyhow::bail!("ciphertext response exceeds local limit");
            }
            let stream = response.bytes_stream().map_err(std::io::Error::other);
            let ciphertext_path = content_store
                .put_verified_with_limit(&sha256, StreamReader::new(stream), MAX_CIPHERTEXT_BYTES)
                .await?;
            let result = async {
                let ciphertext = tokio::fs::read(&ciphertext_path)
                    .await
                    .context("read verified ciphertext")?;
                let plaintext = zeroize::Zeroizing::new(decrypt_encrypted_content(
                    recipient_private_key,
                    recipient,
                    &manifest,
                    &ciphertext,
                )?);
                if plaintext.len()
                    > usize::try_from(ContentStore::MAX_CONTENT_BYTES).unwrap_or(usize::MAX)
                {
                    anyhow::bail!("decrypted content exceeds local limit");
                }
                content_store
                    .put_confidential(std::io::Cursor::new(plaintext))
                    .await
                    .context("persist decrypted content")
            }
            .await;
            let _ = tokio::fs::remove_file(&ciphertext_path).await;
            result
        }
        ContentDescriptor::Uri {
            uri,
            authentication,
            sha256,
            bytes,
        } => {
            if bytes.is_some_and(|value| value > ContentStore::MAX_CONTENT_BYTES) {
                anyhow::bail!("offered URI content exceeds local limit");
            }
            Ok(uri_fetcher
                .fetch_to_store(
                    content_store,
                    &uri,
                    authentication.as_ref(),
                    sha256.as_deref(),
                )
                .await?)
        }
        ContentDescriptor::BusinessDocument {
            policy: _,
            render,
            fallback,
            fallback_allowed,
            decision_reason: _,
        } => {
            let rendered = if render.renderer_abi == RENDERER_ABI
                && render.resource_abi == RESOURCE_ABI
            {
                let specification: piqae_document_renderer::BusinessDocumentV1 =
                    serde_json::from_value(render.specification.clone())
                        .context("decode business document specification")?;
                let resources = resolve_node_render_resources(
                    cloud,
                    job_id,
                    lease_id,
                    lease_token,
                    &specification,
                    &render.resources,
                )
                .await;
                let input_bytes =
                    u64::try_from(serde_json::to_vec(&render.input)?.len()).unwrap_or(u64::MAX);
                let capabilities = NodeDocumentCapabilities::local()
                    .with_persistent_resource_cache(DOCUMENT_RESOURCE_MAX_BYTES);
                resources.map_or_else(
                    |_| NodeRenderResult::UseServerPdf {
                        reason:
                            piqae_agent_core::document_render::FallbackReason::ResourceUnavailable,
                    },
                    |resources| {
                        render_with_resources_or_fallback(
                            &capabilities,
                            &NodeRenderRequirement {
                                negotiation_version: 1,
                                renderer_abi: render.renderer_abi.clone(),
                                renderer_build: piqae_document_renderer::RENDERER_VERSION.into(),
                                spec_version: piqae_document_renderer::BUSINESS_DOCUMENT_FORMAT
                                    .into(),
                                input_bytes,
                                maximum_pdf_bytes: render.expected_pdf_bytes,
                                maximum_pages: 10_000,
                                expected_pdf_sha256: render.expected_pdf_sha256.clone(),
                            },
                            &specification,
                            &render.input,
                            &resources,
                        )
                    },
                )
            } else {
                NodeRenderResult::UseServerPdf {
                    reason: piqae_agent_core::document_render::FallbackReason::ResourceUnavailable,
                }
            };
            if let NodeRenderResult::Pdf(pdf) = rendered {
                let path = content_store
                    .put_verified(&render.expected_pdf_sha256, std::io::Cursor::new(pdf))
                    .await?;
                return Ok(piqae_agent_core::StoredContent {
                    bytes: render.expected_pdf_bytes,
                    path,
                    sha256: render.expected_pdf_sha256,
                });
            }
            if !fallback_allowed {
                anyhow::bail!("node document rendering was required but failed closed");
            }
            Box::pin(materialize_descriptor(
                cloud,
                content_store,
                uri_fetcher,
                job_id,
                lease_id,
                lease_token,
                *fallback,
            ))
            .await
        }
    }
}

async fn resolve_node_render_resources(
    cloud: &CloudConfiguration,
    job_id: JobId,
    lease_id: uuid::Uuid,
    lease_token: &str,
    specification: &piqae_document_renderer::BusinessDocumentV1,
    offered: &[piqae_protocol::agent::BusinessDocumentResourceDescriptor],
) -> Result<piqae_document_renderer::ResolvedResources> {
    let cache = DOCUMENT_RESOURCE_CACHE
        .get()
        .context("document resource cache is unavailable")?;
    let offered = offered
        .iter()
        .map(|resource| {
            (
                resource
                    .digest
                    .trim_start_matches("sha256:")
                    .to_ascii_lowercase(),
                resource,
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut resolved = piqae_document_renderer::ResolvedResources::default();
    for (resource_id, resource) in &specification.resources {
        let piqae_document_renderer::Resource::Image {
            digest,
            media_type,
            byte_length,
        } = resource;
        let digest = digest
            .strip_prefix("sha256:")
            .context("document resource digest is not canonical")?
            .to_ascii_lowercase();
        let manifest = offered
            .get(&digest)
            .context("document resource was not offered")?;
        anyhow::ensure!(
            manifest.media_type == *media_type && manifest.byte_length == *byte_length,
            "document resource manifest disagrees with specification"
        );
        let descriptor = NodeResourceDescriptor {
            digest: digest.clone(),
            media_type: media_type.clone(),
            byte_length: *byte_length,
        };
        let now = Utc::now().timestamp_millis();
        let path = if let Some(path) = cache.resolve_existing(&descriptor, now)? {
            path
        } else {
            let bytes = download_document_resource_bounded(
                cloud,
                job_id,
                lease_id,
                lease_token,
                &descriptor,
            )
            .await?;
            cache.resolve(
                &descriptor,
                || Ok(std::io::Cursor::new(bytes)),
                Utc::now().timestamp_millis(),
            )?
        };
        cache.pin(&digest)?;
        let bytes = std::fs::read(path);
        let released = cache.unpin(&digest);
        let bytes = bytes?;
        released?;
        resolved.images.insert(resource_id.clone(), bytes);
    }
    Ok(resolved)
}

async fn download_document_resource_bounded(
    cloud: &CloudConfiguration,
    job_id: JobId,
    lease_id: uuid::Uuid,
    lease_token: &str,
    descriptor: &NodeResourceDescriptor,
) -> Result<Vec<u8>> {
    let response = cloud
        .client
        .download_document_resource(
            &cloud.identity,
            job_id,
            lease_id,
            lease_token,
            &descriptor.digest,
        )
        .await?;
    anyhow::ensure!(
        response.content_length().is_none_or(|length| {
            length == descriptor.byte_length && length <= DOCUMENT_RESOURCE_MAX_BYTES
        }),
        "document resource response has an invalid length"
    );
    let mut stream = response.bytes_stream();
    let capacity = usize::try_from(descriptor.byte_length.min(64 * 1024)).unwrap_or(64 * 1024);
    let mut bytes = Vec::with_capacity(capacity);
    while let Some(chunk) = stream.try_next().await? {
        anyhow::ensure!(
            bytes.len().saturating_add(chunk.len())
                <= usize::try_from(descriptor.byte_length).unwrap_or(usize::MAX),
            "document resource response exceeds its manifest"
        );
        bytes.extend_from_slice(&chunk);
    }
    anyhow::ensure!(
        u64::try_from(bytes.len()).unwrap_or(u64::MAX) == descriptor.byte_length,
        "document resource response is truncated"
    );
    Ok(bytes)
}

fn decrypt_encrypted_content(
    private_key: &SecretKey,
    recipient: &piqae_domain::EncryptedContentRecipient,
    manifest: &piqae_domain::EncryptedContentManifest,
    ciphertext: &[u8],
) -> Result<Vec<u8>> {
    validate_encrypted_ciphertext_size(ciphertext.len())?;
    if manifest.version != piqae_domain::ENCRYPTED_JOB_V3_VERSION
        || manifest.suite != piqae_domain::ENCRYPTED_JOB_V3_SUITE
        || recipient.algorithm != piqae_domain::ENCRYPTED_JOB_V3_RECIPIENT_ALGORITHM
    {
        anyhow::bail!("unsupported encrypted job envelope or recipient algorithm");
    }
    let ephemeral_bytes = URL_SAFE_NO_PAD
        .decode(&recipient.ephemeral_public_key)
        .context("decode ephemeral P-256 public key")?;
    if ephemeral_bytes.len() != 65 {
        anyhow::bail!("ephemeral P-256 public key has invalid length");
    }
    let ephemeral =
        PublicKey::from_sec1_bytes(&ephemeral_bytes).context("parse ephemeral P-256 public key")?;
    let salt = URL_SAFE_NO_PAD
        .decode(&recipient.hkdf_salt)
        .context("decode recipient HKDF salt")?;
    if salt.len() != 32 {
        anyhow::bail!("recipient HKDF salt has invalid length");
    }
    let wrap_iv = URL_SAFE_NO_PAD
        .decode(&recipient.key_wrap_iv)
        .context("decode content-key wrap IV")?;
    if wrap_iv.len() != 12 {
        anyhow::bail!("content-key wrap IV has invalid length");
    }
    let wrapped = URL_SAFE_NO_PAD
        .decode(&recipient.encrypted_content_key)
        .context("decode wrapped content key")?;
    if wrapped.len() != 48 {
        anyhow::bail!("wrapped content key has invalid length");
    }
    let aad = serde_json::to_vec(&manifest.binding).context("encode authenticated binding")?;
    let shared = diffie_hellman(private_key.to_nonzero_scalar(), ephemeral.as_affine());
    let mut info = b"piqae-content-key-wrap-v3\0".to_vec();
    info.extend_from_slice(manifest.binding.envelope_id.as_bytes());
    info.push(0);
    info.extend_from_slice(recipient.key_id.as_bytes());
    let hkdf = Hkdf::<Sha256>::new(Some(&salt), shared.raw_secret_bytes().as_slice());
    let mut wrapping_key = zeroize::Zeroizing::new([0_u8; 32]);
    hkdf.expand(&info, wrapping_key.as_mut())
        .map_err(|_| anyhow::anyhow!("derive content-key wrapping key"))?;
    let wrapping_cipher = Aes256Gcm::new_from_slice(wrapping_key.as_slice())
        .map_err(|_| anyhow::anyhow!("invalid content-key wrapping key"))?;
    let content_key = zeroize::Zeroizing::new(
        wrapping_cipher
            .decrypt(
                wrap_iv.as_slice().into(),
                Payload {
                    msg: &wrapped,
                    aad: &aad,
                },
            )
            .map_err(|_| anyhow::anyhow!("wrapped content key authentication failed"))?,
    );
    if content_key.len() != 32 {
        anyhow::bail!("unwrapped content key has invalid length");
    }
    let iv = URL_SAFE_NO_PAD
        .decode(&manifest.iv)
        .context("decode encryption IV")?;
    if iv.len() != 12 {
        anyhow::bail!("encryption IV has invalid length");
    }
    let cipher = Aes256Gcm::new_from_slice(&content_key)
        .map_err(|_| anyhow::anyhow!("invalid content key"))?;
    cipher
        .decrypt(
            iv.as_slice().into(),
            Payload {
                msg: ciphertext,
                aad: &aad,
            },
        )
        .map_err(|_| anyhow::anyhow!("encrypted content authentication failed"))
}

const AES_GCM_TAG_BYTES: u64 = 16;
const MAX_CIPHERTEXT_BYTES: u64 = ContentStore::MAX_CONTENT_BYTES + AES_GCM_TAG_BYTES;

fn validate_encrypted_ciphertext_size(bytes: usize) -> Result<()> {
    if u64::try_from(bytes).unwrap_or(u64::MAX) > MAX_CIPHERTEXT_BYTES {
        anyhow::bail!("encrypted content exceeds local limit");
    }
    Ok(())
}

/// How long a node waits between attempts after its identity was rejected.
///
/// Revocation is not fixed by retrying, so hammering the control plane serves
/// nobody. The node still retries slowly because the two recoverable causes —
/// a corrected clock, or an operator restoring the node — resolve without a
/// restart, and the next attempt is what discovers that.
const UNAUTHORIZED_RETRY_SECONDS: u64 = 60;

/// Classifies one sync failure into the connection state an operator should see.
///
/// A rejected signature and an unreachable network are the same event to a
/// retry loop but entirely different problems to the person holding the
/// printer, so they must not share a status.
fn failure_state(error: &ClientError) -> ConnectionState {
    match error.unauthorized_code() {
        // The node's clock is outside the signing window. The client corrects
        // its offset from the rejection itself, so the next attempt should
        // succeed; report a transient fault rather than a revoked identity.
        Some("stale_agent_request") => ConnectionState::Degraded,
        Some(_) => ConnectionState::Unauthorized,
        None => ConnectionState::Offline,
    }
}

/// Returns only the bounded protocol classification suitable for local
/// diagnostics. Transport bodies and request material are deliberately never
/// retained.
fn redacted_sync_error_code(error: &ClientError) -> Option<String> {
    error.unauthorized_code().map(str::to_owned)
}

async fn sync_failed(
    error: &ClientError,
    failures: &mut u32,
    connection: &RwLock<ConnectionState>,
) -> Duration {
    *failures = failures.saturating_add(1);
    let state = failure_state(error);
    *connection.write().await = state;
    if state == ConnectionState::Unauthorized {
        error!(
            code = error.unauthorized_code().unwrap_or("unauthorized"),
            retry_seconds = UNAUTHORIZED_RETRY_SECONDS,
            "the control plane rejected this node's identity; it has been \
             revoked or its device key no longer matches. Pair this node again \
             to restore cloud printing."
        );
        return Duration::from_secs(UNAUTHORIZED_RETRY_SECONDS);
    }
    if let Some(code) = error.unauthorized_code() {
        warn!(
            code,
            "this node's clock is outside the control-plane signing window; \
             the offset has been corrected and the next attempt will use it. \
             Enable time synchronization to avoid repeating this."
        );
    }
    let exponent = (*failures).min(5);
    let delay = 1_u64.checked_shl(exponent).unwrap_or(30).min(30);
    warn!(%error, retry_seconds = delay, "agent sync failed");
    Duration::from_secs(delay)
}

fn sync_request(
    store: &AgentStore,
    agent_id: AgentId,
    started_at: chrono::DateTime<Utc>,
    paused: bool,
    printers: Option<Vec<PrinterSnapshot>>,
) -> Result<AgentSyncRequest, StorageError> {
    let resource_cache_ready = DOCUMENT_RESOURCE_CACHE.get().is_some();
    let counts = store.queue_counts()?;
    let events = store
        .pending_cloud_events(0, 100)?
        .into_iter()
        .map(|event| protocol_event(store, agent_id, event))
        .collect::<Result<Vec<_>, _>>()?;
    let event_cursor = events.last().map(|event| event.id);
    let printer_revision = store
        .setting("printer_inventory_revision")?
        .and_then(|revision| revision.parse::<u64>().ok())
        .unwrap_or(0);
    let (executor_crashes, last_error_code) = store.failure_health()?;
    Ok(AgentSyncRequest {
        agent_id,
        protocol_version: CURRENT_PROTOCOL_VERSION,
        agent_version: env!("CARGO_PKG_VERSION").to_owned(),
        printer_revision,
        acknowledged_command_cursor: store.setting("command_cursor")?,
        event_cursor,
        queue: QueueSnapshot {
            queued_jobs: counts.queued,
            active_jobs: counts.active,
            content_bytes: 0,
            accepts_jobs: !paused,
        },
        health: AgentHealth {
            started_at,
            observed_at: Utc::now(),
            sqlite_integrity_ok: store.integrity_check()?,
            executor_crashes,
            last_error_code,
        },
        printers,
        events,
        diagnostics: pending_diagnostics(store)?,
        document_render: piqae_protocol::agent::DocumentRenderCapabilities {
            renderer_abi: Some(RENDERER_ABI.into()),
            resource_abi: Some(RESOURCE_ABI.into()),
            persistent_cache: resource_cache_ready,
            font_rendering: false,
            image_media_types: vec!["image/jpeg".into()],
            font_media_types: Vec::new(),
            // A process-global CAS may contain resources used by several
            // independently authorized connectors. Never disclose membership
            // across those tenant boundaries; offers still get local hits.
            cached_resource_digests: Vec::new(),
        },
        capabilities: piqae_protocol::agent::AgentProtocolCapabilities {
            features: vec![
                piqae_protocol::agent::AgentFeature::DestinationIdentityV1,
                piqae_protocol::agent::AgentFeature::RouteInventoryV1,
                piqae_protocol::agent::AgentFeature::ProjectionAckV1,
                piqae_protocol::agent::AgentFeature::SpoolerObservationV1,
                piqae_protocol::agent::AgentFeature::RouteFencingV1,
                piqae_protocol::agent::AgentFeature::NativeHandoffEvidenceV1,
                piqae_protocol::agent::AgentFeature::TopologyChangesV1,
                piqae_protocol::agent::AgentFeature::ProfileStockFreshnessV1,
                piqae_protocol::agent::AgentFeature::RouteObservationSequenceV1,
                piqae_protocol::agent::AgentFeature::RouteLeaseRenewalV1,
                piqae_protocol::agent::AgentFeature::AmbiguousHandoffResolutionV1,
            ],
            telemetry_privacy: piqae_protocol::agent::TelemetryPrivacy::CountsOnly,
        },
        route_observations: Vec::new(),
        topology_changes: Vec::new(),
        native_handoffs: Vec::new(),
    })
}

async fn discover_cloud_printers(
    store: &mut AgentStore,
    discovery: &PrinterDiscovery,
    route_coordinator: &Arc<Mutex<route_coordinator::RouteCoordinator>>,
    support_packs: &SupportPackRegistry,
    inventory_revision: u64,
) -> Result<Vec<PrinterSnapshot>> {
    let discovered = run_printer_discovery(discovery).await?;
    let observed_at = Utc::now();
    let mut routes =
        route_coordinator
            .lock()
            .await
            .reconcile(&discovered, inventory_revision, observed_at)?;
    let present_native_ids = discovered
        .iter()
        .map(|printer| printer.native_id.clone())
        .collect::<Vec<_>>();
    let observed_unix_ms = Utc::now().timestamp_millis();
    let snapshots = discovered
        .into_iter()
        .map(|printer| {
            let semantic_capabilities = support_packs
                .normalize(printer.driver_fingerprint.as_ref(), &printer.native_options)
                .context("normalize trusted driver support pack")?;
            let state = serde_json::to_string(&printer.state)?;
            let capabilities = serde_json::to_string(&printer.capabilities)?;
            let stored = store.upsert_printer(
                &printer.native_id,
                &printer.name,
                state.trim_matches('"'),
                printer.is_default,
                &capabilities,
                observed_unix_ms,
            )?;
            let native_options = serde_json::to_string(&printer.native_options)?;
            let profile = store.store_printer_profile(
                &stored.printer_id,
                None,
                &capabilities,
                &native_options,
                observed_unix_ms,
            )?;
            store.ensure_current_printer_defaults_profile(&stored.printer_id, observed_unix_ms)?;
            let profiles = store
                .named_profiles(&stored.printer_id)?
                .into_iter()
                .map(|profile| {
                    Ok(PrinterProfileSnapshot {
                        profile_id: profile.profile_id,
                        revision: profile.revision,
                        name: profile.name,
                        is_default: profile.is_default,
                        options: serde_json::from_str(&profile.options_json)?,
                        status: serde_json::from_value(serde_json::Value::String(profile.status))?,
                        native_kind: if profile.native_kind.is_empty() {
                            None
                        } else {
                            Some(serde_json::from_value(serde_json::Value::String(
                                profile.native_kind,
                            ))?)
                        },
                        native_digest: profile.native_digest,
                        driver_fingerprint: serde_json::from_str(&profile.driver_fingerprint_json)?,
                        summary: serde_json::from_str(&profile.summary_json)?,
                        stock_id: profile.stock_id,
                        safe_overrides: serde_json::from_str(&profile.safe_overrides_json)?,
                        last_validated_unix_ms: profile.last_validated_unix_ms,
                        last_test_job_id: profile.last_test_job_id,
                        // The generated live-default preset is always usable
                        // by an authorized connector even though it is not a
                        // user-published immutable native capture.
                        published: profile.published || profile.uses_current_printer_defaults,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(Some(PrinterSnapshot {
                id: stored.printer_id.parse()?,
                native_id: stored.native_id,
                name: stored.name,
                state: printer.state,
                is_default: printer.is_default,
                capabilities: printer.capabilities,
                exposed: true,
                capability_revision: profile.revision,
                native_options: printer.native_options,
                semantic_capabilities,
                profiles,
                route: routes.remove(&printer.native_id),
            }))
        })
        .collect::<Result<Vec<Option<PrinterSnapshot>>>>()?;
    store.reconcile_printer_presence(&present_native_ids)?;
    Ok(snapshots.into_iter().flatten().collect())
}

fn route_observation_inputs(
    connector_store: &AgentStore,
    inventory_store: &AgentStore,
) -> Vec<(String, std::collections::BTreeSet<String>)> {
    let printers = match inventory_store.present_printers() {
        Ok(printers) => printers,
        Err(error) => {
            warn!(%error, "route observation could not read local printer inventory");
            return Vec::new();
        }
    };
    printers
        .into_iter()
        .take(128)
        .map(|printer| {
            let connector_native_ids = connector_store
                .jobs_for_printer(&printer.printer_id, 200)
                .unwrap_or_default()
                .into_iter()
                .filter_map(|job| job.native_job_id)
                .collect::<std::collections::BTreeSet<_>>();
            (printer.native_id, connector_native_ids)
        })
        .collect()
}

async fn collect_route_observations(
    printers: Vec<(String, std::collections::BTreeSet<String>)>,
    discovery: &PrinterDiscovery,
    route_coordinator: &Arc<Mutex<route_coordinator::RouteCoordinator>>,
    observation_cache: &Arc<Mutex<RouteObservationCache>>,
    inventory_revision: u64,
) -> Vec<piqae_protocol::agent::RouteObservation> {
    use piqae_protocol::agent::RouteObservation;

    let mut observations = Vec::new();
    let sequence_allocation = route_coordinator
        .lock()
        .await
        .allocate_observation_sequences(printers.len());
    let sequences = match sequence_allocation {
        Ok(sequences) => sequences,
        Err(error) => {
            warn!(%error, "route observation sequence could not be persisted");
            return observations;
        }
    };
    for ((native_id, connector_native_ids), sequence) in printers.into_iter().zip(sequences) {
        let route_id = route_coordinator.lock().await.route_id(&native_id);
        let native_id_for_collection = native_id.clone();
        let cached = observation_cache
            .lock()
            .await
            .get_or_collect(&native_id, || async move {
                let observed_at = Utc::now();
                let state = discovery
                    .observe_state(&native_id_for_collection)
                    .await
                    .unwrap_or(piqae_domain::PrinterState::Unknown);
                let queue = match discovery.observe_queue(&native_id_for_collection).await {
                    Ok(jobs) => Some(jobs),
                    Err(error) => {
                        warn!(%error, "privacy-safe native queue observation deferred");
                        None
                    }
                };
                CachedNativeRouteObservation {
                    cached_at: tokio::time::Instant::now(),
                    observed_at,
                    state,
                    queue,
                }
            })
            .await;
        let observed_at = cached.observed_at;
        let state = cached.state;
        let queue = cached
            .queue
            .as_deref()
            .map(|jobs| privacy_safe_queue_observation(jobs, &connector_native_ids));
        let state_reasons = match state {
            piqae_domain::PrinterState::Paused => vec!["paused".into()],
            piqae_domain::PrinterState::PaperOut => vec!["media_empty".into()],
            piqae_domain::PrinterState::Error => vec!["printer_error".into()],
            piqae_domain::PrinterState::Offline => vec!["offline".into()],
            _ => Vec::new(),
        };
        observations.push(RouteObservation {
            local_route_key: route_id,
            sequence,
            observed_at,
            inventory_revision,
            state,
            accepts_jobs: matches!(
                state,
                piqae_domain::PrinterState::Online | piqae_domain::PrinterState::Busy
            ),
            state_reasons,
            queue,
            profile_observed_at: Some(observed_at),
            stock_observed_at: None,
        });
    }
    observations
}

fn privacy_safe_queue_observation(
    jobs: &[piqae_protocol::executor::NativeQueueJob],
    connector_native_ids: &std::collections::BTreeSet<String>,
) -> piqae_protocol::agent::PrivacySafeQueueObservation {
    use piqae_protocol::{agent::PrivacySafeQueueObservation, executor::NativeJobState};

    let total_jobs = u32::try_from(jobs.len()).unwrap_or(u32::MAX);
    let active_jobs = u32::try_from(
        jobs.iter()
            .filter(|job| job.state == NativeJobState::Printing)
            .count(),
    )
    .unwrap_or(u32::MAX);
    let held_jobs = u32::try_from(
        jobs.iter()
            .filter(|job| job.state == NativeJobState::Blocked)
            .count(),
    )
    .unwrap_or(u32::MAX);
    let connector_jobs = u32::try_from(
        jobs.iter()
            .filter(|job| connector_native_ids.contains(&job.native_job_id))
            .count(),
    )
    .unwrap_or(u32::MAX);
    PrivacySafeQueueObservation {
        total_jobs,
        active_jobs,
        held_jobs,
        connector_jobs,
        other_piqae_or_external_jobs: total_jobs.saturating_sub(connector_jobs),
        // Native adapters cannot always classify jobs created by other
        // software. Keep them in one privacy-safe count rather than inspecting
        // or uploading titles, users, paths, or document metadata.
        unknown_jobs: total_jobs.saturating_sub(connector_jobs),
    }
}

async fn run_printer_discovery(discovery: &PrinterDiscovery) -> Result<Vec<DiscoveredPrinter>> {
    let discovered = match discovery {
        PrinterDiscovery::Disabled => Vec::new(),
        PrinterDiscovery::Fake => vec![DiscoveredPrinter {
            native_id: "fake-printer".into(),
            name: "Piqae deterministic fake printer".into(),
            is_default: true,
            state: piqae_domain::PrinterState::Online,
            capabilities: piqae_domain::PrinterCapabilities::default(),
            native_options: std::collections::BTreeMap::new(),
            driver_fingerprint: None,
            identity_evidence: Vec::new(),
        }],
        PrinterDiscovery::Process(executor) => match executor
            .execute_operation(
                ExecutorOperation::DiscoverPrinters,
                Utc::now().timestamp_millis() + 30_000,
            )
            .await
            .map_err(|error| anyhow::anyhow!("{}: {}", error.code, error.message))?
        {
            ExecutorResult::Printers { printers } => printers,
            _ => anyhow::bail!("executor returned the wrong discovery result"),
        },
    };
    Ok(discovered)
}

fn protocol_event(
    store: &AgentStore,
    agent_id: AgentId,
    event: PendingEvent,
) -> Result<JobEvent, StorageError> {
    let event_id = event
        .event_id
        .parse::<EventId>()
        .map_err(|error| StorageError::InvalidLocalEvent(error.to_string()))?;
    let job_id = event
        .job_id
        .parse::<JobId>()
        .map_err(|error| StorageError::InvalidLocalEvent(error.to_string()))?;
    let state = parse_job_state(&event.state)
        .ok_or_else(|| StorageError::InvalidLocalEvent(event.state.clone()))?;
    let occurred_at = chrono::DateTime::from_timestamp_millis(event.observed_unix_ms)
        .ok_or_else(|| StorageError::InvalidLocalEvent("invalid event timestamp".into()))?;
    let native_job_id = store
        .get_job(&event.job_id)?
        .and_then(|job| job.native_job_id);
    Ok(JobEvent {
        id: event_id,
        job_id,
        sequence: u64::try_from(event.job_sequence)
            .map_err(|error| StorageError::InvalidLocalEvent(error.to_string()))?,
        state,
        reason: event.reason.as_deref().and_then(parse_failure_reason),
        message: event.message,
        agent_id: Some(agent_id),
        native_job_id,
        occurred_at,
    })
}

fn parse_job_state(value: &str) -> Option<JobState> {
    Some(match value {
        "registered" => JobState::Registered,
        "content_pending" => JobState::ContentPending,
        "waiting_for_agent" => JobState::WaitingForAgent,
        "agent_downloading" => JobState::AgentDownloading,
        "agent_accepted" => JobState::AgentAccepted,
        "queued_local" => JobState::QueuedLocal,
        "preparing" => JobState::Preparing,
        "rendering" => JobState::Rendering,
        "spool_intent" => JobState::SpoolIntent,
        "accepted_by_spooler" => JobState::AcceptedBySpooler,
        "spooling" => JobState::Spooling,
        "printing" => JobState::Printing,
        "blocked" => JobState::Blocked,
        "completed_reported" => JobState::CompletedReported,
        "delivery_uncertain" => JobState::DeliveryUncertain,
        "cancel_requested" => JobState::CancelRequested,
        "cancelled" => JobState::Cancelled,
        "expired" => JobState::Expired,
        "failed_retryable" => JobState::FailedRetryable,
        "failed_terminal" => JobState::FailedTerminal,
        _ => return None,
    })
}

fn parse_failure_reason(value: &str) -> Option<JobFailureReason> {
    Some(match value {
        "agent_unavailable" => JobFailureReason::AgentUnavailable,
        "content_unavailable" => JobFailureReason::ContentUnavailable,
        "content_checksum_mismatch" => JobFailureReason::ContentChecksumMismatch,
        "download_timed_out" => JobFailureReason::DownloadTimedOut,
        "invalid_pdf" => JobFailureReason::InvalidPdf,
        "unsupported_option" => JobFailureReason::UnsupportedOption,
        "printer_offline" => JobFailureReason::PrinterOffline,
        "printer_paused" => JobFailureReason::PrinterPaused,
        "paper_out" => JobFailureReason::PaperOut,
        "access_denied" => JobFailureReason::AccessDenied,
        "driver_error" => JobFailureReason::DriverError,
        "executor_crashed" => JobFailureReason::ExecutorCrashed,
        "executor_timed_out" => JobFailureReason::ExecutorTimedOut,
        "ambiguous_handoff" => JobFailureReason::AmbiguousHandoff,
        "cancelled_by_user" | "cancelled_by_server" => JobFailureReason::CancelledByUser,
        "expired" | "expired_before_handoff" => JobFailureReason::Expired,
        "internal" => JobFailureReason::Internal,
        _ => return None,
    })
}

fn load_or_create_private_token(path: &Path) -> Result<String> {
    match std::fs::read_to_string(path) {
        Ok(token) if !token.trim().is_empty() && token.len() <= 1024 => {
            return Ok(token.trim().to_owned());
        }
        Ok(_) => anyhow::bail!("local token file is empty or oversized: {}", path.display()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
    }

    let (_, token) = SessionAuthenticator::generate();
    let mut options = std::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .with_context(|| format!("create {}", path.display()))?;
    file.write_all(token.as_bytes())?;
    file.sync_all()?;
    Ok(token)
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    fn test_connector_record(id: &str) -> connector_runtime::ConnectorRecord {
        connector_runtime::ConnectorRecord {
            connector_id: id.to_owned(),
            agent_id: format!("agt_{id}"),
            control_plane_url: Url::parse("https://api.piqae.example/").expect("url"),
            display_name: Some("Example service".to_owned()),
            workspace_name: Some("Example customer".to_owned()),
            authorization_type: Some("platform_customer".to_owned()),
            workspace_id: Some("wsp_test".to_owned()),
            environment_id: Some("env_test".to_owned()),
            requesting_service_account_id: Some("svc_test".to_owned()),
            manage_url: Some(Url::parse("https://app.example/manage").expect("url")),
            device_key_file: format!("connectors/{id}/device.key").into(),
            enabled: true,
            printer_grant: PrinterGrant::SelectedPrinters,
            allowed_printer_ids: vec!["prn_test".to_owned()],
        }
    }

    #[tokio::test]
    async fn stop_signal_remembers_stop_without_a_waiter() {
        let stop = StopSignal::default();
        stop.stop();
        tokio::time::timeout(Duration::from_millis(50), stop.cancelled())
            .await
            .expect("pre-existing stop must not be lost");
        assert!(stop.is_stopped());
    }

    #[tokio::test]
    async fn native_queue_collection_is_shared_but_connector_counts_are_isolated() {
        use piqae_protocol::executor::{NativeJobState, NativeQueueJob};
        use std::collections::BTreeSet;

        let collections = Arc::new(AtomicUsize::new(0));
        let jobs = vec![
            NativeQueueJob {
                native_job_id: "connector-a-job".into(),
                native_printer_id: "native-a".into(),
                title: "must remain local".into(),
                user: Some("must-remain-local".into()),
                state: NativeJobState::Printing,
                native_code: None,
                size_kib: None,
                created_unix_ms: None,
                processing_unix_ms: None,
                completed_unix_ms: None,
            },
            NativeQueueJob {
                native_job_id: "external-job".into(),
                native_printer_id: "native-a".into(),
                title: "never projected".into(),
                user: Some("never-projected".into()),
                state: NativeJobState::Blocked,
                native_code: None,
                size_kib: None,
                created_unix_ms: None,
                processing_unix_ms: None,
                completed_unix_ms: None,
            },
        ];
        let mut cache = RouteObservationCache::default();
        let collect_once = {
            let collections = Arc::clone(&collections);
            let jobs = jobs.clone();
            move || async move {
                collections.fetch_add(1, Ordering::SeqCst);
                CachedNativeRouteObservation {
                    cached_at: tokio::time::Instant::now(),
                    observed_at: Utc::now(),
                    state: piqae_domain::PrinterState::Busy,
                    queue: Some(jobs),
                }
            }
        };
        let first = cache.get_or_collect("native-a", collect_once).await;
        let second = cache
            .get_or_collect("native-a", || async {
                panic!("fresh shared route observation must not poll the OS again")
            })
            .await;
        assert_eq!(collections.load(Ordering::SeqCst), 1);

        let connector_a = privacy_safe_queue_observation(
            first.queue.as_deref().expect("queue"),
            &std::iter::once("connector-a-job".to_owned()).collect(),
        );
        let connector_b = privacy_safe_queue_observation(
            second.queue.as_deref().expect("queue"),
            &BTreeSet::new(),
        );
        assert_eq!(connector_a.total_jobs, 2);
        assert_eq!(connector_b.total_jobs, 2);
        assert_eq!(connector_a.connector_jobs, 1);
        assert_eq!(connector_b.connector_jobs, 0);
        let serialized = serde_json::to_string(&connector_a).expect("privacy-safe JSON");
        assert!(!serialized.contains("must remain local"));
        assert!(!serialized.contains("must-remain-local"));
    }

    #[test]
    fn projection_acknowledgement_negotiates_without_legacy_retry_churn() {
        let matching = piqae_protocol::agent::InventoryProjectionAcknowledgement {
            revision: 7,
            projected_at: Utc::now(),
        };
        let stale = piqae_protocol::agent::InventoryProjectionAcknowledgement {
            revision: 6,
            projected_at: Utc::now(),
        };

        assert!(inventory_projection_confirmed(false, None, 7));
        assert!(inventory_projection_confirmed(false, Some(&matching), 7));
        assert!(!inventory_projection_confirmed(false, Some(&stale), 7));
        assert!(inventory_projection_confirmed(true, Some(&matching), 7));
        assert!(!inventory_projection_confirmed(true, Some(&stale), 7));
        assert!(
            !inventory_projection_confirmed(true, None, 7),
            "a server that advertises projection ACKs must confirm the exact revision"
        );
    }

    #[tokio::test]
    async fn critical_task_completion_and_panic_are_reported_as_failures() {
        let completed = tokio::spawn(async {});
        let error = unexpected_task_exit("test control loop", completed.await);
        assert!(error.to_string().contains("exited unexpectedly"));

        let panicked = tokio::spawn(async { panic!("test task panic") });
        let error = unexpected_task_exit("test connector supervisor", panicked.await);
        assert!(error.to_string().contains("panicked"));
    }

    #[tokio::test]
    async fn connector_worker_liveness_detects_each_exited_task() {
        fn worker_with_tasks(
            sync: tokio::task::JoinHandle<()>,
            scheduler: tokio::task::JoinHandle<()>,
            connection_watch: tokio::task::JoinHandle<()>,
        ) -> ConnectorWorker {
            ConnectorWorker {
                record: test_connector_record("ncon_worker"),
                printer_inventory_dirty: Arc::new(AtomicBool::new(false)),
                wakeup: Arc::new(Notify::new()),
                last_sync_error_code: Arc::new(RwLock::new(None)),
                sync_stop: StopSignal::default(),
                scheduler_stop: StopSignal::default(),
                connection_stop: StopSignal::default(),
                sync,
                scheduler,
                connection_watch,
            }
        }

        for exited_task in 0..3 {
            let stop = StopSignal::default();
            let mut tasks = Vec::new();
            for task_index in 0..3 {
                let stop = stop.clone();
                tasks.push(tokio::spawn(async move {
                    if task_index != exited_task {
                        stop.cancelled().await;
                    }
                }));
            }
            tokio::task::yield_now().await;
            let connection_watch = tasks.pop().expect("connection watcher");
            let scheduler = tasks.pop().expect("scheduler");
            let sync = tasks.pop().expect("sync");
            let worker = worker_with_tasks(sync, scheduler, connection_watch);
            assert!(connector_worker_has_exited(&worker));
            stop.stop();
            let _ = worker.sync.await;
            let _ = worker.scheduler.await;
            let _ = worker.connection_watch.await;
        }
    }

    #[tokio::test]
    async fn connector_worker_restarts_when_reauthentication_rotates_identity() {
        let stop = StopSignal::default();
        let spawn_waiter = || {
            let stop = stop.clone();
            tokio::spawn(async move { stop.cancelled().await })
        };
        let record = test_connector_record("ncon_child");
        let worker = ConnectorWorker {
            record: record.clone(),
            printer_inventory_dirty: Arc::new(AtomicBool::new(false)),
            wakeup: Arc::new(Notify::new()),
            last_sync_error_code: Arc::new(RwLock::new(None)),
            sync_stop: StopSignal::default(),
            scheduler_stop: StopSignal::default(),
            connection_stop: StopSignal::default(),
            sync: spawn_waiter(),
            scheduler: spawn_waiter(),
            connection_watch: spawn_waiter(),
        };
        assert!(connector_worker_matches(&worker, &record));

        let mut rotated = record;
        rotated.device_key_file = "connectors/keys/rotated.key".into();
        assert!(!connector_worker_matches(&worker, &rotated));

        stop.stop();
        let _ = worker.sync.await;
        let _ = worker.scheduler.await;
        let _ = worker.connection_watch.await;
    }

    #[tokio::test]
    async fn connector_control_distinguishes_busy_from_unavailable() {
        let (sender, _receiver) = mpsc::channel(1);
        let (occupied_tx, _occupied_rx) = oneshot::channel();
        sender
            .try_send(ConnectorSupervisorCommand::Reload {
                respond_to: occupied_tx,
            })
            .expect("occupy supervisor queue");
        let (busy_tx, busy_rx) = oneshot::channel();
        let busy = sender
            .try_send(ConnectorSupervisorCommand::Revoke {
                connector_id: "ncon_busy".to_owned(),
                respond_to: busy_tx,
            })
            .expect_err("full queue");
        reject_connector_supervisor_command(busy);
        assert_eq!(
            busy_rx
                .await
                .expect("busy response")
                .expect_err("busy failure")
                .code,
            "connector_revoke_deferred"
        );

        let (closed_sender, closed_receiver) = mpsc::channel(1);
        drop(closed_receiver);
        let (closed_tx, closed_rx) = oneshot::channel();
        let closed = closed_sender
            .try_send(ConnectorSupervisorCommand::Reload {
                respond_to: closed_tx,
            })
            .expect_err("closed queue");
        reject_connector_supervisor_command(closed);
        assert_eq!(
            closed_rx
                .await
                .expect("closed response")
                .expect_err("closed failure")
                .code,
            "connector_supervisor_unavailable"
        );
    }

    #[tokio::test]
    async fn first_enabled_connector_stops_legacy_worker_before_returning() {
        let directory = tempfile::tempdir().expect("tempdir");
        let stop = StopSignal::default();
        let stopped = Arc::new(AtomicBool::new(false));
        let task = {
            let stop = stop.clone();
            let stopped = Arc::clone(&stopped);
            tokio::spawn(async move {
                stop.cancelled().await;
                stopped.store(true, Ordering::SeqCst);
            })
        };
        let mut legacy = Some(LegacyCloudWorker { stop, task });

        // An empty registry keeps the compatible legacy path alive.
        retire_legacy_cloud_worker_if_needed(directory.path(), &mut legacy)
            .await
            .expect("empty registry");
        assert!(legacy.is_some());
        assert!(!stopped.load(Ordering::SeqCst));

        let mut registry =
            connector_runtime::ConnectorRegistry::load(directory.path()).expect("load registry");
        registry
            .add(test_connector_record("ncon_first"))
            .expect("add connector");
        retire_legacy_cloud_worker_if_needed(directory.path(), &mut legacy)
            .await
            .expect("retire legacy worker");
        assert!(legacy.is_none());
        assert!(stopped.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn connector_reload_preserves_successes_and_aggregates_other_failures() {
        let directory = tempfile::tempdir().expect("tempdir");
        let mut registry =
            connector_runtime::ConnectorRegistry::load(directory.path()).expect("load registry");
        registry
            .add(test_connector_record("ncon_bad"))
            .expect("add bad connector");
        registry
            .add(test_connector_record("ncon_good"))
            .expect("add good connector");

        let sync_stop = StopSignal::default();
        let scheduler_stop = StopSignal::default();
        let connection_stop = StopSignal::default();
        let sync = {
            let stop = sync_stop.clone();
            tokio::spawn(async move { stop.cancelled().await })
        };
        let scheduler = {
            let stop = scheduler_stop.clone();
            tokio::spawn(async move { stop.cancelled().await })
        };
        let connection_watch = {
            let stop = connection_stop.clone();
            tokio::spawn(async move { stop.cancelled().await })
        };
        let connections =
            ConnectorConnectionTracker::new(Arc::new(RwLock::new(ConnectionState::LocalOnly)));
        let mut workers = std::collections::BTreeMap::from([(
            "ncon_good".to_owned(),
            ConnectorWorker {
                record: test_connector_record("ncon_good"),
                printer_inventory_dirty: Arc::new(AtomicBool::new(false)),
                wakeup: Arc::new(Notify::new()),
                last_sync_error_code: Arc::new(RwLock::new(None)),
                sync_stop,
                scheduler_stop,
                connection_stop,
                sync,
                scheduler,
                connection_watch,
            },
        )]);
        let executor = SharedRuntimeExecutor {
            runtime: Arc::new(Mutex::new(RuntimeExecutor::Disabled)),
            coordinator: Arc::new(Mutex::new(
                route_coordinator::RouteCoordinator::open(directory.path())
                    .expect("route coordinator"),
            )),
            observation_cache: Arc::new(Mutex::new(RouteObservationCache::default())),
            connector_id: "test".into(),
        };
        let error = reload_connector_workers(
            directory.path(),
            &mut workers,
            &executor,
            &UriFetcher::new(false),
            &PrinterDiscovery::Disabled,
            &Arc::new(SupportPackRegistry::default()),
            &connections,
        )
        .await
        .expect_err("missing bad connector key must fail the aggregate");
        assert!(error.to_string().contains("ncon_bad"));
        assert!(workers.contains_key("ncon_good"));
        stop_connector_worker(&mut workers, "ncon_good", &connections)
            .await
            .expect("stop preserved worker");
    }

    #[tokio::test]
    async fn connector_shutdown_signals_sync_and_scheduler_before_waiting() {
        let sync_stop = StopSignal::default();
        let scheduler_stop = StopSignal::default();
        let connection_stop = StopSignal::default();
        let scheduler_observed = Arc::new(AtomicBool::new(false));
        let sync = {
            let stop = sync_stop.clone();
            tokio::spawn(async move {
                stop.cancelled().await;
                tokio::time::sleep(Duration::from_millis(100)).await;
            })
        };
        let scheduler = {
            let stop = scheduler_stop.clone();
            let observed = Arc::clone(&scheduler_observed);
            tokio::spawn(async move {
                stop.cancelled().await;
                observed.store(true, Ordering::SeqCst);
            })
        };
        let connection_watch = {
            let stop = connection_stop.clone();
            tokio::spawn(async move { stop.cancelled().await })
        };
        let mut workers = std::collections::BTreeMap::from([(
            "ncon_test".to_owned(),
            ConnectorWorker {
                record: test_connector_record("ncon_test"),
                printer_inventory_dirty: Arc::new(AtomicBool::new(false)),
                wakeup: Arc::new(Notify::new()),
                last_sync_error_code: Arc::new(RwLock::new(None)),
                sync_stop,
                scheduler_stop,
                connection_stop,
                sync,
                scheduler,
                connection_watch,
            },
        )]);

        let connections =
            ConnectorConnectionTracker::new(Arc::new(RwLock::new(ConnectionState::LocalOnly)));
        let shutdown = tokio::spawn(async move {
            let _ = stop_connector_worker(&mut workers, "ncon_test", &connections).await;
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(scheduler_observed.load(Ordering::SeqCst));
        shutdown.await.expect("shutdown task");
    }

    #[test]
    fn reprint_identity_is_replay_stable_and_bound_to_original_attempt() {
        let first = reprint_job_identity("job_original", "operator-action-1");
        assert_eq!(
            first,
            reprint_job_identity("job_original", "operator-action-1")
        );
        assert_ne!(
            first,
            reprint_job_identity("job_original", "operator-action-2")
        );
        assert_ne!(
            first,
            reprint_job_identity("job_other", "operator-action-1")
        );
        assert!(first.0.starts_with("job_reprint_"));
    }

    #[test]
    fn connector_printer_grants_are_isolated_and_empty_fails_closed() {
        let first = std::iter::once("prn_first".to_owned()).collect();
        let second = std::iter::once("prn_second".to_owned()).collect();
        let empty = std::collections::BTreeSet::new();
        assert!(printer_is_allowed(Some(&first), "prn_first"));
        assert!(!printer_is_allowed(Some(&first), "prn_second"));
        assert!(printer_is_allowed(Some(&second), "prn_second"));
        assert!(!printer_is_allowed(Some(&second), "prn_first"));
        assert!(!printer_is_allowed(Some(&empty), "prn_first"));
        assert!(printer_is_allowed(None, "prn_legacy"));
    }

    #[test]
    fn all_printer_grant_includes_printers_discovered_later() {
        let mut record = test_connector_record("ncon_all");
        record.printer_grant = PrinterGrant::AllLocalPrinters;
        record.allowed_printer_ids.clear();
        let grant = connector_allowed_printers(&record);
        assert!(printer_is_allowed(grant.as_ref(), "prn_present"));
        assert!(printer_is_allowed(grant.as_ref(), "prn_added_later"));
    }

    #[test]
    fn connector_connection_aggregate_preserves_useful_health() {
        assert_eq!(
            aggregate_connector_connection(std::iter::empty()),
            ConnectionState::LocalOnly
        );
        assert_eq!(
            aggregate_connector_connection(
                [ConnectionState::Offline, ConnectionState::Connected].into_iter()
            ),
            ConnectionState::Connected
        );
        assert_eq!(
            aggregate_connector_connection(
                [ConnectionState::Unauthorized, ConnectionState::Offline].into_iter()
            ),
            ConnectionState::Degraded
        );
        assert_eq!(
            aggregate_connector_connection(std::iter::once(ConnectionState::Unauthorized)),
            ConnectionState::Unauthorized
        );
    }

    #[test]
    fn connector_reload_uses_the_installed_loopback_bind() {
        let directory = tempfile::tempdir().expect("tempdir");
        write_new_json(
            &directory.path().join("agent-config.json"),
            &serde_json::json!({ "local_bind": "127.0.0.1:49231" }),
        )
        .expect("write config");
        let arguments = Arguments::try_parse_from([
            "piqae-agent",
            "--data-dir",
            directory.path().to_str().expect("utf8 path"),
            "--local-bind",
            "127.0.0.1:39100",
        ])
        .expect("arguments");
        assert_eq!(
            installed_local_bind(&arguments).expect("installed bind"),
            "127.0.0.1:49231".parse().expect("socket address")
        );

        replace_json(
            &directory.path().join("agent-config.json"),
            &serde_json::json!({ "local_bind": "0.0.0.0:49231" }),
        )
        .expect("replace config");
        assert!(installed_local_bind(&arguments).is_err());
    }

    #[test]
    fn enrolment_writes_a_new_key_without_overwriting_identity() {
        let directory = tempfile::tempdir().expect("tempdir");
        let key_path = directory.path().join("device.key");
        let secret = [0x5a; 32];
        write_new_device_key(&key_path, &secret).expect("write key");
        assert_eq!(
            std::fs::read_to_string(&key_path).expect("read key"),
            hex::encode(secret)
        );
        assert!(write_new_device_key(&key_path, &[0x11; 32]).is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                std::fs::metadata(&key_path)
                    .expect("metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn rotation_replaces_the_key_in_place_and_keeps_it_owner_only() {
        let directory = tempfile::tempdir().expect("tempdir");
        let key_path = directory.path().join("device.key");
        write_new_device_key(&key_path, &[0x5a; 32]).expect("write key");
        replace_device_key(&key_path, &[0x11; 32]).expect("rotate key");
        assert_eq!(
            std::fs::read_to_string(&key_path).expect("read key"),
            hex::encode([0x11; 32])
        );
        // The staging file must never survive a successful rotation: it holds
        // a usable device key.
        assert!(!key_path.with_extension("key.rotating").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                std::fs::metadata(&key_path)
                    .expect("metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn rotation_reuses_the_recorded_installation_so_the_node_survives() {
        let directory = tempfile::tempdir().expect("tempdir");
        let config_path = directory.path().join("agent-config.json");
        write_new_json(
            &config_path,
            &serde_json::json!({
                "agent_id": "agt_01J0",
                "installation_id": "018f-abcd",
            }),
        )
        .expect("write config");
        let existing = existing_installation(&config_path).expect("read installation");
        assert_eq!(existing.agent_id, "agt_01J0");
        assert_eq!(existing.installation_id, "018f-abcd");
    }

    #[test]
    fn rotation_refuses_a_configuration_without_an_installation_id() {
        // Rotating without one would pair a second node and strand the first.
        let directory = tempfile::tempdir().expect("tempdir");
        let config_path = directory.path().join("agent-config.json");
        write_new_json(&config_path, &serde_json::json!({ "agent_id": "agt_01J0" }))
            .expect("write config");
        let error = existing_installation(&config_path).expect_err("must refuse");
        assert!(
            format!("{error}").contains("installation ID"),
            "unhelpful error: {error}"
        );
        assert!(existing_installation(&directory.path().join("missing.json")).is_err());
    }

    #[test]
    fn configuration_replacement_survives_a_failed_write() {
        let directory = tempfile::tempdir().expect("tempdir");
        let config_path = directory.path().join("agent-config.json");
        write_new_json(&config_path, &serde_json::json!({ "agent_id": "old" })).expect("write");
        replace_json(&config_path, &serde_json::json!({ "agent_id": "new" })).expect("replace");
        let body = std::fs::read_to_string(&config_path).expect("read");
        assert!(body.contains("new"), "{body}");
        assert!(!config_path.with_extension("json.replacing").exists());
    }

    #[test]
    fn a_revoked_node_is_reported_differently_from_an_unreachable_one() {
        use piqae_agent_client::ClientError;
        let invalid_signature = ClientError::Unauthorized {
            code: "invalid_agent_signature".into(),
        };
        assert_eq!(
            redacted_sync_error_code(&invalid_signature).as_deref(),
            Some("invalid_agent_signature")
        );
        assert_eq!(
            failure_state(&invalid_signature),
            ConnectionState::Unauthorized
        );
        assert_eq!(
            failure_state(&ClientError::Unauthorized {
                code: "unknown_agent".into()
            }),
            ConnectionState::Unauthorized
        );
        // Clock skew is self-correcting, so it must not be reported as a
        // revoked identity that an operator has to act on.
        assert_eq!(
            failure_state(&ClientError::Unauthorized {
                code: "stale_agent_request".into()
            }),
            ConnectionState::Degraded
        );
        assert_eq!(
            failure_state(&ClientError::Status {
                status: 503,
                body: String::new()
            }),
            ConnectionState::Offline
        );
        assert_eq!(
            redacted_sync_error_code(&ClientError::Status {
                status: 503,
                body: "must never reach diagnostics".into(),
            }),
            None
        );
    }

    #[test]
    fn rotation_and_first_pairing_are_mutually_exclusive() {
        assert!(
            Arguments::try_parse_from([
                "piqae-agent",
                "--control-plane-url",
                "http://127.0.0.1:8080",
                "--pair",
                "--rotate-key",
            ])
            .is_err()
        );
        let rotate = Arguments::try_parse_from([
            "piqae-agent",
            "--control-plane-url",
            "http://127.0.0.1:8080",
            "--rotate-key",
        ])
        .expect("rotate arguments");
        assert!(rotate.rotate_key);
        assert!(!rotate.pair);
    }

    #[test]
    fn stdin_enrolment_is_bounded_trimmed_and_not_an_argv_secret() {
        assert_eq!(
            read_enrolment_token(&b"  piq_enr_secret\n"[..]).expect("token"),
            "piq_enr_secret"
        );
        assert!(read_enrolment_token(&b" \n"[..]).is_err());
        assert!(read_enrolment_token(vec![b'x'; 257].as_slice()).is_err());

        let arguments = Arguments::try_parse_from([
            "piqae-agent",
            "--enrolment-token-stdin",
            "--control-plane-url",
            "https://api.piqae.com",
        ])
        .expect("stdin enrolment arguments");
        assert!(arguments.enrolment_token_stdin);
        assert!(arguments.enrolment_token.is_none());
        assert!(
            Arguments::try_parse_from([
                "piqae-agent",
                "--enrolment-token-stdin",
                "--enrolment-token",
                "piq_enr_secret",
            ])
            .is_err()
        );
    }

    #[test]
    fn connector_consent_is_bounded_explicit_and_never_an_argv_secret() {
        let input =
            br#"{"token":"piq_enr_0123456789abcdef0123456789abcdef","printer_ids":["prn_1"]}"#;
        let consent = read_connector_consent(input.as_slice()).expect("consent");
        assert_eq!(consent.printer_ids, ["prn_1"]);
        let test_token = format!("piq_enr_{}", "0123456789abcdef".repeat(2));
        let all_input = serde_json::json!({
            "token": test_token,
            "printer_grant": "all_local_printers",
            "printer_ids": [],
        });
        let all =
            read_connector_consent(all_input.to_string().as_bytes()).expect("all-printer consent");
        assert_eq!(all.printer_grant, PrinterGrant::AllLocalPrinters);
        assert!(
            read_connector_consent(
                br#"{"token":"piq_enr_0123456789abcdef0123456789abcdef","printer_ids":[]}"#
                    .as_slice()
            )
            .is_err()
        );
        assert!(read_connector_consent(
            br#"{"token":"piq_enr_0123456789abcdef0123456789abcdef","printer_ids":["prn_1","prn_1"]}"#.as_slice()
        ).is_err());
        let invalid_all_input = serde_json::json!({
            "token": test_token,
            "printer_grant": "all_local_printers",
            "printer_ids": ["prn_1"],
        });
        assert!(read_connector_consent(invalid_all_input.to_string().as_bytes()).is_err());
        let arguments = Arguments::try_parse_from(["piqae-agent", "--add-connector-json-stdin"])
            .expect("connector stdin arguments");
        assert!(arguments.add_connector_json_stdin);
        assert!(
            Arguments::try_parse_from([
                "piqae-agent",
                "--add-connector-json-stdin",
                "--preview-connect-token-stdin"
            ])
            .is_err()
        );
    }

    #[test]
    fn local_first_connection_reuses_the_durable_installation_identity() {
        let directory = tempfile::tempdir().expect("tempdir");
        std::fs::write(directory.path().join("agent-id"), "agt_local_identity\n")
            .expect("agent id");
        std::fs::write(directory.path().join("device.key"), "00".repeat(32)).expect("device key");
        let arguments = Arguments::try_parse_from([
            "piqae-agent",
            "--data-dir",
            directory.path().to_str().expect("path"),
            "--control-plane-url",
            "https://self-hosted.example/api",
        ])
        .expect("arguments");
        let (origin, installation, key) = installed_control_plane(&arguments).expect("identity");
        assert_eq!(origin.as_str(), "https://self-hosted.example/api");
        assert_eq!(installation.installation_id, "agt_local_identity");
        assert_eq!(key, directory.path().join("device.key"));
    }

    #[test]
    fn enrolment_arguments_keep_the_one_time_token_out_of_runtime_defaults() {
        let arguments = Arguments::try_parse_from([
            "piqae-agent",
            "--control-plane-url",
            "http://127.0.0.1:8080",
            "--enrolment-token",
            "piq_enr_secret",
        ])
        .expect("arguments");
        assert_eq!(arguments.enrolment_token.as_deref(), Some("piq_enr_secret"));
        assert!(arguments.agent_id.is_none());
        assert!(arguments.device_key_file.is_none());
    }

    #[tokio::test]
    async fn delayed_content_stream_renews_until_materialized() {
        let directory = tempfile::tempdir().expect("tempdir");
        let store = ContentStore::open(directory.path()).await.expect("store");
        let (mut writer, reader) = tokio::io::duplex(64);
        let source = tokio::spawn(async move {
            writer.write_all(b"first").await.expect("first");
            tokio::time::sleep(Duration::from_millis(11)).await;
            writer.write_all(b"-second").await.expect("second");
            tokio::time::sleep(Duration::from_millis(11)).await;
            writer.write_all(b"-third").await.expect("third");
        });
        let renewals = Arc::new(AtomicUsize::new(0));
        let renewal_counter = Arc::clone(&renewals);
        let content = maintain_lease(
            Utc::now() + chrono::Duration::seconds(30),
            Duration::from_millis(10),
            async { Ok(store.put(reader).await?) },
            move || {
                let counter = Arc::clone(&renewal_counter);
                async move {
                    counter.fetch_add(1, Ordering::Relaxed);
                    Ok(Utc::now() + chrono::Duration::seconds(30))
                }
            },
        )
        .await
        .expect("materialize");
        source.await.expect("source");
        assert_eq!(
            tokio::fs::read(content.path).await.expect("content"),
            b"first-second-third"
        );
        assert!(
            renewals.load(Ordering::Relaxed) >= 2,
            "the delayed stream must cross at least two renewal intervals"
        );
    }

    #[test]
    fn near_expiry_lease_renewal_never_busy_loops() {
        let maximum = Duration::from_millis(10);
        let delay = lease_renewal_delay(Utc::now() - chrono::Duration::seconds(1), maximum);
        assert!(delay >= Duration::from_millis(1));
        assert!(delay <= maximum);
    }

    #[test]
    fn sync_uses_the_persisted_monotonic_printer_revision() {
        let mut store = AgentStore::in_memory().expect("store");
        store
            .set_setting("printer_inventory_revision", "42")
            .expect("revision");
        store
            .record_executor_failure("executor_crashed")
            .expect("crash health");
        store
            .record_executor_failure("executor_timed_out")
            .expect("timeout health");
        let request = sync_request(&store, AgentId::new(), Utc::now(), false, Some(Vec::new()))
            .expect("sync request");
        assert_eq!(request.printer_revision, 42);
        assert_eq!(request.health.executor_crashes, 1);
        assert!(
            request.document_render.cached_resource_digests.is_empty(),
            "connector sync must never expose process-global cache membership"
        );
        assert_eq!(
            request.health.last_error_code.as_deref(),
            Some("executor_timed_out")
        );
    }

    #[tokio::test]
    async fn failed_renewal_prevents_acceptance_and_restart_accepts_once() {
        let directory = tempfile::tempdir().expect("tempdir");
        let database = directory.path().join("agent.sqlite");
        let job = cloud_job();
        let mut store = AgentStore::open(&database).expect("store");
        let error = maintain_lease(
            Utc::now() + chrono::Duration::seconds(30),
            Duration::from_millis(10),
            async {
                tokio::time::sleep(Duration::from_millis(20)).await;
                store.prepare_cloud_job(
                    &job,
                    &uuid::Uuid::new_v4().to_string(),
                    "first-lease",
                    Utc::now().timestamp_millis() + 30_000,
                )?;
                Ok(())
            },
            || async { Err(anyhow::anyhow!("renewal unavailable")) },
        )
        .await
        .expect_err("renewal must fail");
        assert_eq!(error.to_string(), "renewal unavailable");
        assert!(store.get_job(&job.job_id).expect("query").is_none());
        drop(store);

        let mut restarted = AgentStore::open(&database).expect("restart");
        let renewals = Arc::new(AtomicUsize::new(0));
        let renewal_counter = Arc::clone(&renewals);
        maintain_lease(
            Utc::now() + chrono::Duration::seconds(30),
            Duration::from_millis(10),
            async {
                tokio::time::sleep(Duration::from_millis(15)).await;
                restarted.prepare_cloud_job(
                    &job,
                    &uuid::Uuid::new_v4().to_string(),
                    "restart-lease",
                    Utc::now().timestamp_millis() + 30_000,
                )?;
                Ok(())
            },
            move || {
                let counter = Arc::clone(&renewal_counter);
                async move {
                    counter.fetch_add(1, Ordering::Relaxed);
                    Ok(Utc::now() + chrono::Duration::seconds(30))
                }
            },
        )
        .await
        .expect("accept after restart");
        assert!(restarted.runnable_heads(10).expect("runnable").is_empty());
        restarted
            .activate_cloud_job(&job.job_id, 10)
            .expect("remote accepted");
        restarted
            .activate_cloud_job(&job.job_id, 11)
            .expect("duplicate response");
        assert!(
            renewals.load(Ordering::Relaxed) >= 1,
            "the restarted materialization must renew its lease before acceptance"
        );
        assert_eq!(restarted.pending_events(0, 10).expect("events").len(), 1);
    }

    #[tokio::test]
    async fn ambiguous_server_accept_retries_exact_intent_before_activation() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let address = listener.local_addr().expect("address");
        let server = tokio::spawn(async move {
            let mut bodies = Vec::new();
            for attempt in 0..2 {
                let (mut stream, _) = listener.accept().await.expect("accept");
                bodies.push(read_request_body(&mut stream).await);
                if attempt == 0 {
                    continue;
                }
                let body = br#"{"accepted_at":"2026-01-01T00:00:00Z","state":"agent_accepted"}"#;
                stream
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await
                    .expect("headers");
                stream.write_all(body).await.expect("body");
            }
            bodies
        });

        let agent_id = AgentId::new();
        let test_encryption_key = SecretKey::random(&mut rand::rngs::OsRng);
        let cloud = CloudConfiguration {
            client: AgentClient::new(Url::parse(&format!("http://{address}/")).expect("base URL"))
                .expect("client"),
            identity: DeviceIdentity::generate(agent_id),
            agent_id,
            content_encryption_keys: Arc::new(content_key_store::ContentKeyring::from_active(
                "cek_test".into(),
                test_encryption_key,
            )),
            allowed_printer_ids: None,
            connector_id: "test".into(),
        };
        let mut store = AgentStore::in_memory().expect("store");
        let job = cloud_job();
        let lease_id = uuid::Uuid::new_v4();
        let local = store
            .prepare_cloud_job(&job, &lease_id.to_string(), "retry-secret", i64::MAX)
            .expect("prepare");
        let intent = CloudAcceptIntent {
            job_id: job.job_id.clone(),
            lease_id: lease_id.to_string(),
            lease_token: "retry-secret".into(),
            lease_expires_unix_ms: i64::MAX,
            content_sha256: job.content_sha256.clone(),
            local_sequence: u64::try_from(local.printer_sequence).expect("sequence"),
        };
        let coordinator_dir = tempfile::tempdir().expect("coordinator tempdir");
        let route_coordinator = Arc::new(Mutex::new(
            route_coordinator::RouteCoordinator::open(coordinator_dir.path())
                .expect("route coordinator"),
        ));
        confirm_cloud_accept(&cloud, &mut store, &route_coordinator, &intent)
            .await
            .expect_err("first response is ambiguous");
        assert!(store.runnable_heads(10).expect("runnable").is_empty());
        assert_eq!(store.pending_cloud_accepts().expect("intents").len(), 1);

        resume_pending_cloud_accepts(&cloud, &mut store, &route_coordinator).await;
        let bodies = server.await.expect("server");
        assert_eq!(bodies.len(), 2);
        assert_eq!(bodies[0], bodies[1]);
        assert!(bodies[0].contains("\"lease_token\":\"retry-secret\""));
        assert_eq!(store.runnable_heads(10).expect("runnable").len(), 1);
        assert_eq!(store.pending_events(0, 10).expect("events").len(), 1);
        assert!(store.pending_cloud_accepts().expect("intents").is_empty());
    }

    async fn read_request_body(stream: &mut tokio::net::TcpStream) -> String {
        let mut request = Vec::new();
        let body_start;
        let content_length;
        loop {
            let mut chunk = [0_u8; 1024];
            let count = stream.read(&mut chunk).await.expect("request");
            assert!(count > 0);
            request.extend_from_slice(&chunk[..count]);
            if let Some(index) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                body_start = index + 4;
                let headers = String::from_utf8_lossy(&request[..index]);
                content_length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|value| value.trim().parse::<usize>().ok())
                    })
                    .expect("content length");
                break;
            }
        }
        while request.len() < body_start + content_length {
            let mut chunk = [0_u8; 1024];
            let count = stream.read(&mut chunk).await.expect("body");
            assert!(count > 0);
            request.extend_from_slice(&chunk[..count]);
        }
        String::from_utf8(request[body_start..body_start + content_length].to_vec())
            .expect("JSON body")
    }

    fn cloud_job() -> AcceptedJob {
        AcceptedJob {
            job_id: JobId::new().to_string(),
            submission_id: "sub_restart".into(),
            printer_id: "printer".into(),
            printer_native_id: "native".into(),
            title: "Restart-safe receipt".into(),
            content_sha256: "abc".into(),
            content_path: "/content/abc".into(),
            content_kind: "pdf".into(),
            options_json: "{}".into(),
            expires_unix_ms: None,
            accepted_unix_ms: 1,
            cloud_managed: true,
        }
    }

    #[test]
    fn local_driver_test_can_resolve_present_unexposed_printer_but_jobs_cannot() {
        let mut store = AgentStore::in_memory().expect("store");
        let printer = store
            .upsert_printer(
                "native-local-test",
                "Office",
                "online",
                true,
                &serde_json::to_string(&piqae_domain::PrinterCapabilities::default())
                    .expect("capabilities"),
                10,
            )
            .expect("printer");

        assert!(resolve_present_printer(&store, &printer.printer_id).is_ok());
        assert_eq!(
            require_local_driver_test_confirmation(false)
                .expect_err("local test requires confirmation")
                .code,
            "local_test_not_confirmed"
        );
        assert!(require_local_driver_test_confirmation(true).is_ok());
        let blocked = resolve_exposed_printer(&store, &printer.printer_id)
            .expect_err("normal submission must require exposure");
        assert_eq!(blocked.code, "printer_not_exposed");

        store
            .set_printer_exposed(&printer.printer_id, true, 20)
            .expect("expose");
        assert!(resolve_exposed_printer(&store, &printer.printer_id).is_ok());
    }

    #[tokio::test]
    async fn discovered_printer_and_live_default_are_cloud_available_without_global_exposure() {
        let mut store = AgentStore::in_memory().expect("store");
        let coordinator_dir = tempfile::tempdir().expect("coordinator dir");
        let coordinator = Arc::new(Mutex::new(
            route_coordinator::RouteCoordinator::open(coordinator_dir.path())
                .expect("route coordinator"),
        ));
        let first = discover_cloud_printers(
            &mut store,
            &PrinterDiscovery::Fake,
            &coordinator,
            &SupportPackRegistry::default(),
            1,
        )
        .await
        .expect("discovery");
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].profiles.len(), 1);
        assert_eq!(first[0].profiles[0].name, "Current printer defaults");
        assert!(first[0].profiles[0].is_default);
        assert!(first[0].profiles[0].published);
        let printer = store
            .present_printers()
            .expect("printers")
            .into_iter()
            .next()
            .expect("fake printer");
        let profile = store
            .named_profiles(&printer.printer_id)
            .expect("profiles")
            .into_iter()
            .next()
            .expect("default profile");
        assert!(profile.uses_current_printer_defaults);
        assert!(!profile.published);

        let restarted = discover_cloud_printers(
            &mut store,
            &PrinterDiscovery::Fake,
            &coordinator,
            &SupportPackRegistry::default(),
            2,
        )
        .await
        .expect("restart discovery");
        assert_eq!(restarted[0].id, first[0].id);
        assert_eq!(
            restarted[0].profiles[0].profile_id,
            first[0].profiles[0].profile_id
        );
        assert_eq!(
            store
                .named_profiles(&printer.printer_id)
                .expect("profiles")
                .len(),
            1,
            "refresh must not duplicate the generated preset"
        );
    }

    #[tokio::test]
    async fn connector_sync_uses_shared_approved_printer_identity() {
        let mut node_inventory = AgentStore::in_memory().expect("node inventory");
        let coordinator_dir = tempfile::tempdir().expect("coordinator dir");
        let coordinator = Arc::new(Mutex::new(
            route_coordinator::RouteCoordinator::open(coordinator_dir.path())
                .expect("route coordinator"),
        ));
        let observation_cache = Arc::new(Mutex::new(RouteObservationCache::default()));
        // Initial discovery creates the node-owned stable printer identity.
        let initial = discover_cloud_printers(
            &mut node_inventory,
            &PrinterDiscovery::Fake,
            &coordinator,
            &SupportPackRegistry::default(),
            1,
        )
        .await
        .expect("initial discovery");
        assert_eq!(initial.len(), 1);
        let printer = node_inventory
            .present_printers()
            .expect("node printers")
            .into_iter()
            .next()
            .expect("fake printer");
        // Each connector retains an isolated queue database. It must not
        // create a second, unapproved logical printer identity in that store.
        let mut connector_queue = AgentStore::in_memory().expect("connector queue");
        let request = prepare_sync_request(
            &mut connector_queue,
            &mut node_inventory,
            &PrinterDiscovery::Fake,
            &coordinator,
            &observation_cache,
            &SupportPackRegistry::default(),
            "connector-a",
            AgentId::new(),
            Utc::now(),
            false,
            true,
            None,
        )
        .await
        .expect("connector sync");

        let snapshots = request.printers.expect("printer inventory");
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].id.to_string(), printer.printer_id);
        assert_eq!(snapshots[0].profiles.len(), 1);
        assert!(
            connector_queue
                .present_printers()
                .expect("connector printers")
                .is_empty()
        );
        assert!(
            connector_queue
                .printer(&printer.printer_id)
                .expect("connector lookup")
                .is_none(),
            "connector queue must not duplicate node printer identity"
        );
        assert_eq!(
            resolve_cloud_offer_printer(&node_inventory, &printer.printer_id)
                .expect("offer resolves against node inventory")
                .native_id,
            printer.native_id
        );
        assert_eq!(request.printer_revision, 1);

        let mut selected_queue = AgentStore::in_memory().expect("selected connector queue");
        let selected = std::iter::once("prn_some_other_printer".to_owned()).collect();
        let request = prepare_sync_request(
            &mut selected_queue,
            &mut node_inventory,
            &PrinterDiscovery::Fake,
            &coordinator,
            &observation_cache,
            &SupportPackRegistry::default(),
            "connector-selected",
            AgentId::new(),
            Utc::now(),
            false,
            true,
            Some(&selected),
        )
        .await
        .expect("selected connector sync");
        assert!(request.printers.expect("printer inventory").is_empty());
    }

    #[tokio::test]
    async fn multiple_connectors_publish_existing_inventory_after_node_restart() {
        let directory = tempfile::tempdir().expect("tempdir");
        let inventory_path = directory.path().join("agent.sqlite3");
        let coordinator = Arc::new(Mutex::new(
            route_coordinator::RouteCoordinator::open(directory.path().join("routes"))
                .expect("route coordinator"),
        ));
        let observation_cache = Arc::new(Mutex::new(RouteObservationCache::default()));
        let expected_printer_id = {
            let mut node_inventory = AgentStore::open(&inventory_path).expect("node inventory");
            discover_cloud_printers(
                &mut node_inventory,
                &PrinterDiscovery::Fake,
                &coordinator,
                &SupportPackRegistry::default(),
                1,
            )
            .await
            .expect("initial discovery")[0]
                .id
        };

        // Reopening the node catalogue models an installed node restart. Each
        // connector keeps its own queue and revision while sharing the stable
        // physical printer identity.
        let mut restarted_inventory = AgentStore::open(&inventory_path).expect("restart inventory");
        for connector_number in 1..=2 {
            let mut connector_queue = AgentStore::open(
                directory
                    .path()
                    .join(format!("connector-{connector_number}.sqlite3")),
            )
            .expect("connector queue");
            let request = prepare_sync_request(
                &mut connector_queue,
                &mut restarted_inventory,
                &PrinterDiscovery::Fake,
                &coordinator,
                &observation_cache,
                &SupportPackRegistry::default(),
                &format!("connector-{connector_number}"),
                AgentId::new(),
                Utc::now(),
                false,
                true,
                None,
            )
            .await
            .expect("connector sync");
            let printers = request.printers.expect("printer inventory");
            assert_eq!(printers.len(), 1);
            assert_eq!(printers[0].id, expected_printer_id);
            assert_eq!(request.printer_revision, 1);
        }
    }

    #[test]
    fn diagnostic_pdf_is_a4_and_has_a_valid_cross_reference() {
        let pdf = a4_test_pdf();
        assert!(pdf.starts_with(b"%PDF-1.4"));
        let media_box = b"/MediaBox [0 0 595 842]";
        assert!(
            pdf.windows(media_box.len())
                .any(|window| window == media_box)
        );
        assert!(pdf.ends_with(b"%%EOF\n"));
        let text = String::from_utf8_lossy(&pdf);
        let xref = text
            .lines()
            .rev()
            .nth(1)
            .expect("startxref offset")
            .parse::<usize>()
            .expect("numeric offset");
        assert_eq!(&pdf[xref..xref + 4], b"xref");
    }

    #[test]
    fn remote_diagnostics_are_bounded_redacted_durable_and_acknowledged() {
        let directory = tempfile::tempdir().expect("tempdir");
        let database = directory.path().join("diagnostics.sqlite");
        {
            let mut store = AgentStore::open(&database).expect("store");
            for index in 0..10 {
                collect_diagnostics(&mut store, &format!("diag_{index}"))
                    .expect("collect diagnostic");
            }
            let reports = pending_diagnostics(&store).expect("pending diagnostics");
            assert_eq!(reports.len(), MAX_PENDING_DIAGNOSTICS);
            assert_eq!(reports[0].request_id, "diag_2");
            let encoded = serde_json::to_string(&reports).expect("encoded reports");
            for forbidden in [
                "local.token",
                "content_path",
                "lease_token",
                "signed_url",
                "native_blob",
            ] {
                assert!(!encoded.contains(forbidden));
            }
        }
        let mut restarted = AgentStore::open(&database).expect("restart store");
        assert_eq!(
            pending_diagnostics(&restarted)
                .expect("restart pending")
                .len(),
            8
        );
        acknowledge_diagnostics(&mut restarted, &["diag_2".into()]);
        assert_eq!(
            pending_diagnostics(&restarted).expect("ack pending").len(),
            7
        );
    }

    #[test]
    fn profile_pin_metadata_requires_an_exact_revision() {
        let mut metadata = std::collections::BTreeMap::new();
        metadata.insert("piqae.profile_id".into(), "prf_shipping".into());
        assert!(profile_pin_metadata(&metadata).is_err());

        metadata.insert("piqae.profile_revision".into(), "7".into());
        metadata.insert("piqae.stock_id".into(), "stk_a4".into());
        let pin = profile_pin_metadata(&metadata)
            .expect("valid metadata")
            .expect("profile pin");
        assert_eq!(pin.profile_id, "prf_shipping");
        assert_eq!(pin.profile_revision, 7);
        assert_eq!(pin.stock_id.as_deref(), Some("stk_a4"));
    }

    #[test]
    fn profile_pin_metadata_rejects_invalid_media_snapshot() {
        let metadata = std::collections::BTreeMap::from([
            ("piqae.profile_id".into(), "prf_shipping".into()),
            ("piqae.profile_revision".into(), "7".into()),
            ("piqae.loaded_media_snapshot".into(), "{broken".into()),
        ]);
        assert!(profile_pin_metadata(&metadata).is_err());
    }

    #[test]
    fn profile_pin_metadata_accepts_jobs_queued_before_the_piqae_rename() {
        let metadata = std::collections::BTreeMap::from([
            ("spool.target_id".into(), "tgt_legacy".into()),
            ("spool.binding_id".into(), "tgb_legacy".into()),
            ("spool.profile_id".into(), "prf_legacy".into()),
            ("spool.profile_revision".into(), "3".into()),
            ("spool.stock_id".into(), "stk_legacy".into()),
        ]);
        let pin = profile_pin_metadata(&metadata)
            .expect("legacy metadata is valid")
            .expect("legacy profile pin");
        assert_eq!(pin.target_id.as_deref(), Some("tgt_legacy"));
        assert_eq!(pin.binding_id.as_deref(), Some("tgb_legacy"));
        assert_eq!(pin.profile_id, "prf_legacy");
        assert_eq!(pin.profile_revision, 3);
        assert_eq!(pin.stock_id.as_deref(), Some("stk_legacy"));
    }

    #[test]
    fn encrypted_content_requires_exact_authenticated_binding() {
        use aes_gcm::aead::Aead as _;
        let private = SecretKey::random(&mut rand::rngs::OsRng);
        let content_key = [9_u8; 32];
        let iv = [4_u8; 12];
        let binding = piqae_domain::EncryptedContentBinding {
            envelope_id: "env_012345678901234567890123".into(),
            workspace_id: "wsp_test".into(),
            environment_id: "env_test".into(),
            content_type: ContentKind::Pdf,
            printer_id: "prt_test".into(),
            target_id: "tgt_test".into(),
            profile_revision: "prf_test:3".into(),
            options: piqae_domain::JobOptions::default(),
            deliveries: 1,
            expires_at: "2099-01-01T00:00:00Z".into(),
            raw_authorized: false,
        };
        let aad = serde_json::to_vec(&binding).expect("aad");
        assert_eq!(
            String::from_utf8(aad.clone()).expect("utf8"),
            "{\"envelope_id\":\"env_012345678901234567890123\",\"workspace_id\":\"wsp_test\",\"environment_id\":\"env_test\",\"content_type\":\"pdf\",\"printer_id\":\"prt_test\",\"target_id\":\"tgt_test\",\"profile_revision\":\"prf_test:3\",\"options\":{\"bin\":null,\"collate\":null,\"color\":null,\"copies\":null,\"dpi\":null,\"duplex\":null,\"fit_to_page\":null,\"media\":null,\"nup\":null,\"pages\":null,\"paper\":null,\"rotate\":null,\"native_options\":{}},\"deliveries\":1,\"expires_at\":\"2099-01-01T00:00:00Z\",\"raw_authorized\":false}"
        );
        let cipher = Aes256Gcm::new_from_slice(&content_key).expect("aes key");
        let ciphertext = cipher
            .encrypt(
                (&iv).into(),
                Payload {
                    msg: b"%PDF-test",
                    aad: &aad,
                },
            )
            .expect("encrypt");
        let ephemeral = SecretKey::random(&mut rand::rngs::OsRng);
        let ephemeral_public = ephemeral.public_key();
        let salt = [7_u8; 32];
        let wrap_iv = [8_u8; 12];
        let key_id = "cek_test";
        let shared = diffie_hellman(
            ephemeral.to_nonzero_scalar(),
            private.public_key().as_affine(),
        );
        let info = format!(
            "piqae-content-key-wrap-v3\0{}\0{key_id}",
            binding.envelope_id
        );
        let hkdf = Hkdf::<Sha256>::new(Some(&salt), shared.raw_secret_bytes().as_slice());
        let mut wrapping_key = [0_u8; 32];
        hkdf.expand(info.as_bytes(), &mut wrapping_key)
            .expect("derive wrapping key");
        let wrapping_cipher = Aes256Gcm::new_from_slice(&wrapping_key).expect("wrapping cipher");
        let wrapped = wrapping_cipher
            .encrypt(
                (&wrap_iv).into(),
                Payload {
                    msg: &content_key,
                    aad: &aad,
                },
            )
            .expect("wrap content key");
        let recipient = piqae_domain::EncryptedContentRecipient {
            key_id: key_id.into(),
            algorithm: piqae_domain::ENCRYPTED_JOB_V3_RECIPIENT_ALGORITHM.into(),
            ephemeral_public_key: URL_SAFE_NO_PAD.encode(ephemeral_public.to_sec1_bytes().as_ref()),
            hkdf_salt: URL_SAFE_NO_PAD.encode(salt),
            key_wrap_iv: URL_SAFE_NO_PAD.encode(wrap_iv),
            encrypted_content_key: URL_SAFE_NO_PAD.encode(wrapped),
        };
        let manifest = piqae_domain::EncryptedContentManifest {
            version: piqae_domain::ENCRYPTED_JOB_V3_VERSION.into(),
            suite: piqae_domain::ENCRYPTED_JOB_V3_SUITE.into(),
            binding,
            ciphertext_sha256: URL_SAFE_NO_PAD.encode(Sha256::digest(&ciphertext)),
            iv: URL_SAFE_NO_PAD.encode(iv),
            recipients: vec![recipient.clone()],
        };
        assert_eq!(
            decrypt_encrypted_content(&private, &recipient, &manifest, &ciphertext)
                .expect("decrypt"),
            b"%PDF-test"
        );
        let mut tampered = manifest.clone();
        tampered.binding.profile_revision = "prf_test:4".into();
        assert!(decrypt_encrypted_content(&private, &recipient, &tampered, &ciphertext).is_err());

        let mut legacy = tampered;
        legacy.binding.profile_revision = "prf_test:3".into();
        legacy.version = "piqae-encrypted-job-v2".into();
        legacy.suite = "RSA-OAEP-256+A256GCM".into();
        assert!(decrypt_encrypted_content(&private, &recipient, &legacy, &ciphertext).is_err());

        let mut wrong_salt = recipient;
        wrong_salt.hkdf_salt = URL_SAFE_NO_PAD.encode([6_u8; 32]);
        assert!(decrypt_encrypted_content(&private, &wrong_salt, &manifest, &ciphertext).is_err());
    }

    #[test]
    fn encrypted_ciphertext_is_bounded_before_decryption() {
        let maximum = usize::try_from(MAX_CIPHERTEXT_BYTES).expect("ciphertext limit fits usize");
        assert!(validate_encrypted_ciphertext_size(maximum).is_ok());
        assert!(validate_encrypted_ciphertext_size(maximum + 1).is_err());
    }
}
