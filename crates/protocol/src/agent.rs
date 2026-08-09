use chrono::{DateTime, Utc};
use piqae_domain::{
    AgentId, DriverFingerprint, EventId, Job, JobEvent, NativePrinterOption, NativeProfileKind,
    PrinterCapabilities, PrinterId, PrinterState, ProfileStatus, ProfileSummary,
    SafeProfileOverride, UriAuthentication,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

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

#[derive(Clone, Debug, Deserialize, Serialize)]
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
    pub state: piqae_domain::JobState,
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
        job_id: piqae_domain::JobId,
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
    pub profiles: Vec<PrinterProfileSnapshot>,
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
