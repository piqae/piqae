//! Durable host-adapter queue used by embedded Apple and Windows SDKs.
//!
//! The host pulls one persisted operation and acknowledges exact handoff
//! outcomes. No application callback executes while runtime state is locked.

#![allow(
    clippy::missing_errors_doc,
    clippy::needless_collect,
    clippy::too_many_lines,
    reason = "the public durable boundary is validated fail-closed and restart repair intentionally snapshots journal keys before mutation"
)]

use crate::{CloudCommandApplication, CommandRecoveryLedger};
use anyhow::{Context as _, Result, bail};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::Utc;
use piqae_agent_storage::{
    AcceptedJob, AgentStore, CloudAcceptIntent, CloudRouteProof, LocalJob, StoredNamedProfile,
};
use piqae_domain::{
    AgentId, EventId, JobFailureReason, JobId, JobState, NativePrinterOption, PrinterCapabilities,
    PrinterId, PrinterState,
};
use piqae_protocol::{
    CURRENT_PROTOCOL_VERSION,
    agent::{
        AgentCommand, AgentFeature, AgentHealth, AgentProtocolCapabilities, AgentSyncRequest,
        DocumentRenderCapabilities, JobOffer, PrinterSnapshot, QueueSnapshot, TelemetryPrivacy,
    },
};
use piqae_support_packs::{AdapterFingerprint, Platform, SupportPackRegistry};
use rand::RngCore as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::OpenOptions,
    io::Write as _,
    path::{Path, PathBuf},
};

const DOCUMENT_VERSION: u16 = 1;
const MAX_ADAPTERS: usize = 32;
const MAX_PRINTERS_PER_ADAPTER: usize = 256;
const MAX_ACTIVE_OPERATIONS: usize = 256;
const MAX_COMPLETED_ACKS: usize = 1024;
const MAX_DOCUMENT_BYTES: usize = 16 * 1024 * 1024;
const MAX_CONTENT_STORE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_CONTENT_FILES: usize = 4_096;
const OPERATION_DEADLINE_MS: i64 = 30_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmbeddedAdapterFingerprint {
    pub platform: Platform,
    pub adapter_id: String,
    pub adapter_version: String,
    pub device_family: Option<String>,
    pub firmware_version: Option<String>,
}

