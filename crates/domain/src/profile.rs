use crate::{
    AgentId, JobId, NativeProfileBlobId, PhysicalDeviceId, PrinterId, ProfileCaptureSessionId,
    ProfileId, StockId, TargetBindingId, TargetId,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const NATIVE_PROFILE_SCHEMA_VERSION: u16 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeProfileKind {
    PortableOptions,
    WindowsDevmode,
    WindowsPrintTicket,
    MacosPrintcore,
    CupsOptions,
    CupsInstance,
    NativeQueue,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileStatus {
    Draft,
    Capturing,
    Ready,
    NeedsTest,
    Stale,
    DriverMismatch,
    DestinationMissing,
    DependencyMissing,
    InteractiveOnly,
    Invalid,
    Retired,
}

impl Default for ProfileStatus {
    fn default() -> Self {
        Self::NeedsTest
    }
}

impl ProfileStatus {
    #[must_use]
    pub const fn permits_jobs(self) -> bool {
        !matches!(self, Self::Capturing | Self::Invalid | Self::Retired)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SafeProfileOverride {
    Bin,
    Collate,
    Color,
    Copies,
    Dpi,
    Duplex,
    FitToPage,
    Media,
    Nup,
    Pages,
    Paper,
    Rotate,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct DriverFingerprint {
    pub platform: String,
    pub driver_name: String,
    pub driver_version: Option<String>,
    pub architecture: Option<String>,
    pub native_queue_id: String,
    pub device_fingerprint: Option<String>,
    pub driver_package_fingerprint: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct ProfileSummary {
    pub paper: Option<String>,
    pub dimensions_mm: Option<[f64; 2]>,
    pub source: Option<String>,
    pub media: Option<String>,
    pub color: Option<String>,
    pub duplex: Option<String>,
    pub resolution: Option<String>,
    pub copies: Option<u32>,
    /// Redacted, display-safe driver values that do not include native blobs.
    pub native: BTreeMap<String, String>,
    /// Extensible display-safe facts that do not fit the portable fields.
    pub details: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NativeConfigurationRef {
    pub kind: NativeProfileKind,
    pub schema_version: u16,
    pub digest: String,
    pub local_blob_id: NativeProfileBlobId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProfileDependency {
    pub kind: String,
    pub value: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct NativeProfileRevision {
    pub id: ProfileId,
    pub revision: u64,
    pub destination_id: PrinterId,
    pub name: String,
    pub status: ProfileStatus,
    pub driver_fingerprint: DriverFingerprint,
    pub native_configuration: NativeConfigurationRef,
    pub summary: ProfileSummary,
    pub stock_id: Option<StockId>,
    #[serde(default)]
    pub dependencies: Vec<ProfileDependency>,
    #[serde(default)]
    pub safe_overrides: Vec<SafeProfileOverride>,
    pub last_validated_unix_ms: Option<i64>,
    pub last_test_job_id: Option<JobId>,
    #[serde(default)]
    pub published: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StockKind {
    Sheet,
    Roll,
    RollLabel,
    Card,
    Envelope,
    Other,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct MediaDimensionsMm {
    pub width: f64,
    pub length: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Stock {
    pub id: StockId,
    pub name: String,
    pub sku: Option<String>,
    pub kind: StockKind,
    pub dimensions_mm: Option<MediaDimensionsMm>,
    pub media_form: Option<String>,
    pub thickness_mm: Option<f64>,
    pub gap_mm: Option<f64>,
    pub mark_interval_mm: Option<f64>,
    pub loading_instructions: Option<String>,
    pub barcode: Option<String>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LoadedMediaConfidence {
    DeviceReported,
    DriverReported,
    BarcodeScanned,
    OperatorConfirmed,
    Assumed,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LoadedMedia {
    pub device_id: PhysicalDeviceId,
    pub source: String,
    pub stock_id: Option<StockId>,
    pub confidence: LoadedMediaConfidence,
    pub confirmed_unix_ms: i64,
    pub confirmed_by: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetRoutingPolicy {
    PrimaryOnly,
    PrimaryThenStandby,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PrintTarget {
    pub id: TargetId,
    pub name: String,
    pub stock_id: Option<StockId>,
    pub routing_policy: TargetRoutingPolicy,
    #[serde(default)]
    pub published: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingRole {
    Primary,
    Standby,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TargetBinding {
    pub id: TargetBindingId,
    pub target_id: TargetId,
    pub agent_id: AgentId,
    pub destination_id: PrinterId,
    pub profile_id: ProfileId,
    pub profile_revision: u64,
    pub role: BindingRole,
    pub priority: u16,
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetReadiness {
    Ready,
    NodeOffline,
    DestinationOffline,
    StockNotLoaded,
    NeedsOperator,
    ProfileStale,
    DriverMismatch,
    DependencyMissing,
    Busy,
    DeliveryUncertain,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct JobProfilePin {
    pub target_id: Option<TargetId>,
    pub binding_id: Option<TargetBindingId>,
    pub profile_id: Option<ProfileId>,
    pub profile_revision: Option<u64>,
    pub stock_id: Option<StockId>,
    /// Redacted snapshot of loaded-media facts used by the routing decision.
    pub loaded_media_snapshot: Option<serde_json::Value>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileCaptureOperation {
    Create,
    Edit,
    Clone,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileCaptureStatus {
    Authorized,
    Committed,
    Cancelled,
    Expired,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProfileCaptureSession {
    pub id: ProfileCaptureSessionId,
    pub destination_id: PrinterId,
    pub profile_id: Option<ProfileId>,
    pub expected_revision: Option<u64>,
    pub operation: ProfileCaptureOperation,
    pub status: ProfileCaptureStatus,
    pub expires_unix_ms: i64,
    pub created_unix_ms: i64,
}
