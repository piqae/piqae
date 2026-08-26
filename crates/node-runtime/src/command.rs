//! UI-independent command bus shared by brokers, compatibility HTTP and SDKs.

use piqae_local_ipc::{
    ConfirmLoadedMedia, LocalPrinter, LocalPrinterProfile, LocalPrinterQueue, LocalStatus,
    NativeProfileCapturePayload, ProfileCaptureAuthorized, ProfileValidationResult,
};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

#[derive(Debug)]
pub enum RuntimeCommand {
    Status {
        respond_to: oneshot::Sender<LocalStatus>,
    },
    Printers {
        respond_to: oneshot::Sender<Vec<LocalPrinter>>,
    },
    SetPrinterExposure {
        printer_id: String,
        exposed: bool,
        respond_to: oneshot::Sender<Result<LocalPrinter, CommandFailure>>,
    },
    Profiles {
        printer_id: String,
        respond_to: oneshot::Sender<Result<Vec<LocalPrinterProfile>, CommandFailure>>,
    },
    CreateProfile {
        printer_id: String,
        request: ProfileCreate,
        respond_to: oneshot::Sender<Result<LocalPrinterProfile, CommandFailure>>,
    },
    UpdateProfile {
        printer_id: String,
        profile_id: String,
        request: ProfileUpdate,
        respond_to: oneshot::Sender<Result<LocalPrinterProfile, CommandFailure>>,
    },
    DeleteProfile {
        printer_id: String,
        profile_id: String,
        expected_revision: u64,
        respond_to: oneshot::Sender<Result<(), CommandFailure>>,
    },
    BeginProfileCapture {
        printer_id: String,
        request: ProfileCaptureBeginRequest,
        respond_to: oneshot::Sender<Result<ProfileCaptureAuthorized, CommandFailure>>,
    },
    CommitProfileCapture {
        session_id: String,
        capture_token: String,
        capture: Box<NativeProfileCapturePayload>,
        respond_to: oneshot::Sender<Result<LocalPrinterProfile, CommandFailure>>,
    },
    CancelProfileCapture {
        session_id: String,
        capture_token: String,
        respond_to: oneshot::Sender<Result<(), CommandFailure>>,
    },
    ValidateProfile {
        profile_id: String,
        revision: u64,
        respond_to: oneshot::Sender<Result<ProfileValidationResult, CommandFailure>>,
    },
    ConfirmLoadedMedia {
        request: ConfirmLoadedMedia,
        respond_to: oneshot::Sender<Result<(), CommandFailure>>,
    },
    PrinterQueue {
        printer_id: String,
        respond_to: oneshot::Sender<Result<LocalPrinterQueue, CommandFailure>>,
    },
    JobHistory {
        offset: usize,
        limit: usize,
        respond_to: oneshot::Sender<Result<LocalJobHistory, CommandFailure>>,
    },
    ReprintJob {
        job_id: String,
        idempotency_key: String,
        confirmed: bool,
        respond_to: oneshot::Sender<Result<LocalJobAccepted, CommandFailure>>,
    },
    ConnectorDetails {
        respond_to: oneshot::Sender<Result<Vec<LocalConnectorDetail>, CommandFailure>>,
    },
    TestPage {
        printer_id: String,
        profile_id: String,
        confirmed: bool,
        respond_to: oneshot::Sender<Result<LocalJobAccepted, CommandFailure>>,
    },
    Pause {
        respond_to: oneshot::Sender<Result<(), CommandFailure>>,
    },
    Resume {
        respond_to: oneshot::Sender<Result<(), CommandFailure>>,
    },
    ReloadConnectors {
        respond_to: oneshot::Sender<Result<(), CommandFailure>>,
    },
    RevokeConnector {
        connector_id: String,
        respond_to: oneshot::Sender<Result<(), CommandFailure>>,
    },
    SubmitJob {
        request: Box<LocalCreateJob>,
        respond_to: oneshot::Sender<Result<LocalJobAccepted, CommandFailure>>,
    },
}

/// Compatibility name for the one-release loopback HTTP adapter. New code
/// should use `RuntimeCommand` through `piqae-node-client`.
#[deprecated(note = "use RuntimeCommand through piqae-node-client; remove after N/N-1 window")]
pub type ControlRequest = RuntimeCommand;

#[derive(Debug, Clone, Serialize)]
pub struct CommandFailure {
    pub code: String,
    pub message: String,
}

#[deprecated(note = "use CommandFailure; remove with the loopback control adapter")]
pub type ControlFailure = CommandFailure;

#[derive(Debug, Clone, Deserialize)]
pub struct LocalCreateJob {
    pub printer_id: String,
    #[serde(default)]
    pub printer_native_id: Option<String>,
    pub title: String,
    pub content_kind: piqae_domain::ContentKind,
    pub content: LocalContent,
    #[serde(default)]
    pub options: piqae_domain::JobOptions,
    pub expires_unix_ms: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LocalContent {
    Base64 { data: String },
    Uri { uri: String },
}

#[derive(Debug, Clone, Serialize)]
pub struct LocalJobAccepted {
    pub job_id: String,
    pub state: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LocalJobHistory {
    pub jobs: Vec<LocalHistoryJob>,
    pub next_offset: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LocalHistoryJob {
    pub job_id: String,
    pub printer_id: String,
    pub title: String,
    pub state: String,
    pub native_job_id: Option<String>,
    pub can_reprint: bool,
    pub created_unix_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LocalConnectorDetail {
    pub connector_id: String,
    pub display_name: String,
    pub workspace_name: Option<String>,
    pub authorization_type: Option<String>,
    pub workspace_id: Option<String>,
    pub environment_id: Option<String>,
    pub requesting_service_account_id: Option<String>,
    pub endpoint: String,
    pub connection: String,
    pub permission: String,
    pub allowed_printer_ids: Vec<String>,
    pub selected_printer_count: usize,
    pub last_sync_error_code: Option<String>,
    pub local_printer_count: usize,
    pub eligible_printer_count: usize,
    pub inventory_revision: u64,
    pub inventory_refresh_pending: bool,
    pub cross_authority_route_warning: bool,
    pub manage_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExposureUpdate {
    pub exposed: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileCreate {
    pub name: String,
    #[serde(default)]
    pub is_default: bool,
    #[serde(default)]
    pub options: piqae_domain::JobOptions,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileUpdate {
    pub expected_revision: u64,
    pub name: String,
    #[serde(default)]
    pub is_default: bool,
    #[serde(default)]
    pub options: piqae_domain::JobOptions,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeleteProfileQuery {
    pub expected_revision: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TestPageRequest {
    pub profile_id: String,
    #[serde(default)]
    pub confirmed: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileCaptureBeginRequest {
    pub operation: piqae_domain::ProfileCaptureOperation,
    pub profile_id: Option<String>,
    pub expected_revision: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidateProfileRequest {
    pub revision: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfirmLoadedMediaRequest {
    pub stock_id: Option<String>,
    pub confidence: piqae_domain::LoadedMediaConfidence,
    pub confirmed_by: Option<String>,
}
