//! One cloud-connector state machine shared by installed and embedded nodes.
//!
//! Transport, inventory, document materialisation, durable queue admission and
//! command execution are injected capabilities. The ordering in this module is
//! deliberately not injectable: a response acknowledgement is persisted before
//! commands are applied, a lease is renewed while content is materialised and
//! local intent is committed, and remote acceptance happens only after that
//! durable intent exists.

use crate::NodeRuntime;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use piqae_agent_client::{AgentClient, ClientError, DeviceRequestSigner};
use piqae_domain::{EventId, JobId, JobState};
use piqae_protocol::agent::{
    AgentAcceptJobRequest, AgentCommand, AgentReleaseLeaseRequest, AgentRenewLeaseRequest,
    AgentSyncRequest, AgentSyncResponse, InventoryProjectionAcknowledgement, JobOffer,
};
use std::{fmt, future::Future, sync::Arc, time::Duration};
use thiserror::Error;

const LEASE_RENEWAL_INTERVAL: Duration = Duration::from_secs(10);
const LEASE_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// A redacted failure crossing the reusable worker boundary.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
#[error("cloud connector operation failed: {code}")]
pub struct CloudWorkerError {
    pub code: &'static str,
}

impl CloudWorkerError {
    #[must_use]
    pub const fn new(code: &'static str) -> Self {
        Self { code }
    }
}

impl From<ClientError> for CloudWorkerError {
    fn from(value: ClientError) -> Self {
        let code = match value {
            ClientError::Unauthorized { .. } => "unauthorized",
            ClientError::Signing => "signing_failed",
            ClientError::Http(_) => "transport_failed",
            ClientError::Status { status, .. }
                if matches!(status, 408 | 425 | 429) || status >= 500 =>
            {
                "server_retryable"
            }
            ClientError::Status { .. } => "server_rejected",
            ClientError::ResponseTooLarge => "response_too_large",
            ClientError::DeviceAuthorization => "authorization_failed",
            ClientError::Url(_) | ClientError::Header(_) | ClientError::Json(_) => {
                "request_invalid"
            }
        };
        Self::new(code)
    }
}

/// Signed authority operations. Implementations retain the opaque signer and
/// must never expose it through errors or diagnostics.
#[async_trait]
pub trait ConnectorAuthority: fmt::Debug + Send + Sync {
    async fn sync(&self, request: &AgentSyncRequest)
    -> Result<AgentSyncResponse, CloudWorkerError>;
    async fn renew(
        &self,
        job_id: JobId,
        request: &AgentRenewLeaseRequest,
    ) -> Result<DateTime<Utc>, CloudWorkerError>;
    async fn release(
        &self,
        job_id: JobId,
        request: &AgentReleaseLeaseRequest,
    ) -> Result<(), CloudWorkerError>;
    async fn accept(
        &self,
        job_id: JobId,
        request: &AgentAcceptJobRequest,
    ) -> Result<AcceptanceReconciliation, CloudWorkerError>;
    async fn cleanup_release(
        &self,
        job_id: JobId,
        request: &AgentReleaseLeaseRequest,
    ) -> Result<ReleaseCleanupDisposition, CloudWorkerError> {
        self.release(job_id, request).await?;
        Ok(ReleaseCleanupDisposition::Complete)
    }
    async fn reconcile_acceptance(
        &self,
        _job_id: JobId,
        _request: &AgentAcceptJobRequest,
    ) -> Result<AcceptanceReconciliation, CloudWorkerError> {
        Ok(AcceptanceReconciliation::Pending)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReleaseCleanupDisposition {
    Complete,
    Retry,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AcceptanceReconciliation {
    Accepted,
    AcceptedAfterRevocation,
    AbsentAfterRevocation,
    /// The authority predates exact acceptance reconciliation. An exact
    /// accept replay may recover terminal evidence, but can never prove that
    /// revocation did not commit immediately after its response. The worker
    /// therefore retains the durable intent and fails closed until upgrade.
    Unsupported,
    Pending,
}

/// Production signed HTTPS authority used by every runtime host.
pub struct AgentClientAuthority {
    client: AgentClient,
    signer: Arc<dyn DeviceRequestSigner>,
}

impl AgentClientAuthority {
    #[must_use]
    pub fn new(client: AgentClient, signer: Arc<dyn DeviceRequestSigner>) -> Self {
        Self { client, signer }
    }
}

impl fmt::Debug for AgentClientAuthority {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AgentClientAuthority")
            .field("client", &self.client)
            .field("signer", &"[SECURE SIGNER]")
            .finish()
    }
}

#[async_trait]
impl ConnectorAuthority for AgentClientAuthority {
    async fn sync(
        &self,
        request: &AgentSyncRequest,
    ) -> Result<AgentSyncResponse, CloudWorkerError> {
        self.client
            .sync(self.signer.as_ref(), request)
            .await
            .map_err(Into::into)
    }

    async fn renew(
        &self,
        job_id: JobId,
        request: &AgentRenewLeaseRequest,
    ) -> Result<DateTime<Utc>, CloudWorkerError> {
        self.client
            .renew_lease(self.signer.as_ref(), job_id, request)
            .await
            .map(|response| response.lease_expires_at)
            .map_err(Into::into)
    }

    async fn release(
        &self,
        job_id: JobId,
        request: &AgentReleaseLeaseRequest,
    ) -> Result<(), CloudWorkerError> {
        self.client
            .release_lease(self.signer.as_ref(), job_id, request)
            .await
            .map(|_| ())
            .map_err(Into::into)
    }

    async fn accept(
        &self,
        job_id: JobId,
        request: &AgentAcceptJobRequest,
    ) -> Result<AcceptanceReconciliation, CloudWorkerError> {
        self.client
            .accept_job(self.signer.as_ref(), job_id, request)
            .await
            .map(|response| match response.state {
                JobState::AgentAccepted => AcceptanceReconciliation::Accepted,
                state if state.is_terminal() || state == JobState::CancelRequested => {
                    AcceptanceReconciliation::AcceptedAfterRevocation
                }
                _ => AcceptanceReconciliation::Pending,
            })
            .map_err(Into::into)
    }

    async fn cleanup_release(
        &self,
        job_id: JobId,
        request: &AgentReleaseLeaseRequest,
    ) -> Result<ReleaseCleanupDisposition, CloudWorkerError> {
        match self
            .client
            .release_lease(self.signer.as_ref(), job_id, request)
            .await
        {
            Ok(_) => Ok(ReleaseCleanupDisposition::Complete),
            // Authentication failures are not proof that the authority has
            // released or terminalized this lease. Keep the cleanup durable;
            // a later successful connector revoke may clear it explicitly.
            Err(ClientError::Unauthorized { .. }) => Ok(ReleaseCleanupDisposition::Retry),
            Err(ClientError::Status { status, .. })
                if (400..500).contains(&status)
                    && status != 408
                    && status != 409
                    && status != 429 =>
            {
                Ok(ReleaseCleanupDisposition::Complete)
            }
            Err(ClientError::Http(_) | ClientError::Status { .. }) => {
                Ok(ReleaseCleanupDisposition::Retry)
            }
            Err(error) => Err(error.into()),
        }
    }

    async fn reconcile_acceptance(
        &self,
        job_id: JobId,
        request: &AgentAcceptJobRequest,
    ) -> Result<AcceptanceReconciliation, CloudWorkerError> {
        self.client
            .reconcile_acceptance(self.signer.as_ref(), job_id, request)
            .await
            .map(|response| {
                if response.fenced || response.accepted && response.connector_revoked {
                    AcceptanceReconciliation::AcceptedAfterRevocation
                } else if response.accepted {
                    AcceptanceReconciliation::Accepted
                } else if response.connector_revoked {
                    AcceptanceReconciliation::AbsentAfterRevocation
                } else {
                    AcceptanceReconciliation::Pending
                }
            })
            .or_else(|error| match error {
                ClientError::Status {
                    status: 404 | 405, ..
                } => Ok(AcceptanceReconciliation::Unsupported),
                other => Err(other),
            })
            .map_err(Into::into)
    }
}

/// Builds the connector-scoped snapshot and records exact inventory projection
/// acknowledgement. Inventory implementations may read installation-wide
/// printer facts but must filter them through this connector's durable grant.
#[async_trait]
pub trait InventorySnapshotProvider: fmt::Debug + Send + Sync {
    async fn snapshot(&mut self, refresh: bool) -> Result<AgentSyncRequest, CloudWorkerError>;
    async fn projection_acknowledged(
        &mut self,
        submitted_revision: u64,
        supported: bool,
        acknowledgement: Option<&InventoryProjectionAcknowledgement>,
    ) -> Result<(), CloudWorkerError>;
}

/// Persists server ACK cursors before any command or newly offered work is
/// allowed to mutate the node.
#[async_trait]
pub trait EventAcknowledger: fmt::Debug + Send + Sync {
    async fn acknowledge(
        &mut self,
        event_cursor: Option<EventId>,
        handoff_sequence: Option<u64>,
        diagnostics: &[String],
    ) -> Result<(), CloudWorkerError>;
}

/// Applies authenticated commands after response ACKs are durable.
#[async_trait]
pub trait CloudCommandApplier: fmt::Debug + Send + Sync {
    async fn apply(
        &mut self,
        command_cursor: Option<&str>,
        commands: Vec<AgentCommand>,
    ) -> Result<CloudCommandApplication, CloudWorkerError>;
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CloudCommandApplication {
    /// A durable command remains unacknowledged. The worker still advances
    /// inventory, events and offers on the server cadence; only re-applying
    /// this command is deferred so a poison command cannot create a storm.
    pub retry_after: Option<Duration>,
    /// True only when this cycle attempted an operation and recorded a new
    /// classified failure; callers may emit one bounded diagnostic signal.
    pub attempted_failure: bool,
}

impl CloudCommandApplication {
    #[must_use]
    pub const fn complete() -> Self {
        Self {
            retry_after: None,
            attempted_failure: false,
        }
    }
}

/// Turns a lease-scoped descriptor into bounded local content. Implementations
/// validate digests, profiles and printer grants before returning.
#[async_trait]
pub trait ContentMaterializer: fmt::Debug + Send + Sync {
    type Materialized: Send;
    async fn materialize(
        &mut self,
        offer: &JobOffer,
    ) -> Result<Self::Materialized, CloudWorkerError>;
}

/// The exact remote confirmation derived from one already-durable local intent.
#[derive(Clone, Debug)]
pub struct PendingCloudAcceptance {
    pub job_id: JobId,
    pub request: AgentAcceptJobRequest,
    pub remote_accept_confirmed: bool,
}

/// A legacy or corrupt local intent which cannot prove its physical route.
#[derive(Clone, Debug)]
pub struct PendingCloudRelease {
    pub job_id: JobId,
    pub request: AgentReleaseLeaseRequest,
}

/// Persists and activates connector-isolated queue state. `prepare` must be
/// idempotent for a job and commit the no-replay/handoff intent before return.
#[async_trait]
pub trait DurableOfferAcceptor<M>: fmt::Debug + Send + Sync {
    /// Admission generation fence checked at each remote side-effect boundary.
    async fn admission_valid(&mut self) -> Result<bool, CloudWorkerError> {
        Ok(true)
    }
    async fn pending(&mut self) -> Result<Vec<PendingCloudAcceptance>, CloudWorkerError>;
    async fn invalid_pending(&mut self) -> Result<Vec<PendingCloudRelease>, CloudWorkerError> {
        Ok(Vec::new())
    }
    async fn complete_release_cleanup(&mut self, _job_id: JobId) -> Result<(), CloudWorkerError> {
        Ok(())
    }
    async fn prepare(
        &mut self,
        offer: &JobOffer,
        content: M,
    ) -> Result<PendingCloudAcceptance, CloudWorkerError>;
    async fn activate(&mut self, job_id: JobId) -> Result<(), CloudWorkerError>;
    async fn confirm_remote_accept(&mut self, job_id: JobId) -> Result<(), CloudWorkerError>;
    async fn abandon(&mut self, _job_id: JobId) -> Result<(), CloudWorkerError> {
        Ok(())
    }
    async fn has_durable_intent(&mut self, job_id: JobId) -> Result<bool, CloudWorkerError>;
}

/// Wake hints contain no lease. This capability merely requests that the next
/// authenticated snapshot refresh inventory and reconcile pending work.
#[async_trait]
pub trait WakeReconciler: fmt::Debug + Send + Sync {
    async fn reconcile(&mut self) -> Result<(), CloudWorkerError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CloudCycleOutcome {
    pub next_poll_after: Duration,
    pub inventory_submitted: bool,
    pub offers_seen: usize,
    pub command_retry_after: Option<Duration>,
    pub command_failure_recorded: bool,
}

/// Reusable single-connector worker. A supervisor owns retry/backoff and one
/// instance of this type owns all protocol ordering for a connector.
pub struct CloudConnectorWorker<A, I, E, C, M, D, W>
where
    A: ConnectorAuthority,
    I: InventorySnapshotProvider,
    E: EventAcknowledger,
    C: CloudCommandApplier,
    M: ContentMaterializer,
    D: DurableOfferAcceptor<M::Materialized>,
    W: WakeReconciler,
{
    authority: A,
    inventory: I,
    events: E,
    commands: C,
    materializer: M,
    acceptor: D,
    wake: W,
    runtime: Arc<NodeRuntime>,
    refresh_inventory: bool,
}

impl<A, I, E, C, M, D, W> fmt::Debug for CloudConnectorWorker<A, I, E, C, M, D, W>
where
    A: ConnectorAuthority,
    I: InventorySnapshotProvider,
    E: EventAcknowledger,
    C: CloudCommandApplier,
    M: ContentMaterializer,
    D: DurableOfferAcceptor<M::Materialized>,
    W: WakeReconciler,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CloudConnectorWorker")
            .field("authority", &self.authority)
            .field("runtime", &self.runtime)
            .field("refresh_inventory", &self.refresh_inventory)
            .finish_non_exhaustive()
    }
}

impl<A, I, E, C, M, D, W> CloudConnectorWorker<A, I, E, C, M, D, W>
where
    A: ConnectorAuthority,
    I: InventorySnapshotProvider,
    E: EventAcknowledger,
    C: CloudCommandApplier,
    M: ContentMaterializer,
    D: DurableOfferAcceptor<M::Materialized>,
    W: WakeReconciler,
{
    #[must_use]
    #[allow(
        clippy::too_many_arguments,
        reason = "each injected security boundary remains explicit at composition"
    )]
    pub const fn new(
        authority: A,
        inventory: I,
        events: E,
        commands: C,
        materializer: M,
        acceptor: D,
        wake: W,
        runtime: Arc<NodeRuntime>,
    ) -> Self {
        Self {
            authority,
            inventory,
            events,
            commands,
            materializer,
            acceptor,
            wake,
            runtime,
            refresh_inventory: true,
        }
    }

