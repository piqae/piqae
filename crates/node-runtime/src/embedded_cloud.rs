//! Embedded-host composition of the canonical connector worker.
//!
//! Each connector owns an isolated `SQLite` queue, cursor, outbox and lease
//! intent. All connectors share one installation-wide adapter handoff journal,
//! which serializes access to the physical native print APIs.

use crate::connector_registry::{ConnectorRecord, ConnectorRegistry};
use crate::{
    AgentClientAuthority, CloudCommandApplication, CloudCommandApplier, CloudConnectorWorker,
    CloudWorkerError, ConnectorKeyError, ContentMaterializer, DurableOfferAcceptor, EmbeddedQueue,
    EventAcknowledger, GeneratedConnectorKey, HostBackedDeviceIdentity, HostConfigurationStore,
    InventorySnapshotProvider, NodeRuntime, PendingCloudAcceptance, PendingCloudRelease,
    SecureConnectorSigner, SecureKeyHandle, WakeReconciler,
};
use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use chrono::Utc;
use futures::StreamExt as _;
use piqae_agent_client::{AgentClient, ClientError, DeviceRequestSigner};
use piqae_domain::{AgentId, EventId, JobId};
use piqae_protocol::agent::{
    AgentAcceptJobRequest, AgentCommand, AgentIdentityUpdateRequest, AgentSyncRequest,
    ContentDescriptor, InventoryProjectionAcknowledgement, JobOffer, PrinterGrant,
};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
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
    stop_notify: Arc<tokio::sync::Notify>,
    reconcile: Arc<ReconcileControl>,
    thread: Option<JoinHandle<()>>,
}

#[derive(Debug)]
pub struct EmbeddedCloudReconcileRequest {
    control: Arc<ReconcileControl>,
    generation: u64,
}

impl EmbeddedCloudReconcileRequest {
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Returns the exact generation's result without blocking a host executor.
    #[must_use]
    pub fn poll(&self) -> Option<EmbeddedCloudReconcileOutcome> {
        self.control.poll(self.generation)
    }

    /// Waits until the supervisor has completed a connector pass which began
    /// after this request was issued.
    #[must_use]
    pub fn wait(self, timeout: Duration) -> Option<EmbeddedCloudReconcileOutcome> {
        self.control.wait(self.generation, timeout)
    }
}

