//! Embedded-host composition of the canonical connector worker.
//!
//! Each connector owns an isolated `SQLite` queue, cursor, outbox and lease
//! intent. All connectors share one installation-wide adapter handoff journal,
//! which serializes access to the physical native print APIs.

use crate::connector_registry::{ConnectorRecord, ConnectorRegistry};
use crate::{
    AgentClientAuthority, CloudCommandApplier, CloudConnectorWorker, CloudWorkerError,
    ConnectorKeyError, ContentMaterializer, DurableOfferAcceptor, EmbeddedQueue, EventAcknowledger,
    GeneratedConnectorKey, HostBackedDeviceIdentity, InventorySnapshotProvider, NodeRuntime,
    PendingCloudAcceptance, PendingCloudRelease, SecureConnectorSigner, SecureKeyHandle,
    WakeReconciler,
};
use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use chrono::Utc;
use futures::StreamExt as _;
use piqae_agent_client::{AgentClient, ClientError, DeviceRequestSigner};
use piqae_domain::{AgentId, EventId, JobId};
use piqae_protocol::agent::{
    AgentAcceptJobRequest, AgentCommand, AgentSyncRequest, ContentDescriptor,
    InventoryProjectionAcknowledgement, JobOffer, PrinterGrant,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};

const MAX_EMBEDDED_CLOUD_CONTENT: usize = 16 * 1024 * 1024;

/// Returns whether disconnect must retain all local authority and queue
/// evidence until an N-1 authority is upgraded with exact compensation routes.
#[must_use]
pub fn connector_disconnect_requires_authority_upgrade(
    has_pending_acceptance: bool,
    abandon_error: Option<&ClientError>,
    reconciliation_error: Option<&ClientError>,
) -> bool {
    has_pending_acceptance
        && abandon_error.is_some_and(ClientError::is_endpoint_unsupported)
        && reconciliation_error.is_some_and(ClientError::is_endpoint_unsupported)
}

/// Coalesced host signal that runnable adapter work became available. It
/// carries no connector, job, document, lease or credential data.
pub trait WorkAvailableNotifier: std::fmt::Debug + Send + Sync {
    fn notify(&self);
    fn epoch(&self) -> u64;
    fn clear_if_epoch(&self, observed_epoch: u64);
    fn clear(&self);
}

#[derive(Debug)]
pub struct EmbeddedCloudSupervisor {
    stop: Arc<AtomicBool>,
    reconcile: Arc<ReconcileControl>,
    thread: Option<JoinHandle<()>>,
}

#[derive(Debug)]
pub struct EmbeddedCloudReconcileRequest {
    control: Arc<ReconcileControl>,
    generation: u64,
}

impl EmbeddedCloudReconcileRequest {
    /// Waits until the supervisor has completed a connector pass which began
    /// after this request was issued.
    #[must_use]
    pub fn wait(self, timeout: Duration) -> bool {
        self.control.wait(self.generation, timeout)
    }
}

#[derive(Debug, Default)]
struct ReconcileControl {
    requested_generation: AtomicU64,
    completed_generation: Mutex<u64>,
    completed: Condvar,
}

impl ReconcileControl {
    fn request(&self) -> u64 {
        self.requested_generation
            .fetch_add(1, Ordering::AcqRel)
            .saturating_add(1)
    }

    fn requested_after(&self, completed: u64) -> Option<u64> {
        let requested = self.requested_generation.load(Ordering::Acquire);
        (requested > completed).then_some(requested)
    }

    fn complete(&self, generation: u64) {
        let mut completed = self
            .completed_generation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *completed = (*completed).max(generation);
        drop(completed);
        self.completed.notify_all();
    }

    // The mutex guard must be moved into `wait_timeout_while`; Clippy cannot
    // follow that move through the condvar result and reports a false positive.
    #[allow(clippy::significant_drop_tightening)]
    fn wait(&self, target: u64, timeout: Duration) -> bool {
        let completed = self
            .completed_generation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *completed >= target {
            drop(completed);
            return true;
        }
        let (completed, _) = self
            .completed
            .wait_timeout_while(completed, timeout, |completed| *completed < target)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let reached_target = *completed >= target;
        drop(completed);
        reached_target
    }
}