    /// Runs one authenticated reconcile cycle.
    ///
    /// # Errors
    ///
    /// Returns a redacted failure code when snapshotting, authentication,
    /// acknowledgement, command application, materialisation or acceptance
    /// cannot complete safely.
    pub async fn reconcile_once(&mut self) -> Result<CloudCycleOutcome, CloudWorkerError> {
        // One poisoned or temporarily unreachable acceptance must not starve
        // inventory, event acknowledgement, commands, or sibling work. Keep
        // its exact proof durable, finish the bounded cycle, then surface the
        // first redacted error so the supervisor can back off and report it.
        let mut deferred_error = self.resume_pending_acceptances().await.err();
        let request = self.inventory.snapshot(self.refresh_inventory).await?;
        let inventory_submitted = request.printers.is_some();
        let submitted_revision = request.printer_revision;
        let response = self.authority.sync(&request).await?;
        self.inventory
            .projection_acknowledged(
                submitted_revision,
                response.inventory_projection_acknowledgement_supported,
                response.inventory_projection.as_ref(),
            )
            .await?;

        // Never move this below commands/offers: replaying a response after a
        // crash must be harmless before it can trigger another side effect.
        self.events
            .acknowledge(
                response.acknowledged_event_cursor,
                response.acknowledged_handoff_sequence,
                &response.acknowledged_diagnostics,
            )
            .await?;

        if response
            .wake_hints
            .iter()
            .any(|hint| hint.expires_at > Utc::now())
        {
            self.refresh_inventory = true;
            self.wake.reconcile().await?;
        } else {
            self.refresh_inventory = !inventory_submitted;
        }

        let command_application = self
            .commands
            .apply(response.command_cursor.as_deref(), response.commands)
            .await?;
        let offers_seen = response.candidate_jobs.len();
        for offer in response.candidate_jobs {
            if let Err(error) = self.process_offer(offer).await
                && deferred_error.is_none()
            {
                deferred_error = Some(error);
            }
        }
        let server_delay = Duration::from_millis(response.next_poll_after_ms.clamp(250, 60_000));
        let outcome = CloudCycleOutcome {
            next_poll_after: server_delay,
            inventory_submitted,
            offers_seen,
            command_retry_after: command_application.retry_after,
            command_failure_recorded: command_application.attempted_failure,
        };
        deferred_error.map_or(Ok(outcome), Err)
    }