/// Privacy-safe result for one explicitly requested supervisor generation.
/// Counts never identify a connector, workspace, job, printer or document.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub struct EmbeddedCloudReconcileOutcome {
    pub generation: u64,
    pub loop_completed: bool,
    pub connector_count: usize,
    pub succeeded_count: usize,
    pub failed_count: usize,
    pub success_scope: EmbeddedCloudSuccessScope,
    pub retryable: bool,
    pub failure_class: EmbeddedCloudFailureClass,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddedCloudSuccessScope {
    None,
    Partial,
    All,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddedCloudFailureClass {
    None,
    Transient,
    Authentication,
    Configuration,
    LocalState,
    Protocol,
    Mixed,
    Stopped,
}

#[derive(Debug, Default)]
struct ReconcileControl {
    requested_generation: AtomicU64,
    state: Mutex<ReconcileState>,
    completed: Condvar,
}

#[derive(Debug, Default)]
struct ReconcileState {
    completed_generation: u64,
    outcomes: VecDeque<EmbeddedCloudReconcileOutcome>,
    stopped: bool,
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

    fn complete(&self, outcome: EmbeddedCloudReconcileOutcome) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.completed_generation = state.completed_generation.max(outcome.generation);
        state.outcomes.push_back(outcome);
        while state.outcomes.len() > 64 {
            state.outcomes.pop_front();
        }
        drop(state);
        self.completed.notify_all();
    }

    fn stop(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.stopped = true;
        drop(state);
        self.completed.notify_all();
    }

    fn poll(&self, target: u64) -> Option<EmbeddedCloudReconcileOutcome> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .outcomes
            .iter()
            .find(|outcome| outcome.generation >= target)
            .copied()
    }

    // The mutex guard must be moved into `wait_timeout_while`; Clippy cannot
    // follow that move through the condvar result and reports a false positive.
    #[allow(clippy::significant_drop_tightening)]
    fn wait(&self, target: u64, timeout: Duration) -> Option<EmbeddedCloudReconcileOutcome> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.completed_generation >= target || state.stopped {
            drop(state);
            return self.poll(target);
        }
        let (state, _) = self
            .completed
            .wait_timeout_while(state, timeout, |state| {
                state.completed_generation < target && !state.stopped
            })
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let outcome = state
            .outcomes
            .iter()
            .find(|outcome| outcome.generation >= target)
            .copied();
        drop(state);
        outcome
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
        host_configuration: Option<Arc<Mutex<HostConfigurationStore>>>,
        work_notifier: Option<Arc<dyn WorkAvailableNotifier>>,
    ) -> Result<Self, std::io::Error> {
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let stop_notify = Arc::new(tokio::sync::Notify::new());
        let thread_stop_notify = Arc::clone(&stop_notify);
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
                    host_configuration,
                    thread_stop,
                    thread_stop_notify,
                    thread_reconcile,
                    work_notifier,
                ));
            })?;
        Ok(Self {
            stop,
            stop_notify,
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

    /// Polls an earlier request without waiting on the caller's thread.
    #[must_use]
    pub fn poll_reconcile(&self, generation: u64) -> Option<EmbeddedCloudReconcileOutcome> {
        self.reconcile.poll(generation)
    }

    #[must_use]
    pub fn reconcile_now(&self, timeout: Duration) -> bool {
        self.request_reconcile().wait(timeout).is_some()
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        // This supervisor has a single consumer. `notify_one` retains a permit
        // when stop races the consumer registering its next wait, so shutdown
        // cannot fall through to a full network timeout.
        self.stop_notify.notify_one();
        self.reconcile.stop();
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

#[derive(Debug, Default)]
struct ConnectorRetrySchedule {
    next_due: BTreeMap<String, tokio::time::Instant>,
    failures: BTreeMap<String, u32>,
}

impl ConnectorRetrySchedule {
    fn is_due(&self, connector_id: &str, now: tokio::time::Instant) -> bool {
        self.next_due
            .get(connector_id)
            .is_none_or(|due| *due <= now)
    }

    fn record_failure(&mut self, connector_id: &str, now: tokio::time::Instant) -> Duration {
        let count = self.failures.entry(connector_id.to_owned()).or_default();
        *count = count.saturating_add(1);
        let delay = Duration::from_secs(1_u64.checked_shl((*count).min(5)).unwrap_or(30).min(30));
        self.next_due.insert(connector_id.to_owned(), now + delay);
        delay
    }

    fn record_success(
        &mut self,
        connector_id: &str,
        now: tokio::time::Instant,
        next_delay: Option<Duration>,
    ) {
        self.failures.remove(connector_id);
        if let Some(delay) = next_delay {
            self.next_due.insert(connector_id.to_owned(), now + delay);
        } else {
            self.next_due.remove(connector_id);
        }
    }

    fn retain(&mut self, connector_ids: &BTreeSet<String>) {
        self.next_due.retain(|id, _| connector_ids.contains(id));
        self.failures.retain(|id, _| connector_ids.contains(id));
    }
}

#[allow(
    clippy::cognitive_complexity,
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "the explicit durable boundaries and interrupt channel stay visible in one supervisor loop"
)]
async fn run_supervisor(
    queue: Arc<Mutex<EmbeddedQueue>>,
    registry: Arc<Mutex<ConnectorRegistry>>,
    provider: Arc<dyn SecureConnectorSigner>,
    runtime: Arc<NodeRuntime>,
    host_configuration: Option<Arc<Mutex<HostConfigurationStore>>>,
    stop: Arc<AtomicBool>,
    stop_notify: Arc<tokio::sync::Notify>,
    reconcile: Arc<ReconcileControl>,
    work_notifier: Option<Arc<dyn WorkAvailableNotifier>>,
) {
    let mut retry_schedule = ConnectorRetrySchedule::default();
    let started_at = Utc::now();
    let mut completed_reconcile_generation = 0_u64;
    'supervisor: while !stop.load(Ordering::Acquire) {
        let forced_generation = reconcile.requested_after(completed_reconcile_generation);
        let pending_revocations = match registry.lock() {
            Ok(registry) => registry
                .pending_remote_revocations()
                .cloned()
                .collect::<Vec<_>>(),
            Err(_) => break,
        };
        let mut tracked = pending_revocations
            .iter()
            .map(|record| record.connector_id.clone())
            .collect::<BTreeSet<_>>();
        for record in pending_revocations {
            if stop.load(Ordering::Acquire) {
                break;
            }
            let now = tokio::time::Instant::now();
            if !retry_schedule.is_due(&record.connector_id, now) {
                continue;
            }
            let outcome = tokio::select! {
                biased;
                () = stop_notify.notified() => break 'supervisor,
                outcome = finish_remote_connector_revocation(
                    Arc::clone(&queue),
                    Arc::clone(&registry),
                    &record,
                    Arc::clone(&provider),
                ) => outcome,
            };
            let completed_at = tokio::time::Instant::now();
            if outcome.is_ok() {
                retry_schedule.record_success(&record.connector_id, completed_at, None);
            } else {
                retry_schedule.record_failure(&record.connector_id, completed_at);
            }
        }
        let records = match registry.lock() {
            Ok(registry) => registry.enabled().cloned().collect::<Vec<_>>(),
            Err(_) => break,
        };
        tracked.extend(records.iter().map(|record| record.connector_id.clone()));
        retry_schedule.retain(&tracked);
        let connector_count = records.len();
        let mut succeeded_count = 0_usize;
        let mut pass_failures = Vec::new();
        for record in records {
            if stop.load(Ordering::Acquire) {
                break;
            }
            let now = tokio::time::Instant::now();
            if forced_generation.is_none() && !retry_schedule.is_due(&record.connector_id, now) {
                continue;
            }
            let outcome = tokio::select! {
                biased;
                () = stop_notify.notified() => break 'supervisor,
                outcome = reconcile_connector(
                    Arc::clone(&queue),
                    Arc::clone(&registry),
                    Arc::clone(&provider),
                    Arc::clone(&runtime),
                    host_configuration.clone(),
                    &record,
                    started_at,
                    work_notifier.clone(),
                ) => outcome,
            };
            let completed_at = tokio::time::Instant::now();
            match outcome {
                Ok(delay) => {
                    succeeded_count = succeeded_count.saturating_add(1);
                    retry_schedule.record_success(&record.connector_id, completed_at, Some(delay));
                }
                Err(error) => {
                    pass_failures.push(classify_failure(error));
                    retry_schedule.record_failure(&record.connector_id, completed_at);
                }
            }
        }
        if !stop.load(Ordering::Acquire)
            && let Some(generation) = forced_generation
        {
            completed_reconcile_generation = generation;
            let failed_count = pass_failures.len();
            reconcile.complete(EmbeddedCloudReconcileOutcome {
                generation,
                loop_completed: true,
                connector_count,
                succeeded_count,
                failed_count,
                success_scope: if failed_count == 0 {
                    EmbeddedCloudSuccessScope::All
                } else if succeeded_count > 0 {
                    EmbeddedCloudSuccessScope::Partial
                } else {
                    EmbeddedCloudSuccessScope::None
                },
                retryable: !pass_failures.is_empty()
                    && pass_failures.iter().all(|failure| failure.retryable),
                failure_class: combined_failure_class(&pass_failures),
            });
        }
        tokio::select! {
            () = stop_notify.notified() => break,
            () = tokio::time::sleep(Duration::from_millis(250)) => {}
        }
    }
}