impl EmbeddedAdapterFingerprint {
    fn support_pack(&self) -> AdapterFingerprint {
        AdapterFingerprint {
            platform: self.platform,
            adapter_id: self.adapter_id.clone(),
            adapter_version: self.adapter_version.clone(),
            device_family: self.device_family.clone(),
            firmware_version: self.firmware_version.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmbeddedAdapterRegistration {
    pub fingerprint: EmbeddedAdapterFingerprint,
    /// Display-safe declarations such as supported document kinds and
    /// transport constraints. Executable code and credentials are forbidden.
    pub capability_contract: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmbeddedPrinterObservation {
    pub native_id: String,
    pub name: String,
    pub state: String,
    pub is_default: bool,
    #[serde(default)]
    pub native_options: BTreeMap<String, NativePrinterOption>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbeddedPrinterSnapshot {
    pub printer_id: String,
    pub adapter_id: String,
    pub native_id: String,
    pub name: String,
    pub state: String,
    pub is_default: bool,
    pub observed_unix_ms: i64,
    pub semantic_capabilities: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddedJobRequest {
    pub adapter_id: String,
    pub idempotency_key: String,
    pub printer_id: String,
    pub title: String,
    pub content_kind: String,
    pub content: Vec<u8>,
    pub options_json: String,
    pub expires_unix_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbeddedJobAccepted {
    pub job_id: String,
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdapterOperationPhase {
    Claimed,
    HandoffStarted,
    Accepted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterOperation {
    pub operation_id: String,
    pub adapter_id: String,
    /// `local` or the durable connector id whose isolated queue owns the job.
    #[serde(default = "local_queue_scope")]
    pub queue_scope: String,
    pub job_id: String,
    pub idempotency_key: String,
    pub fence: String,
    pub deadline_unix_ms: i64,
    pub printer_id: String,
    pub printer_native_id: String,
    pub title: String,
    pub content_path: String,
    pub content_kind: String,
    pub content_sha256: String,
    pub options_json: String,
    pub phase: AdapterOperationPhase,
    pub native_job_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum AdapterOperationOutcome {
    /// The host guarantees it did not invoke the native printing API.
    RejectedBeforeHandoff {
        code: String,
        retryable: bool,
    },
    /// The native API returned an authoritative identifier. A later terminal
    /// result uses the same operation id and fence.
    Accepted {
        native_job_id: String,
    },
    CompletedReported {
        native_job_id: String,
    },
    FailedTerminal {
        native_job_id: String,
        code: String,
    },
    /// The host cannot prove whether the native API accepted the document.
    Ambiguous {
        code: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterOperationAck {
    pub operation_id: String,
    pub job_id: String,
    pub state: String,
    pub duplicate: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DurableAdapter {
    registration: EmbeddedAdapterRegistration,
    printers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CompletedAck {
    adapter_id: String,
    #[serde(default = "local_queue_scope")]
    queue_scope: String,
    fence_sha256: String,
    outcome_sha256: String,
    ack: AdapterOperationAck,
    outcome: AdapterOperationOutcome,
    completed_unix_ms: i64,
}

#[derive(Debug, Serialize, Deserialize)]
struct EmbeddedDocument {
    version: u16,
    adapters: BTreeMap<String, DurableAdapter>,
    #[serde(default)]
    connector_scopes: BTreeSet<String>,
    operations: BTreeMap<String, AdapterOperation>,
    completed: BTreeMap<String, CompletedAck>,
}

pub struct EmbeddedQueue {
    root: PathBuf,
    store: AgentStore,
    connector_stores: BTreeMap<String, AgentStore>,
    support_packs: SupportPackRegistry,
    document: EmbeddedDocument,
}

impl std::fmt::Debug for EmbeddedQueue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EmbeddedQueue")
            .field("root", &self.root)
            .field("adapters", &self.document.adapters.len())
            .field("active_operations", &self.document.operations.len())
            .finish_non_exhaustive()
    }
}

impl EmbeddedQueue {
    /// Opens the application-scoped queue and replays unresolved operations.
    pub fn open(root: impl AsRef<Path>, support_packs: SupportPackRegistry) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        std::fs::create_dir_all(root.join("content"))?;
        let path = root.join("embedded-runtime.json");
        let document = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice::<EmbeddedDocument>(&bytes)
                .with_context(|| format!("parse {}", path.display()))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => EmbeddedDocument {
                version: DOCUMENT_VERSION,
                adapters: BTreeMap::new(),
                connector_scopes: BTreeSet::new(),
                operations: BTreeMap::new(),
                completed: BTreeMap::new(),
            },
            Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
        };
        validate_document(&document)?;
        let store = AgentStore::open(root.join("embedded.sqlite3"))?;
        let mut queue = Self {
            root,
            store,
            connector_stores: BTreeMap::new(),
            support_packs,
            document,
        };
        queue.reconcile_content_for_scope("local")?;
        queue.open_persisted_connector_scopes()?;
        queue.expire_waiting_across_scopes(Utc::now().timestamp_millis())?;
        queue.repair_after_restart()?;
        Ok(queue)
    }

    pub fn register_adapter(&mut self, registration: EmbeddedAdapterRegistration) -> Result<()> {
        validate_adapter(&registration)?;
        let adapter_id = registration.fingerprint.adapter_id.clone();
        if !self.document.adapters.contains_key(&adapter_id)
            && self.document.adapters.len() >= MAX_ADAPTERS
        {
            bail!("adapter limit reached");
        }
        let printers = self
            .document
            .adapters
            .get(&adapter_id)
            .map_or_else(BTreeMap::new, |adapter| adapter.printers.clone());
        self.document.adapters.insert(
            adapter_id,
            DurableAdapter {
                registration,
                printers,
            },
        );
        self.persist()
    }

    pub fn observe_inventory(
        &mut self,
        adapter_id: &str,
        printers: &[EmbeddedPrinterObservation],
    ) -> Result<Vec<EmbeddedPrinterSnapshot>> {
        if printers.len() > MAX_PRINTERS_PER_ADAPTER {
            bail!("printer observation exceeds supported bounds");
        }
        let registration = self
            .document
            .adapters
            .get(adapter_id)
            .context("adapter is not registered")?
            .registration
            .clone();
        let mut seen = BTreeSet::new();
        let mut snapshots = Vec::with_capacity(printers.len());
        let observed = Utc::now().timestamp_millis();
        for printer in printers {
            validate_printer(printer)?;
            if !seen.insert(printer.native_id.clone()) {
                bail!("printer observation contains a duplicate native id");
            }
            let semantic = self
                .support_packs
                .normalize_adapter(
                    &registration.fingerprint.support_pack(),
                    &printer.native_options,
                )
                .context("normalize adapter capabilities")?;
            let capabilities_json = serde_json::to_string(&serde_json::json!({
                "semantic": semantic,
                "native_options": printer.native_options,
                "adapter": registration.fingerprint,
            }))?;
            let stored = self.store.upsert_printer(
                &scoped_native_id(adapter_id, &printer.native_id),
                &printer.name,
                &printer.state,
                printer.is_default,
                &capabilities_json,
                observed,
            )?;
            snapshots.push(EmbeddedPrinterSnapshot {
                printer_id: stored.printer_id,
                adapter_id: adapter_id.to_owned(),
                native_id: printer.native_id.clone(),
                name: printer.name.clone(),
                state: printer.state.clone(),
                is_default: printer.is_default,
                observed_unix_ms: observed,
                semantic_capabilities: serde_json::to_value(semantic)?,
            });
        }
        let new_printers = snapshots
            .iter()
            .map(|printer| (printer.native_id.clone(), printer.printer_id.clone()))
            .collect();
        self.document
            .adapters
            .get_mut(adapter_id)
            .context("adapter disappeared during observation")?
            .printers = new_printers;
        let all_present = self
            .document
            .adapters
            .iter()
            .flat_map(|(id, adapter)| {
                adapter
                    .printers
                    .keys()
                    .map(move |native| scoped_native_id(id, native))
            })
            .collect::<Vec<_>>();
        self.store.reconcile_printer_presence(&all_present)?;
        self.persist()?;
        Ok(snapshots)
    }

    pub fn printer_snapshots(&self) -> Result<Vec<EmbeddedPrinterSnapshot>> {
        let mut snapshots = Vec::new();
        for (adapter_id, adapter) in &self.document.adapters {
            for (native_id, printer_id) in &adapter.printers {
                let Some(printer) = self.store.printer(printer_id)? else {
                    continue;
                };
                let capabilities: serde_json::Value =
                    serde_json::from_str(&printer.capabilities_json)?;
                snapshots.push(EmbeddedPrinterSnapshot {
                    printer_id: printer.printer_id,
                    adapter_id: adapter_id.clone(),
                    native_id: native_id.clone(),
                    name: printer.name,
                    state: printer.state,
                    is_default: printer.is_default,
                    observed_unix_ms: printer.observed_unix_ms,
                    semantic_capabilities: capabilities
                        .get("semantic")
                        .cloned()
                        .unwrap_or_else(|| serde_json::json!({})),
                });
            }
        }
        Ok(snapshots)
    }

    /// Opens the durable queue projection for one authenticated connector.
    /// Each connector owns its cursors, outbox, leases and job rows while the
    /// adapter operation journal remains installation-wide.
    pub fn ensure_connector_queue(&mut self, connector_id: &str) -> Result<()> {
        validate_queue_scope(connector_id)?;
        if self.connector_stores.contains_key(connector_id) {
            return Ok(());
        }
        if self.document.connector_scopes.len() >= 128 {
            bail!("embedded connector queue limit reached");
        }
        let root = self.root.join("connectors").join(connector_id);
        std::fs::create_dir_all(&root)?;
        let store = AgentStore::open(root.join("agent.sqlite3"))?;
        reconcile_content_root(&store, &root.join("content"))?;
        self.document
            .connector_scopes
            .insert(connector_id.to_owned());
        if let Err(error) = self.persist() {
            self.document.connector_scopes.remove(connector_id);
            return Err(error);
        }
        self.connector_stores.insert(connector_id.to_owned(), store);
        Ok(())
    }

    pub fn connector_sync_snapshot(
        &mut self,
        connector_id: &str,
        agent_id: AgentId,
        started_at: chrono::DateTime<Utc>,
        refresh_inventory: bool,
        allowed_printer_ids: Option<&BTreeSet<String>>,
        runtime: &crate::NodeRuntime,
    ) -> Result<AgentSyncRequest> {
        self.ensure_connector_queue(connector_id)?;
        self.store_for_scope_mut(connector_id)?
            .expire_waiting(Utc::now().timestamp_millis())?;
        let printers = if refresh_inventory {
            let snapshots = self
                .printer_snapshots()?
                .into_iter()
                .filter(|printer| {
                    allowed_printer_ids.is_none_or(|allowed| allowed.contains(&printer.printer_id))
                })
                .map(embedded_protocol_printer)
                .collect::<Result<Vec<_>>>()?;
            Some(snapshots)
        } else {
            None
        };
        let store = self.store_for_scope_mut(connector_id)?;
        let revision = store
            .setting("printer_inventory_revision")?
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0);
        let printer_revision = if refresh_inventory {
            let next = revision.saturating_add(1);
            store.set_setting("printer_inventory_revision", &next.to_string())?;
            next
        } else {
            revision
        };
        let runtime_sequence = store
            .setting("runtime_observation_sequence")?
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0)
            .checked_add(1)
            .context("embedded runtime observation sequence exhausted")?;
        store.set_setting(
            "runtime_observation_sequence",
            &runtime_sequence.to_string(),
        )?;
        let counts = store.queue_counts()?;
        let events = store
            .pending_cloud_events(0, 100)?
            .into_iter()
            .map(|event| embedded_protocol_event(agent_id, event))
            .collect::<Result<Vec<_>>>()?;
        let event_cursor = events.last().map(|event| event.id);
        let (executor_crashes, last_error_code) = store.failure_health()?;
        Ok(AgentSyncRequest {
            agent_id,
            protocol_version: CURRENT_PROTOCOL_VERSION,
            agent_version: env!("CARGO_PKG_VERSION").into(),
            printer_revision,
            acknowledged_command_cursor: store.setting("command_cursor")?,
            event_cursor,
            queue: QueueSnapshot {
                queued_jobs: counts.queued,
                active_jobs: counts.active,
                content_bytes: 0,
                accepts_jobs: runtime.snapshot().accepting_cloud_leases,
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
            diagnostics: Vec::new(),
            document_render: DocumentRenderCapabilities::default(),
            capabilities: AgentProtocolCapabilities {
                features: vec![
                    AgentFeature::EmbeddedHostV1,
                    AgentFeature::RuntimeAvailabilityV1,
                ],
                telemetry_privacy: TelemetryPrivacy::CountsOnly,
            },
            route_observations: Vec::new(),
            topology_changes: Vec::new(),
            native_handoffs: Vec::new(),
            runtime: Some(runtime.observation(
                runtime_sequence,
                Utc::now(),
                std::time::Duration::from_secs(90),
            )),
        })
    }

    pub fn acknowledge_connector_response(
        &mut self,
        connector_id: &str,
        event_cursor: Option<EventId>,
        command_cursor: Option<&str>,
    ) -> Result<()> {
        let store = self.store_for_scope_mut(connector_id)?;
        if let Some(cursor) = event_cursor {
            store.acknowledge_cloud_event(&cursor.to_string(), Utc::now().timestamp_millis())?;
        }
        if let Some(cursor) = command_cursor {
            store.set_setting("command_cursor", cursor)?;
        }
        Ok(())
    }

    pub fn apply_connector_commands(
        &mut self,
        connector_id: &str,
        commands: &[AgentCommand],
    ) -> Result<()> {
        let store = self.store_for_scope_mut(connector_id)?;
        for command in commands {
            match command {
                AgentCommand::CancelJob { job_id } => {
                    store.request_cancel(&job_id.to_string(), Utc::now().timestamp_millis())?;
                }
                AgentCommand::Pause => store.set_setting("paused", "true")?,
                AgentCommand::Resume => store.set_setting("paused", "false")?,
                AgentCommand::RefreshPrinters | AgentCommand::UpdateAvailable { .. } => {}
                AgentCommand::CollectDiagnostics { .. }
                | AgentCommand::ResolveAmbiguousHandoff { .. } => {
                    bail!("embedded connector command requires an unsupported host capability")
                }
            }
        }
        Ok(())
    }

    /// Applies every due command independently and persists bounded retry
    /// metadata so one malformed or transient command cannot starve siblings.
    pub fn apply_connector_commands_recovering(
        &mut self,
        connector_id: &str,
        command_cursor: Option<&str>,
        commands: &[AgentCommand],
    ) -> Result<CloudCommandApplication> {
        let store = self.store_for_scope_mut(connector_id)?;
        let now = Utc::now().timestamp_millis();
        let mut ledger = CommandRecoveryLedger::load(store)?;
        ledger.retain_batch(commands)?;
        let mut attempted_failure = false;
        for command in commands {
            let key = CommandRecoveryLedger::key(command)?;
            if ledger.is_applied(&key) || !ledger.is_due(&key, now) {
                continue;
            }
            let result = match command {
                AgentCommand::CancelJob { job_id } => store
                    .request_cancel(&job_id.to_string(), now)
                    .map(|_| ())
                    .map_err(anyhow::Error::from),
                AgentCommand::Pause => store
                    .set_setting("paused", "true")
                    .map_err(anyhow::Error::from),
                AgentCommand::Resume => store
                    .set_setting("paused", "false")
                    .map_err(anyhow::Error::from),
                AgentCommand::RefreshPrinters | AgentCommand::UpdateAvailable { .. } => Ok(()),
                AgentCommand::CollectDiagnostics { .. }
                | AgentCommand::ResolveAmbiguousHandoff { .. } => {
                    Err(anyhow::anyhow!("unsupported embedded host command"))
                }
            };
            match result {
                Ok(()) => ledger.record_applied(key),
                Err(error) => {
                    let code = if error
                        .downcast_ref::<piqae_agent_storage::StorageError>()
                        .is_some_and(|error| {
                            matches!(error, piqae_agent_storage::StorageError::JobNotFound(_))
                        }) {
                        "local_job_absent_unproved"
                    } else {
                        "embedded_command_retry"
                    };
                    ledger.record_retry(key, now, code);
                    attempted_failure = true;
                }
            }
            ledger.persist(store)?;
        }
        if ledger.complete() {
            if let Some(cursor) = command_cursor {
                store.set_setting("command_cursor", cursor)?;
            }
            ledger.clear(store)?;
            return Ok(CloudCommandApplication::complete());
        }
        Ok(CloudCommandApplication {
            retry_after: ledger.retry_after(now),
            attempted_failure,
        })
    }

    pub fn pending_connector_accepts(&self, connector_id: &str) -> Result<Vec<CloudAcceptIntent>> {
        self.store_for_scope(connector_id)?
            .pending_cloud_accepts()
            .map_err(Into::into)
    }

    pub fn prepare_connector_offer(
        &mut self,
        connector_id: &str,
        offer: &JobOffer,
        content: &[u8],
        allowed_printer_ids: Option<&BTreeSet<String>>,
    ) -> Result<CloudAcceptIntent> {
        self.ensure_connector_queue(connector_id)?;
        let printer_id = offer.job.printer_id.to_string();
        if !allowed_printer_ids.is_none_or(|allowed| allowed.contains(&printer_id)) {
            bail!("offered printer is outside the connector grant");
        }
        let snapshot = self
            .printer_snapshots()?
            .into_iter()
            .find(|printer| printer.printer_id == printer_id)
            .context("offered embedded printer is not present")?;
        let reservation = offer
            .route_reservation
            .as_ref()
            .context("embedded cloud offer has no route reservation")?;
        let route_proof = CloudRouteProof {
            reservation_id: reservation.reservation_id.to_string(),
            generation: reservation.generation,
            fencing_token: reservation.fencing_token.clone(),
        };
        let digest = hex::encode(Sha256::digest(content));
        let expected = offer_content_digest(offer);
        if expected.is_some_and(|expected| expected != digest) {
            bail!("offered embedded content digest does not match");
        }
        let path = self.persist_content_for_scope(connector_id, &digest, content)?;
        let prepared = self.store_for_scope_mut(connector_id)?.prepare_cloud_job(
            &AcceptedJob {
                job_id: offer.job.id.to_string(),
                submission_id: format!("sub_{}", offer.job.id),
                printer_id,
                printer_native_id: scoped_native_id(&snapshot.adapter_id, &snapshot.native_id),
                title: offer.job.title.clone(),
                content_sha256: digest.clone(),
                content_path: path.to_string_lossy().into_owned(),
                content_kind: match offer.job.content_kind {
                    piqae_domain::ContentKind::Pdf => "pdf",
                    piqae_domain::ContentKind::Raw => "raw",
                }
                .into(),
                options_json: serde_json::to_string(&offer.job.options)?,
                expires_unix_ms: Some(offer.job.expires_at.timestamp_millis()),
                accepted_unix_ms: Utc::now().timestamp_millis(),
                cloud_managed: true,
            },
            &offer.lease_id.to_string(),
            &offer.lease_token,
            offer.lease_expires_at.timestamp_millis(),
            &route_proof,
        );
        let local = match prepared {
            Ok(local) => local,
            Err(error) => {
                self.reconcile_content_for_scope(connector_id)?;
                return Err(error.into());
            }
        };
        Ok(CloudAcceptIntent {
            job_id: offer.job.id.to_string(),
            lease_id: offer.lease_id.to_string(),
            lease_token: offer.lease_token.clone(),
            lease_expires_unix_ms: offer.lease_expires_at.timestamp_millis(),
            content_sha256: digest,
            local_sequence: u64::try_from(local.printer_sequence).unwrap_or(u64::MAX),
            route_reservation_id: Some(route_proof.reservation_id),
            route_generation: Some(route_proof.generation),
            route_fencing_token: Some(route_proof.fencing_token),
            remote_accept_confirmed: false,
        })
    }

    pub fn confirm_connector_offer(&mut self, connector_id: &str, job_id: JobId) -> Result<()> {
        self.store_for_scope_mut(connector_id)?
            .confirm_cloud_accept(&job_id.to_string(), Utc::now().timestamp_millis())?;
        Ok(())
    }

    pub fn quarantine_invalid_connector_offers(
        &mut self,
        connector_id: &str,
    ) -> Result<Vec<piqae_agent_storage::CloudReleaseCleanup>> {
        Ok(self
            .store_for_scope_mut(connector_id)?
            .quarantine_invalid_cloud_accepts(Utc::now().timestamp_millis())?)
    }

    pub fn complete_connector_release_cleanup(
        &mut self,
        connector_id: &str,
        job_id: JobId,
    ) -> Result<()> {
        self.store_for_scope_mut(connector_id)?
            .complete_cloud_release_cleanup(&job_id.to_string())?;
        Ok(())
    }

    /// Resolves proofless quarantine rows only after the authority has durably
    /// revoked the connector, which is their terminal remote compensation.
    ///
    /// # Errors
    ///
    /// Returns an error if the connector queue cannot be opened or updated.
    pub fn complete_all_connector_release_cleanups(&mut self, connector_id: &str) -> Result<()> {
        self.store_for_scope_mut(connector_id)?
            .complete_all_cloud_release_cleanups()?;
        Ok(())
    }

    pub fn activate_connector_offer(&mut self, connector_id: &str, job_id: JobId) -> Result<()> {
        self.store_for_scope_mut(connector_id)?
            .activate_cloud_job(&job_id.to_string(), Utc::now().timestamp_millis())?;
        Ok(())
    }

    pub fn abandon_connector_offer(&mut self, connector_id: &str, job_id: JobId) -> Result<()> {
        self.store_for_scope_mut(connector_id)?
            .abandon_cloud_accept(&job_id.to_string(), Utc::now().timestamp_millis())?;
        Ok(())
    }

    pub fn enqueue(&mut self, request: EmbeddedJobRequest) -> Result<EmbeddedJobAccepted> {
        validate_job_request(&request)?;
        let adapter = self
            .document
            .adapters
            .get(&request.adapter_id)
            .context("adapter is not registered")?;
        let native_id = adapter
            .printers
            .iter()
            .find_map(|(native, logical)| (logical == &request.printer_id).then(|| native.clone()))
            .context("printer is not owned by the requested adapter")?;
        let job_id = stable_job_id(&request.idempotency_key);
        let digest = hex::encode(Sha256::digest(&request.content));
        let content_path = self.persist_content(&digest, &request.content)?;
        let accepted = self.store.accept_job(&AcceptedJob {
            job_id: job_id.clone(),
            submission_id: request.idempotency_key,
            printer_id: request.printer_id,
            printer_native_id: scoped_native_id(&request.adapter_id, &native_id),
            title: request.title,
            content_sha256: digest,
            content_path: content_path.to_string_lossy().into_owned(),
            content_kind: request.content_kind,
            options_json: request.options_json,
            expires_unix_ms: request.expires_unix_ms,
            accepted_unix_ms: Utc::now().timestamp_millis(),
            cloud_managed: false,
        });
        let accepted = match accepted {
            Ok(accepted) => accepted,
            Err(error) => {
                self.reconcile_content_for_scope("local")?;
                return Err(error.into());
            }
        };
        Ok(EmbeddedJobAccepted {
            job_id,
            state: accepted.state,
        })
    }

    /// Returns the same unresolved operation after restart. A new fence is
    /// generated only after an explicit pre-handoff rejection.
    pub fn next_operation(&mut self, adapter_id: &str) -> Result<Option<AdapterOperation>> {
        if !self.document.adapters.contains_key(adapter_id) {
            bail!("adapter is not registered");
        }
        self.expire_waiting_across_scopes(Utc::now().timestamp_millis())?;
        if let Some(operation) = self.document.operations.values().find(|operation| {
            operation.adapter_id == adapter_id && operation.phase != AdapterOperationPhase::Accepted
        }) {
            return Ok(Some(operation.clone()));
        }
        if self.document.operations.len() >= MAX_ACTIVE_OPERATIONS {
            bail!("active adapter operation limit reached");
        }
        let now = Utc::now().timestamp_millis();
        let Some(job) = self
            .runnable_jobs_across_scopes(now)?
            .into_iter()
            .find(|(_, job)| self.adapter_owns_job(adapter_id, job))
        else {
            return Ok(None);
        };
        let (queue_scope, job) = job;
        let operation = AdapterOperation {
            operation_id: random_operation_id(),
            adapter_id: adapter_id.to_owned(),
            queue_scope,
            job_id: job.job_id.clone(),
            idempotency_key: job.submission_id.clone(),
            fence: random_fence(),
            deadline_unix_ms: now.saturating_add(OPERATION_DEADLINE_MS),
            printer_id: job.printer_id.clone(),
            printer_native_id: unscoped_native_id(adapter_id, &job.printer_native_id)?,
            title: job.title.clone(),
            content_path: job.content_path.clone(),
            content_kind: job.content_kind.clone(),
            content_sha256: job.content_sha256.clone(),
            options_json: job.options_json.clone(),
            phase: AdapterOperationPhase::Claimed,
            native_job_id: None,
        };
        self.document
            .operations
            .insert(operation.operation_id.clone(), operation.clone());
        self.persist()?;
        // The persisted operation is the spool intent. A crash before this
        // event simply replays the exact operation and fence.
        let store = self.store_for_scope_mut(&operation.queue_scope)?;
        store.set_setting(
            &adapter_operation_setting(&operation.job_id),
            &operation.operation_id,
        )?;
        store.append_next_event(
            &EventId::new().to_string(),
            &job.job_id,
            "spool_intent",
            None,
            Some("Embedded adapter operation persisted before exposure"),
            "{}",
            now,
        )?;
        Ok(Some(operation))
    }

    /// Returns the accepted native handoffs which need status observation.
    /// Results are installation-owned, adapter-scoped, stable ordered and
    /// bounded by the active-operation journal limit.
    pub fn adapter_observations(&self, adapter_id: &str) -> Result<Vec<AdapterOperation>> {
        if !self.document.adapters.contains_key(adapter_id) {
            bail!("adapter is not registered");
        }
        let mut operations = self
            .document
            .operations
            .values()
            .filter(|operation| {
                operation.adapter_id == adapter_id
                    && operation.phase == AdapterOperationPhase::Accepted
            })
            .cloned()
            .collect::<Vec<_>>();
        operations.sort_by(|left, right| left.operation_id.cmp(&right.operation_id));
        operations.truncate(MAX_ACTIVE_OPERATIONS);
        Ok(operations)
    }

    /// Proves whether any adapter in this installation has work the host can
    /// drain now. Accepted native handoffs are deliberately excluded: their
    /// bounded status observation is independent from the edge-triggered
    /// runnable-work signal.
    pub fn has_runnable_adapter_work(&self) -> Result<bool> {
        if self
            .document
            .operations
            .values()
            .any(|operation| operation.phase != AdapterOperationPhase::Accepted)
        {
            return Ok(true);
        }
        Ok(!self
            .runnable_jobs_across_scopes(Utc::now().timestamp_millis())?
            .is_empty())
    }

    /// Durably records that the host is about to call a native printing API.
    /// After this succeeds the operation is never eligible for resubmission;
    /// restart recovery requires status reconciliation or an ambiguous result.
    pub fn begin_handoff(
        &mut self,
        adapter_id: &str,
        operation_id: &str,
        fence: &str,
    ) -> Result<AdapterOperation> {
        let operation = self
            .document
            .operations
            .get(operation_id)
            .cloned()
            .context("adapter operation was not found")?;
        verify_operation(&operation, adapter_id, fence)?;
        if operation.phase == AdapterOperationPhase::Claimed {
            if operation.deadline_unix_ms <= Utc::now().timestamp_millis() {
                bail!("adapter operation deadline elapsed before handoff");
            }
            self.document
                .operations
                .get_mut(operation_id)
                .context("adapter operation disappeared")?
                .phase = AdapterOperationPhase::HandoffStarted;
            self.persist()?;
        }
        self.document
            .operations
            .get(operation_id)
            .cloned()
            .context("adapter operation disappeared")
    }

    pub fn complete_operation(
        &mut self,
        adapter_id: &str,
        operation_id: &str,
        fence: &str,
        outcome: &AdapterOperationOutcome,
    ) -> Result<AdapterOperationAck> {
        let outcome_sha256 = json_digest(outcome)?;
        if let Some(completed) = self.document.completed.get(operation_id).cloned() {
            if completed.adapter_id != adapter_id
                || !constant_time_eq(&completed.fence_sha256, &token_digest(fence))
                || !constant_time_eq(&completed.outcome_sha256, &outcome_sha256)
            {
                bail!("stale or mismatched adapter completion");
            }
            self.apply_completed_to_store(&completed)?;
            let mut ack = completed.ack;
            ack.duplicate = true;
            return Ok(ack);
        }
        let operation = self
            .document
            .operations
            .get(operation_id)
            .cloned()
            .context("adapter operation was not found")?;
        verify_operation(&operation, adapter_id, fence)?;
        let now = Utc::now().timestamp_millis();
        let state = match outcome {
            AdapterOperationOutcome::Accepted { native_job_id } => {
                validate_native_job_id(native_job_id)?;
                if operation.phase == AdapterOperationPhase::Accepted {
                    if operation.native_job_id.as_deref() != Some(native_job_id) {
                        bail!("accepted operation native id changed");
                    }
                    return Ok(AdapterOperationAck {
                        operation_id: operation_id.to_owned(),
                        job_id: operation.job_id,
                        state: "accepted_by_spooler".into(),
                        duplicate: true,
                    });
                }
                if operation.phase != AdapterOperationPhase::HandoffStarted {
                    bail!("native acceptance requires a durable handoff-start acknowledgement");
                }
                let active = self
                    .document
                    .operations
                    .get_mut(operation_id)
                    .context("adapter operation disappeared")?;
                active.phase = AdapterOperationPhase::Accepted;
                active.native_job_id = Some(native_job_id.clone());
                // Persist the no-replay barrier before updating the secondary
                // queue projection. Startup repair completes the projection.
                self.persist()?;
                self.repair_active_operation(operation_id)?;
                return Ok(AdapterOperationAck {
                    operation_id: operation_id.to_owned(),
                    job_id: operation.job_id,
                    state: "accepted_by_spooler".into(),
                    duplicate: false,
                });
            }
            AdapterOperationOutcome::CompletedReported { native_job_id } => {
                require_accepted_native_id(&operation, native_job_id)?;
                "completed_reported"
            }
            AdapterOperationOutcome::FailedTerminal {
                native_job_id,
                code,
            } => {
                require_accepted_native_id(&operation, native_job_id)?;
                validate_code(code)?;
                "failed_terminal"
            }
            AdapterOperationOutcome::Ambiguous { code } => {
                validate_code(code)?;
                if operation.phase == AdapterOperationPhase::Claimed {
                    bail!("ambiguous result requires a durable handoff-start acknowledgement");
                }
                "delivery_uncertain"
            }
            AdapterOperationOutcome::RejectedBeforeHandoff { code, retryable } => {
                if operation.phase != AdapterOperationPhase::Claimed {
                    bail!("cannot reject an operation after native acceptance");
                }
                validate_code(code)?;
                if *retryable {
                    "failed_retryable"
                } else {
                    "failed_terminal"
                }
            }
        };
        let ack = AdapterOperationAck {
            operation_id: operation_id.to_owned(),
            job_id: operation.job_id,
            state: state.into(),
            duplicate: false,
        };
        let completed = CompletedAck {
            adapter_id: adapter_id.to_owned(),
            queue_scope: operation.queue_scope,
            fence_sha256: token_digest(fence),
            outcome_sha256,
            ack: ack.clone(),
            outcome: outcome.clone(),
            completed_unix_ms: now,
        };
        self.document.operations.remove(operation_id);
        self.document
            .completed
            .insert(operation_id.to_owned(), completed.clone());
        while self.document.completed.len() > MAX_COMPLETED_ACKS {
            let Some(oldest) = self
                .document
                .completed
                .iter()
                .min_by_key(|(_, completed)| completed.completed_unix_ms)
                .map(|(id, _)| id.clone())
            else {
                break;
            };
            self.document.completed.remove(&oldest);
        }
        // Persist the terminal no-replay barrier before updating `SQLite`.
        // A restart repairs the projection from this acknowledgement.
        self.persist()?;
        self.apply_completed_to_store(&completed)?;
        Ok(ack)
    }

    pub fn job(&self, job_id: &str) -> Result<Option<LocalJob>> {
        if let Some(job) = self.store.get_job(job_id)? {
            return Ok(Some(job));
        }
        for store in self.connector_stores.values() {
            if let Some(job) = store.get_job(job_id)? {
                return Ok(Some(job));
            }
        }
        Ok(None)
    }

    /// Lists retained jobs across the local queue and every isolated connector
    /// scope. Job IDs are time-sortable and globally unique, so the merged
    /// newest-first view does not reveal connector ownership or need a second
    /// history database in an SDK shell.
    pub fn job_history(&self, offset: usize, limit: usize) -> Result<(Vec<LocalJob>, bool)> {
        let limit = limit.clamp(1, 200);
        let fetch = offset.saturating_add(limit).saturating_add(1).min(10_201);
        let mut jobs = self.store.local_job_history(0, fetch)?;
        for store in self.connector_stores.values() {
            jobs.extend(store.local_job_history(0, fetch)?);
        }
        jobs.sort_by(|left, right| right.job_id.cmp(&left.job_id));
        jobs.dedup_by(|left, right| left.job_id == right.job_id);
        let mut page = jobs
            .into_iter()
            .skip(offset)
            .take(limit + 1)
            .collect::<Vec<_>>();
        let has_more = page.len() > limit;
        page.truncate(limit);
        Ok((page, has_more))
    }

    pub fn profiles(&self, printer_id: &str) -> Result<Vec<StoredNamedProfile>> {
        self.store.named_profiles(printer_id).map_err(Into::into)
    }

    pub fn create_profile(
        &mut self,
        printer_id: &str,
        name: &str,
        is_default: bool,
        options_json: &str,
    ) -> Result<StoredNamedProfile> {
        self.store
            .create_named_profile(
                printer_id,
                name,
                is_default,
                options_json,
                Utc::now().timestamp_millis(),
            )
            .map_err(Into::into)
    }

    pub fn update_profile(
        &mut self,
        printer_id: &str,
        profile_id: &str,
        expected_revision: u64,
        name: &str,
        is_default: bool,
        options_json: &str,
    ) -> Result<StoredNamedProfile> {
        self.store
            .update_named_profile(
                printer_id,
                profile_id,
                expected_revision,
                name,
                is_default,
                options_json,
                Utc::now().timestamp_millis(),
            )
            .map_err(Into::into)
    }

    pub fn delete_profile(
        &mut self,
        printer_id: &str,
        profile_id: &str,
        expected_revision: u64,
    ) -> Result<()> {
        self.store
            .delete_named_profile(
                printer_id,
                profile_id,
                expected_revision,
                Utc::now().timestamp_millis(),
            )
            .map_err(Into::into)
    }

    fn adapter_owns_job(&self, adapter_id: &str, job: &LocalJob) -> bool {
        self.document
            .adapters
            .get(adapter_id)
            .is_some_and(|adapter| adapter.printers.values().any(|id| id == &job.printer_id))
    }

    fn runnable_jobs_across_scopes(&self, now: i64) -> Result<Vec<(String, LocalJob)>> {
        let mut jobs = self
            .store
            .runnable_heads(now)?
            .into_iter()
            .map(|job| (local_queue_scope(), job))
            .collect::<Vec<_>>();
        for (connector_id, store) in &self.connector_stores {
            jobs.extend(
                store
                    .runnable_heads(now)?
                    .into_iter()
                    .map(|job| (connector_id.clone(), job)),
            );
        }
        jobs.sort_by(|left, right| {
            left.1
                .printer_sequence
                .cmp(&right.1.printer_sequence)
                .then_with(|| left.0.cmp(&right.0))
        });
        Ok(jobs)
    }

    fn expire_waiting_across_scopes(&mut self, now: i64) -> Result<usize> {
        let mut expired = self.store.expire_waiting(now)?;
        for store in self.connector_stores.values_mut() {
            expired = expired.saturating_add(store.expire_waiting(now)?);
        }
        Ok(expired)
    }

    fn store_for_scope_mut(&mut self, scope: &str) -> Result<&mut AgentStore> {
        if scope == "local" {
            return Ok(&mut self.store);
        }
        self.connector_stores
            .get_mut(scope)
            .context("connector queue scope is not active")
    }

    fn store_for_scope(&self, scope: &str) -> Result<&AgentStore> {
        if scope == "local" {
            return Ok(&self.store);
        }
        self.connector_stores
            .get(scope)
            .context("connector queue scope is not active")
    }

    fn persist_content(&mut self, digest: &str, content: &[u8]) -> Result<PathBuf> {
        self.persist_content_for_scope("local", digest, content)
    }

    fn persist_content_for_scope(
        &mut self,
        queue_scope: &str,
        digest: &str,
        content: &[u8],
    ) -> Result<PathBuf> {
        let content_root = if queue_scope == "local" {
            self.root.join("content")
        } else {
            validate_queue_scope(queue_scope)?;
            self.root
                .join("connectors")
                .join(queue_scope)
                .join("content")
        };
        std::fs::create_dir_all(&content_root)?;
        let path = content_root.join(format!("{digest}.bin"));
        let additional = u64::try_from(content.len()).unwrap_or(u64::MAX);
        if !path.exists() {
            self.reclaim_content_until_available(queue_scope, &content_root, additional)?;
        }
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                file.write_all(content)?;
                file.sync_all()?;
                #[cfg(unix)]
                std::fs::File::open(&content_root)?.sync_all()?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let existing = std::fs::read(&path)?;
                if Sha256::digest(existing) != Sha256::digest(content) {
                    bail!("content digest collision");
                }
            }
            Err(error) => return Err(error.into()),
        }
        Ok(path)
    }

    fn reclaim_content_until_available(
        &mut self,
        queue_scope: &str,
        content_root: &Path,
        additional: u64,
    ) -> Result<()> {
        let mut used = directory_bytes(content_root)?;
        if used.saturating_add(additional) <= MAX_CONTENT_STORE_BYTES {
            return Ok(());
        }
        let candidates = self
            .store_for_scope_mut(queue_scope)?
            .claim_reclaimable_terminal_content(256)?;
        for candidate in candidates {
            match std::fs::remove_file(&candidate.path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    self.store_for_scope_mut(queue_scope)?
                        .cancel_terminal_content_reclaim(&candidate.sha256, &candidate.path)?;
                    return Err(error.into());
                }
            }
            let _ = self
                .store_for_scope_mut(queue_scope)?
                .mark_terminal_content_reclaimed(&candidate.sha256, &candidate.path)?;
            used = directory_bytes(content_root)?;
            if used.saturating_add(additional) <= MAX_CONTENT_STORE_BYTES {
                return Ok(());
            }
        }
        bail!("embedded content retention limit reached")
    }

    fn persist(&self) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(&self.document)?;
        crate::durable_file::replace_json(&self.root.join("embedded-runtime.json"), &bytes)
    }

    fn repair_after_restart(&mut self) -> Result<()> {
        for operation_id in self.document.operations.keys().cloned().collect::<Vec<_>>() {
            self.repair_active_operation(&operation_id)?;
        }
        for completed in self
            .document
            .completed
            .values()
            .cloned()
            .collect::<Vec<_>>()
        {
            self.apply_completed_to_store(&completed)?;
        }
        Ok(())
    }

    fn open_persisted_connector_scopes(&mut self) -> Result<()> {
        let mut scopes = self.document.connector_scopes.clone();
        scopes.extend(
            self.document
                .operations
                .values()
                .map(|operation| operation.queue_scope.clone())
                .chain(
                    self.document
                        .completed
                        .values()
                        .map(|completed| completed.queue_scope.clone()),
                )
                .filter(|scope| scope != "local"),
        );
        let migrated = scopes != self.document.connector_scopes;
        self.document.connector_scopes = scopes.clone();
        if migrated {
            self.persist()?;
        }
        for scope in &scopes {
            self.open_connector_store(scope)?;
        }
        Ok(())
    }

    fn open_connector_store(&mut self, connector_id: &str) -> Result<()> {
        let root = self.root.join("connectors").join(connector_id);
        std::fs::create_dir_all(&root)?;
        let store = AgentStore::open(root.join("agent.sqlite3"))?;
        reconcile_content_root(&store, &root.join("content"))?;
        self.connector_stores.insert(connector_id.to_owned(), store);
        Ok(())
    }

    fn reconcile_content_for_scope(&self, queue_scope: &str) -> Result<()> {
        let content_root = if queue_scope == "local" {
            self.root.join("content")
        } else {
            validate_queue_scope(queue_scope)?;
            self.root
                .join("connectors")
                .join(queue_scope)
                .join("content")
        };
        reconcile_content_root(self.store_for_scope(queue_scope)?, &content_root)
    }

    fn repair_active_operation(&mut self, operation_id: &str) -> Result<()> {
        let operation = self
            .document
            .operations
            .get(operation_id)
            .cloned()
            .context("adapter operation disappeared")?;
        let job = self
            .store_for_scope(&operation.queue_scope)?
            .get_job(&operation.job_id)?
            .context("adapter operation references a missing job")?;
        self.store_for_scope_mut(&operation.queue_scope)?
            .set_setting(
                &adapter_operation_setting(&operation.job_id),
                &operation.operation_id,
            )?;
        match operation.phase {
            AdapterOperationPhase::Claimed if job.state != "spool_intent" => {
                if matches!(job.state.as_str(), "queued_local" | "failed_retryable") {
                    self.store_for_scope_mut(&operation.queue_scope)?
                        .append_next_event(
                            &EventId::new().to_string(),
                            &job.job_id,
                            "spool_intent",
                            None,
                            Some("Recovered persisted embedded adapter operation"),
                            "{}",
                            Utc::now().timestamp_millis(),
                        )?;
                }
            }
            AdapterOperationPhase::Accepted if job.state == "spool_intent" => {
                let native_job_id = operation
                    .native_job_id
                    .as_deref()
                    .context("accepted operation has no native job id")?;
                let now = Utc::now().timestamp_millis();
                self.store_for_scope_mut(&operation.queue_scope)?
                    .record_native_acceptance(
                        &EventId::new().to_string(),
                        &job.job_id,
                        native_job_id,
                        "{}",
                        now,
                        now.saturating_add(1_000),
                        now.saturating_add(5 * 60_000),
                    )?;
            }
            AdapterOperationPhase::Claimed
            | AdapterOperationPhase::HandoffStarted
            | AdapterOperationPhase::Accepted => {}
        }
        Ok(())
    }

    fn apply_completed_to_store(&mut self, completed: &CompletedAck) -> Result<()> {
        let Some(job) = self
            .store_for_scope(&completed.queue_scope)?
            .get_job(&completed.ack.job_id)?
        else {
            bail!("completed adapter operation references a missing job");
        };
        if job.state == completed.ack.state {
            return Ok(());
        }
        let current_operation = self
            .store_for_scope(&completed.queue_scope)?
            .setting(&adapter_operation_setting(&completed.ack.job_id))?;
        if current_operation
            .as_deref()
            .is_some_and(|operation_id| operation_id != completed.ack.operation_id)
        {
            // A newer durable adapter generation superseded this retained
            // acknowledgement. Historical acknowledgements remain available
            // for idempotent duplicate responses but must never regress the
            // current job projection during restart repair.
            return Ok(());
        }
        let now = completed.completed_unix_ms;
        let store = self.store_for_scope_mut(&completed.queue_scope)?;
        match &completed.outcome {
            AdapterOperationOutcome::CompletedReported { native_job_id } => {
                if job.state == "spool_intent" {
                    store.record_native_acceptance(
                        &EventId::new().to_string(),
                        &job.job_id,
                        native_job_id,
                        "{}",
                        now,
                        now.saturating_add(1_000),
                        now.saturating_add(5 * 60_000),
                    )?;
                }
                store.append_next_event(
                    &EventId::new().to_string(),
                    &job.job_id,
                    "completed_reported",
                    None,
                    Some("Embedded adapter reported native completion"),
                    "{}",
                    now,
                )?;
                store.finish_reconciliation(&job.job_id)?;
            }
            AdapterOperationOutcome::FailedTerminal {
                native_job_id,
                code,
            } => {
                if job.state == "spool_intent" {
                    store.record_native_acceptance(
                        &EventId::new().to_string(),
                        &job.job_id,
                        native_job_id,
                        "{}",
                        now,
                        now.saturating_add(1_000),
                        now.saturating_add(5 * 60_000),
                    )?;
                }
                store.append_next_event(
                    &EventId::new().to_string(),
                    &job.job_id,
                    "failed_terminal",
                    Some(code),
                    Some("Embedded adapter reported terminal native failure"),
                    "{}",
                    now,
                )?;
                store.finish_reconciliation(&job.job_id)?;
            }
            AdapterOperationOutcome::Ambiguous { code } => {
                store.append_next_event(
                    &EventId::new().to_string(),
                    &job.job_id,
                    "delivery_uncertain",
                    Some(code),
                    Some("Embedded adapter could not prove native handoff outcome"),
                    "{}",
                    now,
                )?;
                store.finish_reconciliation(&job.job_id)?;
            }
            AdapterOperationOutcome::RejectedBeforeHandoff { code, retryable } => {
                store.append_next_event(
                    &EventId::new().to_string(),
                    &job.job_id,
                    if *retryable {
                        "failed_retryable"
                    } else {
                        "failed_terminal"
                    },
                    Some(code),
                    Some("Embedded adapter rejected before native handoff"),
                    "{}",
                    now,
                )?;
            }
            AdapterOperationOutcome::Accepted { .. } => {
                bail!("accepted acknowledgements are retained as active operations");
            }
        }
        Ok(())
    }
}

fn adapter_operation_setting(job_id: &str) -> String {
    format!("embedded_adapter_operation:{job_id}")
}

fn validate_document(document: &EmbeddedDocument) -> Result<()> {
    if document.version != DOCUMENT_VERSION
        || document.adapters.len() > MAX_ADAPTERS
        || document.connector_scopes.len() > 128
        || document.operations.len() > MAX_ACTIVE_OPERATIONS
        || document.completed.len() > MAX_COMPLETED_ACKS
    {
        bail!("unsupported or unbounded embedded runtime journal");
    }
    for scope in &document.connector_scopes {
        validate_queue_scope(scope)?;
    }
    for adapter in document.adapters.values() {
        validate_adapter(&adapter.registration)?;
        if adapter.printers.len() > MAX_PRINTERS_PER_ADAPTER {
            bail!("embedded adapter printer map exceeds supported bounds");
        }
    }
    Ok(())
}

fn validate_adapter(registration: &EmbeddedAdapterRegistration) -> Result<()> {
    let fingerprint = &registration.fingerprint;
    if !valid_identifier(&fingerprint.adapter_id, 255)
        || fingerprint.adapter_version.is_empty()
        || fingerprint.adapter_version.len() > 64
        || serde_json::to_vec(&registration.capability_contract)?.len() > 64 * 1024
    {
        bail!("invalid embedded adapter registration");
    }
    Ok(())
}

fn validate_printer(printer: &EmbeddedPrinterObservation) -> Result<()> {
    if printer.native_id.is_empty()
        || printer.native_id.len() > 512
        || printer.name.is_empty()
        || printer.name.len() > 512
        || printer.state.is_empty()
        || printer.state.len() > 64
        || printer.native_options.len() > 512
    {
        bail!("invalid embedded printer observation");
    }
    Ok(())
}

fn validate_job_request(request: &EmbeddedJobRequest) -> Result<()> {
    if !valid_identifier(&request.adapter_id, 255)
        || request.idempotency_key.is_empty()
        || request.idempotency_key.len() > 255
        || request.printer_id.is_empty()
        || request.printer_id.len() > 255
        || request.title.len() > 512
        || request.content.is_empty()
        || request.content.len() > MAX_DOCUMENT_BYTES
        || request.content_kind.is_empty()
        || request.content_kind.len() > 64
        || request.options_json.len() > 256 * 1024
        || serde_json::from_str::<serde_json::Value>(&request.options_json).is_err()
    {
        bail!("invalid embedded job request");
    }
    Ok(())
}

fn validate_native_job_id(value: &str) -> Result<()> {
    if value.is_empty() || value.len() > 512 || value.chars().any(char::is_control) {
        bail!("invalid native job id");
    }
    Ok(())
}

fn validate_code(value: &str) -> Result<()> {
    if !valid_identifier(value, 128) {
        bail!("invalid adapter result code");
    }
    Ok(())
}

fn require_accepted_native_id(operation: &AdapterOperation, native_job_id: &str) -> Result<()> {
    validate_native_job_id(native_job_id)?;
    if operation.phase != AdapterOperationPhase::Accepted
        || operation.native_job_id.as_deref() != Some(native_job_id)
    {
        bail!("terminal operation does not match the accepted native job");
    }
    Ok(())
}

fn valid_identifier(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

fn stable_job_id(idempotency_key: &str) -> String {
    format!(
        "job_emb_{}",
        hex::encode(
            &Sha256::digest(
                [
                    b"piqae-embedded-job-v1\0".as_slice(),
                    idempotency_key.as_bytes()
                ]
                .concat()
            )[..16]
        )
    )
}

fn random_operation_id() -> String {
    let mut bytes = [0_u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    format!("aop_{}", hex::encode(bytes))
}

fn random_fence() -> String {
    let mut bytes = [0_u8; 24];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn token_digest(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

fn json_digest(value: &impl Serialize) -> Result<String> {
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(value)?)))
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    left.len() == right.len()
        && left
            .bytes()
            .zip(right.bytes())
            .fold(0_u8, |difference, (left, right)| {
                difference | (left ^ right)
            })
            == 0
}

fn verify_operation(operation: &AdapterOperation, adapter_id: &str, fence: &str) -> Result<()> {
    if operation.adapter_id != adapter_id
        || !constant_time_eq(&token_digest(&operation.fence), &token_digest(fence))
    {
        bail!("stale or mismatched adapter operation");
    }
    Ok(())
}

fn directory_bytes(path: &Path) -> Result<u64> {
    let mut total = 0_u64;
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            total = total.saturating_add(entry.metadata()?.len());
        }
    }
    Ok(total)
}

fn reconcile_content_root(store: &AgentStore, content_root: &Path) -> Result<()> {
    std::fs::create_dir_all(content_root)?;
    let tracked = store
        .tracked_content_paths()?
        .into_iter()
        .map(PathBuf::from)
        .collect::<BTreeSet<_>>();
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(content_root)? {
        if entries.len() >= MAX_CONTENT_FILES {
            bail!("embedded content inventory exceeds reconciliation bounds");
        }
        entries.push(entry?);
    }
    #[cfg(unix)]
    let mut removed = false;
    for entry in entries {
        let path = entry.path();
        if entry.file_type()?.is_file()
            && path.extension().is_some_and(|extension| extension == "bin")
            && !tracked.contains(&path)
        {
            std::fs::remove_file(&path)?;
            #[cfg(unix)]
            {
                removed = true;
            }
        }
    }
    #[cfg(unix)]
    if removed {
        std::fs::File::open(content_root)?.sync_all()?;
    }
    Ok(())
}

fn scoped_native_id(adapter_id: &str, native_id: &str) -> String {
    format!("{adapter_id}\0{native_id}")
}

fn local_queue_scope() -> String {
    "local".into()
}

fn validate_queue_scope(scope: &str) -> Result<()> {
    if scope.is_empty()
        || scope == "local"
        || scope.len() > 160
        || !scope
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        bail!("connector queue scope is invalid");
    }
    Ok(())
}

fn embedded_protocol_printer(snapshot: EmbeddedPrinterSnapshot) -> Result<PrinterSnapshot> {
    let id: PrinterId = snapshot.printer_id.parse()?;
    let state: PrinterState = serde_json::from_value(serde_json::Value::String(snapshot.state))?;
    let semantic = serde_json::from_value(snapshot.semantic_capabilities).unwrap_or_default();
    Ok(PrinterSnapshot {
        id,
        native_id: scoped_native_id(&snapshot.adapter_id, &snapshot.native_id),
        name: snapshot.name,
        state,
        is_default: snapshot.is_default,
        capabilities: PrinterCapabilities::default(),
        exposed: true,
        capability_revision: u64::try_from(snapshot.observed_unix_ms).unwrap_or_default(),
        native_options: BTreeMap::new(),
        semantic_capabilities: semantic,
        profiles: Vec::new(),
        route: None,
    })
}

fn embedded_protocol_event(
    agent_id: AgentId,
    event: piqae_agent_storage::PendingEvent,
) -> Result<piqae_domain::JobEvent> {
    let state: JobState = serde_json::from_value(serde_json::Value::String(event.state))?;
    let reason = event
        .reason
        .map(|reason| serde_json::from_value::<JobFailureReason>(serde_json::Value::String(reason)))
        .transpose()
        .ok()
        .flatten();
    Ok(piqae_domain::JobEvent {
        id: event.event_id.parse()?,
        job_id: event.job_id.parse()?,
        sequence: u64::try_from(event.job_sequence).unwrap_or_default(),
        state,
        reason,
        message: event.message,
        agent_id: Some(agent_id),
        native_job_id: None,
        occurred_at: chrono::DateTime::from_timestamp_millis(event.observed_unix_ms)
            .context("embedded event timestamp is invalid")?,
    })
}

fn offer_content_digest(offer: &JobOffer) -> Option<String> {
    match &offer.content {
        piqae_protocol::agent::ContentDescriptor::InlineBase64 { sha256, .. }
        | piqae_protocol::agent::ContentDescriptor::Uri { sha256, .. } => sha256.clone(),
        piqae_protocol::agent::ContentDescriptor::Download { sha256, .. }
        | piqae_protocol::agent::ContentDescriptor::EncryptedDownload { sha256, .. } => {
            Some(sha256.clone())
        }
        piqae_protocol::agent::ContentDescriptor::PrintPacket { .. } => None,
    }
}

fn unscoped_native_id(adapter_id: &str, value: &str) -> Result<String> {
    value
        .strip_prefix(&format!("{adapter_id}\0"))
        .map(str::to_owned)
        .context("printer route does not belong to adapter")
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn registration() -> EmbeddedAdapterRegistration {
        EmbeddedAdapterRegistration {
            fingerprint: EmbeddedAdapterFingerprint {
                platform: Platform::IosAirPrint,
                adapter_id: "com.example.pos.airprint".into(),
                adapter_version: "1.0.0".into(),
                device_family: Some("ipad".into()),
                firmware_version: None,
            },
            capability_contract: serde_json::json!({"document_kinds": ["pdf"]}),
        }
    }

    fn prepare(root: &Path) -> (EmbeddedQueue, EmbeddedPrinterSnapshot) {
        let mut queue = EmbeddedQueue::open(root, SupportPackRegistry::default()).unwrap();
        queue.register_adapter(registration()).unwrap();
        let printer = queue
            .observe_inventory(
                "com.example.pos.airprint",
                &[EmbeddedPrinterObservation {
                    native_id: "ipp://printer/ipp/print".into(),
                    name: "Kitchen".into(),
                    state: "available".into(),
                    is_default: true,
                    native_options: BTreeMap::new(),
                }],
            )
            .unwrap()
            .remove(0);
        (queue, printer)
    }

    fn request(printer: &EmbeddedPrinterSnapshot) -> EmbeddedJobRequest {
        EmbeddedJobRequest {
            adapter_id: printer.adapter_id.clone(),
            idempotency_key: "order-42-label-1".into(),
            printer_id: printer.printer_id.clone(),
            title: "Order 42".into(),
            content_kind: "pdf".into(),
            content: b"%PDF fake fixture".to_vec(),
            options_json: "{}".into(),
            expires_unix_ms: None,
        }
    }

    fn cloud_offer(printer: &EmbeddedPrinterSnapshot, job_id: JobId, content: &[u8]) -> JobOffer {
        let encoded = base64::engine::general_purpose::STANDARD.encode(content);
        JobOffer {
            job: piqae_domain::Job {
                id: job_id,
                workspace_id: piqae_domain::WorkspaceId::new(),
                environment_id: piqae_domain::EnvironmentId::new(),
                printer_id: printer.printer_id.parse().unwrap(),
                title: "privacy-safe fixture".into(),
                source: None,
                content_kind: piqae_domain::ContentKind::Pdf,
                content: piqae_domain::ContentSource::Base64 {
                    data: encoded.clone(),
                },
                options: piqae_domain::JobOptions::default(),
                metadata: BTreeMap::new(),
                deliveries: 1,
                state: JobState::WaitingForAgent,
                created_at: Utc::now(),
                expires_at: Utc::now() + chrono::TimeDelta::minutes(5),
                delivery_uncertain_since: None,
            },
            expected_capability_revision: None,
            resolved_ticket_digest: None,
            lease_id: uuid::Uuid::new_v4(),
            lease_token: "redacted-test-lease".into(),
            lease_expires_at: Utc::now() + chrono::TimeDelta::minutes(1),
            content: piqae_protocol::agent::ContentDescriptor::InlineBase64 {
                data: encoded,
                sha256: Some(hex::encode(Sha256::digest(content))),
                bytes: Some(u64::try_from(content.len()).unwrap()),
            },
            route_reservation: Some(piqae_protocol::agent::CloudRouteReservation {
                route_id: "route_fixture".into(),
                local_route_key: format!("{}\0{}", printer.adapter_id, printer.native_id),
                reservation_id: uuid::Uuid::new_v4(),
                generation: 1,
                fencing_token: "deterministic-route-fence".into(),
                lease_expires_at: Utc::now() + chrono::TimeDelta::minutes(1),
            }),
        }
    }

    #[test]
    fn connector_runtime_presence_sequence_is_monotonic_across_restart() {
        let root = tempfile::tempdir().unwrap();
        let runtime = crate::NodeRuntime::start(crate::RuntimeConfiguration {
            data_directory: root.path().join("runtime"),
            mode: crate::NodeRuntimeMode::CloudCapable,
            host: crate::HostCapabilities {
                host_kind: crate::HostKind::EmbeddedApplication,
                availability: crate::AvailabilityClass::ForegroundOnly,
                secure_storage: true,
                local_ipc_broker: false,
                can_prevent_idle_sleep_during_handoff: false,
                can_receive_remote_wake_hint: false,
                printer_transports: BTreeSet::new(),
            },
        })
        .unwrap();
        let started_at = Utc::now();
        let agent_id = AgentId::new();
        let mut queue =
            EmbeddedQueue::open(root.path().join("embedded"), SupportPackRegistry::default())
                .unwrap();
        let first = queue
            .connector_sync_snapshot("ncon_presence", agent_id, started_at, false, None, &runtime)
            .unwrap();
        let second = queue
            .connector_sync_snapshot("ncon_presence", agent_id, started_at, false, None, &runtime)
            .unwrap();
        assert_eq!(first.runtime.unwrap().sequence, 1);
        assert_eq!(second.runtime.unwrap().sequence, 2);
        drop(queue);

        let mut restarted =
            EmbeddedQueue::open(root.path().join("embedded"), SupportPackRegistry::default())
                .unwrap();
        let third = restarted
            .connector_sync_snapshot("ncon_presence", agent_id, started_at, false, None, &runtime)
            .unwrap();
        assert_eq!(third.runtime.unwrap().sequence, 3);
    }

    #[test]
    fn restart_expires_waiting_local_and_connector_jobs_without_adapter_handoff() {
        let root = tempfile::tempdir().unwrap();
        let (mut queue, printer) = prepare(root.path());
        let mut local_request = request(&printer);
        local_request.expires_unix_ms = Some(Utc::now().timestamp_millis() - 1);
        let local = queue.enqueue(local_request).unwrap();

        let connector_job_id = JobId::new();
        let mut offer = cloud_offer(&printer, connector_job_id, b"expired connector fixture");
        offer.job.expires_at = Utc::now() - chrono::TimeDelta::seconds(1);
        queue
            .prepare_connector_offer("ncon_expiry", &offer, b"expired connector fixture", None)
            .unwrap();
        queue
            .confirm_connector_offer("ncon_expiry", connector_job_id)
            .unwrap();
        queue
            .activate_connector_offer("ncon_expiry", connector_job_id)
            .unwrap();
        drop(queue);

        let mut restarted =
            EmbeddedQueue::open(root.path(), SupportPackRegistry::default()).unwrap();
        assert_eq!(
            restarted.job(&local.job_id).unwrap().unwrap().state,
            "expired"
        );
        assert_eq!(
            restarted
                .job(&connector_job_id.to_string())
                .unwrap()
                .unwrap()
                .state,
            "expired"
        );
        assert!(
            restarted
                .next_operation("com.example.pos.airprint")
                .unwrap()
                .is_none()
        );
        let connector_events = restarted
            .store_for_scope("ncon_expiry")
            .unwrap()
            .pending_cloud_events(0, 20)
            .unwrap();
        assert!(connector_events.iter().any(|event| {
            event.job_id == connector_job_id.to_string() && event.state == "expired"
        }));
    }

    #[test]
    fn connector_queues_are_isolated_but_share_one_serial_native_handoff() {
        let root = tempfile::tempdir().unwrap();
        let (mut queue, printer) = prepare(root.path());
        let first_id = JobId::new();
        let second_id = JobId::new();
        let first = cloud_offer(&printer, first_id, b"first connector fixture");
        let second = cloud_offer(&printer, second_id, b"second connector fixture");
        queue
            .prepare_connector_offer("ncon_first", &first, b"first connector fixture", None)
            .unwrap();
        queue
            .confirm_connector_offer("ncon_first", first_id)
            .unwrap();
        queue
            .activate_connector_offer("ncon_first", first_id)
            .unwrap();
        queue
            .prepare_connector_offer("ncon_second", &second, b"second connector fixture", None)
            .unwrap();
        queue
            .confirm_connector_offer("ncon_second", second_id)
            .unwrap();
        queue
            .activate_connector_offer("ncon_second", second_id)
            .unwrap();

        assert!(
            queue.connector_stores["ncon_first"]
                .get_job(&second_id.to_string())
                .unwrap()
                .is_none()
        );
        assert!(
            queue.connector_stores["ncon_second"]
                .get_job(&first_id.to_string())
                .unwrap()
                .is_none()
        );
        let first_path = queue.connector_stores["ncon_first"]
            .get_job(&first_id.to_string())
            .unwrap()
            .unwrap()
            .content_path;
        let second_path = queue.connector_stores["ncon_second"]
            .get_job(&second_id.to_string())
            .unwrap()
            .unwrap()
            .content_path;
        assert_ne!(
            Path::new(&first_path).parent(),
            Path::new(&second_path).parent()
        );
        let operation = queue
            .next_operation("com.example.pos.airprint")
            .unwrap()
            .unwrap();
        let duplicate_poll = queue
            .next_operation("com.example.pos.airprint")
            .unwrap()
            .unwrap();
        assert_eq!(duplicate_poll.operation_id, operation.operation_id);
        let expected_next = if operation.job_id == first_id.to_string() {
            second_id
        } else {
            first_id
        };
        queue
            .begin_handoff(
                &operation.adapter_id,
                &operation.operation_id,
                &operation.fence,
            )
            .unwrap();
        queue
            .complete_operation(
                &operation.adapter_id,
                &operation.operation_id,
                &operation.fence,
                &AdapterOperationOutcome::Accepted {
                    native_job_id: "native-shared-1".into(),
                },
            )
            .unwrap();
        queue
            .complete_operation(
                &operation.adapter_id,
                &operation.operation_id,
                &operation.fence,
                &AdapterOperationOutcome::CompletedReported {
                    native_job_id: "native-shared-1".into(),
                },
            )
            .unwrap();
        drop(queue);

        let mut restarted =
            EmbeddedQueue::open(root.path(), SupportPackRegistry::default()).unwrap();
        assert_eq!(restarted.connector_stores.len(), 2);
        assert_eq!(
            restarted
                .next_operation("com.example.pos.airprint")
                .unwrap()
                .unwrap()
                .job_id,
            expected_next.to_string()
        );
    }

    #[test]
    fn connector_content_reconcile_removes_only_untracked_files_after_restart() {
        let root = tempfile::tempdir().unwrap();
        let (mut queue, printer) = prepare(root.path());
        let job_id = JobId::new();
        let content = b"tracked connector fixture";
        let offer = cloud_offer(&printer, job_id, content);
        queue
            .prepare_connector_offer("ncon_reconcile", &offer, content, None)
            .unwrap();
        let tracked = queue.connector_stores["ncon_reconcile"]
            .get_job(&job_id.to_string())
            .unwrap()
            .unwrap()
            .content_path;
        let content_root = root.path().join("connectors/ncon_reconcile/content");
        let orphan = content_root.join(format!("{}.bin", "f".repeat(64)));
        std::fs::write(&orphan, b"orphan after sqlite failure").unwrap();
        drop(queue);

        let restarted = EmbeddedQueue::open(root.path(), SupportPackRegistry::default()).unwrap();
        assert!(Path::new(&tracked).exists());
        assert!(!orphan.exists());
        assert_eq!(
            restarted.connector_stores["ncon_reconcile"]
                .pending_cloud_accepts()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn sqlite_prepare_failure_reclaims_new_connector_content_before_returning() {
        let root = tempfile::tempdir().unwrap();
        let (mut queue, printer) = prepare(root.path());
        let content = b"sqlite rollback fixture";
        let mut offer = cloud_offer(&printer, JobId::new(), content);
        offer.route_reservation.as_mut().unwrap().generation = u64::MAX;
        assert!(
            queue
                .prepare_connector_offer("ncon_failed", &offer, content, None)
                .is_err()
        );
        let digest = hex::encode(Sha256::digest(content));
        assert!(
            !root
                .path()
                .join("connectors/ncon_failed/content")
                .join(format!("{digest}.bin"))
                .exists()
        );
        drop(queue);
        EmbeddedQueue::open(root.path(), SupportPackRegistry::default()).unwrap();
    }

    #[test]
    fn empty_adapter_pull_does_not_hide_another_adapters_work() {
        let root = tempfile::tempdir().unwrap();
        let (mut queue, _) = prepare(root.path());
        let second_id = "com.example.pos.second";
        let mut second = registration();
        second.fingerprint.adapter_id = second_id.into();
        queue.register_adapter(second).unwrap();
        let printer = queue
            .observe_inventory(
                second_id,
                &[EmbeddedPrinterObservation {
                    native_id: "ipp://second/ipp/print".into(),
                    name: "Second kitchen".into(),
                    state: "available".into(),
                    is_default: false,
                    native_options: BTreeMap::new(),
                }],
            )
            .unwrap()
            .remove(0);
        let job_id = JobId::new();
        let content = b"second adapter only";
        let offer = cloud_offer(&printer, job_id, content);
        queue
            .prepare_connector_offer("ncon_second_adapter", &offer, content, None)
            .unwrap();
        queue
            .confirm_connector_offer("ncon_second_adapter", job_id)
            .unwrap();
        queue
            .activate_connector_offer("ncon_second_adapter", job_id)
            .unwrap();

        assert!(
            queue
                .next_operation("com.example.pos.airprint")
                .unwrap()
                .is_none()
        );
        assert!(queue.has_runnable_adapter_work().unwrap());
        assert!(queue.next_operation(second_id).unwrap().is_some());
    }

    #[test]
    fn accepted_observation_does_not_block_runnable_work_for_another_printer() {
        let root = tempfile::tempdir().unwrap();
        let (mut queue, first_printer) = prepare(root.path());
        let second_printer = queue
            .observe_inventory(
                "com.example.pos.airprint",
                &[
                    EmbeddedPrinterObservation {
                        native_id: first_printer.native_id.clone(),
                        name: first_printer.name.clone(),
                        state: "available".into(),
                        is_default: true,
                        native_options: BTreeMap::new(),
                    },
                    EmbeddedPrinterObservation {
                        native_id: "ipp://second/ipp/print".into(),
                        name: "Second kitchen".into(),
                        state: "available".into(),
                        is_default: false,
                        native_options: BTreeMap::new(),
                    },
                ],
            )
            .unwrap()
            .into_iter()
            .find(|printer| printer.native_id == "ipp://second/ipp/print")
            .unwrap();
        let first_job = queue.enqueue(request(&first_printer)).unwrap();
        let first_operation = queue
            .next_operation("com.example.pos.airprint")
            .unwrap()
            .unwrap();
        queue
            .begin_handoff(
                &first_operation.adapter_id,
                &first_operation.operation_id,
                &first_operation.fence,
            )
            .unwrap();
        queue
            .complete_operation(
                &first_operation.adapter_id,
                &first_operation.operation_id,
                &first_operation.fence,
                &AdapterOperationOutcome::Accepted {
                    native_job_id: "native-first".into(),
                },
            )
            .unwrap();

        let mut second_request = request(&second_printer);
        second_request.idempotency_key = "order-43-label-1".into();
        let second_job = queue.enqueue(second_request).unwrap();
        assert!(queue.has_runnable_adapter_work().unwrap());
        let second_operation = queue
            .next_operation("com.example.pos.airprint")
            .unwrap()
            .unwrap();

        assert_eq!(second_operation.job_id, second_job.job_id);
        assert_ne!(second_operation.operation_id, first_operation.operation_id);
        assert_eq!(
            queue
                .adapter_observations("com.example.pos.airprint")
                .unwrap(),
            vec![AdapterOperation {
                phase: AdapterOperationPhase::Accepted,
                native_job_id: Some("native-first".into()),
                ..first_operation
            }]
        );
        assert_eq!(
            queue.job(&first_job.job_id).unwrap().unwrap().state,
            "accepted_by_spooler"
        );
    }

    #[test]
    fn enqueue_is_idempotent_and_conflicts_on_changed_document() {
        let root = tempfile::tempdir().unwrap();
        let (mut queue, printer) = prepare(root.path());
        let first = queue.enqueue(request(&printer)).unwrap();
        let second = queue.enqueue(request(&printer)).unwrap();
        assert_eq!(first.job_id, second.job_id);
        let mut changed = request(&printer);
        changed.content = b"different".to_vec();
        assert!(queue.enqueue(changed).is_err());
    }

    #[test]
    fn every_apple_adapter_platform_registers_and_normalizes_inventory() {
        for platform in [
            Platform::IosAirPrint,
            Platform::IosNetwork,
            Platform::IosBluetoothLe,
            Platform::IosExternalAccessory,
        ] {
            let root = tempfile::tempdir().unwrap();
            let mut queue =
                EmbeddedQueue::open(root.path(), SupportPackRegistry::default()).unwrap();
            let adapter_id = format!("com.example.pos.{platform:?}").to_ascii_lowercase();
            queue
                .register_adapter(EmbeddedAdapterRegistration {
                    fingerprint: EmbeddedAdapterFingerprint {
                        platform,
                        adapter_id: adapter_id.clone(),
                        adapter_version: "1.0.0".into(),
                        device_family: Some("ipad".into()),
                        firmware_version: Some("fixture".into()),
                    },
                    capability_contract: serde_json::json!({"document_kinds": ["pdf"]}),
                })
                .unwrap();
            let observed = queue
                .observe_inventory(
                    &adapter_id,
                    &[EmbeddedPrinterObservation {
                        native_id: "opaque-installation-keyed-id".into(),
                        name: "Fixture printer".into(),
                        state: "available".into(),
                        is_default: false,
                        native_options: BTreeMap::new(),
                    }],
                )
                .unwrap();
            assert_eq!(observed.len(), 1);
            assert_eq!(observed[0].adapter_id, adapter_id);
        }
    }

    #[test]
    fn operation_is_persisted_replayed_and_exactly_acknowledged() {
        let root = tempfile::tempdir().unwrap();
        let (mut queue, printer) = prepare(root.path());
        let job = queue.enqueue(request(&printer)).unwrap();
        let operation = queue
            .next_operation("com.example.pos.airprint")
            .unwrap()
            .unwrap();
        drop(queue);
        let mut restarted =
            EmbeddedQueue::open(root.path(), SupportPackRegistry::default()).unwrap();
        assert_eq!(
            restarted
                .next_operation("com.example.pos.airprint")
                .unwrap()
                .unwrap(),
            operation
        );
        let started = restarted
            .begin_handoff(
                &operation.adapter_id,
                &operation.operation_id,
                &operation.fence,
            )
            .unwrap();
        assert_eq!(started.phase, AdapterOperationPhase::HandoffStarted);
        let accepted = AdapterOperationOutcome::Accepted {
            native_job_id: "cups-42".into(),
        };
        assert_eq!(
            restarted
                .complete_operation(
                    &operation.adapter_id,
                    &operation.operation_id,
                    &operation.fence,
                    &accepted,
                )
                .unwrap()
                .state,
            "accepted_by_spooler"
        );
        assert!(
            restarted
                .complete_operation(
                    "wrong.adapter",
                    &operation.operation_id,
                    &operation.fence,
                    &accepted,
                )
                .is_err()
        );
        let completed = AdapterOperationOutcome::CompletedReported {
            native_job_id: "cups-42".into(),
        };
        let ack = restarted
            .complete_operation(
                &operation.adapter_id,
                &operation.operation_id,
                &operation.fence,
                &completed,
            )
            .unwrap();
        assert_eq!(ack.state, "completed_reported");
        assert!(!ack.duplicate);
        let duplicate = restarted
            .complete_operation(
                &operation.adapter_id,
                &operation.operation_id,
                &operation.fence,
                &completed,
            )
            .unwrap();
        assert!(duplicate.duplicate);
        assert_eq!(
            restarted.job(&job.job_id).unwrap().unwrap().state,
            "completed_reported"
        );
    }

    #[test]
    fn ambiguous_handoff_is_terminal_and_never_replayed() {
        let root = tempfile::tempdir().unwrap();
        let (mut queue, printer) = prepare(root.path());
        let job = queue.enqueue(request(&printer)).unwrap();
        let operation = queue
            .next_operation("com.example.pos.airprint")
            .unwrap()
            .unwrap();
        queue
            .begin_handoff(
                &operation.adapter_id,
                &operation.operation_id,
                &operation.fence,
            )
            .unwrap();
        let ack = queue
            .complete_operation(
                &operation.adapter_id,
                &operation.operation_id,
                &operation.fence,
                &AdapterOperationOutcome::Ambiguous {
                    code: "native_timeout".into(),
                },
            )
            .unwrap();
        assert_eq!(ack.state, "delivery_uncertain");
        assert!(
            queue
                .next_operation(&operation.adapter_id)
                .unwrap()
                .is_none()
        );
        drop(queue);
        let mut restarted =
            EmbeddedQueue::open(root.path(), SupportPackRegistry::default()).unwrap();
        assert!(
            restarted
                .next_operation(&operation.adapter_id)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            restarted.job(&job.job_id).unwrap().unwrap().state,
            "delivery_uncertain"
        );
    }

    #[test]
    fn restart_repairs_each_cross_journal_crash_boundary_without_rehandoff() {
        let root = tempfile::tempdir().unwrap();
        let (mut queue, printer) = prepare(root.path());
        let job = queue.enqueue(request(&printer)).unwrap();
        let operation = queue
            .next_operation("com.example.pos.airprint")
            .unwrap()
            .unwrap();

        // Simulate a crash after the operation journal commit but before the
        // SQLite spool-intent projection.
        queue
            .store
            .append_next_event(
                &EventId::new().to_string(),
                &job.job_id,
                "failed_retryable",
                Some("injected_boundary"),
                None,
                "{}",
                Utc::now().timestamp_millis(),
            )
            .unwrap();
        drop(queue);
        let mut queue = EmbeddedQueue::open(root.path(), SupportPackRegistry::default()).unwrap();
        assert_eq!(
            queue.job(&job.job_id).unwrap().unwrap().state,
            "spool_intent"
        );

        queue
            .begin_handoff(
                &operation.adapter_id,
                &operation.operation_id,
                &operation.fence,
            )
            .unwrap();
        // Simulate accepted journal persisted before SQLite acceptance.
        let accepted = queue
            .document
            .operations
            .get_mut(&operation.operation_id)
            .unwrap();
        accepted.phase = AdapterOperationPhase::Accepted;
        accepted.native_job_id = Some("native-crash-boundary".into());
        queue.persist().unwrap();
        drop(queue);
        let mut queue = EmbeddedQueue::open(root.path(), SupportPackRegistry::default()).unwrap();
        assert!(
            queue
                .next_operation(&operation.adapter_id)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            queue
                .adapter_observations(&operation.adapter_id)
                .unwrap()
                .first()
                .unwrap()
                .phase,
            AdapterOperationPhase::Accepted
        );
        assert_eq!(
            queue.job(&job.job_id).unwrap().unwrap().state,
            "accepted_by_spooler"
        );

        // Simulate terminal journal persisted before SQLite completion.
        let outcome = AdapterOperationOutcome::CompletedReported {
            native_job_id: "native-crash-boundary".into(),
        };
        let ack = AdapterOperationAck {
            operation_id: operation.operation_id.clone(),
            job_id: job.job_id.clone(),
            state: "completed_reported".into(),
            duplicate: false,
        };
        let completed = CompletedAck {
            adapter_id: operation.adapter_id.clone(),
            queue_scope: operation.queue_scope.clone(),
            fence_sha256: token_digest(&operation.fence),
            outcome_sha256: json_digest(&outcome).unwrap(),
            ack,
            outcome,
            completed_unix_ms: Utc::now().timestamp_millis(),
        };
        queue.document.operations.remove(&operation.operation_id);
        queue
            .document
            .completed
            .insert(operation.operation_id.clone(), completed);
        queue.persist().unwrap();
        drop(queue);
        let mut queue = EmbeddedQueue::open(root.path(), SupportPackRegistry::default()).unwrap();
        assert_eq!(
            queue.job(&job.job_id).unwrap().unwrap().state,
            "completed_reported"
        );
        assert!(
            queue
                .next_operation(&operation.adapter_id)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn retry_uses_a_new_operation_and_expired_claim_cannot_begin_handoff() {
        let root = tempfile::tempdir().unwrap();
        let (mut queue, printer) = prepare(root.path());
        let job = queue.enqueue(request(&printer)).unwrap();
        let first = queue
            .next_operation("com.example.pos.airprint")
            .unwrap()
            .unwrap();
        queue
            .complete_operation(
                &first.adapter_id,
                &first.operation_id,
                &first.fence,
                &AdapterOperationOutcome::RejectedBeforeHandoff {
                    code: "host_backgrounded".into(),
                    retryable: true,
                },
            )
            .unwrap();
        drop(queue);
        let mut queue = EmbeddedQueue::open(root.path(), SupportPackRegistry::default()).unwrap();
        let second = queue
            .next_operation("com.example.pos.airprint")
            .unwrap()
            .unwrap();
        assert_ne!(first.operation_id, second.operation_id);
        assert_ne!(first.fence, second.fence);
        queue
            .document
            .operations
            .get_mut(&second.operation_id)
            .unwrap()
            .deadline_unix_ms = Utc::now().timestamp_millis() - 1;
        queue.persist().unwrap();
        assert!(
            queue
                .begin_handoff(&second.adapter_id, &second.operation_id, &second.fence)
                .is_err()
        );
        queue
            .complete_operation(
                &second.adapter_id,
                &second.operation_id,
                &second.fence,
                &AdapterOperationOutcome::RejectedBeforeHandoff {
                    code: "claim_expired".into(),
                    retryable: true,
                },
            )
            .unwrap();
        let third = queue
            .next_operation("com.example.pos.airprint")
            .unwrap()
            .unwrap();
        queue
            .begin_handoff(&third.adapter_id, &third.operation_id, &third.fence)
            .unwrap();
        queue
            .complete_operation(
                &third.adapter_id,
                &third.operation_id,
                &third.fence,
                &AdapterOperationOutcome::Accepted {
                    native_job_id: "native-retry".into(),
                },
            )
            .unwrap();
        queue
            .complete_operation(
                &third.adapter_id,
                &third.operation_id,
                &third.fence,
                &AdapterOperationOutcome::CompletedReported {
                    native_job_id: "native-retry".into(),
                },
            )
            .unwrap();
        drop(queue);
        for _ in 0..2 {
            let restarted =
                EmbeddedQueue::open(root.path(), SupportPackRegistry::default()).unwrap();
            assert_eq!(
                restarted.job(&job.job_id).unwrap().unwrap().state,
                "completed_reported"
            );
            drop(restarted);
        }
    }
}
