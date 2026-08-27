use chrono::{DateTime, Utc};
use piqae_domain::{
    AgentId, DriverFingerprint, EventId, Job, JobEvent, NativePrinterOption, NativeProfileKind,
    PrinterCapabilities, PrinterId, PrinterState, ProfileStatus, ProfileSummary,
    SafeProfileOverride, UriAuthentication,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NodeDisplayIdentity {
    pub display_name: String,
    pub site: Option<String>,
    pub location: Option<String>,
    pub labels: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentIdentityUpdateRequest {
    pub expected_revision: u64,
    pub display_name: String,
    pub site: Option<String>,
    pub location: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentIdentityUpdateResponse {
    pub revision: u64,
    pub identity: NodeDisplayIdentity,
}

fn append_proof_field(message: &mut Vec<u8>, value: &[u8]) {
    let length = u64::try_from(value.len()).unwrap_or(u64::MAX);
    message.extend_from_slice(&length.to_be_bytes());
    message.extend_from_slice(value);
}

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
    /// Stable, non-secret physical installation identifier. Supplying this
    /// adds an isolated connector; it never replaces another tenant's key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installation_id: Option<String>,
    /// Public key for a new, stable installation principal. This key is
    /// distinct from every connector key and is accepted only on first use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installation_public_key: Option<String>,
    /// Locally approved printer identifiers. Empty never means all printers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_printer_ids: Vec<String>,
    #[serde(default)]
    pub printer_grant: PrinterGrant,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installation_proof: Option<String>,
}

#[must_use]
pub fn connector_proof_message(
    token: &str,
    installation_id: &str,
    connector_public_key: &str,
    printer_ids: &[String],
) -> Vec<u8> {
    use sha2::{Digest as _, Sha256};
    let mut printers = printer_ids.to_vec();
    printers.sort();
    printers.dedup();
    let mut message = b"piqae-connect-proof-v2".to_vec();
    append_proof_field(&mut message, &Sha256::digest(token.as_bytes()));
    append_proof_field(&mut message, installation_id.as_bytes());
    append_proof_field(&mut message, connector_public_key.as_bytes());
    message.extend_from_slice(
        &u64::try_from(printers.len())
            .unwrap_or(u64::MAX)
            .to_be_bytes(),
    );
    for printer in printers {
        append_proof_field(&mut message, printer.as_bytes());
    }
    message
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PrinterGrant {
    #[default]
    SelectedPrinters,
    AllLocalPrinters,
}

#[must_use]
pub fn connector_grant_proof_message(
    token: &str,
    installation_id: &str,
    connector_public_key: &str,
    grant: PrinterGrant,
    printer_ids: &[String],
) -> Vec<u8> {
    use sha2::{Digest as _, Sha256};
    let mut printers = printer_ids.to_vec();
    printers.sort();
    printers.dedup();

    let mut message = b"piqae-connect-proof-v3".to_vec();
    append_proof_field(&mut message, &Sha256::digest(token.as_bytes()));
    append_proof_field(&mut message, installation_id.as_bytes());
    append_proof_field(&mut message, connector_public_key.as_bytes());
    append_proof_field(
        &mut message,
        match grant {
            PrinterGrant::SelectedPrinters => b"selected_printers",
            PrinterGrant::AllLocalPrinters => b"all_local_printers",
        },
    );
    message.extend_from_slice(
        &u64::try_from(printers.len())
            .unwrap_or(u64::MAX)
            .to_be_bytes(),
    );
    for printer in printers {
        append_proof_field(&mut message, printer.as_bytes());
    }
    message
}

#[cfg(test)]
mod connector_proof_tests {
    use super::{PrinterGrant, connector_grant_proof_message, connector_proof_message};
    use sha2::{Digest as _, Sha256};

    #[test]
    fn proof_is_order_stable_and_binds_every_security_input() {
        let first = connector_proof_message(
            "piq_enr_secret",
            "installation_1",
            "connector-key",
            &["b".into(), "a".into()],
        );
        assert_eq!(
            first,
            connector_proof_message(
                "piq_enr_secret",
                "installation_1",
                "connector-key",
                &["a".into(), "b".into()],
            )
        );
        assert_ne!(
            first,
            connector_proof_message(
                "piq_enr_other",
                "installation_1",
                "connector-key",
                &["a".into(), "b".into()],
            )
        );
        assert_ne!(
            first,
            connector_proof_message(
                "piq_enr_secret",
                "installation_2",
                "connector-key",
                &["a".into(), "b".into()],
            )
        );
        assert_ne!(
            first,
            connector_proof_message(
                "piq_enr_secret",
                "installation_1",
                "attacker-key",
                &["a".into(), "b".into()],
            )
        );
        assert_ne!(
            first,
            connector_proof_message(
                "piq_enr_secret",
                "installation_1",
                "connector-key",
                &["a".into(), "c".into()],
            )
        );
    }

    #[test]
    fn durable_grant_proof_binds_all_vs_selected_policy() {
        let all = connector_grant_proof_message(
            "piq_enr_secret",
            "installation_1",
            "connector-key",
            PrinterGrant::AllLocalPrinters,
            &[],
        );
        let selected = connector_grant_proof_message(
            "piq_enr_secret",
            "installation_1",
            "connector-key",
            PrinterGrant::SelectedPrinters,
            &[],
        );
        assert_ne!(all, selected);
    }

    #[test]
    fn proof_encoding_has_no_printer_separator_collisions() {
        assert_ne!(
            connector_proof_message(
                "piq_enr_secret",
                "installation_1",
                "connector-key",
                &["printer-a,printer-b".into()],
            ),
            connector_proof_message(
                "piq_enr_secret",
                "installation_1",
                "connector-key",
                &["printer-a".into(), "printer-b".into()],
            )
        );
    }

    #[test]
    fn proof_encoding_matches_the_v2_golden_vector() {
        let message = connector_proof_message(
            "piq_enr_secret",
            "installation_1",
            "connector-key",
            &["printer-b".into(), "printer-a".into()],
        );
        assert_eq!(
            format!("{:x}", Sha256::digest(message)),
            "0707161834e0f3efdccdb35e9804ea409dcbe9cb5152614cd0f0b4c9fb0cb863"
        );
    }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connector_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ConnectSessionPreviewRequest {
    pub token: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ConnectSessionPreview {
    pub workspace_id: String,
    pub workspace_name: String,
    pub requesting_service_account_id: Option<String>,
    pub requesting_service_name: Option<String>,
    pub authorization_type: String,
    pub environment_id: String,
    pub requested_scopes: Vec<String>,
    pub printer_grant: String,
    pub expires_at: DateTime<Utc>,
    pub return_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CreateDeviceAuthorizationRequest {
    pub public_key: String,
    pub installation_id: String,
    pub proposed_name: String,
    pub hostname: String,
    pub platform: String,
    pub architecture: String,
    pub installation_mode: InstallationMode,
    pub agent_version: String,
    pub protocol_version: u16,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CreatedDeviceAuthorization {
    pub id: String,
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: i64,
    pub interval: u8,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DeviceAuthorizationStatus {
    pub id: String,
    pub state: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DeviceAuthorizationExchange {
    pub node_id: AgentId,
    pub workspace_id: piqae_domain::WorkspaceId,
    pub environment_id: piqae_domain::EnvironmentId,
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
    #[serde(default)]
    pub diagnostics: Vec<DiagnosticReport>,
    /// Bounded, non-secret renderer/cache capability snapshot. The control
    /// plane uses this only to make an auditable delivery decision; the node
    /// still validates every offered manifest and fails closed.
    #[serde(default)]
    pub document_render: DocumentRenderCapabilities,
    /// Additive capability negotiation for optional node features. A missing
    /// value is the legacy v1 feature set.
    #[serde(default)]
    pub capabilities: AgentProtocolCapabilities,
    /// Privacy-minimised live observations for the local OS queues visible to
    /// this authenticated connector. These contain counts, never job titles,
    /// usernames, paths, document data, or native option payloads.
    #[serde(default)]
    pub route_observations: Vec<RouteObservation>,
    /// Bounded installation topology deltas, including removals which cannot
    /// be represented by a current printer snapshot.
    #[serde(default)]
    pub topology_changes: Vec<RouteTopologyChange>,
    /// Evidence emitted after a fenced local handoff. Entries are scoped to
    /// this connector and intentionally omit document metadata.
    #[serde(default)]
    pub native_handoffs: Vec<NativeHandoffEvidence>,
    /// Host lifecycle and execution availability reported by runtimes which
    /// can be embedded in foreground-constrained applications. Missing keeps
    /// the legacy desktop-node admission behaviour during rolling upgrades.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<NodeRuntimeObservation>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentProtocolCapabilities {
    /// Stable identifiers allow independent deployment without relying on a
    /// single monotonically increasing protocol version.
    #[serde(default)]
    pub features: Vec<AgentFeature>,
    #[serde(default)]
    pub telemetry_privacy: TelemetryPrivacy,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentFeature {
    DestinationIdentityV1,
    RouteInventoryV1,
    ProjectionAckV1,
    SpoolerObservationV1,
    RouteFencingV1,
    NativeHandoffEvidenceV1,
    TopologyChangesV1,
    ProfileStockFreshnessV1,
    RouteObservationSequenceV1,
    RouteLeaseRenewalV1,
    AmbiguousHandoffResolutionV1,
    EmbeddedHostV1,
    RuntimeAvailabilityV1,
    WakeHintsV1,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TelemetryPrivacy {
    #[default]
    CountsOnly,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct DocumentRenderCapabilities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub renderer_abi: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_abi: Option<String>,
    #[serde(default)]
    pub persistent_cache: bool,
    #[serde(default)]
    pub font_rendering: bool,
    #[serde(default)]
    pub image_media_types: Vec<String>,
    #[serde(default)]
    pub font_media_types: Vec<String>,
    /// At most 256 lowercase SHA-256 values. This is scoped to the authenticated
    /// tenant/node sync and must never be exposed to another tenant.
    #[serde(default)]
    pub cached_resource_digests: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BusinessDocumentRenderPolicy {
    #[default]
    Automatic,
    CloudOnly,
    PreferNode,
    RequireNode,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BusinessDocumentResourceDescriptor {
    pub digest: String,
    pub media_type: String,
    pub byte_length: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BusinessDocumentNodeRender {
    pub renderer_abi: String,
    pub resource_abi: String,
    pub specification: serde_json::Value,
    pub input: serde_json::Value,
    pub resources: Vec<BusinessDocumentResourceDescriptor>,
    pub expected_pdf_sha256: String,
    pub expected_pdf_bytes: u64,
    /// Authoritative page count produced with the same immutable specification,
    /// input, resources, renderer build, and PDF digest. Nodes use this value as
    /// the render page limit instead of substituting a local guess.
    #[serde(default)]
    pub expected_page_count: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentSyncResponse {
    pub server_time: DateTime<Utc>,
    pub acknowledged_event_cursor: Option<EventId>,
    pub command_cursor: Option<String>,
    pub commands: Vec<AgentCommand>,
    pub candidate_jobs: Vec<JobOffer>,
    pub next_poll_after_ms: u64,
    #[serde(default)]
    pub acknowledged_diagnostics: Vec<String>,
    /// Explicit rolling-upgrade negotiation for inventory projection ACKs.
    /// Missing/false is the legacy V1 contract where a successful sync was
    /// the only acknowledgement available.
    #[serde(default)]
    pub inventory_projection_acknowledgement_supported: bool,
    /// Confirms that the exact connector-scoped inventory revision was
    /// durably projected. Nodes retry inventory until this acknowledgement is
    /// observed; a successful heartbeat alone is not sufficient.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inventory_projection: Option<InventoryProjectionAcknowledgement>,
    /// Highest local handoff evidence sequence durably consumed by the server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acknowledged_handoff_sequence: Option<u64>,
    /// Wake requests are advisory and never carry a job lease or document.
    /// A runtime only claims work after it is authenticated and eligible.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub wake_hints: Vec<AgentWakeHint>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeHostMode {
    MachineService,
    UserAgent,
    EmbeddedApplication,
    AttachedClient,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeAvailabilityClass {
    ContinuousWhileAwake,
    ForegroundOnly,
    BackgroundOpportunistic,
    ManagedKiosk,
    WakeRelayCapable,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeAvailability {
    Available,
    Foreground,
    Background,
    Suspending,
    Suspended,
    Waking,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WakeMechanism {
    LocalBroker,
    ApnsBackground,
    BluetoothAccessory,
    ExternalAccessory,
    WakeOnLan,
    Manual,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NodeRuntimeObservation {
    /// Monotonic per connector so replayed lifecycle observations are
    /// idempotent and out-of-order suspension reports fail closed.
    pub sequence: u64,
    pub host_mode: NodeHostMode,
    pub availability_class: NodeAvailabilityClass,
    pub lifecycle_state: NodeAvailability,
    pub accepts_cloud_jobs: bool,
    pub observed_at: DateTime<Utc>,
    pub fresh_until: DateTime<Utc>,
    /// Remaining operating-system execution budget, when the host can measure
    /// it. Opportunistic background hosts require a bounded positive budget
    /// before the server may offer work.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_budget_ms: Option<u64>,
    #[serde(default)]
    pub wake_mechanisms: Vec<WakeMechanism>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentWakeHint {
    pub id: String,
    pub reason: String,
    /// This hint was observed through the already-authenticated sync session.
    /// It is not evidence that an external push woke the host.
    pub delivery_channel: WakeDeliveryChannel,
    pub requested_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WakeDeliveryChannel {
    ConnectedSession,
    ExternalPush,
    LocalRelay,
    Manual,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InventoryProjectionAcknowledgement {
    pub revision: u64,
    pub projected_at: DateTime<Utc>,
}

/// A deliberately small, structured support snapshot. It contains no logs,
/// paths, document data, credentials, native profile data, or signed URLs.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DiagnosticReport {
    pub request_id: String,
    pub observed_at: DateTime<Utc>,
    pub state: String,
    pub agent_version: String,
    pub platform: String,
    pub architecture: String,
    pub queued_jobs: u32,
    pub active_jobs: u32,
    pub sqlite_integrity_ok: bool,
    pub executor_crashes: u64,
    pub last_error_code: Option<String>,
    pub collection_error_code: Option<String>,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct JobOffer {
    pub job: Job,
    /// Capability revision against which job-scoped options were resolved.
    /// The node must fail closed when its current revision differs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_capability_revision: Option<u64>,
    /// Digest of the immutable display-safe resolved ticket. Executable
    /// options remain integrity-bound in `job.options` (and encrypted v3 AAD).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_ticket_digest: Option<String>,
    pub lease_id: Uuid,
    /// Opaque, single-lease capability. It must never be logged.
    pub lease_token: String,
    pub lease_expires_at: DateTime<Utc>,
    pub content: ContentDescriptor,
    /// Authoritative control-plane route fence for multi-route scheduling.
    /// Older servers omit this and the installation applies its local fence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_reservation: Option<CloudRouteReservation>,
}

impl std::fmt::Debug for JobOffer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("JobOffer")
            .field("job_id", &self.job.id)
            .field(
                "expected_capability_revision",
                &self.expected_capability_revision,
            )
            .field("resolved_ticket_digest", &self.resolved_ticket_digest)
            .field("lease_id", &self.lease_id)
            .field("lease_token", &"[REDACTED]")
            .field("lease_expires_at", &self.lease_expires_at)
            .field("content", &"[REDACTED]")
            .field("route_reservation", &self.route_reservation)
            .finish()
    }
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
pub struct CloudRouteReservation {
    /// Canonical control-plane route resource. This identity may predate the
    /// node's first route-key projection during a rolling upgrade.
    pub route_id: String,
    /// Installation-stable opaque key for this exact OS queue. The node
    /// validates this value; it must not try to derive the server route ID.
    pub local_route_key: String,
    pub reservation_id: Uuid,
    pub generation: u64,
    /// Opaque server capability. It is persisted owner-only, echoed only in
    /// the authenticated control-plane acceptance/handoff protocol, and must
    /// never be logged, returned through operator APIs, or forwarded to a
    /// native executor.
    pub fencing_token: String,
    pub lease_expires_at: DateTime<Utc>,
}

impl std::fmt::Debug for CloudRouteReservation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CloudRouteReservation")
            .field("route_id", &self.route_id)
            .field("local_route_key", &self.local_route_key)
            .field("reservation_id", &self.reservation_id)
            .field("generation", &self.generation)
            .field("fencing_token", &"[REDACTED]")
            .field("lease_expires_at", &self.lease_expires_at)
            .finish()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentDescriptor {
    Download {
        url: String,
        sha256: String,
        bytes: u64,
    },
    EncryptedDownload {
        url: String,
        sha256: String,
        bytes: u64,
        manifest: Box<piqae_domain::EncryptedContentManifest>,
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
    /// Exact node-render input with the already approved server PDF as a
    /// mandatory fallback. `require_node` is represented by `fallback_allowed`
    /// false and must fail closed instead of printing the fallback.
    BusinessDocument {
        policy: BusinessDocumentRenderPolicy,
        render: Box<BusinessDocumentNodeRender>,
        fallback: Box<ContentDescriptor>,
        fallback_allowed: bool,
        decision_reason: String,
    },
}

#[derive(Clone, Deserialize, Serialize)]
pub struct AgentAcceptJobRequest {
    pub lease_id: Uuid,
    pub lease_token: String,
    pub content_sha256: String,
    pub local_sequence: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_reservation_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_generation: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_fencing_token: Option<String>,
}

impl std::fmt::Debug for AgentAcceptJobRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AgentAcceptJobRequest")
            .field("lease_id", &self.lease_id)
            .field("lease_token", &"[REDACTED]")
            .field("content_sha256", &self.content_sha256)
            .field("local_sequence", &self.local_sequence)
            .field("route_reservation_id", &self.route_reservation_id)
            .field("route_generation", &self.route_generation)
            .field("route_fencing_token", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentAcceptJobResponse {
    pub accepted_at: DateTime<Utc>,
    pub state: piqae_domain::JobState,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentAcceptanceReconciliationResponse {
    pub accepted: bool,
    pub connector_revoked: bool,
    pub fenced: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentAcceptanceAbandonResponse {
    pub abandoned: bool,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct AgentRenewLeaseRequest {
    pub lease_id: Uuid,
    pub lease_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_reservation_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_generation: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_fencing_token: Option<String>,
}

impl std::fmt::Debug for AgentRenewLeaseRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AgentRenewLeaseRequest")
            .field("lease_id", &self.lease_id)
            .field("lease_token", &"[REDACTED]")
            .field("route_reservation_id", &self.route_reservation_id)
            .field("route_generation", &self.route_generation)
            .field("route_fencing_token", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentRenewLeaseResponse {
    pub lease_expires_at: DateTime<Utc>,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct AgentReleaseLeaseRequest {
    pub lease_id: Uuid,
    pub lease_token: String,
    pub reason: String,
}

impl std::fmt::Debug for AgentReleaseLeaseRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AgentReleaseLeaseRequest")
            .field("lease_id", &self.lease_id)
            .field("lease_token", &"[REDACTED]")
            .field("reason", &self.reason)
            .finish()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentCommand {
    RefreshPrinters,
    CancelJob {
        job_id: piqae_domain::JobId,
    },
    ResolveAmbiguousHandoff {
        job_id: piqae_domain::JobId,
        local_route_key: String,
        reservation_id: Uuid,
        generation: u64,
        resolution: AmbiguousHandoffResolution,
    },
    Pause,
    Resume,
    UpdateAvailable {
        version: String,
        channel: String,
        metadata_url: String,
    },
    CollectDiagnostics {
        request_id: String,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AmbiguousHandoffResolution {
    /// An operator or authoritative spooler reconciliation proved that native
    /// handoff did not occur, so the job may be offered again.
    ReleaseForRetry,
    /// The uncertain attempt is accepted as delivered/handled and must never
    /// be replayed automatically.
    ConfirmAccepted,
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
    #[serde(default)]
    pub exposed: bool,
    #[serde(default)]
    pub capability_revision: u64,
    #[serde(default)]
    pub native_options: BTreeMap<String, NativePrinterOption>,
    #[serde(default)]
    pub semantic_capabilities: piqae_domain::SemanticPrinterCapabilities,
    #[serde(default)]
    pub profiles: Vec<PrinterProfileSnapshot>,
    /// Installation-wide route identity shared by every connector projection
    /// of this exact OS queue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<PrinterRouteSnapshot>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PrinterRouteSnapshot {
    pub local_route_key: String,
    pub inventory_revision: u64,
    pub topology_revision: u64,
    pub observed_at: DateTime<Utc>,
    #[serde(default)]
    pub identity_evidence: Vec<PhysicalIdentityEvidence>,
    #[serde(default)]
    pub identity_confidence: IdentityConfidence,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topology_change: Option<TopologyChange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_observed_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stock_observed_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PhysicalIdentityEvidenceKind {
    IppPrinterUuid,
    DeviceSerial,
    UsbSerial,
    CertificateKey,
    NetworkMac,
    NetworkEndpoint,
    ManufacturerModel,
    CapabilityFingerprint,
    DriverFingerprint,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PhysicalIdentityEvidence {
    pub kind: PhysicalIdentityEvidenceKind,
    /// Lowercase SHA-256 of the canonical value. Raw serials, MAC addresses,
    /// endpoints, and certificates never leave the node through this field.
    pub value_sha256: String,
    pub strength: IdentityEvidenceStrength,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentityEvidenceStrength {
    Strong,
    Medium,
    Weak,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentityConfidence {
    Verified,
    High,
    Possible,
    Conflict,
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TopologyChange {
    Added,
    Changed,
    Removed,
    Reconciled,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RouteTopologyChange {
    pub local_route_key: String,
    pub topology_revision: u64,
    pub observed_at: DateTime<Utc>,
    pub change: TopologyChange,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RouteObservation {
    pub local_route_key: String,
    pub sequence: u64,
    pub observed_at: DateTime<Utc>,
    pub inventory_revision: u64,
    pub state: PrinterState,
    pub accepts_jobs: bool,
    /// Bounded machine classifications such as `media_empty` or `paused`.
    /// Native free-form driver messages are not allowed here.
    #[serde(default)]
    pub state_reasons: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue: Option<PrivacySafeQueueObservation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_observed_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stock_observed_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct PrivacySafeQueueObservation {
    pub total_jobs: u32,
    pub active_jobs: u32,
    pub held_jobs: u32,
    pub connector_jobs: u32,
    pub other_piqae_or_external_jobs: u32,
    pub unknown_jobs: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeHandoffOutcome {
    Accepted,
    RejectedBeforeHandoff,
    Ambiguous,
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
pub struct NativeHandoffEvidence {
    pub sequence: u64,
    /// Canonical server route when the offer carried a cloud reservation.
    /// Legacy offers can only identify the installation-local route key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_id: Option<String>,
    pub local_route_key: String,
    pub job_id: piqae_domain::JobId,
    pub reservation_id: Uuid,
    pub fencing_generation: u64,
    /// Authenticated reservation proof. Servers must redact this field from
    /// logs and never return it through operator APIs.
    pub fencing_token: String,
    pub observed_at: DateTime<Utc>,
    pub outcome: NativeHandoffOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_job_id: Option<String>,
}

impl std::fmt::Debug for NativeHandoffEvidence {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NativeHandoffEvidence")
            .field("sequence", &self.sequence)
            .field("route_id", &self.route_id)
            .field("local_route_key", &self.local_route_key)
            .field("job_id", &self.job_id)
            .field("reservation_id", &self.reservation_id)
            .field("fencing_generation", &self.fencing_generation)
            .field("fencing_token", &"[REDACTED]")
            .field("observed_at", &self.observed_at)
            .field("outcome", &self.outcome)
            .field("native_job_id", &self.native_job_id)
            .finish()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PrinterProfileSnapshot {
    pub profile_id: String,
    pub revision: u64,
    pub name: String,
    pub is_default: bool,
    pub options: piqae_domain::JobOptions,
    #[serde(default)]
    pub status: ProfileStatus,
    #[serde(default)]
    pub native_kind: Option<NativeProfileKind>,
    #[serde(default)]
    pub native_digest: Option<String>,
    #[serde(default)]
    pub driver_fingerprint: DriverFingerprint,
    #[serde(default)]
    pub summary: ProfileSummary,
    #[serde(default)]
    pub stock_id: Option<String>,
    #[serde(default)]
    pub safe_overrides: Vec<SafeProfileOverride>,
    #[serde(default)]
    pub last_validated_unix_ms: Option<i64>,
    #[serde(default)]
    pub last_test_job_id: Option<String>,
    #[serde(default)]
    pub published: bool,
}

#[cfg(test)]
mod route_protocol_tests {
    use super::*;

    #[test]
    fn legacy_sync_response_does_not_claim_projection_acknowledgements() {
        let Ok(response): Result<AgentSyncResponse, _> =
            serde_json::from_value(serde_json::json!({
            "server_time": "2026-08-26T00:00:00Z",
            "acknowledged_event_cursor": null,
            "command_cursor": null,
            "commands": [],
            "candidate_jobs": [],
            "next_poll_after_ms": 1000
            }))
        else {
            panic!("legacy sync response must deserialize");
        };
        assert!(!response.inventory_projection_acknowledgement_supported);
        assert!(response.inventory_projection.is_none());
    }

    #[test]
    fn legacy_document_offer_without_page_count_fails_closed_additively() {
        let Ok(render): Result<BusinessDocumentNodeRender, _> =
            serde_json::from_value(serde_json::json!({
                "renderer_abi": "piqae.business-document-pdf/v1",
                "resource_abi": "piqae.document-resources/v1",
                "specification": {},
                "input": {},
                "resources": [],
                "expected_pdf_sha256": "a".repeat(64),
                "expected_pdf_bytes": 100
            }))
        else {
            panic!("legacy document offer must remain decodable");
        };
        assert_eq!(render.expected_page_count, 0);
    }

    #[test]
    fn lease_renewal_route_proof_is_additive_and_debug_redacted() {
        let Ok(legacy): Result<AgentRenewLeaseRequest, _> =
            serde_json::from_value(serde_json::json!({
                    "lease_id": Uuid::nil(),
                    "lease_token": "legacy-secret"
            }))
        else {
            panic!("legacy renewal must deserialize");
        };
        assert!(legacy.route_reservation_id.is_none());

        let request = AgentRenewLeaseRequest {
            lease_id: Uuid::nil(),
            lease_token: "lease-secret".into(),
            route_reservation_id: Some(Uuid::nil()),
            route_generation: Some(7),
            route_fencing_token: Some("fence-secret".into()),
        };
        let debug = format!("{request:?}");
        assert!(!debug.contains("lease-secret"));
        assert!(!debug.contains("fence-secret"));
        assert!(debug.contains("[REDACTED]"));
    }

    #[test]
    fn job_offer_and_release_debug_omit_content_and_lease_tokens() {
        let offer = JobOffer {
            job: piqae_domain::Job {
                id: piqae_domain::JobId::new(),
                workspace_id: piqae_domain::WorkspaceId::new(),
                environment_id: piqae_domain::EnvironmentId::new(),
                printer_id: piqae_domain::PrinterId::new(),
                title: "private title".into(),
                source: None,
                content_kind: piqae_domain::ContentKind::Pdf,
                content: piqae_domain::ContentSource::Base64 {
                    data: "private-document".into(),
                },
                options: piqae_domain::JobOptions::default(),
                metadata: std::collections::BTreeMap::new(),
                deliveries: 0,
                state: piqae_domain::JobState::Registered,
                created_at: Utc::now(),
                expires_at: Utc::now(),
                delivery_uncertain_since: None,
            },
            expected_capability_revision: None,
            resolved_ticket_digest: None,
            lease_id: Uuid::nil(),
            lease_token: "lease-secret".into(),
            lease_expires_at: Utc::now(),
            content: ContentDescriptor::InlineBase64 {
                data: "private-document".into(),
                sha256: None,
                bytes: None,
            },
            route_reservation: None,
        };
        let offer_debug = format!("{offer:?}");
        assert!(!offer_debug.contains("lease-secret"));
        assert!(!offer_debug.contains("private-document"));
        assert!(!offer_debug.contains("private title"));

        let release = AgentReleaseLeaseRequest {
            lease_id: Uuid::nil(),
            lease_token: "release-secret".into(),
            reason: "bounded reason".into(),
        };
        let release_debug = format!("{release:?}");
        assert!(!release_debug.contains("release-secret"));
        assert!(release_debug.contains("[REDACTED]"));
    }
}