impl EmbeddedCloudSupervisor {
    /// Starts dynamic connector reconciliation for one embedded installation.
    ///
    /// # Errors
    ///
    /// Returns an error if the bounded supervisor thread cannot be created.
    pub fn start(
        queue: Arc<Mutex<EmbeddedQueue>>,
        registry: Arc<Mutex<ConnectorRegistry>>,
        provider: Arc<dyn SecureConnectorSigner>,
        runtime: Arc<NodeRuntime>,
        work_notifier: Option<Arc<dyn WorkAvailableNotifier>>,
    ) -> Result<Self, std::io::Error> {
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let reconcile = Arc::new(ReconcileControl::default());
        let thread_reconcile = Arc::clone(&reconcile);
        let thread = std::thread::Builder::new()
            .name("piqae-embedded-cloud".into())
            .spawn(move || {
                let Ok(tokio) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                else {
                    return;
                };
                tokio.block_on(run_supervisor(
                    queue,
                    registry,
                    provider,
                    runtime,
                    thread_stop,
                    thread_reconcile,
                    work_notifier,
                ));
            })?;
        Ok(Self {
            stop,
            reconcile,
            thread: Some(thread),
        })
    }

    /// Requests one immediate connector pass and waits for the exact request
    /// generation to finish. Requests coalesce, while a request arriving in
    /// the middle of a pass is guaranteed a later pass rather than being
    /// mistaken for completion of the in-flight work.
    #[must_use]
    pub fn request_reconcile(&self) -> EmbeddedCloudReconcileRequest {
        let generation = self.reconcile.request();
        EmbeddedCloudReconcileRequest {
            control: Arc::clone(&self.reconcile),
            generation,
        }
    }

