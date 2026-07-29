use anyhow::{Context, Result};
use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use chrono::Utc;
use clap::{Parser, ValueEnum};
use futures::TryStreamExt;
use spool_agent_client::{AgentClient, ClientError, DeviceIdentity};
use spool_agent_core::{
    AgentEngine, ContentStore, Executor, ExecutorFailure, FakeExecutor, LocalSubmission,
    NativeAcceptance, SystemClock,
};
use spool_agent_storage::{AcceptedJob, AgentStore, PendingEvent, QueueCounts, StorageError};
use spool_domain::{
    AgentId, ContentKind, EventId, JobEvent, JobFailureReason, JobId, JobState, UriAuthentication,
};
use spool_executor_supervisor::{ExecutorSupervisor, SupervisedExecutor};
use spool_local_api::{
    ControlFailure, ControlRequest, LocalApiState, LocalContent, LocalCreateJob, LocalJobAccepted,
};
use spool_local_ipc::{ConnectionState, LocalPrinter, LocalStatus, SessionAuthenticator};
use spool_protocol::{
    CURRENT_PROTOCOL_VERSION,
    agent::{
        AgentAcceptJobRequest, AgentCommand, AgentHealth, AgentReleaseLeaseRequest,
        AgentSyncRequest, AgentSyncResponse, ContentDescriptor, JobOffer, QueueSnapshot,
    },
    executor::{DiscoveredPrinter, ExecutorOperation, ExecutorResult},
};
use std::{
    io::Write as _,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::{RwLock, mpsc};
use tokio_util::io::StreamReader;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;
use url::Url;

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
#[command(version, about = "Spool headless print agent")]
struct Arguments {
    /// Runtime mode. Hosted modes require enrolment before cloud sync begins.
    #[arg(long, env = "SPOOL_AGENT_MODE", default_value = "local")]
    mode: AgentMode,

    /// Durable application-data directory.
    #[arg(long, env = "SPOOL_DATA_DIR", default_value = ".spool")]
    data_dir: PathBuf,

    /// Loopback address for the local operational API.
    #[arg(long, env = "SPOOL_LOCAL_BIND", default_value = "127.0.0.1:39100")]
    local_bind: SocketAddr,

    /// Hosted or self-hosted Rust control-plane origin.
    #[arg(long, env = "SPOOL_CONTROL_PLANE_URL")]
    control_plane_url: Option<Url>,

    /// Enrolled agent ID. Required outside local mode.
    #[arg(long, env = "SPOOL_AGENT_ID")]
    agent_id: Option<String>,

    /// File containing the enrolled Ed25519 private key as 64 hex characters.
    #[arg(long, env = "SPOOL_DEVICE_KEY_FILE")]
    device_key_file: Option<PathBuf>,

    /// Native executor selection. Fake is only for development and tests.
    #[arg(long, env = "SPOOL_EXECUTOR", default_value = "disabled")]
    executor: ExecutorMode,

    /// Executor child-process path when --executor=process.
    #[arg(long, env = "SPOOL_EXECUTOR_PATH")]
    executor_path: Option<PathBuf>,
}

#[derive(Debug)]
struct CloudConfiguration {
    client: AgentClient,
    identity: DeviceIdentity,
    agent_id: AgentId,
}

#[derive(Debug)]
enum RuntimeExecutor {
    Disabled,
    Fake(FakeExecutor),
    Process(SupervisedExecutor),
}

impl RuntimeExecutor {
    async fn printers(&self) -> Result<Vec<LocalPrinter>, ControlFailure> {
        let printers = match self {
            Self::Disabled => Vec::new(),
            Self::Fake(_) => vec![DiscoveredPrinter {
                native_id: "fake-printer".into(),
                name: "Fake Printer".into(),
                is_default: true,
                state: spool_domain::PrinterState::Online,
                capabilities: spool_domain::PrinterCapabilities::default(),
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
        Ok(printers
            .into_iter()
            .map(|printer| LocalPrinter {
                printer_id: printer.native_id,
                name: printer.name,
                state: serde_json::to_value(printer.state)
                    .ok()
                    .and_then(|value| value.as_str().map(ToOwned::to_owned))
                    .unwrap_or_else(|| "unknown".into()),
                is_default: printer.is_default,
            })
            .collect())
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
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .json()
        .init();

    let arguments = Arguments::parse();
    std::fs::create_dir_all(&arguments.data_dir)
        .with_context(|| format!("create {}", arguments.data_dir.display()))?;
    let database_path = arguments.data_dir.join("agent.sqlite3");
    let store = AgentStore::open(&database_path)
        .with_context(|| format!("open {}", database_path.display()))?;
    if !store.integrity_check()? {
        anyhow::bail!("agent database integrity check failed");
    }
    let initially_paused = store.setting("paused")?.as_deref() == Some("true");

    let challenge = load_or_create_private_token(&arguments.data_dir.join("local.token"))?;
    let content_store = ContentStore::open(arguments.data_dir.join("content")).await?;
    let cloud_content_store = content_store.clone();
    let executor = match arguments.executor {
        ExecutorMode::Disabled => RuntimeExecutor::Disabled,
        ExecutorMode::Fake => RuntimeExecutor::Fake(FakeExecutor::default()),
        ExecutorMode::Process => {
            RuntimeExecutor::Process(SupervisedExecutor::new(ExecutorSupervisor::new(
                arguments
                    .executor_path
                    .clone()
                    .context("--executor-path is required with --executor=process")?,
                Duration::from_secs(120),
            )))
        }
    };
    let engine = AgentEngine::new(store, executor, SystemClock);

    let connection = Arc::new(RwLock::new(if arguments.mode == AgentMode::Local {
        ConnectionState::LocalOnly
    } else {
        ConnectionState::Connecting
    }));
    let paused = Arc::new(AtomicBool::new(initially_paused));
    let (control_tx, control_rx) = mpsc::channel(32);
    tokio::spawn(control_loop(
        control_rx,
        engine,
        content_store,
        env!("CARGO_PKG_VERSION").to_owned(),
        Arc::clone(&connection),
        Arc::clone(&paused),
    ));

    if arguments.mode != AgentMode::Local {
        let cloud = cloud_configuration(&arguments)?;
        tokio::spawn(cloud_sync_loop(
            cloud,
            database_path.clone(),
            cloud_content_store,
            Arc::clone(&connection),
            Arc::clone(&paused),
        ));
    }

    info!(
        mode = ?arguments.mode,
        database = %database_path.display(),
        bind = %arguments.local_bind,
        "Spool agent started"
    );
    spool_local_api::serve(
        arguments.local_bind,
        LocalApiState::new(&challenge, control_tx),
    )
    .await
    .context("serve local API")
}

async fn control_loop(
    mut requests: mpsc::Receiver<ControlRequest>,
    mut engine: AgentEngine<RuntimeExecutor>,
    content_store: ContentStore,
    version: String,
    connection: Arc<RwLock<ConnectionState>>,
    paused: Arc<AtomicBool>,
) {
    let mut scheduler = tokio::time::interval(Duration::from_millis(250));
    scheduler.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            request = requests.recv() => {
                let Some(request) = request else {
                    break;
                };
                handle_control_request(
                    request,
                    &mut engine,
                    &content_store,
                    &version,
                    &connection,
                    &paused,
                ).await;
            }
            _ = scheduler.tick(), if !paused.load(Ordering::Relaxed) => {
                if let Err(error) = engine.run_once().await {
                    error!(%error, "local print scheduler iteration failed");
                }
            }
        }
    }
    warn!("local control channel closed");
}

async fn handle_control_request(
    request: ControlRequest,
    engine: &mut AgentEngine<RuntimeExecutor>,
    content_store: &ContentStore,
    version: &str,
    connection: &RwLock<ConnectionState>,
    paused: &AtomicBool,
) {
    match request {
        ControlRequest::Status { respond_to } => {
            let counts = match engine.store().queue_counts() {
                Ok(counts) => counts,
                Err(error) => {
                    error!(%error, "failed to read local queue counts");
                    QueueCounts::default()
                }
            };
            let _ = respond_to.send(LocalStatus {
                agent_id: None,
                workspace_name: None,
                version: version.to_owned(),
                connection: *connection.read().await,
                queued_jobs: counts.queued,
                active_jobs: counts.active,
                printer_warnings: 0,
                paused: paused.load(Ordering::Relaxed),
            });
        }
        ControlRequest::Printers { respond_to } => match engine.executor_mut().printers().await {
            Ok(printers) => {
                let _ = respond_to.send(printers);
            }
            Err(error) => {
                warn!(code = %error.code, message = %error.message, "printer discovery failed");
                let _ = respond_to.send(Vec::new());
            }
        },
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
        ControlRequest::SubmitJob {
            request,
            respond_to,
        } => {
            let result = submit_local_job(
                engine,
                content_store,
                *request,
                paused.load(Ordering::Relaxed),
            )
            .await;
            let _ = respond_to.send(result);
        }
    }
}

async fn submit_local_job(
    engine: &mut AgentEngine<RuntimeExecutor>,
    content_store: &ContentStore,
    request: LocalCreateJob,
    paused: bool,
) -> Result<LocalJobAccepted, ControlFailure> {
    if paused {
        return Err(control_failure(
            "agent_paused",
            "the agent is not accepting new local jobs",
        ));
    }
    let input: Box<dyn tokio::io::AsyncRead + Unpin + Send> = match request.content {
        LocalContent::Base64 { data } => {
            let bytes = STANDARD
                .decode(data)
                .map_err(|_| control_failure("invalid_base64", "content is not valid base64"))?;
            Box::new(std::io::Cursor::new(bytes))
        }
        LocalContent::Uri { uri } => {
            let url = Url::parse(&uri)
                .map_err(|_| control_failure("invalid_uri", "content URI is invalid"))?;
            if !matches!(url.scheme(), "http" | "https") {
                return Err(control_failure(
                    "invalid_uri_scheme",
                    "only HTTP and HTTPS content URIs are supported",
                ));
            }
            let response = reqwest::Client::new()
                .get(url)
                .timeout(Duration::from_secs(120))
                .send()
                .await
                .map_err(|_| control_failure("content_unavailable", "content URI request failed"))?
                .error_for_status()
                .map_err(|_| {
                    control_failure("content_unavailable", "content URI returned an error")
                })?;
            if response
                .content_length()
                .is_some_and(|length| length > ContentStore::MAX_CONTENT_BYTES)
            {
                return Err(control_failure(
                    "content_too_large",
                    "content exceeds the local 50 MiB limit",
                ));
            }
            let stream = response.bytes_stream().map_err(std::io::Error::other);
            Box::new(StreamReader::new(stream))
        }
    };
    let stored = content_store
        .put(input)
        .await
        .map_err(|error| control_failure("content_store_failed", &error.to_string()))?;
    let job_id = JobId::new().to_string();
    let options_json = serde_json::to_string(&request.options)
        .map_err(|_| control_failure("invalid_options", "print options are invalid"))?;
    engine
        .accept(&AcceptedJob {
            job_id: job_id.clone(),
            submission_id: format!("sub_{}", uuid::Uuid::new_v4()),
            printer_id: request.printer_id,
            printer_native_id: request.printer_native_id,
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
    Ok(CloudConfiguration {
        client: AgentClient::new(base_url)?,
        identity: DeviceIdentity::from_secret_bytes(agent_id, &secret),
        agent_id,
    })
}

async fn cloud_sync_loop(
    cloud: CloudConfiguration,
    database_path: PathBuf,
    content_store: ContentStore,
    connection: Arc<RwLock<ConnectionState>>,
    paused: Arc<AtomicBool>,
) {
    let mut store = match AgentStore::open(&database_path) {
        Ok(store) => store,
        Err(error) => {
            error!(%error, "cloud sync cannot open the agent database");
            *connection.write().await = ConnectionState::Degraded;
            return;
        }
    };
    let started_at = Utc::now();
    let mut failures = 0_u32;
    loop {
        let request = match sync_request(
            &store,
            cloud.agent_id,
            started_at,
            paused.load(Ordering::Relaxed),
        ) {
            Ok(request) => request,
            Err(error) => {
                error!(%error, "cloud sync cannot read queue health");
                *connection.write().await = ConnectionState::Degraded;
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };
        let delay = match cloud.client.sync(&cloud.identity, &request).await {
            Ok(response) => {
                sync_succeeded(
                    response,
                    &cloud,
                    &mut store,
                    &content_store,
                    &paused,
                    &mut failures,
                    &connection,
                )
                .await
            }
            Err(error) => sync_failed(&error, &mut failures, &connection).await,
        };
        tokio::time::sleep(delay).await;
    }
}

async fn sync_succeeded(
    response: AgentSyncResponse,
    cloud: &CloudConfiguration,
    store: &mut AgentStore,
    content_store: &ContentStore,
    paused: &AtomicBool,
    failures: &mut u32,
    connection: &RwLock<ConnectionState>,
) -> Duration {
    let AgentSyncResponse {
        acknowledged_event_cursor,
        command_cursor,
        commands,
        candidate_jobs,
        next_poll_after_ms,
        ..
    } = response;
    *failures = 0;
    *connection.write().await = ConnectionState::Connected;
    apply_event_acknowledgement(store, acknowledged_event_cursor);
    apply_commands(store, paused, commands, command_cursor);
    for offer in candidate_jobs {
        if let Err(error) = accept_offer(cloud, store, content_store, offer).await {
            warn!(%error, "job offer could not be durably accepted");
        }
    }
    Duration::from_millis(next_poll_after_ms.clamp(250, 30_000))
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

fn apply_commands(
    store: &mut AgentStore,
    paused: &AtomicBool,
    commands: Vec<AgentCommand>,
    command_cursor: Option<String>,
) {
    for command in commands {
        if let Err(error) = apply_command(store, paused, command) {
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
            store.cancel_before_handoff(&job_id.to_string(), Utc::now().timestamp_millis())?;
        }
        AgentCommand::RefreshPrinters => {
            warn!("printer refresh requested but inventory watcher is not enabled");
        }
        AgentCommand::UpdateAvailable { version, .. } => {
            info!(%version, "signed update is available");
        }
    }
    Ok(())
}

async fn accept_offer(
    cloud: &CloudConfiguration,
    store: &mut AgentStore,
    content_store: &ContentStore,
    offer: JobOffer,
) -> Result<()> {
    let job_id = offer.job.id;
    let stored = match materialize_descriptor(content_store, offer.content).await {
        Ok(stored) => stored,
        Err(error) => {
            let _ = cloud
                .client
                .release_lease(
                    &cloud.identity,
                    job_id,
                    &AgentReleaseLeaseRequest {
                        lease_id: offer.lease_id,
                        lease_token: offer.lease_token,
                        reason: "content_unavailable".into(),
                    },
                )
                .await;
            return Err(error);
        }
    };
    let local = store.accept_job(&AcceptedJob {
        job_id: job_id.to_string(),
        submission_id: format!("sub_{job_id}"),
        printer_id: offer.job.printer_id.to_string(),
        // The inventory contract does not yet include the agent-native queue
        // key; retain the logical ID and fail closed if it does not resolve.
        printer_native_id: offer.job.printer_id.to_string(),
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
        accepted_unix_ms: Utc::now().timestamp_millis(),
        cloud_managed: true,
    })?;
    cloud
        .client
        .accept_job(
            &cloud.identity,
            job_id,
            &AgentAcceptJobRequest {
                lease_id: offer.lease_id,
                lease_token: offer.lease_token,
                content_sha256: stored.sha256,
                local_sequence: u64::try_from(local.printer_sequence).unwrap_or(u64::MAX),
            },
        )
        .await?;
    Ok(())
}

async fn materialize_descriptor(
    content_store: &ContentStore,
    descriptor: ContentDescriptor,
) -> Result<spool_agent_core::StoredContent> {
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
                Ok(spool_agent_core::StoredContent {
                    bytes: tokio::fs::metadata(&path).await?.len(),
                    path,
                    sha256: expected,
                })
            } else {
                Ok(content_store.put(std::io::Cursor::new(decoded)).await?)
            }
        }
        ContentDescriptor::Download { url, sha256, bytes } => {
            if bytes > ContentStore::MAX_CONTENT_BYTES {
                anyhow::bail!("offered content exceeds local limit");
            }
            let response = reqwest::get(url).await?.error_for_status()?;
            let stream = response.bytes_stream().map_err(std::io::Error::other);
            let path = content_store
                .put_verified(&sha256, StreamReader::new(stream))
                .await?;
            Ok(spool_agent_core::StoredContent {
                bytes,
                path,
                sha256,
            })
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
            let mut request = reqwest::Client::new().get(uri);
            if let Some(UriAuthentication::Basic { username, password }) = authentication {
                request = request.basic_auth(username, Some(password));
            } else if authentication.is_some() {
                anyhow::bail!("Digest URI authentication is not enabled in this build");
            }
            let response = request.send().await?.error_for_status()?;
            let stream = response.bytes_stream().map_err(std::io::Error::other);
            if let Some(expected) = sha256 {
                let path = content_store
                    .put_verified(&expected, StreamReader::new(stream))
                    .await?;
                Ok(spool_agent_core::StoredContent {
                    bytes: tokio::fs::metadata(&path).await?.len(),
                    path,
                    sha256: expected,
                })
            } else {
                Ok(content_store.put(StreamReader::new(stream)).await?)
            }
        }
    }
}

async fn sync_failed(
    error: &ClientError,
    failures: &mut u32,
    connection: &RwLock<ConnectionState>,
) -> Duration {
    *failures = failures.saturating_add(1);
    *connection.write().await = ConnectionState::Offline;
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
) -> Result<AgentSyncRequest, StorageError> {
    let counts = store.queue_counts()?;
    let events = store
        .pending_cloud_events(0, 100)?
        .into_iter()
        .map(|event| protocol_event(store, agent_id, event))
        .collect::<Result<Vec<_>, _>>()?;
    let event_cursor = events.last().map(|event| event.id);
    Ok(AgentSyncRequest {
        agent_id,
        protocol_version: CURRENT_PROTOCOL_VERSION,
        agent_version: env!("CARGO_PKG_VERSION").to_owned(),
        printer_revision: 0,
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
            executor_crashes: 0,
            last_error_code: None,
        },
        printers: None,
        events,
    })
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