    async fn resume_pending_acceptances(&mut self) -> Result<(), CloudWorkerError> {
        for invalid in self.acceptor.invalid_pending().await? {
            if matches!(
                self.authority
                    .cleanup_release(invalid.job_id, &invalid.request)
                    .await,
                Ok(ReleaseCleanupDisposition::Complete)
            ) {
                self.acceptor
                    .complete_release_cleanup(invalid.job_id)
                    .await?;
            }
        }
        let mut first_error = None;
        for pending in self.acceptor.pending().await? {
            if let Err(error) = self.resume_pending_acceptance(pending).await
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    async fn resume_pending_acceptance(
        &mut self,
        pending: PendingCloudAcceptance,
    ) -> Result<(), CloudWorkerError> {
        let reconciliation = self
            .authority
            .reconcile_acceptance(pending.job_id, &pending.request)
            .await?;
        match reconciliation {
            AcceptanceReconciliation::Accepted => {
                self.acceptor.confirm_remote_accept(pending.job_id).await?;
                return self.acceptor.activate(pending.job_id).await;
            }
            AcceptanceReconciliation::AcceptedAfterRevocation
            | AcceptanceReconciliation::AbsentAfterRevocation => {
                return self.acceptor.abandon(pending.job_id).await;
            }
            AcceptanceReconciliation::Unsupported => {
                if pending.remote_accept_confirmed || !self.acceptor.admission_valid().await? {
                    return Err(CloudWorkerError::new(
                        "connector_authority_upgrade_required",
                    ));
                }
                // Exact replay against an older authority can recover a
                // terminal result, but AgentAccepted is not a current fence:
                // an external revoke may commit immediately after the 200.
                let replay = self
                    .authority
                    .accept(pending.job_id, &pending.request)
                    .await?;
                match replay {
                    AcceptanceReconciliation::Accepted => {}
                    AcceptanceReconciliation::AcceptedAfterRevocation
                    | AcceptanceReconciliation::AbsentAfterRevocation => {
                        return self.acceptor.abandon(pending.job_id).await;
                    }
                    AcceptanceReconciliation::Unsupported | AcceptanceReconciliation::Pending => {
                        return Ok(());
                    }
                }
                self.acceptor.confirm_remote_accept(pending.job_id).await?;
                return Err(CloudWorkerError::new(
                    "connector_authority_upgrade_required",
                ));
            }
            AcceptanceReconciliation::Pending => {}
        }
        if pending.remote_accept_confirmed || !self.acceptor.admission_valid().await? {
            return Ok(());
        }
        let accepted = self
            .authority
            .accept(pending.job_id, &pending.request)
            .await?;
        match accepted {
            AcceptanceReconciliation::Accepted => {}
            AcceptanceReconciliation::AcceptedAfterRevocation
            | AcceptanceReconciliation::AbsentAfterRevocation => {
                return self.acceptor.abandon(pending.job_id).await;
            }
            AcceptanceReconciliation::Unsupported | AcceptanceReconciliation::Pending => {
                return Ok(());
            }
        }
        self.acceptor.confirm_remote_accept(pending.job_id).await?;
        // Close accept-versus-revoke after the authority commit and before the
        // first runnable local transition. Revoke serializes against this
        // exact acceptance and turns the outcome into a fenced local abandon.
        match self
            .authority
            .reconcile_acceptance(pending.job_id, &pending.request)
            .await?
        {
            AcceptanceReconciliation::Accepted => self.acceptor.activate(pending.job_id).await,
            AcceptanceReconciliation::AcceptedAfterRevocation
            | AcceptanceReconciliation::AbsentAfterRevocation => {
                self.acceptor.abandon(pending.job_id).await
            }
            AcceptanceReconciliation::Unsupported => Err(CloudWorkerError::new(
                "connector_authority_upgrade_required",
            )),
            AcceptanceReconciliation::Pending => Ok(()),
        }
    }

    async fn process_offer(&mut self, offer: JobOffer) -> Result<(), CloudWorkerError> {
        if offer.route_reservation.is_none() {
            self.release_offer(&offer, "route_reservation_required")
                .await?;
            return Ok(());
        }
        if !self.runtime.snapshot().accepting_cloud_leases
            || !self.acceptor.admission_valid().await?
        {
            self.release_offer(&offer, "host_lifecycle_unavailable")
                .await?;
            return Ok(());
        }
        let job_id = offer.job.id;
        let lease_token = offer.lease_token.clone();
        let lease_id = offer.lease_id;
        let route = offer.route_reservation.clone();
        let work = async {
            let content = self.materializer.materialize(&offer).await?;
            self.acceptor.prepare(&offer, content).await
        };
        let authority = &self.authority;
        let result = maintain_lease(offer.lease_expires_at, work, || {
            let lease_token = lease_token.clone();
            let route = route.clone();
            async move {
                authority
                    .renew(
                        job_id,
                        &AgentRenewLeaseRequest {
                            lease_id,
                            lease_token,
                            route_reservation_id: route.as_ref().map(|value| value.reservation_id),
                            route_generation: route.as_ref().map(|value| value.generation),
                            route_fencing_token: route.map(|value| value.fencing_token),
                        },
                    )
                    .await
            }
        })
        .await;
        let _pending = match result {
            Ok(pending) => pending,
            Err(error) => {
                if self.acceptor.has_durable_intent(job_id).await == Ok(false) {
                    self.release_offer(&offer, error.code).await?;
                }
                return Err(error);
            }
        };
        if !self.acceptor.admission_valid().await? {
            self.acceptor.abandon(job_id).await?;
            let _ = self
                .release_offer(&offer, "connector_admission_revoked")
                .await;
            return Err(CloudWorkerError::new("connector_admission_revoked"));
        }
        self.resume_pending_acceptances().await
    }

    async fn release_offer(
        &self,
        offer: &JobOffer,
        reason: &'static str,
    ) -> Result<(), CloudWorkerError> {
        self.authority
            .release(
                offer.job.id,
                &AgentReleaseLeaseRequest {
                    lease_id: offer.lease_id,
                    lease_token: offer.lease_token.clone(),
                    reason: reason.into(),
                },
            )
            .await
    }
}

async fn maintain_lease<T, F, R, RF>(
    initial_expiry: DateTime<Utc>,
    work: F,
    mut renew: R,
) -> Result<T, CloudWorkerError>
where
    F: Future<Output = Result<T, CloudWorkerError>>,
    R: FnMut() -> RF,
    RF: Future<Output = Result<DateTime<Utc>, CloudWorkerError>>,
{
    let mut expiry = initial_expiry;
    tokio::pin!(work);
    loop {
        let delay = lease_renewal_delay(expiry);
        tokio::select! {
            biased;
            () = tokio::time::sleep(delay) => {
                expiry = tokio::time::timeout(LEASE_REQUEST_TIMEOUT, renew())
                    .await
                    .map_err(|_| CloudWorkerError::new("lease_renewal_timeout"))??;
            }
            value = &mut work => return value,
        }
    }
}

fn lease_renewal_delay(expiry: DateTime<Utc>) -> Duration {
    let remaining = (expiry - Utc::now()).to_std().unwrap_or(Duration::ZERO);
    remaining
        .saturating_sub(Duration::from_secs(5))
        .min(LEASE_RENEWAL_INTERVAL)
        .max(Duration::from_millis(250))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::too_many_lines)]
mod tests {
    use super::*;
    use crate::{
        AvailabilityClass, GeneratedConnectorKey, HostBackedDeviceIdentity, HostCapabilities,
        HostKind, LifecycleEvent, NodeRuntimeMode, PrinterTransport, RuntimeConfiguration,
        SecureConnectorSigner, SecureKeyHandle,
    };
    use async_trait::async_trait;
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use chrono::TimeDelta;
    use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier as _};
    use piqae_agent_client::DeviceIdentity;
    use piqae_domain::{
        AgentId, ContentKind, ContentSource, EnvironmentId, Job, JobId, JobOptions, JobState,
        PrinterId, WorkspaceId,
    };
    use piqae_node_host_api::ConnectorKeyError;
    use piqae_protocol::agent::{
        AgentHealth, AgentProtocolCapabilities, ContentDescriptor, DocumentRenderCapabilities,
        QueueSnapshot,
    };
    use std::{
        collections::{BTreeMap, BTreeSet, VecDeque},
        path::{Path, PathBuf},
        sync::{
            Mutex,
            atomic::{AtomicBool, Ordering},
        },
    };
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    type Log = Arc<Mutex<Vec<&'static str>>>;

