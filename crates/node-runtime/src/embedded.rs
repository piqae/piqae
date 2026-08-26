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

use anyhow::{Context as _, Result, bail};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::Utc;
use piqae_agent_storage::{AcceptedJob, AgentStore, LocalJob, StoredNamedProfile};
use piqae_domain::{EventId, NativePrinterOption};
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
    operations: BTreeMap<String, AdapterOperation>,
    completed: BTreeMap<String, CompletedAck>,
}

pub struct EmbeddedQueue {
    root: PathBuf,
    store: AgentStore,
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
            support_packs,
            document,
        };
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
        })?;
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
        if let Some(operation) = self
            .document
            .operations
            .values()
            .find(|operation| operation.adapter_id == adapter_id)
        {
            return Ok(Some(operation.clone()));
        }
        if self.document.operations.len() >= MAX_ACTIVE_OPERATIONS {
            bail!("active adapter operation limit reached");
        }
        let now = Utc::now().timestamp_millis();
        let Some(job) = self
            .store
            .runnable_heads(now)?
            .into_iter()
            .find(|job| self.adapter_owns_job(adapter_id, job))
        else {
            return Ok(None);
        };
        let operation = AdapterOperation {
            operation_id: random_operation_id(),
            adapter_id: adapter_id.to_owned(),
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
        self.store.append_next_event(
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
        self.store.get_job(job_id).map_err(Into::into)
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

    fn persist_content(&mut self, digest: &str, content: &[u8]) -> Result<PathBuf> {
        let path = self.root.join("content").join(format!("{digest}.bin"));
        let additional = u64::try_from(content.len()).unwrap_or(u64::MAX);
        if !path.exists() {
            self.reclaim_content_until_available(additional)?;
        }
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                file.write_all(content)?;
                file.sync_all()?;
                #[cfg(unix)]
                std::fs::File::open(self.root.join("content"))?.sync_all()?;
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

    fn reclaim_content_until_available(&mut self, additional: u64) -> Result<()> {
        let content_root = self.root.join("content");
        let mut used = directory_bytes(&content_root)?;
        if used.saturating_add(additional) <= MAX_CONTENT_STORE_BYTES {
            return Ok(());
        }
        for candidate in self.store.claim_reclaimable_terminal_content(256)? {
            match std::fs::remove_file(&candidate.path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    self.store
                        .cancel_terminal_content_reclaim(&candidate.sha256, &candidate.path)?;
                    return Err(error.into());
                }
            }
            let _ = self
                .store
                .mark_terminal_content_reclaimed(&candidate.sha256, &candidate.path)?;
            used = directory_bytes(&content_root)?;
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

    fn repair_active_operation(&mut self, operation_id: &str) -> Result<()> {
        let operation = self
            .document
            .operations
            .get(operation_id)
            .cloned()
            .context("adapter operation disappeared")?;
        let job = self
            .store
            .get_job(&operation.job_id)?
            .context("adapter operation references a missing job")?;
        match operation.phase {
            AdapterOperationPhase::Claimed if job.state != "spool_intent" => {
                if matches!(job.state.as_str(), "queued_local" | "failed_retryable") {
                    self.store.append_next_event(
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
                self.store.record_native_acceptance(
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
        let Some(job) = self.store.get_job(&completed.ack.job_id)? else {
            bail!("completed adapter operation references a missing job");
        };
        if job.state == completed.ack.state {
            return Ok(());
        }
        let now = completed.completed_unix_ms;
        match &completed.outcome {
            AdapterOperationOutcome::CompletedReported { native_job_id } => {
                if job.state == "spool_intent" {
                    self.store.record_native_acceptance(
                        &EventId::new().to_string(),
                        &job.job_id,
                        native_job_id,
                        "{}",
                        now,
                        now.saturating_add(1_000),
                        now.saturating_add(5 * 60_000),
                    )?;
                }
                self.store.append_next_event(
                    &EventId::new().to_string(),
                    &job.job_id,
                    "completed_reported",
                    None,
                    Some("Embedded adapter reported native completion"),
                    "{}",
                    now,
                )?;
                self.store.finish_reconciliation(&job.job_id)?;
            }
            AdapterOperationOutcome::FailedTerminal {
                native_job_id,
                code,
            } => {
                if job.state == "spool_intent" {
                    self.store.record_native_acceptance(
                        &EventId::new().to_string(),
                        &job.job_id,
                        native_job_id,
                        "{}",
                        now,
                        now.saturating_add(1_000),
                        now.saturating_add(5 * 60_000),
                    )?;
                }
                self.store.append_next_event(
                    &EventId::new().to_string(),
                    &job.job_id,
                    "failed_terminal",
                    Some(code),
                    Some("Embedded adapter reported terminal native failure"),
                    "{}",
                    now,
                )?;
                self.store.finish_reconciliation(&job.job_id)?;
            }
            AdapterOperationOutcome::Ambiguous { code } => {
                self.store.append_next_event(
                    &EventId::new().to_string(),
                    &job.job_id,
                    "delivery_uncertain",
                    Some(code),
                    Some("Embedded adapter could not prove native handoff outcome"),
                    "{}",
                    now,
                )?;
                self.store.finish_reconciliation(&job.job_id)?;
            }
            AdapterOperationOutcome::RejectedBeforeHandoff { code, retryable } => {
                self.store.append_next_event(
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

fn validate_document(document: &EmbeddedDocument) -> Result<()> {
    if document.version != DOCUMENT_VERSION
        || document.adapters.len() > MAX_ADAPTERS
        || document.operations.len() > MAX_ACTIVE_OPERATIONS
        || document.completed.len() > MAX_COMPLETED_ACKS
    {
        bail!("unsupported or unbounded embedded runtime journal");
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

fn scoped_native_id(adapter_id: &str, native_id: &str) -> String {
    format!("{adapter_id}\0{native_id}")
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
        assert_eq!(
            queue
                .next_operation(&operation.adapter_id)
                .unwrap()
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
        let _ = queue.enqueue(request(&printer)).unwrap();
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
    }
}
