mod uri_fetch;

use anyhow::{Context, Result};
use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use chrono::Utc;
use clap::{Parser, ValueEnum};
use futures::TryStreamExt;
use spool_agent_client::{AgentClient, ClientError, DeviceIdentity};
use spool_agent_core::{
    AgentEngine, ContentStore, Executor, ExecutorFailure, FakeExecutor, LocalSubmission,
    NativeAcceptance, NativeJobReference, SystemClock,
};
use spool_agent_storage::{
    AcceptedJob, AgentStore, CloudAcceptIntent, NativeProfileCapture, PendingEvent, QueueCounts,
    StorageError, StoredLoadedMedia, StoredNamedProfile, StoredPrinter,
};
use spool_domain::{
    AgentId, ContentKind, EventId, JobEvent, JobFailureReason, JobId, JobState, NativeProfileKind,
    ProfileCaptureOperation, ProfileStatus,
};
use spool_executor_supervisor::{ExecutorSupervisor, SupervisedExecutor};
use spool_local_api::{
    ControlFailure, ControlRequest, LocalApiState, LocalContent, LocalCreateJob, LocalJobAccepted,
    ProfileCreate, ProfileUpdate,
};
use spool_local_ipc::{
    ConnectionState, LocalNativeQueueJob, LocalPrinter, LocalPrinterProfile, LocalPrinterQueue,
    LocalPrinterQueueCounts, LocalQueueJob, LocalStatus, NativeProfileCapturePayload,
    NativeProfileSeed, ProfileCaptureAuthorized, ProfileValidationResult, SessionAuthenticator,
    capture_token_digest, generate_capture_token,
};
use spool_protocol::{
    CURRENT_PROTOCOL_VERSION,
    agent::{
        AgentAcceptJobRequest, AgentCommand, AgentHealth, AgentReleaseLeaseRequest,
        AgentRenewLeaseRequest, AgentSyncRequest, AgentSyncResponse, ContentDescriptor, JobOffer,
        PrinterProfileSnapshot, PrinterSnapshot, QueueSnapshot,
    },
    executor::{DiscoveredPrinter, ExecutorOperation, ExecutorResult, NativeJobObservation},
};
use std::{
    future::Future,
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
use uri_fetch::UriFetcher;
use url::Url;

const LEASE_RENEWAL_INTERVAL: Duration = Duration::from_secs(10);
const LEASE_RENEWAL_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const PROFILE_CAPTURE_LIFETIME: Duration = Duration::from_secs(5 * 60);
const LOCAL_PROFILE_HOST_ID: &str = "authenticated-loopback-profile-host";

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

    /// Allow trusted private, loopback, and link-local URI content sources.
    /// Cloud metadata and unspecified/multicast destinations remain blocked.
    #[arg(long, env = "SPOOL_ALLOW_PRIVATE_URI_SOURCES", default_value_t = false)]
    allow_private_uri_sources: bool,
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
                state: spool_domain::PrinterState::Online,
                capabilities: spool_domain::PrinterCapabilities::default(),
                native_options: std::collections::BTreeMap::new(),
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
    ) -> Result<Vec<spool_protocol::executor::NativeQueueJob>, ControlFailure> {
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
        uri_fetcher,
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
            cloud_uri_fetcher,
            printer_discovery,
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
    uri_fetcher: UriFetcher,
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
                    &uri_fetcher,
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

#[allow(
    clippy::too_many_lines,
    reason = "keeps the exhaustive authenticated control command dispatch in one audit point"
)]
async fn handle_control_request(
    request: ControlRequest,
    engine: &mut AgentEngine<RuntimeExecutor>,
    content_store: &ContentStore,
    uri_fetcher: &UriFetcher,
    version: &str,
    connection: &RwLock<ConnectionState>,
    paused: &AtomicBool,
) {
    match request {
        ControlRequest::Status { respond_to } => {
            let current_connection = *connection.read().await;
            let _ = respond_to.send(local_status(
                engine.store(),
                version,
                current_connection,
                paused,
            ));
        }
        ControlRequest::Printers { respond_to } => match refresh_local_printers(engine).await {
            Ok(printers) => {
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
            let result = begin_profile_capture(engine.store_mut(), &printer_id, request);
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
            let result = validate_profile_revision(engine.store(), &profile_id, revision);
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
        ControlRequest::TestPage {
            printer_id,
            profile_id,
            respond_to,
        } => {
            let result = submit_test_page(
                engine,
                content_store,
                &printer_id,
                &profile_id,
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
        agent_id: None,
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
    engine: &mut AgentEngine<RuntimeExecutor>,
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
            return accept_stored_local_job(engine, request, stored).await;
        }
    };
    let stored = content_store
        .put(input)
        .await
        .map_err(|error| control_failure("content_store_failed", &error.to_string()))?;
    accept_stored_local_job(engine, request, stored).await
}

async fn refresh_local_printers(
    engine: &mut AgentEngine<RuntimeExecutor>,
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
    request: spool_local_api::ProfileCaptureBeginRequest,
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
    let existing = request
        .profile_id
        .as_deref()
        .map(|profile_id| store.named_profile(printer_id, profile_id))
        .transpose()
        .map_err(storage_control_failure)?
        .flatten();
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

    let session_id = spool_domain::ProfileCaptureSessionId::new().to_string();
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
    if native_blob.len() > spool_local_ipc::MAX_NATIVE_CAPTURE_BYTES {
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
    store: &AgentStore,
    profile_id: &str,
    revision: u64,
) -> Result<ProfileValidationResult, ControlFailure> {
    let validated_unix_ms = Utc::now().timestamp_millis();
    let blob = store
        .native_profile_blob(profile_id, revision)
        .map_err(storage_control_failure)?
        .ok_or_else(|| control_failure("profile_not_found", "profile revision was not found"))?;
    let _: NativeProfileKind = parse_stored_enum(&blob.native_kind, "native profile kind")?;
    Ok(ProfileValidationResult {
        profile_id: profile_id.to_owned(),
        revision,
        status: ProfileStatus::NeedsTest,
        code: Some("driver_test_required".into()),
        message: Some(
            "The immutable native settings are intact; run a driver test before publishing.".into(),
        ),
        validated_unix_ms,
    })
}

fn confirm_loaded_media(
    store: &mut AgentStore,
    request: spool_local_ipc::ConfirmLoadedMedia,
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
    if !printer.exposed {
        return Err(control_failure(
            "printer_not_exposed",
            "printer exposure must be explicitly enabled before submission",
        ));
    }
    Ok(printer)
}

fn validate_options(
    printer: &StoredPrinter,
    options: &spool_domain::JobOptions,
) -> Result<(), ControlFailure> {
    let capabilities: spool_domain::PrinterCapabilities =
        serde_json::from_str(&printer.capabilities_json)
            .map_err(|error| control_failure("capabilities_invalid", &error.to_string()))?;
    let native_options: std::collections::BTreeMap<String, spool_domain::NativePrinterOption> =
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
    engine: &mut AgentEngine<RuntimeExecutor>,
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

async fn submit_test_page(
    engine: &mut AgentEngine<RuntimeExecutor>,
    content_store: &ContentStore,
    printer_id: &str,
    profile_id: &str,
    paused: bool,
) -> Result<LocalJobAccepted, ControlFailure> {
    if paused {
        return Err(control_failure("agent_paused", "the agent is paused"));
    }
    let printer = resolve_exposed_printer(engine.store(), printer_id)?;
    let profile = engine
        .store()
        .named_profile(printer_id, profile_id)
        .map_err(storage_control_failure)?
        .ok_or_else(|| control_failure("profile_not_found", "print profile was not found"))?;
    let mut options: spool_domain::JobOptions = serde_json::from_str(&profile.options_json)
        .map_err(|error| control_failure("profile_invalid", &error.to_string()))?;
    let capabilities: spool_domain::PrinterCapabilities =
        serde_json::from_str(&printer.capabilities_json)
            .map_err(|error| control_failure("capabilities_invalid", &error.to_string()))?;
    let native_definitions: std::collections::BTreeMap<String, spool_domain::NativePrinterOption> =
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
    accept_stored_local_job(
        engine,
        LocalCreateJob {
            printer_id: printer_id.to_owned(),
            printer_native_id: Some(printer.native_id),
            title: "Spool A4 diagnostic".into(),
            content_kind: ContentKind::Pdf,
            content: LocalContent::Base64 {
                data: String::new(),
            },
            options,
            expires_unix_ms: Some(Utc::now().timestamp_millis() + 300_000),
        },
        stored,
    )
    .await
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
    let content = b"BT /F1 22 Tf 72 760 Td (Spool A4 diagnostic) Tj /F1 11 Tf 0 -30 Td (Local queue and driver test) Tj 0 -22 Td (No external content was used.) Tj ET";
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
        StorageError::InvalidPrinterProfile(_) => {
            control_failure("profile_invalid", &error.to_string())
        }
        StorageError::CaptureSessionNotFound(_) => {
            control_failure("profile_capture_not_found", &error.to_string())
        }
        StorageError::CaptureSessionNotAuthorized(_) => {
            control_failure("profile_capture_timed_out", &error.to_string())
        }
        StorageError::InvalidCaptureToken => {
            control_failure("profile_capture_token_invalid", &error.to_string())
        }
        StorageError::NativeBlobTooLarge(_) => {
            control_failure("profile_invalid", &error.to_string())
        }
        _ => control_failure("local_storage_failed", &error.to_string()),
    }
}

async fn accept_stored_local_job(
    engine: &mut AgentEngine<RuntimeExecutor>,
    request: LocalCreateJob,
    stored: spool_agent_core::StoredContent,
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
    uri_fetcher: UriFetcher,
    printer_discovery: PrinterDiscovery,
    connection: Arc<RwLock<ConnectionState>>,
    paused: Arc<AtomicBool>,
) {
    let store = match AgentStore::open(&database_path) {
        Ok(store) => store,
        Err(error) => {
            error!(%error, "cloud sync cannot open the agent database");
            *connection.write().await = ConnectionState::Degraded;
            return;
        }
    };
    run_cloud_sync_loop(
        cloud,
        store,
        content_store,
        uri_fetcher,
        printer_discovery,
        connection,
        paused,
    )
    .await;
}

async fn run_cloud_sync_loop(
    cloud: CloudConfiguration,
    mut store: AgentStore,
    content_store: ContentStore,
    uri_fetcher: UriFetcher,
    printer_discovery: PrinterDiscovery,
    connection: Arc<RwLock<ConnectionState>>,
    paused: Arc<AtomicBool>,
) {
    let started_at = Utc::now();
    let mut failures = 0_u32;
    loop {
        resume_pending_cloud_accepts(&cloud, &mut store).await;
        let request = match prepare_sync_request(
            &mut store,
            &printer_discovery,
            cloud.agent_id,
            started_at,
            paused.load(Ordering::Relaxed),
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
        let delay = match cloud.client.sync(&cloud.identity, &request).await {
            Ok(response) => {
                sync_succeeded(
                    response,
                    SyncContext {
                        cloud: &cloud,
                        store: &mut store,
                        content_store: &content_store,
                        uri_fetcher: &uri_fetcher,
                        paused: &paused,
                        failures: &mut failures,
                        connection: &connection,
                    },
                )
                .await
            }
            Err(error) => sync_failed(&error, &mut failures, &connection).await,
        };
        tokio::time::sleep(delay).await;
    }
}

async fn prepare_sync_request(
    store: &mut AgentStore,
    printer_discovery: &PrinterDiscovery,
    agent_id: AgentId,
    started_at: chrono::DateTime<Utc>,
    paused: bool,
) -> Result<AgentSyncRequest> {
    let printers = match discover_cloud_printers(store, printer_discovery).await {
        Ok(printers) => Some(printers),
        Err(error) => {
            warn!(%error, "native printer inventory refresh failed");
            None
        }
    };
    Ok(sync_request(store, agent_id, started_at, paused, printers)?)
}

struct SyncContext<'a> {
    cloud: &'a CloudConfiguration,
    store: &'a mut AgentStore,
    content_store: &'a ContentStore,
    uri_fetcher: &'a UriFetcher,
    paused: &'a AtomicBool,
    failures: &'a mut u32,
    connection: &'a RwLock<ConnectionState>,
}

async fn sync_succeeded(response: AgentSyncResponse, context: SyncContext<'_>) -> Duration {
    let AgentSyncResponse {
        acknowledged_event_cursor,
        command_cursor,
        commands,
        candidate_jobs,
        next_poll_after_ms,
        ..
    } = response;
    *context.failures = 0;
    *context.connection.write().await = ConnectionState::Connected;
    apply_event_acknowledgement(context.store, acknowledged_event_cursor);
    apply_commands(context.store, context.paused, commands, command_cursor);
    for offer in candidate_jobs {
        if let Err(error) = accept_offer(
            context.cloud,
            context.store,
            context.content_store,
            context.uri_fetcher,
            offer,
        )
        .await
        {
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
            store.request_cancel(&job_id.to_string(), Utc::now().timestamp_millis())?;
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
    uri_fetcher: &UriFetcher,
    offer: JobOffer,
) -> Result<()> {
    let lease_id = offer.lease_id;
    let lease_token = offer.lease_token.clone();
    let job_id = offer.job.id;
    let result = maintain_lease(
        offer.lease_expires_at,
        LEASE_RENEWAL_INTERVAL,
        accept_offer_under_lease(cloud, store, content_store, uri_fetcher, offer),
        || async {
            tokio::time::timeout(
                LEASE_RENEWAL_REQUEST_TIMEOUT,
                cloud.client.renew_lease(
                    &cloud.identity,
                    job_id,
                    &AgentRenewLeaseRequest {
                        lease_id,
                        lease_token: lease_token.clone(),
                    },
                ),
            )
            .await
            .map_err(|_| anyhow::anyhow!("job lease renewal failed"))?
            .map(|response| response.lease_expires_at)
            .map_err(|_| anyhow::anyhow!("job lease renewal failed"))
        },
    )
    .await;
    if let Err(error) = &result {
        let has_durable_intent = store
            .pending_cloud_accepts()?
            .iter()
            .any(|intent| intent.job_id == job_id.to_string());
        if !has_durable_intent {
            let _ = cloud
                .client
                .release_lease(
                    &cloud.identity,
                    job_id,
                    &AgentReleaseLeaseRequest {
                        lease_id,
                        lease_token,
                        reason: if error.to_string().contains("printer_not_present") {
                            "printer_not_present"
                        } else if error.to_string().contains("printer_not_exposed") {
                            "printer_not_exposed"
                        } else if error.to_string().contains("unsupported_profile_option")
                            || error.to_string().contains("native option")
                        {
                            "unsupported_profile_option"
                        } else {
                            "acceptance_failed"
                        }
                        .into(),
                    },
                )
                .await;
        }
    }
    result
}

async fn accept_offer_under_lease(
    cloud: &CloudConfiguration,
    store: &mut AgentStore,
    content_store: &ContentStore,
    uri_fetcher: &UriFetcher,
    offer: JobOffer,
) -> Result<()> {
    let job_id = offer.job.id;
    let logical_printer_id = offer.job.printer_id.to_string();
    let printer = store
        .printer(&logical_printer_id)?
        .with_context(|| format!("printer_not_found: {logical_printer_id}"))?;
    if !printer.present {
        anyhow::bail!("printer_not_present: {logical_printer_id}");
    }
    if !printer.exposed {
        anyhow::bail!("printer_not_exposed: {logical_printer_id}");
    }
    validate_options(&printer, &offer.job.options)
        .map_err(|failure| anyhow::anyhow!("{}: {}", failure.code, failure.message))?;
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
            printer_native_id: printer.native_id,
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
    confirm_cloud_accept(
        cloud,
        store,
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

async fn resume_pending_cloud_accepts(cloud: &CloudConfiguration, store: &mut AgentStore) {
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
            confirm_cloud_accept(cloud, store, &intent),
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
    intent: &CloudAcceptIntent,
) -> Result<()> {
    let job_id = intent.job_id.parse::<JobId>()?;
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
    let remaining = (expires_at - Utc::now()).to_std().unwrap_or(Duration::ZERO);
    remaining
        .saturating_sub(SAFETY_MARGIN)
        .min(maximum_interval)
}

async fn materialize_descriptor(
    cloud: &CloudConfiguration,
    content_store: &ContentStore,
    uri_fetcher: &UriFetcher,
    job_id: JobId,
    lease_id: uuid::Uuid,
    lease_token: &str,
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
            Ok(uri_fetcher
                .fetch_to_store(
                    content_store,
                    &uri,
                    authentication.as_ref(),
                    sha256.as_deref(),
                )
                .await?)
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
    printers: Option<Vec<PrinterSnapshot>>,
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
        printer_revision: printers.as_ref().map_or(0, |items| items.len() as u64),
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
        printers,
        events,
    })
}

async fn discover_cloud_printers(
    store: &mut AgentStore,
    discovery: &PrinterDiscovery,
) -> Result<Vec<PrinterSnapshot>> {
    let discovered = match discovery {
        PrinterDiscovery::Disabled => Vec::new(),
        PrinterDiscovery::Fake => vec![DiscoveredPrinter {
            native_id: "fake-printer".into(),
            name: "Spool deterministic fake printer".into(),
            is_default: true,
            state: spool_domain::PrinterState::Online,
            capabilities: spool_domain::PrinterCapabilities::default(),
            native_options: std::collections::BTreeMap::new(),
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
    let present_native_ids = discovered
        .iter()
        .map(|printer| printer.native_id.clone())
        .collect::<Vec<_>>();
    let observed_unix_ms = Utc::now().timestamp_millis();
    let snapshots = discovered
        .into_iter()
        .map(|printer| {
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
            if !store
                .printer(&stored.printer_id)?
                .is_some_and(|printer| printer.exposed)
            {
                return Ok(None);
            }
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
                        published: profile.published,
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
                profiles,
            }))
        })
        .collect::<Result<Vec<Option<PrinterSnapshot>>>>()?;
    store.reconcile_printer_presence(&present_native_ids)?;
    Ok(snapshots.into_iter().flatten().collect())
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
        assert_eq!(renewals.load(Ordering::Relaxed), 1);
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
        let cloud = CloudConfiguration {
            client: AgentClient::new(Url::parse(&format!("http://{address}/")).expect("base URL"))
                .expect("client"),
            identity: DeviceIdentity::generate(agent_id),
            agent_id,
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
        confirm_cloud_accept(&cloud, &mut store, &intent)
            .await
            .expect_err("first response is ambiguous");
        assert!(store.runnable_heads(10).expect("runnable").is_empty());
        assert_eq!(store.pending_cloud_accepts().expect("intents").len(), 1);

        resume_pending_cloud_accepts(&cloud, &mut store).await;
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
}
