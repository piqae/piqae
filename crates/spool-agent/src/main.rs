use anyhow::{Context, Result};
use chrono::Utc;
use clap::{Parser, ValueEnum};
use spool_agent_client::{AgentClient, ClientError, DeviceIdentity};
use spool_agent_storage::{AgentStore, QueueCounts, StorageError};
use spool_domain::AgentId;
use spool_local_api::{ControlRequest, LocalApiState};
use spool_local_ipc::{ConnectionState, LocalStatus, SessionAuthenticator};
use spool_protocol::{
    CURRENT_PROTOCOL_VERSION,
    agent::{AgentHealth, AgentSyncRequest, AgentSyncResponse, QueueSnapshot},
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
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;
use url::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum AgentMode {
    Local,
    Hosted,
    SelfHosted,
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
}

#[derive(Debug)]
struct CloudConfiguration {
    client: AgentClient,
    identity: DeviceIdentity,
    agent_id: AgentId,
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

    let challenge = load_or_create_private_token(&arguments.data_dir.join("local.token"))?;

    let connection = Arc::new(RwLock::new(if arguments.mode == AgentMode::Local {
        ConnectionState::LocalOnly
    } else {
        ConnectionState::Connecting
    }));
    let paused = Arc::new(AtomicBool::new(false));
    let (control_tx, control_rx) = mpsc::channel(32);
    tokio::spawn(control_loop(
        control_rx,
        store,
        env!("CARGO_PKG_VERSION").to_owned(),
        Arc::clone(&connection),
        Arc::clone(&paused),
    ));

    if arguments.mode != AgentMode::Local {
        let cloud = cloud_configuration(&arguments)?;
        tokio::spawn(cloud_sync_loop(
            cloud,
            database_path.clone(),
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
    store: AgentStore,
    version: String,
    connection: Arc<RwLock<ConnectionState>>,
    paused: Arc<AtomicBool>,
) {
    while let Some(request) = requests.recv().await {
        match request {
            ControlRequest::Status { respond_to } => {
                let counts = match store.queue_counts() {
                    Ok(counts) => counts,
                    Err(error) => {
                        error!(%error, "failed to read local queue counts");
                        QueueCounts::default()
                    }
                };
                let _ = respond_to.send(LocalStatus {
                    agent_id: None,
                    workspace_name: None,
                    version: version.clone(),
                    connection: *connection.read().await,
                    queued_jobs: counts.queued,
                    active_jobs: counts.active,
                    printer_warnings: 0,
                    paused: paused.load(Ordering::Relaxed),
                });
            }
            ControlRequest::Printers { respond_to } => {
                let _ = respond_to.send(Vec::new());
            }
            ControlRequest::Pause { respond_to } => {
                paused.store(true, Ordering::Relaxed);
                let _ = respond_to.send(Ok(()));
            }
            ControlRequest::Resume { respond_to } => {
                paused.store(false, Ordering::Relaxed);
                let _ = respond_to.send(Ok(()));
            }
        }
    }
    warn!("local control channel closed");
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
            Ok(response) => sync_succeeded(response, &mut failures, &connection).await,
            Err(error) => sync_failed(&error, &mut failures, &connection).await,
        };
        tokio::time::sleep(delay).await;
    }
}

async fn sync_succeeded(
    response: AgentSyncResponse,
    failures: &mut u32,
    connection: &RwLock<ConnectionState>,
) -> Duration {
    *failures = 0;
    *connection.write().await = ConnectionState::Connected;
    if !response.candidate_jobs.is_empty() {
        warn!(
            candidates = response.candidate_jobs.len(),
            "server offered jobs but no native executor is enabled; leaving them unclaimed"
        );
    }
    if !response.commands.is_empty() {
        warn!(
            commands = response.commands.len(),
            "agent commands are not enabled in this build"
        );
    }
    Duration::from_millis(response.next_poll_after_ms.clamp(250, 30_000))
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
    Ok(AgentSyncRequest {
        agent_id,
        protocol_version: CURRENT_PROTOCOL_VERSION,
        agent_version: env!("CARGO_PKG_VERSION").to_owned(),
        printer_revision: 0,
        acknowledged_command_cursor: None,
        event_cursor: None,
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
        // Outbox conversion is deliberately disabled until server
        // acknowledgement uses a monotonic local cursor.
        events: Vec::new(),
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