    #[derive(Debug)]
    struct FakeSecureStore {
        key: SigningKey,
        deleted: Mutex<bool>,
    }

    impl SecureConnectorSigner for FakeSecureStore {
        fn generate(
            &self,
            application_scope: &str,
        ) -> Result<GeneratedConnectorKey, ConnectorKeyError> {
            if application_scope != "com.example.pos" || *self.deleted.lock().unwrap() {
                return Err(ConnectorKeyError::Rejected);
            }
            Ok(GeneratedConnectorKey {
                handle: SecureKeyHandle::new("keychain/connectors/c4beta".into())?,
                public_key: self.key.verifying_key().to_bytes(),
            })
        }

        fn sign(
            &self,
            handle: &SecureKeyHandle,
            message: &[u8],
        ) -> Result<[u8; 64], ConnectorKeyError> {
            if handle.as_str() != "keychain/connectors/c4beta" || *self.deleted.lock().unwrap() {
                return Err(ConnectorKeyError::Unavailable);
            }
            Ok(self.key.sign(message).to_bytes())
        }

        fn delete(&self, handle: &SecureKeyHandle) -> Result<(), ConnectorKeyError> {
            if handle.as_str() != "keychain/connectors/c4beta" {
                return Err(ConnectorKeyError::InvalidKeyMaterial);
            }
            *self.deleted.lock().unwrap() = true;
            Ok(())
        }
    }

    struct FakeAuthority {
        signer: Arc<dyn DeviceRequestSigner>,
        verifying_key: ed25519_dalek::VerifyingKey,
        responses: Mutex<VecDeque<Result<AgentSyncResponse, CloudWorkerError>>>,
        accepted: Arc<Mutex<BTreeSet<JobId>>>,
        released: Mutex<Vec<JobId>>,
        cleanup_retries: Mutex<usize>,
        accept_failures_after_commit: Mutex<usize>,
        accept_outcome: Option<AcceptanceReconciliation>,
        reconciliation_supported: bool,
        reconciliation_errors: Mutex<BTreeSet<JobId>>,
        revoke_after_accept: Option<Arc<AtomicBool>>,
        log: Log,
    }

