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
use piqae_agent_client::{AgentClient, DeviceRequestSigner};
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
            if revoke_remote_connector(&record, Arc::clone(&provider))
                .await
                .is_ok()
                && let Ok(mut registry) = registry.lock()
                && registry
                    .confirm_remote_revocation(&record.connector_id)
                    .is_ok()
            {
                retry_secure_cleanup(&mut registry, provider.as_ref());
            }
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

async fn revoke_remote_connector(
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
    AgentClient::new(record.control_plane_url.clone())
        .map_err(|_| CloudWorkerError::new("connector_origin_invalid"))?
        .revoke_connector(&identity, &record.connector_id)
        .await
        .map_err(Into::into)
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
            .pending_connector_accepts(&self.0.connector_id)
            .map_err(|_| CloudWorkerError::new("embedded_pending_accept_failed"))?;
        let mut releases = Vec::new();
        for intent in intents
            .into_iter()
            .filter(|intent| intent.route_proof().is_none())
        {
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