    #[must_use]
    pub fn reconcile_now(&self, timeout: Duration) -> bool {
        self.request_reconcile().wait(timeout)
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for EmbeddedCloudSupervisor {
    fn drop(&mut self) {
        self.stop();
    }
}

async fn run_supervisor(
    queue: Arc<Mutex<EmbeddedQueue>>,
    registry: Arc<Mutex<ConnectorRegistry>>,
    provider: Arc<dyn SecureConnectorSigner>,
    runtime: Arc<NodeRuntime>,
    stop: Arc<AtomicBool>,
    reconcile: Arc<ReconcileControl>,
    work_notifier: Option<Arc<dyn WorkAvailableNotifier>>,
) {
    let mut next_due = BTreeMap::<String, tokio::time::Instant>::new();
    let mut failures = BTreeMap::<String, u32>::new();
    let started_at = Utc::now();
    let mut completed_reconcile_generation = 0_u64;
    while !stop.load(Ordering::Acquire) {
        let forced_generation = reconcile.requested_after(completed_reconcile_generation);
        let pending_revocations = match registry.lock() {
            Ok(registry) => registry
                .pending_remote_revocations()
                .cloned()
                .collect::<Vec<_>>(),
            Err(_) => break,
        };
        for record in pending_revocations {
            let _ = finish_remote_connector_revocation(
                Arc::clone(&queue),
                Arc::clone(&registry),
                &record,
                Arc::clone(&provider),
            )
            .await;
        }
        let records = match registry.lock() {
            Ok(registry) => registry.enabled().cloned().collect::<Vec<_>>(),
            Err(_) => break,
        };
        let active = records
            .iter()
            .map(|record| record.connector_id.clone())
            .collect::<BTreeSet<_>>();
        next_due.retain(|id, _| active.contains(id));
        failures.retain(|id, _| active.contains(id));
        for record in records {
            if stop.load(Ordering::Acquire) {
                break;
            }
            if forced_generation.is_none()
                && next_due
                    .get(&record.connector_id)
                    .is_some_and(|due| *due > tokio::time::Instant::now())
            {
                continue;
            }
            let outcome = reconcile_connector(
                Arc::clone(&queue),
                Arc::clone(&registry),
                Arc::clone(&provider),
                Arc::clone(&runtime),
                &record,
                started_at,
                work_notifier.clone(),
            )
            .await;
            let delay = if let Ok(delay) = outcome {
                failures.remove(&record.connector_id);
                delay
            } else {
                let count = failures.entry(record.connector_id.clone()).or_default();
                *count = count.saturating_add(1);
                Duration::from_secs(1_u64.checked_shl((*count).min(5)).unwrap_or(30).min(30))
            };
            next_due.insert(record.connector_id, tokio::time::Instant::now() + delay);
        }
        if !stop.load(Ordering::Acquire)
            && let Some(generation) = forced_generation
        {
            completed_reconcile_generation = generation;
            reconcile.complete(generation);
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

#[cfg(test)]
mod reconcile_control_tests {
    use super::ReconcileControl;
    use std::{sync::Arc, time::Duration};

    #[test]
    fn exact_request_generation_does_not_complete_early() {
        let control = Arc::new(ReconcileControl::default());
        let first = control.request();
        let second = control.request();
        control.complete(first);
        assert!(control.wait(first, Duration::ZERO));
        assert!(!control.wait(second, Duration::from_millis(1)));
        control.complete(second);
        assert!(control.wait(second, Duration::ZERO));
    }

    #[test]
    fn requests_are_monotonic_and_coalescible() {
        let control = ReconcileControl::default();
        let first = control.request();
        let second = control.request();
        assert!(second > first);
        assert_eq!(control.requested_after(0), Some(second));
        control.complete(second);
        assert!(control.requested_after(second).is_none());
    }
}

async fn finish_remote_connector_revocation(
    queue: Arc<Mutex<EmbeddedQueue>>,
    registry: Arc<Mutex<ConnectorRegistry>>,
    record: &ConnectorRecord,
    provider: Arc<dyn SecureConnectorSigner>,
) -> Result<(), CloudWorkerError> {
    revoke_remote_connector(queue, record, Arc::clone(&provider)).await?;
    let mut registry = registry
        .lock()
        .map_err(|_| CloudWorkerError::new("connector_registry_unavailable"))?;
    registry
        .confirm_remote_revocation(&record.connector_id)
        .map_err(|_| CloudWorkerError::new("connector_revocation_persist_failed"))?;
    retry_secure_cleanup(&mut registry, provider.as_ref());
    drop(registry);
    Ok(())
}

async fn revoke_remote_connector(
    queue: Arc<Mutex<EmbeddedQueue>>,
    record: &ConnectorRecord,
    provider: Arc<dyn SecureConnectorSigner>,
) -> Result<(), CloudWorkerError> {
    let handle = record
        .secure_key_handle
        .clone()
        .ok_or_else(|| CloudWorkerError::new("secure_connector_key_missing"))?;
    let agent_id = record
        .agent_id
        .parse::<AgentId>()
        .map_err(|_| CloudWorkerError::new("connector_identity_invalid"))?;
    let identity = HostBackedDeviceIdentity::new(agent_id, handle, provider);
    let client = AgentClient::new(record.control_plane_url.clone())
        .map_err(|_| CloudWorkerError::new("connector_origin_invalid"))?;
    let intents = {
        let mut queue = queue
            .lock()
            .map_err(|_| CloudWorkerError::new("embedded_queue_unavailable"))?;
        queue
            .quarantine_invalid_connector_offers(&record.connector_id)
            .map_err(|_| CloudWorkerError::new("embedded_pending_quarantine_failed"))?;
        queue
            .pending_connector_accepts(&record.connector_id)
            .map_err(|_| CloudWorkerError::new("embedded_pending_accept_failed"))?
    };
    let mut abandon_after_revoke = Vec::new();
    for intent in intents {
        let pending = pending_acceptance(intent)?;
        let abandon = client
            .abandon_acceptance(&identity, pending.job_id, &pending.request)
            .await;
        if !matches!(&abandon, Ok(true)) {
            let reconciliation = match client
                .reconcile_acceptance(&identity, pending.job_id, &pending.request)
                .await
            {
                Ok(reconciliation) => reconciliation,
                Err(error)
                    if connector_disconnect_requires_authority_upgrade(
                        true,
                        abandon.as_ref().err(),
                        Some(&error),
                    ) =>
                {
                    return Err(CloudWorkerError::new(
                        "connector_authority_upgrade_required",
                    ));
                }
                Err(error) => return Err(error.into()),
            };
            if reconciliation.accepted && !reconciliation.fenced {
                return Err(abandon.err().map_or_else(
                    || CloudWorkerError::new("acceptance_compensation_rejected"),
                    CloudWorkerError::from,
                ));
            }
            if !reconciliation.connector_revoked && !reconciliation.fenced {
                abandon_after_revoke.push(pending.job_id);
                continue;
            }
        }
        let mut queue = queue
            .lock()
            .map_err(|_| CloudWorkerError::new("embedded_queue_unavailable"))?;
        queue
            .abandon_connector_offer(&record.connector_id, pending.job_id)
            .map_err(|_| CloudWorkerError::new("embedded_accept_abandon_failed"))?;
    }
    client
        .revoke_connector(&identity, &record.connector_id)
        .await
        .map_err(CloudWorkerError::from)?;
    let mut queue = queue
        .lock()
        .map_err(|_| CloudWorkerError::new("embedded_queue_unavailable"))?;
    for job_id in abandon_after_revoke {
        queue
            .abandon_connector_offer(&record.connector_id, job_id)
            .map_err(|_| CloudWorkerError::new("embedded_accept_abandon_failed"))?;
    }
    queue
        .complete_all_connector_release_cleanups(&record.connector_id)
        .map_err(|_| CloudWorkerError::new("embedded_release_cleanup_complete_failed"))?;
    drop(queue);
    Ok(())
}

fn retry_secure_cleanup(registry: &mut ConnectorRegistry, provider: &dyn SecureConnectorSigner) {
    for handle in registry.key_cleanup().to_vec() {
        if provider.delete(&handle).is_ok() {
            let _ = registry.confirm_key_cleanup(&handle);
        }
    }
}

async fn reconcile_connector(
    queue: Arc<Mutex<EmbeddedQueue>>,
    registry: Arc<Mutex<ConnectorRegistry>>,
    provider: Arc<dyn SecureConnectorSigner>,
    runtime: Arc<NodeRuntime>,
    record: &ConnectorRecord,
    started_at: chrono::DateTime<Utc>,
    work_notifier: Option<Arc<dyn WorkAvailableNotifier>>,
) -> Result<Duration, CloudWorkerError> {
    let handle = record
        .secure_key_handle
        .clone()
        .ok_or_else(|| CloudWorkerError::new("secure_connector_key_missing"))?;
    let agent_id = record
        .agent_id
        .parse::<AgentId>()
        .map_err(|_| CloudWorkerError::new("connector_identity_invalid"))?;
    let guarded_provider: Arc<dyn SecureConnectorSigner> = Arc::new(RevocationAwareProvider {
        connector_id: record.connector_id.clone(),
        secure_key_handle: handle.clone(),
        registry: Arc::clone(&registry),
        inner: provider,
    });
    let signer: Arc<dyn DeviceRequestSigner> = Arc::new(HostBackedDeviceIdentity::new(
        agent_id,
        handle,
        guarded_provider,
    ));
    let client = AgentClient::new(record.control_plane_url.clone())
        .map_err(|_| CloudWorkerError::new("connector_origin_invalid"))?;
    let common = EmbeddedConnectorContext {
        queue,
        connector_id: record.connector_id.clone(),
        agent_id,
        started_at,
        allowed: connector_allowed(record),
        runtime: Arc::clone(&runtime),
        registry,
        work_notifier,
    };
    let authority = AgentClientAuthority::new(client.clone(), Arc::clone(&signer));
    let mut worker = CloudConnectorWorker::new(
        authority,
        EmbeddedInventory(common.clone()),
        EmbeddedEvents(common.clone()),
        EmbeddedCommands(common.clone()),
        EmbeddedMaterializer { client, signer },
        EmbeddedAcceptor(common),
        EmbeddedWake,
        runtime,
    );
    worker
        .reconcile_once()
        .await
        .map(|value| value.next_poll_after)
}

#[derive(Clone)]
struct RevocationAwareProvider {
    connector_id: String,
    secure_key_handle: SecureKeyHandle,
    registry: Arc<Mutex<ConnectorRegistry>>,
    inner: Arc<dyn SecureConnectorSigner>,
}

impl std::fmt::Debug for RevocationAwareProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RevocationAwareProvider")
            .field("connector_id", &self.connector_id)
            .field("secure_key_handle", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

impl SecureConnectorSigner for RevocationAwareProvider {
    fn generate(&self, scope: &str) -> Result<GeneratedConnectorKey, ConnectorKeyError> {
        self.inner.generate(scope)
    }

    fn sign(
        &self,
        handle: &SecureKeyHandle,
        message: &[u8],
    ) -> Result<[u8; 64], ConnectorKeyError> {
        let enabled = self
            .registry
            .lock()
            .map_err(|_| ConnectorKeyError::Unavailable)?
            .enabled()
            .any(|record| {
                record.connector_id == self.connector_id
                    && record.secure_key_handle.as_ref().is_some_and(|active| {
                        active.as_str() == self.secure_key_handle.as_str()
                            && active.as_str() == handle.as_str()
                    })
            });
        if !enabled {
            return Err(ConnectorKeyError::Rejected);
        }
        self.inner.sign(handle, message)
    }

    fn delete(&self, handle: &SecureKeyHandle) -> Result<(), ConnectorKeyError> {
        self.inner.delete(handle)
    }
}

fn connector_allowed(record: &ConnectorRecord) -> Option<BTreeSet<String>> {
    match record.printer_grant {
        PrinterGrant::AllLocalPrinters => None,
        PrinterGrant::SelectedPrinters => {
            Some(record.allowed_printer_ids.iter().cloned().collect())
        }
    }
}

#[derive(Clone)]
struct EmbeddedConnectorContext {
    queue: Arc<Mutex<EmbeddedQueue>>,
    connector_id: String,
    agent_id: AgentId,
    started_at: chrono::DateTime<Utc>,
    allowed: Option<BTreeSet<String>>,
    runtime: Arc<NodeRuntime>,
    registry: Arc<Mutex<ConnectorRegistry>>,
    work_notifier: Option<Arc<dyn WorkAvailableNotifier>>,
}

impl std::fmt::Debug for EmbeddedConnectorContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EmbeddedConnectorContext")
            .field("connector_id", &self.connector_id)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug)]
struct EmbeddedInventory(EmbeddedConnectorContext);

#[async_trait]
impl InventorySnapshotProvider for EmbeddedInventory {
    async fn snapshot(&mut self, refresh: bool) -> Result<AgentSyncRequest, CloudWorkerError> {
        self.0
            .queue
            .lock()
            .map_err(|_| CloudWorkerError::new("embedded_queue_unavailable"))?
            .connector_sync_snapshot(
                &self.0.connector_id,
                self.0.agent_id,
                self.0.started_at,
                refresh,
                self.0.allowed.as_ref(),
                &self.0.runtime,
            )
            .map_err(|_| CloudWorkerError::new("embedded_snapshot_failed"))
    }

    async fn projection_acknowledged(
        &mut self,
        submitted_revision: u64,
        supported: bool,
        acknowledgement: Option<&InventoryProjectionAcknowledgement>,
    ) -> Result<(), CloudWorkerError> {
        if supported
            && acknowledgement
                .is_none_or(|acknowledgement| acknowledgement.revision != submitted_revision)
        {
            return Err(CloudWorkerError::new("inventory_projection_unacknowledged"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
struct EmbeddedEvents(EmbeddedConnectorContext);

#[async_trait]
impl EventAcknowledger for EmbeddedEvents {
    async fn acknowledge(
        &mut self,
        event_cursor: Option<EventId>,
        _handoff_sequence: Option<u64>,
        _diagnostics: &[String],
    ) -> Result<(), CloudWorkerError> {
        self.0
            .queue
            .lock()
            .map_err(|_| CloudWorkerError::new("embedded_queue_unavailable"))?
            .acknowledge_connector_response(&self.0.connector_id, event_cursor, None)
            .map_err(|_| CloudWorkerError::new("embedded_event_ack_failed"))
    }
}

#[derive(Clone, Debug)]
struct EmbeddedCommands(EmbeddedConnectorContext);

#[async_trait]
impl CloudCommandApplier for EmbeddedCommands {
    async fn apply(
        &mut self,
        command_cursor: Option<&str>,
        commands: Vec<AgentCommand>,
    ) -> Result<(), CloudWorkerError> {
        let mut queue = self
            .0
            .queue
            .lock()
            .map_err(|_| CloudWorkerError::new("embedded_queue_unavailable"))?;
        queue
            .apply_connector_commands(&self.0.connector_id, &commands)
            .and_then(|()| {
                queue.acknowledge_connector_response(&self.0.connector_id, None, command_cursor)
            })
            .map_err(|_| CloudWorkerError::new("embedded_command_failed"))
    }
}

#[derive(Clone)]
struct EmbeddedMaterializer {
    client: AgentClient,
    signer: Arc<dyn DeviceRequestSigner>,
}

impl std::fmt::Debug for EmbeddedMaterializer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("EmbeddedMaterializer([REDACTED])")
    }
}

#[async_trait]
impl ContentMaterializer for EmbeddedMaterializer {
    type Materialized = Vec<u8>;

    async fn materialize(&mut self, offer: &JobOffer) -> Result<Vec<u8>, CloudWorkerError> {
        match &offer.content {
            ContentDescriptor::InlineBase64 { data, bytes, .. } => {
                let content = STANDARD
                    .decode(data)
                    .map_err(|_| CloudWorkerError::new("embedded_content_invalid"))?;
                if content.len() > MAX_EMBEDDED_CLOUD_CONTENT
                    || bytes.is_some_and(|expected| {
                        expected != u64::try_from(content.len()).unwrap_or(u64::MAX)
                    })
                {
                    return Err(CloudWorkerError::new("embedded_content_too_large"));
                }
                Ok(content)
            }
            ContentDescriptor::Download { bytes, .. } => {
                if *bytes > u64::try_from(MAX_EMBEDDED_CLOUD_CONTENT).unwrap_or(u64::MAX) {
                    return Err(CloudWorkerError::new("embedded_content_too_large"));
                }
                let response = self
                    .client
                    .download_content(
                        self.signer.as_ref(),
                        offer.job.id,
                        offer.lease_id,
                        &offer.lease_token,
                    )
                    .await
                    .map_err(CloudWorkerError::from)?;
                let mut stream = response.bytes_stream();
                let mut content = Vec::with_capacity(usize::try_from(*bytes).unwrap_or_default());
                while let Some(chunk) = stream.next().await {
                    let chunk = chunk.map_err(|_| CloudWorkerError::new("transport_failed"))?;
                    if content.len().saturating_add(chunk.len()) > MAX_EMBEDDED_CLOUD_CONTENT {
                        return Err(CloudWorkerError::new("embedded_content_too_large"));
                    }
                    content.extend_from_slice(&chunk);
                }
                Ok(content)
            }
            ContentDescriptor::EncryptedDownload { .. } => {
                Err(CloudWorkerError::new("embedded_content_key_required"))
            }
            ContentDescriptor::Uri { .. } | ContentDescriptor::BusinessDocument { .. } => {
                Err(CloudWorkerError::new("embedded_content_unsupported"))
            }
        }
    }
}

#[derive(Clone, Debug)]
struct EmbeddedAcceptor(EmbeddedConnectorContext);

#[async_trait]
impl DurableOfferAcceptor<Vec<u8>> for EmbeddedAcceptor {
    async fn admission_valid(&mut self) -> Result<bool, CloudWorkerError> {
        Ok(self
            .0
            .registry
            .lock()
            .map_err(|_| CloudWorkerError::new("embedded_registry_unavailable"))?
            .enabled()
            .any(|record| record.connector_id == self.0.connector_id))
    }

    async fn pending(&mut self) -> Result<Vec<PendingCloudAcceptance>, CloudWorkerError> {
        let intents = self
            .0
            .queue
            .lock()
            .map_err(|_| CloudWorkerError::new("embedded_queue_unavailable"))?
            .pending_connector_accepts(&self.0.connector_id)
            .map_err(|_| CloudWorkerError::new("embedded_pending_accept_failed"))?;
        let mut pending = Vec::with_capacity(intents.len());
        for intent in intents {
            if intent.route_proof().is_some() {
                pending.push(pending_acceptance(intent)?);
            }
        }
        Ok(pending)
    }

    async fn invalid_pending(&mut self) -> Result<Vec<PendingCloudRelease>, CloudWorkerError> {
        let intents = self
            .0
            .queue
            .lock()
            .map_err(|_| CloudWorkerError::new("embedded_queue_unavailable"))?
            .quarantine_invalid_connector_offers(&self.0.connector_id)
            .map_err(|_| CloudWorkerError::new("embedded_pending_quarantine_failed"))?;
        let mut releases = Vec::new();
        for intent in intents {
            let job_id = intent
                .job_id
                .parse()
                .map_err(|_| CloudWorkerError::new("embedded_pending_accept_invalid"))?;
            let lease_id = intent
                .lease_id
                .parse()
                .map_err(|_| CloudWorkerError::new("embedded_pending_accept_invalid"))?;
            releases.push(PendingCloudRelease {
                job_id,
                request: piqae_protocol::agent::AgentReleaseLeaseRequest {
                    lease_id,
                    lease_token: intent.lease_token,
                    reason: "route_reservation_required".into(),
                },
            });
        }
        Ok(releases)
    }

    async fn complete_release_cleanup(&mut self, job_id: JobId) -> Result<(), CloudWorkerError> {
        self.0
            .queue
            .lock()
            .map_err(|_| CloudWorkerError::new("embedded_queue_unavailable"))?
            .complete_connector_release_cleanup(&self.0.connector_id, job_id)
            .map_err(|_| CloudWorkerError::new("embedded_release_cleanup_complete_failed"))
    }

    async fn prepare(
        &mut self,
        offer: &JobOffer,
        content: Vec<u8>,
    ) -> Result<PendingCloudAcceptance, CloudWorkerError> {
        let intent = self
            .0
            .queue
            .lock()
            .map_err(|_| CloudWorkerError::new("embedded_queue_unavailable"))?
            .prepare_connector_offer(
                &self.0.connector_id,
                offer,
                &content,
                self.0.allowed.as_ref(),
            )
            .map_err(|_| CloudWorkerError::new("embedded_accept_prepare_failed"))?;
        pending_acceptance(intent)
    }

    async fn activate(&mut self, job_id: JobId) -> Result<(), CloudWorkerError> {
        let mut queue = self
            .0
            .queue
            .lock()
            .map_err(|_| CloudWorkerError::new("embedded_queue_unavailable"))?;
        queue
            .activate_connector_offer(&self.0.connector_id, job_id)
            .map_err(|_| CloudWorkerError::new("embedded_accept_activate_failed"))?;
        drop(queue);
        if let Some(notifier) = &self.0.work_notifier {
            notifier.notify();
        }
        Ok(())
    }

    async fn confirm_remote_accept(&mut self, job_id: JobId) -> Result<(), CloudWorkerError> {
        self.0
            .queue
            .lock()
            .map_err(|_| CloudWorkerError::new("embedded_queue_unavailable"))?
            .confirm_connector_offer(&self.0.connector_id, job_id)
            .map_err(|_| CloudWorkerError::new("embedded_accept_confirm_failed"))
    }

    async fn abandon(&mut self, job_id: JobId) -> Result<(), CloudWorkerError> {
        self.0
            .queue
            .lock()
            .map_err(|_| CloudWorkerError::new("embedded_queue_unavailable"))?
            .abandon_connector_offer(&self.0.connector_id, job_id)
            .map_err(|_| CloudWorkerError::new("embedded_accept_abandon_failed"))
    }

    async fn has_durable_intent(&mut self, job_id: JobId) -> Result<bool, CloudWorkerError> {
        Ok(self
            .0
            .queue
            .lock()
            .map_err(|_| CloudWorkerError::new("embedded_queue_unavailable"))?
            .pending_connector_accepts(&self.0.connector_id)
            .map_err(|_| CloudWorkerError::new("embedded_pending_accept_failed"))?
            .iter()
            .any(|intent| intent.job_id == job_id.to_string()))
    }
}

fn pending_acceptance(
    intent: piqae_agent_storage::CloudAcceptIntent,
) -> Result<PendingCloudAcceptance, CloudWorkerError> {
    let route_proof = intent
        .route_proof()
        .ok_or_else(|| CloudWorkerError::new("embedded_route_reservation_missing"))?;
    Ok(PendingCloudAcceptance {
        job_id: intent
            .job_id
            .parse()
            .map_err(|_| CloudWorkerError::new("embedded_pending_accept_invalid"))?,
        remote_accept_confirmed: intent.remote_accept_confirmed,
        request: AgentAcceptJobRequest {
            lease_id: intent
                .lease_id
                .parse()
                .map_err(|_| CloudWorkerError::new("embedded_pending_accept_invalid"))?,
            lease_token: intent.lease_token,
            content_sha256: intent.content_sha256,
            local_sequence: intent.local_sequence,
            route_reservation_id: Some(
                route_proof
                    .reservation_id
                    .parse()
                    .map_err(|_| CloudWorkerError::new("embedded_route_reservation_invalid"))?,
            ),
            route_generation: Some(route_proof.generation),
            route_fencing_token: Some(route_proof.fencing_token),
        },
    })
}

#[derive(Clone, Copy, Debug)]
struct EmbeddedWake;

#[async_trait]
impl WakeReconciler for EmbeddedWake {
    async fn reconcile(&mut self) -> Result<(), CloudWorkerError> {
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod disconnect_compatibility_tests {
    use super::{
        connector_disconnect_requires_authority_upgrade, finish_remote_connector_revocation,
    };
    use crate::connector_registry::{ConnectorRecord, ConnectorRegistry};
    use crate::{
        ConnectorKeyError, EmbeddedQueue, GeneratedConnectorKey, SecureConnectorSigner,
        SecureKeyHandle,
    };
    use chrono::Utc;
    use ed25519_dalek::{Signer as _, SigningKey};
    use piqae_agent_client::ClientError;
    use piqae_agent_storage::{AcceptedJob, AgentStore, CloudRouteProof};
    use piqae_domain::{AgentId, JobId};
    use piqae_protocol::agent::PrinterGrant;
    use piqae_support_packs::SupportPackRegistry;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use url::Url;

    #[derive(Debug)]
    struct TestSigner(SigningKey);

    impl SecureConnectorSigner for TestSigner {
        fn generate(&self, _scope: &str) -> Result<GeneratedConnectorKey, ConnectorKeyError> {
            Err(ConnectorKeyError::Unavailable)
        }

        fn sign(
            &self,
            _handle: &SecureKeyHandle,
            message: &[u8],
        ) -> Result<[u8; 64], ConnectorKeyError> {
            Ok(self.0.sign(message).to_bytes())
        }

        fn delete(&self, _handle: &SecureKeyHandle) -> Result<(), ConnectorKeyError> {
            panic!("failed revocation must retain the secure key")
        }
    }

    async fn unsupported_authority() -> (Url, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = [0_u8; 16 * 1024];
                let _ = stream.read(&mut request).await.unwrap();
                let body = br#"{"error":{"code":"not_found"}}"#;
                stream
                    .write_all(
                        format!(
                            "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
                stream.write_all(body).await.unwrap();
            }
        });
        (Url::parse(&format!("http://{address}/")).unwrap(), task)
    }

    #[test]
    fn n_minus_one_disconnect_is_open_without_pending_and_fenced_with_pending_evidence() {
        let abandon = ClientError::Status {
            status: 404,
            body: "not found".into(),
        };
        let reconcile = ClientError::Status {
            status: 405,
            body: "method not allowed".into(),
        };
        assert!(!connector_disconnect_requires_authority_upgrade(
            false,
            Some(&abandon),
            Some(&reconcile)
        ));
        assert!(connector_disconnect_requires_authority_upgrade(
            true,
            Some(&abandon),
            Some(&reconcile)
        ));
        assert!(!connector_disconnect_requires_authority_upgrade(
            true,
            Some(&abandon),
            Some(&ClientError::Status {
                status: 500,
                body: "retry".into(),
            })
        ));
    }

    #[tokio::test]
    async fn embedded_n_minus_one_disconnect_retains_registry_key_and_pending_queue() {
        let directory = tempfile::tempdir().unwrap();
        let connector_id = "ncon_embedded_upgrade";
        let agent_id = AgentId::new();
        let handle = SecureKeyHandle::new("connector/test/upgrade".into()).unwrap();
        let (origin, server) = unsupported_authority().await;
        let record = ConnectorRecord {
            connector_id: connector_id.into(),
            agent_id: agent_id.to_string(),
            control_plane_url: origin,
            display_name: Some("Old authority".into()),
            workspace_name: Some("Fixture".into()),
            authorization_type: Some("workspace".into()),
            workspace_id: Some("wsp_fixture".into()),
            environment_id: Some("env_fixture".into()),
            requesting_service_account_id: None,
            manage_url: None,
            device_key_file: None,
            secure_key_handle: Some(handle.clone()),
            enabled: true,
            printer_grant: PrinterGrant::AllLocalPrinters,
            allowed_printer_ids: Vec::new(),
        };
        let mut registry = ConnectorRegistry::load(directory.path()).unwrap();
        registry
            .register_prepared_key(handle, [7; 32], Utc::now().timestamp_millis() + 60_000)
            .unwrap();
        registry.complete_prepared(record.clone()).unwrap();
        registry.revoke(connector_id).unwrap();
        let registry = Arc::new(Mutex::new(registry));

        let mut queue =
            EmbeddedQueue::open(directory.path(), SupportPackRegistry::default()).unwrap();
        queue.ensure_connector_queue(connector_id).unwrap();
        let job_id = JobId::new();
        let mut store = AgentStore::open(
            directory
                .path()
                .join("connectors")
                .join(connector_id)
                .join("agent.sqlite3"),
        )
        .unwrap();
        store
            .prepare_cloud_job(
                &AcceptedJob {
                    job_id: job_id.to_string(),
                    submission_id: "sub_upgrade".into(),
                    printer_id: "prn_upgrade".into(),
                    printer_native_id: "fake:upgrade".into(),
                    title: "Upgrade fence".into(),
                    content_sha256: "0".repeat(64),
                    content_path: directory
                        .path()
                        .join("fixture.pdf")
                        .to_string_lossy()
                        .into(),
                    content_kind: "pdf".into(),
                    options_json: "{}".into(),
                    expires_unix_ms: None,
                    accepted_unix_ms: Utc::now().timestamp_millis(),
                    cloud_managed: true,
                },
                &uuid::Uuid::new_v4().to_string(),
                "redacted-lease-token",
                Utc::now().timestamp_millis() + 60_000,
                &CloudRouteProof {
                    reservation_id: uuid::Uuid::new_v4().to_string(),
                    generation: 1,
                    fencing_token: "redacted-route-fence".into(),
                },
            )
            .unwrap();
        drop(store);
        let queue = Arc::new(Mutex::new(queue));
        let provider: Arc<dyn SecureConnectorSigner> =
            Arc::new(TestSigner(SigningKey::from_bytes(&[9; 32])));

        let error = finish_remote_connector_revocation(
            Arc::clone(&queue),
            Arc::clone(&registry),
            &record,
            provider,
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "connector_authority_upgrade_required");
        server.await.unwrap();
        assert_eq!(
            queue
                .lock()
                .unwrap()
                .pending_connector_accepts(connector_id)
                .unwrap()
                .len(),
            1
        );
        let restarted = ConnectorRegistry::load(directory.path()).unwrap();
        assert_eq!(restarted.pending_remote_revocations().count(), 1);
        assert!(restarted.key_cleanup().is_empty());
    }
}