#[derive(Clone, Copy)]
struct ClassifiedFailure {
    class: EmbeddedCloudFailureClass,
    retryable: bool,
}

fn classify_failure(error: CloudWorkerError) -> ClassifiedFailure {
    let (class, retryable) = match error.code {
        "transport_failed" | "server_retryable" | "lease_renewal_timeout" => {
            (EmbeddedCloudFailureClass::Transient, true)
        }
        "unauthorized"
        | "signing_failed"
        | "authorization_failed"
        | "invalid_signature"
        | "connector_admission_revoked"
        | "secure_connector_key_missing" => (EmbeddedCloudFailureClass::Authentication, false),
        "connector_origin_invalid"
        | "connector_identity_invalid"
        | "request_invalid"
        | "embedded_content_invalid"
        | "embedded_content_too_large"
        | "embedded_content_unsupported"
        | "embedded_content_key_required"
        | "content_invalid"
        | "embedded_pending_accept_invalid"
        | "embedded_route_reservation_missing"
        | "embedded_route_reservation_invalid" => (EmbeddedCloudFailureClass::Configuration, false),
        "server_rejected" | "response_too_large" | "inventory_projection_unacknowledged" => {
            (EmbeddedCloudFailureClass::Protocol, false)
        }
        "durable_write_failed"
        | "embedded_queue_unavailable"
        | "embedded_registry_unavailable"
        | "embedded_event_ack_failed"
        | "embedded_pending_accept_failed"
        | "embedded_accept_prepare_failed"
        | "embedded_accept_activate_failed"
        | "embedded_accept_abandon_failed" => (EmbeddedCloudFailureClass::LocalState, true),
        // New failure codes are not retryable until their safety has been
        // classified explicitly. This keeps host wake loops fail-closed.
        _ => (EmbeddedCloudFailureClass::LocalState, false),
    };
    ClassifiedFailure { class, retryable }
}

