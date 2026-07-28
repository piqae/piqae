use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use spool_domain::{
    AgentId, EventId, Job, JobEvent, PrinterCapabilities, PrinterId, PrinterState,
    UriAuthentication,
};
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EnrolRequest {
    pub token: String,
    pub public_key: String,
    pub name: String,
    pub hostname: String,
    pub platform: String,
    pub architecture: String,
    pub installation_mode: InstallationMode,
    pub agent_version: String,
    pub protocol_version: u16,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallationMode {
    Machine,
    User,
    Local,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EnrolResponse {
    pub agent_id: AgentId,
    pub environment: String,
    pub server_time: DateTime<Utc>,
    pub sync_after_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentSyncRequest {
    pub agent_id: AgentId,
    pub protocol_version: u16,
    pub agent_version: String,
    pub printer_revision: u64,
    pub acknowledged_command_cursor: Option<String>,
    pub event_cursor: Option<EventId>,
    pub queue: QueueSnapshot,
    pub health: AgentHealth,
    pub printers: Option<Vec<PrinterSnapshot>>,
    pub events: Vec<JobEvent>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentSyncResponse {
    pub server_time: DateTime<Utc>,
    pub acknowledged_event_cursor: Option<EventId>,
    pub command_cursor: Option<String>,
    pub commands: Vec<AgentCommand>,
    pub candidate_jobs: Vec<JobOffer>,
    pub next_poll_after_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct JobOffer {
    pub job: Job,
    pub lease_id: Uuid,
    /// Opaque, single-lease capability. It must never be logged.
    pub lease_token: String,
    pub lease_expires_at: DateTime<Utc>,
    pub content: ContentDescriptor,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentDescriptor {
    Download {
        url: String,
        sha256: String,
        bytes: u64,
    },
    InlineBase64 {
        data: String,
        sha256: Option<String>,
        bytes: Option<u64>,
    },
    Uri {
        uri: String,
        authentication: Option<UriAuthentication>,
        sha256: Option<String>,
        bytes: Option<u64>,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentAcceptJobRequest {
    pub lease_id: Uuid,
    pub lease_token: String,
    pub content_sha256: String,
    pub local_sequence: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentAcceptJobResponse {
    pub accepted_at: DateTime<Utc>,
    pub state: spool_domain::JobState,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentRenewLeaseRequest {
    pub lease_id: Uuid,
    pub lease_token: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentRenewLeaseResponse {
    pub lease_expires_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentReleaseLeaseRequest {
    pub lease_id: Uuid,
    pub lease_token: String,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentCommand {
    RefreshPrinters,
    CancelJob {
        job_id: spool_domain::JobId,
    },
    Pause,
    Resume,
    UpdateAvailable {
        version: String,
        channel: String,
        metadata_url: String,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct QueueSnapshot {
    pub queued_jobs: u32,
    pub active_jobs: u32,
    pub content_bytes: u64,
    pub accepts_jobs: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentHealth {
    pub started_at: DateTime<Utc>,
    pub observed_at: DateTime<Utc>,
    pub sqlite_integrity_ok: bool,
    pub executor_crashes: u64,
    pub last_error_code: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PrinterSnapshot {
    pub id: PrinterId,
    pub native_id: String,
    pub name: String,
    pub state: PrinterState,
    pub is_default: bool,
    pub capabilities: PrinterCapabilities,
}