    impl fmt::Debug for FakeAuthority {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("FakeAuthority([REDACTED])")
        }
    }

    #[async_trait]
    impl ConnectorAuthority for FakeAuthority {
        async fn sync(
            &self,
            request: &AgentSyncRequest,
        ) -> Result<AgentSyncResponse, CloudWorkerError> {
            let body = serde_json::to_vec(request).unwrap();
            let signature =
                Signature::from_bytes(&self.signer.sign(&body).map_err(CloudWorkerError::from)?);
            self.verifying_key
                .verify(&body, &signature)
                .map_err(|_| CloudWorkerError::new("invalid_signature"))?;
            self.log.lock().unwrap().push("signed_sync");
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| Ok(empty_response()))
        }

        async fn renew(
            &self,
            _job_id: JobId,
            _request: &AgentRenewLeaseRequest,
        ) -> Result<DateTime<Utc>, CloudWorkerError> {
            self.log.lock().unwrap().push("renew");
            Ok(Utc::now() + TimeDelta::minutes(1))
        }

        async fn release(
            &self,
            job_id: JobId,
            _request: &AgentReleaseLeaseRequest,
        ) -> Result<(), CloudWorkerError> {
            self.log.lock().unwrap().push("remote_release");
            self.released.lock().unwrap().push(job_id);
            Ok(())
        }

        async fn cleanup_release(
            &self,
            job_id: JobId,
            request: &AgentReleaseLeaseRequest,
        ) -> Result<ReleaseCleanupDisposition, CloudWorkerError> {
            let retry = {
                let mut retries = self.cleanup_retries.lock().unwrap();
                let retry = *retries > 0;
                if retry {
                    *retries -= 1;
                }
                retry
            };
            if retry {
                self.log.lock().unwrap().push("cleanup_retry");
                return Ok(ReleaseCleanupDisposition::Retry);
            }
            self.release(job_id, request).await?;
            Ok(ReleaseCleanupDisposition::Complete)
        }

        async fn accept(
            &self,
            job_id: JobId,
            _request: &AgentAcceptJobRequest,
        ) -> Result<AcceptanceReconciliation, CloudWorkerError> {
            self.log.lock().unwrap().push("remote_accept");
            self.accepted.lock().unwrap().insert(job_id);
            if let Some(admission) = &self.revoke_after_accept {
                admission.store(false, Ordering::Release);
            }
            let fail_after_commit = {
                let mut failures = self.accept_failures_after_commit.lock().unwrap();
                let should_fail = *failures > 0;
                if should_fail {
                    *failures -= 1;
                }
                should_fail
            };
            if fail_after_commit {
                return Err(CloudWorkerError::new("accept_response_lost"));
            }
            Ok(self.accept_outcome.unwrap_or_else(|| {
                if self
                    .revoke_after_accept
                    .as_ref()
                    .is_some_and(|admission| !admission.load(Ordering::Acquire))
                {
                    AcceptanceReconciliation::AcceptedAfterRevocation
                } else {
                    AcceptanceReconciliation::Accepted
                }
            }))
        }

        async fn reconcile_acceptance(
            &self,
            job_id: JobId,
            _request: &AgentAcceptJobRequest,
        ) -> Result<AcceptanceReconciliation, CloudWorkerError> {
            self.log.lock().unwrap().push("reconcile_acceptance");
            if self.reconciliation_errors.lock().unwrap().contains(&job_id) {
                return Err(CloudWorkerError::new("acceptance_reconcile_unavailable"));
            }
            if !self.reconciliation_supported {
                return Ok(AcceptanceReconciliation::Unsupported);
            }
            let accepted = self.accepted.lock().unwrap().contains(&job_id);
            let revoked = self
                .revoke_after_accept
                .as_ref()
                .is_some_and(|admission| !admission.load(Ordering::Acquire));
            Ok(match (accepted, revoked) {
                (true, true) => AcceptanceReconciliation::AcceptedAfterRevocation,
                (true, false) => AcceptanceReconciliation::Accepted,
                (false, true) => AcceptanceReconciliation::AbsentAfterRevocation,
                (false, false) => AcceptanceReconciliation::Pending,
            })
        }
    }

    #[derive(Debug)]
    struct FakeInventory {
        agent_id: AgentId,
        log: Log,
    }

    #[async_trait]
    impl InventorySnapshotProvider for FakeInventory {
        async fn snapshot(&mut self, refresh: bool) -> Result<AgentSyncRequest, CloudWorkerError> {
            self.log.lock().unwrap().push("inventory");
            Ok(request(self.agent_id, refresh))
        }

        async fn projection_acknowledged(
            &mut self,
            _submitted_revision: u64,
            _supported: bool,
            _acknowledgement: Option<&InventoryProjectionAcknowledgement>,
        ) -> Result<(), CloudWorkerError> {
            self.log.lock().unwrap().push("inventory_ack");
            Ok(())
        }
    }

    #[derive(Debug)]
    struct FakeEvents(Log);

    #[async_trait]
    impl EventAcknowledger for FakeEvents {
        async fn acknowledge(
            &mut self,
            _event_cursor: Option<EventId>,
            _handoff_sequence: Option<u64>,
            _diagnostics: &[String],
        ) -> Result<(), CloudWorkerError> {
            self.0.lock().unwrap().push("event_ack");
            Ok(())
        }
    }

    #[derive(Debug)]
    struct FakeCommands(Log);

    #[async_trait]
    impl CloudCommandApplier for FakeCommands {
        async fn apply(
            &mut self,
            _cursor: Option<&str>,
            _commands: Vec<AgentCommand>,
        ) -> Result<CloudCommandApplication, CloudWorkerError> {
            self.0.lock().unwrap().push("commands");
            Ok(CloudCommandApplication::complete())
        }
    }

    #[derive(Debug)]
    struct DeferredCommands(Log);

    #[async_trait]
    impl CloudCommandApplier for DeferredCommands {
        async fn apply(
            &mut self,
            _cursor: Option<&str>,
            _commands: Vec<AgentCommand>,
        ) -> Result<CloudCommandApplication, CloudWorkerError> {
            self.0.lock().unwrap().push("commands_deferred");
            Ok(CloudCommandApplication {
                retry_after: Some(Duration::from_secs(30)),
                attempted_failure: true,
            })
        }
    }

    #[derive(Debug)]
    struct FakeMaterializer {
        log: Log,
        fail: bool,
    }

    #[async_trait]
    impl ContentMaterializer for FakeMaterializer {
        type Materialized = Vec<u8>;

        async fn materialize(
            &mut self,
            offer: &JobOffer,
        ) -> Result<Self::Materialized, CloudWorkerError> {
            self.log.lock().unwrap().push("materialize");
            if self.fail {
                return Err(CloudWorkerError::new("content_invalid"));
            }
            match &offer.content {
                ContentDescriptor::InlineBase64 { data, .. } => STANDARD
                    .decode(data)
                    .map_err(|_| CloudWorkerError::new("content_invalid")),
                _ => Err(CloudWorkerError::new("content_invalid")),
            }
        }
    }

    #[derive(Debug, serde::Serialize, serde::Deserialize, Default)]
    struct DurableDocument {
        pending: BTreeMap<JobId, AgentAcceptJobRequest>,
        confirmed: BTreeSet<JobId>,
        release_cleanups: BTreeMap<JobId, AgentReleaseLeaseRequest>,
        active: BTreeSet<JobId>,
        handoffs: BTreeSet<JobId>,
    }

    #[derive(Debug)]
    struct FileAcceptor {
        path: PathBuf,
        document: DurableDocument,
        log: Log,
        admission: Option<Arc<AtomicBool>>,
    }

    impl FileAcceptor {
        fn open(path: &Path, log: Log) -> Self {
            let document = std::fs::read(path)
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .unwrap_or_default();
            Self {
                path: path.to_owned(),
                document,
                log,
                admission: None,
            }
        }

        fn persist(&self) -> Result<(), CloudWorkerError> {
            let bytes = serde_json::to_vec(&self.document)
                .map_err(|_| CloudWorkerError::new("durable_encode_failed"))?;
            std::fs::write(&self.path, bytes)
                .map_err(|_| CloudWorkerError::new("durable_write_failed"))
        }
    }

    #[async_trait]
    impl DurableOfferAcceptor<Vec<u8>> for FileAcceptor {
        async fn admission_valid(&mut self) -> Result<bool, CloudWorkerError> {
            Ok(self
                .admission
                .as_ref()
                .is_none_or(|admission| admission.load(Ordering::Acquire)))
        }

        async fn pending(&mut self) -> Result<Vec<PendingCloudAcceptance>, CloudWorkerError> {
            Ok(self
                .document
                .pending
                .iter()
                .filter(|(job_id, _)| !self.document.active.contains(job_id))
                .map(|(job_id, request)| PendingCloudAcceptance {
                    job_id: *job_id,
                    request: request.clone(),
                    remote_accept_confirmed: self.document.confirmed.contains(job_id),
                })
                .collect())
        }

        async fn invalid_pending(&mut self) -> Result<Vec<PendingCloudRelease>, CloudWorkerError> {
            let invalid = self
                .document
                .pending
                .iter()
                .filter(|(_, request)| {
                    request.route_reservation_id.is_none()
                        || request.route_generation.is_none()
                        || request.route_fencing_token.is_none()
                })
                .map(|(job_id, request)| (*job_id, request.clone()))
                .collect::<Vec<_>>();
            let mut releases = Vec::with_capacity(invalid.len());
            for (job_id, request) in invalid {
                let cleanup = AgentReleaseLeaseRequest {
                    lease_id: request.lease_id,
                    lease_token: request.lease_token,
                    reason: "route_reservation_required".into(),
                };
                self.document.pending.remove(&job_id);
                self.document.release_cleanups.insert(job_id, cleanup);
                self.log.lock().unwrap().push("quarantine");
            }
            self.persist()?;
            for (job_id, request) in &self.document.release_cleanups {
                releases.push(PendingCloudRelease {
                    job_id: *job_id,
                    request: request.clone(),
                });
            }
            Ok(releases)
        }

        async fn complete_release_cleanup(
            &mut self,
            job_id: JobId,
        ) -> Result<(), CloudWorkerError> {
            self.document.release_cleanups.remove(&job_id);
            self.persist()
        }

        async fn prepare(
            &mut self,
            offer: &JobOffer,
            content: Vec<u8>,
        ) -> Result<PendingCloudAcceptance, CloudWorkerError> {
            if content != b"fixture" {
                return Err(CloudWorkerError::new("content_invalid"));
            }
            let request = self
                .document
                .pending
                .entry(offer.job.id)
                .or_insert_with(|| {
                    let route = offer.route_reservation.as_ref();
                    AgentAcceptJobRequest {
                        lease_id: offer.lease_id,
                        lease_token: offer.lease_token.clone(),
                        content_sha256:
                            "d308e0b2d4b253d56eeca365fa4f032c65bf3fb7696b4799840f886abc3f6c7c"
                                .into(),
                        local_sequence: 1,
                        route_reservation_id: route.map(|value| value.reservation_id),
                        route_generation: route.map(|value| value.generation),
                        route_fencing_token: route.map(|value| value.fencing_token.clone()),
                    }
                })
                .clone();
            if self.document.handoffs.insert(offer.job.id) {
                self.log.lock().unwrap().push("durable_handoff");
            }
            self.persist()?;
            Ok(PendingCloudAcceptance {
                job_id: offer.job.id,
                request,
                remote_accept_confirmed: false,
            })
        }

        async fn activate(&mut self, job_id: JobId) -> Result<(), CloudWorkerError> {
            self.log.lock().unwrap().push("activate");
            self.document.active.insert(job_id);
            self.persist()
        }

        async fn confirm_remote_accept(&mut self, job_id: JobId) -> Result<(), CloudWorkerError> {
            self.document.confirmed.insert(job_id);
            self.persist()
        }

        async fn abandon(&mut self, job_id: JobId) -> Result<(), CloudWorkerError> {
            self.log.lock().unwrap().push("abandon");
            self.document.pending.remove(&job_id);
            self.persist()
        }

        async fn has_durable_intent(&mut self, job_id: JobId) -> Result<bool, CloudWorkerError> {
            Ok(self.document.pending.contains_key(&job_id))
        }
    }

    #[derive(Debug)]
    struct FakeWake(Log);

    #[async_trait]
    impl WakeReconciler for FakeWake {
        async fn reconcile(&mut self) -> Result<(), CloudWorkerError> {
            self.0.lock().unwrap().push("wake_reconcile");
            Ok(())
        }
    }

    fn request(agent_id: AgentId, printers: bool) -> AgentSyncRequest {
        AgentSyncRequest {
            agent_id,
            protocol_version: 1,
            agent_version: "test".into(),
            printer_revision: 1,
            acknowledged_command_cursor: None,
            event_cursor: None,
            queue: QueueSnapshot {
                queued_jobs: 0,
                active_jobs: 0,
                content_bytes: 0,
                accepts_jobs: true,
            },
            health: AgentHealth {
                started_at: Utc::now(),
                observed_at: Utc::now(),
                sqlite_integrity_ok: true,
                executor_crashes: 0,
                last_error_code: None,
            },
            printers: printers.then(Vec::new),
            events: Vec::new(),
            diagnostics: Vec::new(),
            document_render: DocumentRenderCapabilities::default(),
            capabilities: AgentProtocolCapabilities::default(),
            route_observations: Vec::new(),
            topology_changes: Vec::new(),
            native_handoffs: Vec::new(),
            runtime: None,
        }
    }

    fn offer(job_id: JobId) -> JobOffer {
        JobOffer {
            job: Job {
                id: job_id,
                workspace_id: WorkspaceId::new(),
                environment_id: EnvironmentId::new(),
                printer_id: PrinterId::new(),
                title: "redacted fixture".into(),
                source: None,
                content_kind: ContentKind::Pdf,
                content: ContentSource::Base64 {
                    data: STANDARD.encode(b"fixture"),
                },
                options: JobOptions::default(),
                metadata: BTreeMap::new(),
                deliveries: 1,
                state: JobState::WaitingForAgent,
                created_at: Utc::now(),
                expires_at: Utc::now() + TimeDelta::minutes(5),
                delivery_uncertain_since: None,
            },
            expected_capability_revision: None,
            resolved_ticket_digest: None,
            lease_id: uuid::Uuid::new_v4(),
            lease_token: "never-log-lease-token".into(),
            lease_expires_at: Utc::now() + TimeDelta::minutes(1),
            content: ContentDescriptor::InlineBase64 {
                data: STANDARD.encode(b"fixture"),
                sha256: None,
                bytes: Some(7),
            },
            route_reservation: Some(piqae_protocol::agent::CloudRouteReservation {
                route_id: "route_fixture".into(),
                local_route_key: "fake:fixture".into(),
                reservation_id: uuid::Uuid::new_v4(),
                generation: 1,
                fencing_token: "deterministic-route-fence".into(),
                lease_expires_at: Utc::now() + TimeDelta::minutes(1),
            }),
        }
    }

    fn empty_response() -> AgentSyncResponse {
        AgentSyncResponse {
            server_time: Utc::now(),
            acknowledged_event_cursor: None,
            command_cursor: None,
            commands: Vec::new(),
            candidate_jobs: Vec::new(),
            next_poll_after_ms: 1_000,
            acknowledged_diagnostics: Vec::new(),
            inventory_projection_acknowledgement_supported: true,
            inventory_projection: Some(InventoryProjectionAcknowledgement {
                revision: 1,
                projected_at: Utc::now(),
            }),
            acknowledged_handoff_sequence: None,
            wake_hints: Vec::new(),
        }
    }

    fn runtime(root: &Path) -> Arc<NodeRuntime> {
        let runtime = Arc::new(
            NodeRuntime::start(RuntimeConfiguration {
                data_directory: root.to_owned(),
                mode: NodeRuntimeMode::CloudCapable,
                host: HostCapabilities {
                    host_kind: HostKind::EmbeddedApplication,
                    availability: AvailabilityClass::ContinuousWhileAwake,
                    secure_storage: true,
                    local_ipc_broker: false,
                    can_prevent_idle_sleep_during_handoff: false,
                    can_receive_remote_wake_hint: true,
                    printer_transports: std::iter::once(PrinterTransport::Fake).collect(),
                },
            })
            .unwrap(),
        );
        let _ = runtime.apply_lifecycle(LifecycleEvent::Started);
        runtime
    }

    fn worker(
        authority: FakeAuthority,
        log: &Log,
        state: &Path,
        runtime: Arc<NodeRuntime>,
        fail_materialization: bool,
    ) -> CloudConnectorWorker<
        FakeAuthority,
        FakeInventory,
        FakeEvents,
        FakeCommands,
        FakeMaterializer,
        FileAcceptor,
        FakeWake,
    > {
        let agent_id = *authority.signer.agent_id();
        CloudConnectorWorker::new(
            authority,
            FakeInventory {
                agent_id,
                log: Arc::clone(log),
            },
            FakeEvents(Arc::clone(log)),
            FakeCommands(Arc::clone(log)),
            FakeMaterializer {
                log: Arc::clone(log),
                fail: fail_materialization,
            },
            FileAcceptor::open(state, Arc::clone(log)),
            FakeWake(Arc::clone(log)),
            runtime,
        )
    }

    fn authority(
        signer: Arc<dyn DeviceRequestSigner>,
        verifying_key: ed25519_dalek::VerifyingKey,
        responses: Vec<Result<AgentSyncResponse, CloudWorkerError>>,
        log: Log,
    ) -> FakeAuthority {
        FakeAuthority {
            signer,
            verifying_key,
            responses: Mutex::new(responses.into()),
            accepted: Arc::new(Mutex::new(BTreeSet::new())),
            released: Mutex::new(Vec::new()),
            cleanup_retries: Mutex::new(0),
            accept_failures_after_commit: Mutex::new(0),
            accept_outcome: None,
            reconciliation_supported: true,
            reconciliation_errors: Mutex::new(BTreeSet::new()),
            revoke_after_accept: None,
            log,
        }
    }

    #[tokio::test]
    async fn invitation_secure_signing_sync_handoff_ack_and_restart_are_ordered_once() {
        let directory = tempfile::tempdir().unwrap();
        let log = Arc::new(Mutex::new(Vec::new()));
        let provider = Arc::new(FakeSecureStore {
            key: SigningKey::from_bytes(&[11; 32]),
            deleted: Mutex::new(false),
        });
        // This is the invitation exchange boundary: only the public key and
        // opaque handle leave the host provider.
        let generated = provider.generate("com.example.pos").unwrap();
        let agent_id = AgentId::new();
        let identity: Arc<dyn DeviceRequestSigner> = Arc::new(HostBackedDeviceIdentity::new(
            agent_id,
            generated.handle,
            provider,
        ));
        let job_id = JobId::new();
        let mut response = empty_response();
        response.candidate_jobs.push(offer(job_id));
        response.command_cursor = Some("cmd_1".into());
        let runtime_root = directory.path().join("runtime");
        let runtime = runtime(&runtime_root);
        let state = directory.path().join("queue.json");
        let mut first = worker(
            authority(
                Arc::clone(&identity),
                ed25519_dalek::VerifyingKey::from_bytes(&generated.public_key).unwrap(),
                vec![Ok(response)],
                Arc::clone(&log),
            ),
            &log,
            &state,
            Arc::clone(&runtime),
            false,
        );
        first.reconcile_once().await.unwrap();
        drop(first);

        // A replay after process restart reaches the same durable acceptor and
        // cannot create a second local handoff.
        let mut replay = empty_response();
        replay.candidate_jobs.push(offer(job_id));
        let mut second = worker(
            authority(
                identity,
                ed25519_dalek::VerifyingKey::from_bytes(&generated.public_key).unwrap(),
                vec![Ok(replay)],
                Arc::clone(&log),
            ),
            &log,
            &state,
            runtime,
            false,
        );
        second.reconcile_once().await.unwrap();
        let entries = log.lock().unwrap().clone();
        assert_eq!(
            entries
                .iter()
                .filter(|entry| **entry == "durable_handoff")
                .count(),
            1
        );
        let position = |name| entries.iter().position(|entry| *entry == name).unwrap();
        assert!(position("event_ack") < position("commands"));
        assert!(position("commands") < position("materialize"));
        assert!(position("durable_handoff") < position("remote_accept"));
        assert!(position("remote_accept") < position("activate"));
    }

    #[tokio::test]
    async fn suspended_hosts_release_offers_and_materialization_failures_release_without_intent() {
        let directory = tempfile::tempdir().unwrap();
        let provider = Arc::new(FakeSecureStore {
            key: SigningKey::from_bytes(&[13; 32]),
            deleted: Mutex::new(false),
        });
        let generated = provider.generate("com.example.pos").unwrap();
        let identity: Arc<dyn DeviceRequestSigner> = Arc::new(HostBackedDeviceIdentity::new(
            AgentId::new(),
            generated.handle,
            provider,
        ));
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut response = empty_response();
        response.candidate_jobs.push(offer(JobId::new()));
        let suspended_runtime = runtime(&directory.path().join("runtime"));
        let _ = suspended_runtime.apply_lifecycle(LifecycleEvent::SuspendImminent);
        let mut suspended = worker(
            authority(
                Arc::clone(&identity),
                ed25519_dalek::VerifyingKey::from_bytes(&generated.public_key).unwrap(),
                vec![Ok(response)],
                Arc::clone(&log),
            ),
            &log,
            &directory.path().join("suspended.json"),
            suspended_runtime,
            false,
        );
        suspended.reconcile_once().await.unwrap();
        assert!(log.lock().unwrap().contains(&"remote_release"));
        assert!(!log.lock().unwrap().contains(&"materialize"));

        let log = Arc::new(Mutex::new(Vec::new()));
        let mut response = empty_response();
        response.candidate_jobs.push(offer(JobId::new()));
        let mut failed = worker(
            authority(
                identity,
                ed25519_dalek::VerifyingKey::from_bytes(&generated.public_key).unwrap(),
                vec![Ok(response)],
                Arc::clone(&log),
            ),
            &log,
            &directory.path().join("failed.json"),
            runtime(&directory.path().join("runtime-2")),
            true,
        );
        assert_eq!(
            failed.reconcile_once().await.unwrap_err().code,
            "content_invalid"
        );
        assert!(log.lock().unwrap().contains(&"remote_release"));
    }

    #[tokio::test]
    async fn deferred_command_does_not_starve_inventory_or_sibling_offer() {
        let directory = tempfile::tempdir().unwrap();
        let log = Arc::new(Mutex::new(Vec::new()));
        let provider = Arc::new(FakeSecureStore {
            key: SigningKey::from_bytes(&[29; 32]),
            deleted: Mutex::new(false),
        });
        let generated = provider.generate("com.example.pos").unwrap();
        let agent_id = AgentId::new();
        let identity: Arc<dyn DeviceRequestSigner> = Arc::new(HostBackedDeviceIdentity::new(
            agent_id,
            generated.handle,
            provider,
        ));
        let mut response = empty_response();
        response.commands.push(AgentCommand::Pause);
        response.command_cursor = Some("cmd_deferred".into());
        response.candidate_jobs.push(offer(JobId::new()));
        let runtime = runtime(&directory.path().join("runtime"));
        let mut worker = CloudConnectorWorker::new(
            authority(
                identity,
                ed25519_dalek::VerifyingKey::from_bytes(&generated.public_key).unwrap(),
                vec![Ok(response)],
                Arc::clone(&log),
            ),
            FakeInventory {
                agent_id,
                log: Arc::clone(&log),
            },
            FakeEvents(Arc::clone(&log)),
            DeferredCommands(Arc::clone(&log)),
            FakeMaterializer {
                log: Arc::clone(&log),
                fail: false,
            },
            FileAcceptor::open(&directory.path().join("queue.json"), Arc::clone(&log)),
            FakeWake(Arc::clone(&log)),
            runtime,
        );
        let outcome = worker.reconcile_once().await.unwrap();
        assert_eq!(outcome.command_retry_after, Some(Duration::from_secs(30)));
        assert_eq!(outcome.next_poll_after, Duration::from_secs(1));
        assert!(outcome.command_failure_recorded);
        assert!(outcome.inventory_submitted);
        assert_eq!(outcome.offers_seen, 1);
        let entries = log.lock().unwrap();
        assert!(entries.contains(&"inventory_ack"));
        assert!(entries.contains(&"commands_deferred"));
        assert!(entries.contains(&"durable_handoff"));
        assert!(entries.contains(&"activate"));
        drop(entries);
    }

    #[tokio::test]
    async fn offers_without_route_proof_are_released_before_materialization() {
        let directory = tempfile::tempdir().unwrap();
        let log = Arc::new(Mutex::new(Vec::new()));
        let provider = Arc::new(FakeSecureStore {
            key: SigningKey::from_bytes(&[14; 32]),
            deleted: Mutex::new(false),
        });
        let generated = provider.generate("com.example.pos").unwrap();
        let identity: Arc<dyn DeviceRequestSigner> = Arc::new(HostBackedDeviceIdentity::new(
            AgentId::new(),
            generated.handle,
            provider,
        ));
        let mut missing = offer(JobId::new());
        missing.route_reservation = None;
        let mut response = empty_response();
        response.candidate_jobs.push(missing);
        let mut worker = worker(
            authority(
                identity,
                ed25519_dalek::VerifyingKey::from_bytes(&generated.public_key).unwrap(),
                vec![Ok(response)],
                Arc::clone(&log),
            ),
            &log,
            &directory.path().join("missing-route.json"),
            runtime(&directory.path().join("runtime-missing-route")),
            false,
        );
        worker.reconcile_once().await.unwrap();
        let entries = log.lock().unwrap();
        assert!(entries.contains(&"remote_release"));
        assert!(!entries.contains(&"materialize"));
        drop(entries);
    }

    #[tokio::test]
    async fn legacy_pending_accept_without_route_proof_is_abandoned_and_released() {
        let directory = tempfile::tempdir().unwrap();
        let log = Arc::new(Mutex::new(Vec::new()));
        let provider = Arc::new(FakeSecureStore {
            key: SigningKey::from_bytes(&[15; 32]),
            deleted: Mutex::new(false),
        });
        let generated = provider.generate("com.example.pos").unwrap();
        let identity: Arc<dyn DeviceRequestSigner> = Arc::new(HostBackedDeviceIdentity::new(
            AgentId::new(),
            generated.handle,
            provider,
        ));
        let state = directory.path().join("legacy-pending.json");
        let mut worker = worker(
            authority(
                identity,
                ed25519_dalek::VerifyingKey::from_bytes(&generated.public_key).unwrap(),
                vec![Ok(empty_response()), Ok(empty_response())],
                Arc::clone(&log),
            ),
            &log,
            &state,
            runtime(&directory.path().join("runtime-legacy")),
            false,
        );
        let job_id = JobId::new();
        worker.acceptor.document.pending.insert(
            job_id,
            AgentAcceptJobRequest {
                lease_id: uuid::Uuid::new_v4(),
                lease_token: "redacted-legacy-token".into(),
                content_sha256: "d308e0b2d4b253d56eeca365fa4f032c65bf3fb7696b4799840f886abc3f6c7c"
                    .into(),
                local_sequence: 1,
                route_reservation_id: None,
                route_generation: None,
                route_fencing_token: None,
            },
        );
        worker.acceptor.persist().unwrap();
        *worker.authority.cleanup_retries.lock().unwrap() = 1;
        worker.reconcile_once().await.unwrap();
        assert!(
            worker
                .acceptor
                .document
                .release_cleanups
                .contains_key(&job_id)
        );
        worker.reconcile_once().await.unwrap();
        let entries = log.lock().unwrap();
        assert!(entries.contains(&"quarantine"));
        assert!(entries.contains(&"cleanup_retry"));
        assert!(entries.contains(&"remote_release"));
        assert!(entries.contains(&"signed_sync"));
        assert!(!entries.contains(&"remote_accept"));
        drop(entries);
    }

    #[tokio::test]
    async fn cleanup_authentication_failure_stays_durable_for_retry() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = vec![0_u8; 16 * 1024];
            let _ = stream.read(&mut request).await.unwrap();
            let body = br#"{"error":{"code":"invalid_agent_signature"}}"#;
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            stream.write_all(body).await.unwrap();
        });
        let identity: Arc<dyn DeviceRequestSigner> = Arc::new(DeviceIdentity::from_secret_bytes(
            AgentId::new(),
            &[29_u8; 32],
        ));
        let authority = AgentClientAuthority::new(
            AgentClient::new(url::Url::parse(&format!("http://{address}/")).unwrap()).unwrap(),
            identity,
        );
        let disposition = authority
            .cleanup_release(
                JobId::new(),
                &AgentReleaseLeaseRequest {
                    lease_id: uuid::Uuid::new_v4(),
                    lease_token: "redacted-cleanup-capability".into(),
                    reason: "route_reservation_required".into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(disposition, ReleaseCleanupDisposition::Retry);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn n_minus_one_authority_retains_exact_accept_after_response_loss_and_restart() {
        let directory = tempfile::tempdir().unwrap();
        let log = Arc::new(Mutex::new(Vec::new()));
        let provider = Arc::new(FakeSecureStore {
            key: SigningKey::from_bytes(&[31; 32]),
            deleted: Mutex::new(false),
        });
        let generated = provider.generate("com.example.pos").unwrap();
        let identity: Arc<dyn DeviceRequestSigner> = Arc::new(HostBackedDeviceIdentity::new(
            AgentId::new(),
            generated.handle,
            provider,
        ));
        let job_id = JobId::new();
        let mut response = empty_response();
        response.candidate_jobs.push(offer(job_id));
        let state = directory.path().join("n-minus-one-restart.json");
        let runtime = runtime(&directory.path().join("runtime"));
        let mut first = worker(
            authority(
                Arc::clone(&identity),
                ed25519_dalek::VerifyingKey::from_bytes(&generated.public_key).unwrap(),
                vec![Ok(response)],
                Arc::clone(&log),
            ),
            &log,
            &state,
            Arc::clone(&runtime),
            false,
        );
        first.authority.reconciliation_supported = false;
        *first.authority.accept_failures_after_commit.lock().unwrap() = 1;
        assert_eq!(
            first.reconcile_once().await.unwrap_err().code,
            "accept_response_lost"
        );
        assert!(first.acceptor.document.pending.contains_key(&job_id));
        assert!(!first.acceptor.document.confirmed.contains(&job_id));
        assert!(!first.acceptor.document.active.contains(&job_id));
        let remote_acceptances = Arc::clone(&first.authority.accepted);
        drop(first);

        let mut restarted = worker(
            authority(
                identity,
                ed25519_dalek::VerifyingKey::from_bytes(&generated.public_key).unwrap(),
                vec![Ok(empty_response())],
                Arc::clone(&log),
            ),
            &log,
            &state,
            runtime,
            false,
        );
        restarted.authority.reconciliation_supported = false;
        restarted.authority.accepted = remote_acceptances;
        assert_eq!(
            restarted.reconcile_once().await.unwrap_err().code,
            "connector_authority_upgrade_required"
        );
        assert!(restarted.acceptor.document.confirmed.contains(&job_id));
        assert!(restarted.acceptor.document.pending.contains_key(&job_id));
        assert!(!restarted.acceptor.document.active.contains(&job_id));
        let (handoffs, accepts) = {
            let entries = log.lock().unwrap();
            (
                entries
                    .iter()
                    .filter(|entry| **entry == "durable_handoff")
                    .count(),
                entries
                    .iter()
                    .filter(|entry| **entry == "remote_accept")
                    .count(),
            )
        };
        assert_eq!(handoffs, 1);
        assert_eq!(accepts, 2);
        assert!(log.lock().unwrap().contains(&"signed_sync"));
    }

    #[tokio::test]
    async fn unresolved_confirmed_acceptance_does_not_block_other_work_or_sync() {
        let directory = tempfile::tempdir().unwrap();
        let log = Arc::new(Mutex::new(Vec::new()));
        let provider = Arc::new(FakeSecureStore {
            key: SigningKey::from_bytes(&[33; 32]),
            deleted: Mutex::new(false),
        });
        let generated = provider.generate("com.example.pos").unwrap();
        let identity: Arc<dyn DeviceRequestSigner> = Arc::new(HostBackedDeviceIdentity::new(
            AgentId::new(),
            generated.handle,
            provider,
        ));
        let mut worker = worker(
            authority(
                identity,
                ed25519_dalek::VerifyingKey::from_bytes(&generated.public_key).unwrap(),
                vec![Ok(empty_response())],
                Arc::clone(&log),
            ),
            &log,
            &directory.path().join("isolated-pending.json"),
            runtime(&directory.path().join("runtime-isolated-pending")),
            false,
        );
        let unresolved_job = JobId::new();
        let accepted_job = JobId::new();
        worker
            .acceptor
            .prepare(&offer(unresolved_job), b"fixture".to_vec())
            .await
            .unwrap();
        worker
            .acceptor
            .confirm_remote_accept(unresolved_job)
            .await
            .unwrap();
        worker
            .acceptor
            .prepare(&offer(accepted_job), b"fixture".to_vec())
            .await
            .unwrap();
        worker
            .authority
            .accepted
            .lock()
            .unwrap()
            .insert(accepted_job);

        worker.reconcile_once().await.unwrap();
        assert!(
            worker
                .acceptor
                .document
                .pending
                .contains_key(&unresolved_job)
        );
        assert!(!worker.acceptor.document.active.contains(&unresolved_job));
        assert!(worker.acceptor.document.active.contains(&accepted_job));
        assert!(log.lock().unwrap().contains(&"signed_sync"));
    }

    #[tokio::test]
    async fn reconcile_error_on_one_acceptance_does_not_block_sibling_or_sync() {
        let directory = tempfile::tempdir().unwrap();
        let log = Arc::new(Mutex::new(Vec::new()));
        let provider = Arc::new(FakeSecureStore {
            key: SigningKey::from_bytes(&[34; 32]),
            deleted: Mutex::new(false),
        });
        let generated = provider.generate("com.example.pos").unwrap();
        let identity: Arc<dyn DeviceRequestSigner> = Arc::new(HostBackedDeviceIdentity::new(
            AgentId::new(),
            generated.handle,
            provider,
        ));
        let mut worker = worker(
            authority(
                identity,
                ed25519_dalek::VerifyingKey::from_bytes(&generated.public_key).unwrap(),
                vec![Ok(empty_response())],
                Arc::clone(&log),
            ),
            &log,
            &directory.path().join("isolated-error.json"),
            runtime(&directory.path().join("runtime-isolated-error")),
            false,
        );
        let poisoned_job = JobId::new();
        let accepted_job = JobId::new();
        worker
            .acceptor
            .prepare(&offer(poisoned_job), b"fixture".to_vec())
            .await
            .unwrap();
        worker
            .acceptor
            .prepare(&offer(accepted_job), b"fixture".to_vec())
            .await
            .unwrap();
        worker
            .authority
            .reconciliation_errors
            .lock()
            .unwrap()
            .insert(poisoned_job);
        worker
            .authority
            .accepted
            .lock()
            .unwrap()
            .insert(accepted_job);

        assert_eq!(
            worker.reconcile_once().await.unwrap_err().code,
            "acceptance_reconcile_unavailable"
        );
        assert!(worker.acceptor.document.pending.contains_key(&poisoned_job));
        assert!(!worker.acceptor.document.active.contains(&poisoned_job));
        assert!(worker.acceptor.document.active.contains(&accepted_job));
        assert!(log.lock().unwrap().contains(&"signed_sync"));
    }

    #[tokio::test]
    async fn n_minus_one_terminal_accept_response_is_fenced_without_activation() {
        let directory = tempfile::tempdir().unwrap();
        let log = Arc::new(Mutex::new(Vec::new()));
        let provider = Arc::new(FakeSecureStore {
            key: SigningKey::from_bytes(&[35; 32]),
            deleted: Mutex::new(false),
        });
        let generated = provider.generate("com.example.pos").unwrap();
        let identity: Arc<dyn DeviceRequestSigner> = Arc::new(HostBackedDeviceIdentity::new(
            AgentId::new(),
            generated.handle,
            provider,
        ));
        let job_id = JobId::new();
        let mut response = empty_response();
        response.candidate_jobs.push(offer(job_id));
        let state = directory.path().join("n-minus-one-terminal.json");
        let runtime = runtime(&directory.path().join("runtime-terminal"));
        let mut initial_worker = worker(
            authority(
                Arc::clone(&identity),
                ed25519_dalek::VerifyingKey::from_bytes(&generated.public_key).unwrap(),
                vec![Ok(response)],
                Arc::clone(&log),
            ),
            &log,
            &state,
            Arc::clone(&runtime),
            false,
        );
        initial_worker.authority.reconciliation_supported = false;
        initial_worker.authority.accept_outcome =
            Some(AcceptanceReconciliation::AcceptedAfterRevocation);
        initial_worker.reconcile_once().await.unwrap();
        assert!(!initial_worker.acceptor.document.active.contains(&job_id));
        assert!(
            !initial_worker
                .acceptor
                .document
                .pending
                .contains_key(&job_id)
        );
        drop(initial_worker);

        let mut restarted = worker(
            authority(
                identity,
                ed25519_dalek::VerifyingKey::from_bytes(&generated.public_key).unwrap(),
                vec![Ok(empty_response())],
                Arc::clone(&log),
            ),
            &log,
            &state,
            runtime,
            false,
        );
        restarted.authority.reconciliation_supported = false;
        restarted.reconcile_once().await.unwrap();
        assert!(!restarted.acceptor.document.active.contains(&job_id));
        let entries = log.lock().unwrap();
        assert!(entries.contains(&"abandon"));
        assert!(!entries.contains(&"activate"));
        drop(entries);
    }

    #[tokio::test]
    async fn revoked_secure_handle_fails_closed_before_sync_or_inventory_mutation() {
        let directory = tempfile::tempdir().unwrap();
        let log = Arc::new(Mutex::new(Vec::new()));
        let provider = Arc::new(FakeSecureStore {
            key: SigningKey::from_bytes(&[17; 32]),
            deleted: Mutex::new(false),
        });
        let generated = provider.generate("com.example.pos").unwrap();
        let handle = generated.handle.clone();
        let identity: Arc<dyn DeviceRequestSigner> = Arc::new(HostBackedDeviceIdentity::new(
            AgentId::new(),
            generated.handle,
            provider.clone(),
        ));
        provider.delete(&handle).unwrap();
        let mut worker = worker(
            authority(
                identity,
                ed25519_dalek::VerifyingKey::from_bytes(&generated.public_key).unwrap(),
                vec![Ok(empty_response())],
                Arc::clone(&log),
            ),
            &log,
            &directory.path().join("revoked.json"),
            runtime(&directory.path().join("runtime")),
            false,
        );
        assert_eq!(
            worker.reconcile_once().await.unwrap_err().code,
            "signing_failed"
        );
        assert!(!log.lock().unwrap().contains(&"event_ack"));
    }

    #[tokio::test]
    async fn revoke_after_remote_accept_fences_local_activation() {
        let directory = tempfile::tempdir().unwrap();
        let log = Arc::new(Mutex::new(Vec::new()));
        let provider = Arc::new(FakeSecureStore {
            key: SigningKey::from_bytes(&[19; 32]),
            deleted: Mutex::new(false),
        });
        let generated = provider.generate("com.example.pos").unwrap();
        let identity: Arc<dyn DeviceRequestSigner> = Arc::new(HostBackedDeviceIdentity::new(
            AgentId::new(),
            generated.handle,
            provider,
        ));
        let mut response = empty_response();
        response.candidate_jobs.push(offer(JobId::new()));
        let admission = Arc::new(AtomicBool::new(true));
        let mut worker = worker(
            authority(
                identity,
                ed25519_dalek::VerifyingKey::from_bytes(&generated.public_key).unwrap(),
                vec![Ok(response)],
                Arc::clone(&log),
            ),
            &log,
            &directory.path().join("revoke-race.json"),
            runtime(&directory.path().join("runtime")),
            false,
        );
        worker.authority.revoke_after_accept = Some(Arc::clone(&admission));
        worker.acceptor.admission = Some(admission);
        worker.reconcile_once().await.unwrap();
        let entries = log.lock().unwrap();
        assert!(entries.contains(&"remote_accept"));
        assert!(entries.contains(&"abandon"));
        assert!(!entries.contains(&"activate"));
        drop(entries);
    }
}