fn combined_failure_class(failures: &[ClassifiedFailure]) -> EmbeddedCloudFailureClass {
    let Some(first) = failures.first() else {
        return EmbeddedCloudFailureClass::None;
    };
    if failures.iter().all(|failure| failure.class == first.class) {
        first.class
    } else {
        EmbeddedCloudFailureClass::Mixed
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test fixtures fail loudly")]
mod reconcile_control_tests {
    use super::{
        CloudWorkerError, EmbeddedCloudFailureClass, EmbeddedCloudReconcileOutcome,
        EmbeddedCloudSuccessScope, EmbeddedCloudSupervisor, ReconcileControl, classify_failure,
    };
    use crate::{
        AvailabilityClass, ConnectorKeyError, EmbeddedQueue, GeneratedConnectorKey,
        HostCapabilities, HostKind, NodeRuntime, NodeRuntimeMode, PrinterTransport,
        RuntimeConfiguration, SecureConnectorSigner, SecureKeyHandle,
        connector_registry::ConnectorRegistry,
    };
    use piqae_support_packs::SupportPackRegistry;
    use std::{collections::BTreeSet, sync::Arc, time::Duration};

    #[derive(Debug)]
    struct UnusedSigner;

    impl SecureConnectorSigner for UnusedSigner {
        fn generate(&self, _scope: &str) -> Result<GeneratedConnectorKey, ConnectorKeyError> {
            Err(ConnectorKeyError::Unavailable)
        }

        fn sign(
            &self,
            _handle: &SecureKeyHandle,
            _message: &[u8],
        ) -> Result<[u8; 64], ConnectorKeyError> {
            Err(ConnectorKeyError::Unavailable)
        }

        fn delete(&self, _handle: &SecureKeyHandle) -> Result<(), ConnectorKeyError> {
            Err(ConnectorKeyError::Unavailable)
        }
    }

    #[test]
    fn exact_request_generation_does_not_complete_early() {
        let control = Arc::new(ReconcileControl::default());
        let first = control.request();
        let second = control.request();
        control.complete(success(first));
        assert_eq!(control.wait(first, Duration::ZERO), Some(success(first)));
        assert_eq!(control.wait(second, Duration::from_millis(1)), None);
        control.complete(success(second));
        assert_eq!(control.wait(second, Duration::ZERO), Some(success(second)));
    }

    #[test]
    fn request_arriving_mid_pass_requires_a_later_generation() {
        let control = ReconcileControl::default();
        let first = control.request();
        let pass_generation = control.requested_after(0).unwrap();
        assert_eq!(pass_generation, first);

        // This request arrives after the supervisor captured its pass fence.
        let mid_pass = control.request();
        control.complete(success(pass_generation));

        assert_eq!(control.poll(first), Some(success(first)));
        assert_eq!(control.poll(mid_pass), None);
        assert_eq!(control.requested_after(pass_generation), Some(mid_pass));
    }

    #[test]
    fn partial_failure_outcome_never_contains_connector_identity() {
        let outcome = EmbeddedCloudReconcileOutcome {
            generation: 7,
            loop_completed: true,
            connector_count: 3,
            succeeded_count: 2,
            failed_count: 1,
            success_scope: EmbeddedCloudSuccessScope::Partial,
            retryable: true,
            failure_class: EmbeddedCloudFailureClass::Transient,
        };
        let encoded = serde_json::to_string(&outcome).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&encoded).unwrap()["failed_count"],
            1
        );
        assert!(!encoded.contains("connector_id"));
        assert!(!encoded.contains("workspace"));
    }

    #[test]
    fn only_explicitly_classified_failures_are_retryable() {
        assert!(classify_failure(CloudWorkerError::new("transport_failed")).retryable);
        assert!(classify_failure(CloudWorkerError::new("embedded_queue_unavailable")).retryable);
        assert!(!classify_failure(CloudWorkerError::new("invalid_signature")).retryable);
        assert!(!classify_failure(CloudWorkerError::new("new_unclassified_code")).retryable);
    }

    #[test]
    fn requests_are_monotonic_and_coalescible() {
        let control = ReconcileControl::default();
        let first = control.request();
        let second = control.request();
        assert!(second > first);
        assert_eq!(control.requested_after(0), Some(second));
        control.complete(success(second));
        assert!(control.requested_after(second).is_none());
    }

    #[test]
    fn stop_wakes_waiters_without_claiming_completion() {
        let control = Arc::new(ReconcileControl::default());
        let generation = control.request();
        let waiter = {
            let control = Arc::clone(&control);
            std::thread::spawn(move || control.wait(generation, Duration::from_secs(30)))
        };
        control.stop();
        assert_eq!(waiter.join().unwrap(), None);
    }

    #[test]
    fn real_supervisor_completes_empty_cloud_generation_and_stops_promptly() {
        let directory = tempfile::tempdir().unwrap();
        let runtime = Arc::new(
            NodeRuntime::start(RuntimeConfiguration {
                data_directory: directory.path().join("runtime"),
                mode: NodeRuntimeMode::CloudCapable,
                host: HostCapabilities {
                    host_kind: HostKind::EmbeddedApplication,
                    availability: AvailabilityClass::BackgroundOpportunistic,
                    secure_storage: true,
                    local_ipc_broker: false,
                    can_prevent_idle_sleep_during_handoff: false,
                    can_receive_remote_wake_hint: true,
                    printer_transports: BTreeSet::<PrinterTransport>::new(),
                },
            })
            .unwrap(),
        );
        let queue = Arc::new(std::sync::Mutex::new(
            EmbeddedQueue::open(
                directory.path().join("embedded"),
                SupportPackRegistry::default(),
            )
            .unwrap(),
        ));
        let registry = Arc::new(std::sync::Mutex::new(
            ConnectorRegistry::load(directory.path().join("embedded")).unwrap(),
        ));
        let mut supervisor = EmbeddedCloudSupervisor::start(
            queue,
            registry,
            Arc::new(UnusedSigner),
            runtime,
            None,
            None,
        )
        .unwrap();

        let request = supervisor.request_reconcile();
        let generation = request.generation();
        let outcome = request.wait(Duration::from_secs(2)).unwrap();
        assert_eq!(outcome.generation, generation);
        assert!(outcome.loop_completed);
        assert_eq!(outcome.connector_count, 0);
        assert_eq!(outcome.success_scope, EmbeddedCloudSuccessScope::All);

        let started = std::time::Instant::now();
        supervisor.stop();
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    const fn success(generation: u64) -> EmbeddedCloudReconcileOutcome {
        EmbeddedCloudReconcileOutcome {
            generation,
            loop_completed: true,
            connector_count: 0,
            succeeded_count: 0,
            failed_count: 0,
            success_scope: EmbeddedCloudSuccessScope::All,
            retryable: false,
            failure_class: EmbeddedCloudFailureClass::None,
        }
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
    host_configuration: Option<Arc<Mutex<HostConfigurationStore>>>,
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
        registry: Arc::clone(&registry),
        work_notifier,
    };
    let authority = AgentClientAuthority::new(client.clone(), Arc::clone(&signer));
    let mut worker = CloudConnectorWorker::new(
        authority,
        EmbeddedInventory(common.clone()),
        EmbeddedEvents(common.clone()),
        EmbeddedCommands(common.clone()),
        EmbeddedMaterializer {
            client: client.clone(),
            signer: Arc::clone(&signer),
        },
        EmbeddedAcceptor(common),
        EmbeddedWake,
        runtime,
    );
    let next_poll_after = worker.reconcile_once().await?.next_poll_after;
    if let Some(host_configuration) = host_configuration {
        // Identity metadata is intentionally reconciled after jobs, events and
        // inventory. Failure or an N-1 authority therefore cannot block the
        // connector's printing path.
        reconcile_embedded_connector_identity(
            &client,
            signer.as_ref(),
            &registry,
            record,
            &host_configuration,
        )
        .await;
    }
    Ok(next_poll_after)
}

async fn reconcile_embedded_connector_identity(
    client: &AgentClient,
    signer: &dyn DeviceRequestSigner,
    registry: &Arc<Mutex<ConnectorRegistry>>,
    record: &ConnectorRecord,
    host_configuration: &Arc<Mutex<HostConfigurationStore>>,
) {
    let (local_revision, identity) = match host_configuration.lock() {
        Ok(store) => {
            let (revision, configuration) = store.snapshot();
            (revision, configuration.identity)
        }
        Err(_) => return,
    };
    if record.node_identity_applied_local_revision == Some(local_revision)
        || record.node_identity_conflict_local_revision == Some(local_revision)
    {
        return;
    }
    let expected_revision = record
        .node_identity_conflict_revision
        .or(record.node_identity_revision)
        .unwrap_or(1);
    let request = AgentIdentityUpdateRequest {
        expected_revision,
        display_name: identity.display_name,
        site: identity.site,
        location: identity.location,
        labels: identity.labels,
    };
    match client.update_node_identity(signer, &request).await {
        Ok(updated) => {
            if let Ok(mut registry) = registry.lock() {
                let _ = registry.update_identity_reconciliation(
                    &record.connector_id,
                    Some(updated.revision),
                    Some(local_revision),
                    None,
                    None,
                );
            }
        }
        Err(ClientError::NodeIdentityRevisionConflict { current_revision }) => {
            if let Ok(mut registry) = registry.lock() {
                let _ = registry.update_identity_reconciliation(
                    &record.connector_id,
                    record.node_identity_revision,
                    record.node_identity_applied_local_revision,
                    Some(current_revision),
                    Some(local_revision),
                );
            }
        }
        // Transport and N-1 unsupported-route failures remain pending. A
        // later normal connector pass retries them without affecting queue or
        // inventory reconciliation.
        Err(_) => {}
    }
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
    ) -> Result<CloudCommandApplication, CloudWorkerError> {
        let mut queue = self
            .0
            .queue
            .lock()
            .map_err(|_| CloudWorkerError::new("embedded_queue_unavailable"))?;
        queue
            .apply_connector_commands_recovering(&self.0.connector_id, command_cursor, &commands)
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
        ConnectorRetrySchedule, connector_disconnect_requires_authority_upgrade,
        finish_remote_connector_revocation, reconcile_embedded_connector_identity,
    };
    use crate::connector_registry::{ConnectorRecord, ConnectorRegistry};
    use crate::{
        ConnectionPolicy, ConnectorKeyError, EmbeddedQueue, GeneratedConnectorKey,
        HostBackedDeviceIdentity, HostConfiguration, HostConfigurationStore, HostProduct,
        InstalledHostPolicy, NodeIdentity, SecureConnectorSigner, SecureKeyHandle,
    };
    use chrono::Utc;
    use ed25519_dalek::{Signer as _, SigningKey};
    use piqae_agent_client::{AgentClient, ClientError, DeviceRequestSigner};
    use piqae_agent_storage::{AcceptedJob, AgentStore, CloudRouteProof};
    use piqae_domain::{AgentId, JobId};
    use piqae_protocol::agent::PrinterGrant;
    use piqae_support_packs::SupportPackRegistry;
    use std::{
        collections::BTreeSet,
        sync::{Arc, Mutex},
        time::Duration,
    };
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

    async fn identity_authority() -> (
        Url,
        tokio::sync::mpsc::Receiver<String>,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (sender, receiver) = tokio::sync::mpsc::channel(2);
        let task = tokio::spawn(async move {
            for response_index in 0..2 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = vec![0_u8; 16 * 1024];
                let length = stream.read(&mut request).await.unwrap();
                request.truncate(length);
                sender
                    .send(String::from_utf8_lossy(&request).into_owned())
                    .await
                    .unwrap();
                let (status, body) = if response_index == 0 {
                    (
                        "409 Conflict",
                        r#"{"error":{"code":"node_identity_revision_conflict","message":"conflict","request_id":"req_test","retryable":false,"details":{"current_revision":5}}}"#,
                    )
                } else {
                    (
                        "200 OK",
                        r#"{"revision":6,"identity":{"display_name":"Dispatch iPad","site":null,"location":null,"labels":[]}}"#,
                    )
                };
                stream
                    .write_all(
                        format!(
                            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
            }
        });
        (
            Url::parse(&format!("http://{address}/")).unwrap(),
            receiver,
            task,
        )
    }

    #[tokio::test]
    async fn connector_identity_conflict_waits_for_explicit_new_local_revision() {
        let directory = tempfile::tempdir().unwrap();
        let (origin, mut requests, server) = identity_authority().await;
        let agent_id = AgentId::new();
        let handle = SecureKeyHandle::new("connector/test/identity".into()).unwrap();
        let mut record = ConnectorRecord {
            connector_id: "ncon_identity".into(),
            agent_id: agent_id.to_string(),
            control_plane_url: origin.clone(),
            display_name: None,
            workspace_name: None,
            authorization_type: None,
            workspace_id: None,
            environment_id: None,
            requesting_service_account_id: None,
            manage_url: None,
            device_key_file: None,
            secure_key_handle: Some(handle.clone()),
            enabled: true,
            printer_grant: PrinterGrant::AllLocalPrinters,
            allowed_printer_ids: Vec::new(),
            node_identity_revision: Some(1),
            node_identity_applied_local_revision: None,
            node_identity_conflict_revision: None,
            node_identity_conflict_local_revision: None,
        };
        let mut registry = ConnectorRegistry::load(directory.path().join("embedded")).unwrap();
        registry
            .register_prepared_key(
                handle.clone(),
                [4; 32],
                Utc::now().timestamp_millis() + 60_000,
            )
            .unwrap();
        registry.complete_prepared(record.clone()).unwrap();
        let registry = Arc::new(Mutex::new(registry));
        let configuration = HostConfiguration {
            contract: 1,
            product: HostProduct::Embedded,
            application_id: "com.example.pos".into(),
            identity: NodeIdentity::new("Kitchen iPad", None, None, Vec::new()).unwrap(),
            installed_host_policy: InstalledHostPolicy::IsolatedApplication,
            connection_policy: ConnectionPolicy::user_managed(),
        };
        let store = Arc::new(Mutex::new(
            HostConfigurationStore::open_or_create(directory.path(), configuration).unwrap(),
        ));
        let signer: Arc<dyn DeviceRequestSigner> = Arc::new(HostBackedDeviceIdentity::new(
            agent_id,
            handle,
            Arc::new(TestSigner(SigningKey::from_bytes(&[3; 32]))),
        ));
        let client = AgentClient::new(origin).unwrap();

        reconcile_embedded_connector_identity(&client, signer.as_ref(), &registry, &record, &store)
            .await;
        record = registry.lock().unwrap().records().next().unwrap().clone();
        assert_eq!(record.node_identity_conflict_revision, Some(5));
        assert_eq!(record.node_identity_conflict_local_revision, Some(1));
        assert_eq!(
            store
                .lock()
                .unwrap()
                .configuration()
                .identity
                .display_name,
            "Kitchen iPad"
        );

        // The exact conflicting edit is suppressed and produces no request.
        reconcile_embedded_connector_identity(&client, signer.as_ref(), &registry, &record, &store)
            .await;
        assert!(requests.try_recv().is_ok());
        assert!(requests.try_recv().is_err());

        store
            .lock()
            .unwrap()
            .update_identity(
                1,
                NodeIdentity::new("Dispatch iPad", None, None, Vec::new()).unwrap(),
            )
            .unwrap();
        reconcile_embedded_connector_identity(&client, signer.as_ref(), &registry, &record, &store)
            .await;
        let second = requests.recv().await.unwrap();
        assert!(second.contains(r#""expected_revision":5"#));
        let updated = registry.lock().unwrap().records().next().unwrap().clone();
        assert_eq!(updated.node_identity_revision, Some(6));
        assert_eq!(updated.node_identity_applied_local_revision, Some(2));
        assert_eq!(updated.node_identity_conflict_revision, None);
        server.await.unwrap();
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

    #[test]
    fn pending_revocation_failures_back_off_and_success_clears_retry_state() {
        let connector_id = "ncon_retry";
        let started_at = tokio::time::Instant::now();
        let mut schedule = ConnectorRetrySchedule::default();
        assert!(schedule.is_due(connector_id, started_at));

        assert_eq!(
            schedule.record_failure(connector_id, started_at),
            Duration::from_secs(2)
        );
        assert!(!schedule.is_due(connector_id, started_at + Duration::from_millis(250)));
        assert!(schedule.is_due(connector_id, started_at + Duration::from_secs(2)));

        let second_attempt = started_at + Duration::from_secs(2);
        assert_eq!(
            schedule.record_failure(connector_id, second_attempt),
            Duration::from_secs(4)
        );
        assert!(!schedule.is_due(connector_id, second_attempt + Duration::from_secs(3)));
        assert!(schedule.is_due(connector_id, second_attempt + Duration::from_secs(4)));

        let mut capped_at = second_attempt + Duration::from_secs(4);
        for _ in 0..8 {
            assert!(schedule.record_failure(connector_id, capped_at) <= Duration::from_secs(30));
            capped_at += Duration::from_secs(30);
        }

        schedule.record_success(connector_id, capped_at, None);
        assert!(schedule.is_due(connector_id, capped_at));
        assert!(!schedule.failures.contains_key(connector_id));
        assert!(!schedule.next_due.contains_key(connector_id));

        schedule.record_failure("removed", started_at);
        schedule.retain(&BTreeSet::from([connector_id.to_owned()]));
        assert!(!schedule.failures.contains_key("removed"));
        assert!(!schedule.next_due.contains_key("removed"));
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
            node_identity_revision: None,
            node_identity_applied_local_revision: None,
            node_identity_conflict_revision: None,
            node_identity_conflict_local_revision: None,
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
