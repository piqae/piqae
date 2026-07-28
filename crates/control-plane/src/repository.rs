#![allow(
    clippy::significant_drop_tightening,
    clippy::too_many_arguments,
    clippy::use_self
)]

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use spool_domain::{
    AgentId, EnvironmentId, EventId, Job, JobEvent, JobFailureReason, JobId, JobState,
    PrinterCapabilities, PrinterId, PrinterState, WorkspaceId, validate_transition,
};
use spool_storage_postgres::{
    AgentAuthenticationRecord, CreateJobResult as PgCreateJobResult, EnrolledAgent, JobLease,
    PostgresStore, StorageError, StoredAgent, StoredPrinter, StoredUpload, StoredWebhook,
    StoredWebhookDelivery, WebhookDeliveryWork,
};
use std::{collections::HashMap, sync::Arc};
use thiserror::Error;
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub enum CreateResult {
    Created(Job),
    Existing(Job),
}

#[derive(Debug, Error)]
pub enum RepositoryError {
    #[error("resource not found")]
    NotFound,
    #[error("idempotency conflict")]
    IdempotencyConflict,
    #[error("concurrent state change")]
    ConcurrentStateChange,
    #[error("invalid state transition")]
    InvalidTransition,
    #[error("persistence failure: {0}")]
    Persistence(String),
}

impl From<StorageError> for RepositoryError {
    fn from(error: StorageError) -> Self {
        match error {
            StorageError::NotFound => Self::NotFound,
            StorageError::IdempotencyConflict => Self::IdempotencyConflict,
            StorageError::ConcurrentStateChange => Self::ConcurrentStateChange,
            StorageError::InvalidTransition => Self::InvalidTransition,
            other => Self::Persistence(other.to_string()),
        }
    }
}

#[async_trait]
pub trait Repository: Send + Sync + 'static {
    async fn ready(&self) -> Result<(), RepositoryError>;
    async fn agent_for_authentication(
        &self,
        agent_id: AgentId,
    ) -> Result<AgentAuthenticationRecord, RepositoryError>;
    async fn reserve_agent_nonce(
        &self,
        agent_id: AgentId,
        nonce: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<(), RepositoryError>;
    async fn list_agents(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
    ) -> Result<Vec<StoredAgent>, RepositoryError>;
    async fn list_printers(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        limit: i64,
    ) -> Result<Vec<StoredPrinter>, RepositoryError>;
    async fn create_enrolment(
        &self,
        id: &str,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        secret_hash: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<(), RepositoryError>;
    async fn enrol_agent(
        &self,
        secret_hash: &str,
        public_key: &[u8],
        name: &str,
        hostname: &str,
        platform: &str,
        architecture: &str,
        version: &str,
        protocol_version: u16,
    ) -> Result<EnrolledAgent, RepositoryError>;
    async fn list_webhooks(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
    ) -> Result<Vec<StoredWebhook>, RepositoryError>;
    async fn create_webhook(
        &self,
        id: &str,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        url: &str,
        events: &[String],
        secret_ciphertext: &[u8],
    ) -> Result<StoredWebhook, RepositoryError>;
    async fn delete_webhook(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        id: &str,
    ) -> Result<(), RepositoryError>;
    async fn list_webhook_deliveries(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        webhook_id: &str,
    ) -> Result<Vec<StoredWebhookDelivery>, RepositoryError>;
    async fn replay_webhook_delivery(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        delivery_id: &str,
    ) -> Result<(), RepositoryError>;
    async fn enqueue_webhook_event(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        event_type: &str,
        payload: &serde_json::Value,
    ) -> Result<String, RepositoryError>;
    async fn claim_webhook_deliveries(
        &self,
        limit: i64,
    ) -> Result<Vec<WebhookDeliveryWork>, RepositoryError>;
    async fn record_webhook_attempt(
        &self,
        delivery_id: &str,
        status: Option<i32>,
        response_excerpt: Option<&str>,
        next_attempt_at: Option<DateTime<Utc>>,
        delivered: bool,
    ) -> Result<(), RepositoryError>;
    async fn create_upload(
        &self,
        upload: &StoredUpload,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
    ) -> Result<(), RepositoryError>;
    async fn get_upload(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        upload_id: &str,
    ) -> Result<StoredUpload, RepositoryError>;
    async fn complete_upload(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        upload_id: &str,
        actual_sha256: &str,
        actual_bytes: i64,
    ) -> Result<StoredUpload, RepositoryError>;
    async fn resolve_printer_agent(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        printer_id: PrinterId,
    ) -> Result<AgentId, RepositoryError>;
    async fn create_job(
        &self,
        job: &Job,
        agent_id: AgentId,
        idempotency_key: Option<&str>,
        request_bytes: &[u8],
    ) -> Result<CreateResult, RepositoryError>;
    async fn get_job(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        job_id: JobId,
    ) -> Result<Job, RepositoryError>;
    async fn list_jobs(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        limit: i64,
    ) -> Result<Vec<Job>, RepositoryError>;
    async fn list_job_events(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        job_id: JobId,
    ) -> Result<Vec<JobEvent>, RepositoryError>;
    async fn transition_job(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        job_id: JobId,
        state: JobState,
        reason: Option<JobFailureReason>,
        message: Option<String>,
        agent_id: Option<AgentId>,
        native_job_id: Option<String>,
    ) -> Result<Job, RepositoryError>;
    async fn claim_jobs(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        owner: &str,
        limit: i64,
    ) -> Result<Vec<JobLease>, RepositoryError>;
    async fn renew_lease(
        &self,
        job_id: JobId,
        owner: &str,
    ) -> Result<DateTime<Utc>, RepositoryError>;
    async fn renew_agent_lease(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        job_id: JobId,
        lease_id: Uuid,
        lease_token: &str,
    ) -> Result<DateTime<Utc>, RepositoryError>;
    async fn release_agent_lease(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        job_id: JobId,
        lease_id: Uuid,
        lease_token: &str,
    ) -> Result<(), RepositoryError>;
    async fn accept_agent_job(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        job_id: JobId,
        lease_id: Uuid,
        lease_token: &str,
        content_sha256: Option<&str>,
        local_sequence: u64,
    ) -> Result<Job, RepositoryError>;
    async fn compatibility_id(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        resource_type: &str,
        resource_id: &str,
    ) -> Result<i64, RepositoryError>;
    async fn resolve_compatibility_id(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        resource_type: &str,
        compatibility_id: i64,
    ) -> Result<String, RepositoryError>;
}

#[async_trait]
impl Repository for PostgresStore {
    async fn ready(&self) -> Result<(), RepositoryError> {
        self.readiness().await.map_err(Into::into)
    }

    async fn agent_for_authentication(
        &self,
        agent_id: AgentId,
    ) -> Result<AgentAuthenticationRecord, RepositoryError> {
        Self::agent_for_authentication(self, agent_id)
            .await
            .map_err(Into::into)
    }

    async fn reserve_agent_nonce(
        &self,
        agent_id: AgentId,
        nonce: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        Self::reserve_agent_nonce(self, agent_id, nonce, expires_at)
            .await
            .map_err(Into::into)
    }

    async fn list_agents(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
    ) -> Result<Vec<StoredAgent>, RepositoryError> {
        Self::list_agents(self, workspace_id, environment_id)
            .await
            .map_err(Into::into)
    }

    async fn list_printers(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        limit: i64,
    ) -> Result<Vec<StoredPrinter>, RepositoryError> {
        Self::list_printers(self, workspace_id, environment_id, limit)
            .await
            .map_err(Into::into)
    }

    async fn create_enrolment(
        &self,
        id: &str,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        secret_hash: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        Self::create_enrolment(
            self,
            id,
            workspace_id,
            environment_id,
            secret_hash,
            expires_at,
        )
        .await
        .map_err(Into::into)
    }

    async fn resolve_printer_agent(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        printer_id: PrinterId,
    ) -> Result<AgentId, RepositoryError> {
        PostgresStore::resolve_printer_agent(self, workspace_id, environment_id, printer_id)
            .await
            .map_err(Into::into)
    }

    async fn enrol_agent(
        &self,
        secret_hash: &str,
        public_key: &[u8],
        name: &str,
        hostname: &str,
        platform: &str,
        architecture: &str,
        version: &str,
        protocol_version: u16,
    ) -> Result<EnrolledAgent, RepositoryError> {
        Self::enrol_agent(
            self,
            secret_hash,
            public_key,
            name,
            hostname,
            platform,
            architecture,
            version,
            protocol_version,
        )
        .await
        .map_err(Into::into)
    }

    async fn list_webhooks(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
    ) -> Result<Vec<StoredWebhook>, RepositoryError> {
        Self::list_webhooks(self, workspace_id, environment_id)
            .await
            .map_err(Into::into)
    }

    async fn create_webhook(
        &self,
        id: &str,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        url: &str,
        events: &[String],
        secret_ciphertext: &[u8],
    ) -> Result<StoredWebhook, RepositoryError> {
        Self::create_webhook(
            self,
            id,
            workspace_id,
            environment_id,
            url,
            events,
            secret_ciphertext,
        )
        .await
        .map_err(Into::into)
    }

    async fn delete_webhook(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        id: &str,
    ) -> Result<(), RepositoryError> {
        Self::delete_webhook(self, workspace_id, environment_id, id)
            .await
            .map_err(Into::into)
    }

    async fn list_webhook_deliveries(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        webhook_id: &str,
    ) -> Result<Vec<StoredWebhookDelivery>, RepositoryError> {
        Self::list_webhook_deliveries(self, workspace_id, environment_id, webhook_id)
            .await
            .map_err(Into::into)
    }

    async fn replay_webhook_delivery(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        delivery_id: &str,
    ) -> Result<(), RepositoryError> {
        Self::replay_webhook_delivery(self, workspace_id, environment_id, delivery_id)
            .await
            .map_err(Into::into)
    }

    async fn enqueue_webhook_event(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        event_type: &str,
        payload: &serde_json::Value,
    ) -> Result<String, RepositoryError> {
        Self::enqueue_webhook_event(self, workspace_id, environment_id, event_type, payload)
            .await
            .map_err(Into::into)
    }

    async fn claim_webhook_deliveries(
        &self,
        limit: i64,
    ) -> Result<Vec<WebhookDeliveryWork>, RepositoryError> {
        Self::claim_webhook_deliveries(self, limit)
            .await
            .map_err(Into::into)
    }

    async fn record_webhook_attempt(
        &self,
        delivery_id: &str,
        status: Option<i32>,
        response_excerpt: Option<&str>,
        next_attempt_at: Option<DateTime<Utc>>,
        delivered: bool,
    ) -> Result<(), RepositoryError> {
        Self::record_webhook_attempt(
            self,
            delivery_id,
            status,
            response_excerpt,
            next_attempt_at,
            delivered,
        )
        .await
        .map_err(Into::into)
    }

    async fn create_upload(
        &self,
        upload: &StoredUpload,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
    ) -> Result<(), RepositoryError> {
        Self::create_upload(self, upload, workspace_id, environment_id)
            .await
            .map_err(Into::into)
    }

    async fn get_upload(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        upload_id: &str,
    ) -> Result<StoredUpload, RepositoryError> {
        Self::get_upload(self, workspace_id, environment_id, upload_id)
            .await
            .map_err(Into::into)
    }

    async fn complete_upload(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        upload_id: &str,
        actual_sha256: &str,
        actual_bytes: i64,
    ) -> Result<StoredUpload, RepositoryError> {
        Self::complete_upload(
            self,
            workspace_id,
            environment_id,
            upload_id,
            actual_sha256,
            actual_bytes,
        )
        .await
        .map_err(Into::into)
    }

    async fn create_job(
        &self,
        job: &Job,
        agent_id: AgentId,
        idempotency_key: Option<&str>,
        request_bytes: &[u8],
    ) -> Result<CreateResult, RepositoryError> {
        match PostgresStore::create_job(self, job, agent_id, idempotency_key, request_bytes).await?
        {
            PgCreateJobResult::Created(job) => Ok(CreateResult::Created(job)),
            PgCreateJobResult::Existing(job) => Ok(CreateResult::Existing(job)),
        }
    }

    async fn get_job(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        job_id: JobId,
    ) -> Result<Job, RepositoryError> {
        PostgresStore::get_job(self, workspace_id, environment_id, job_id)
            .await
            .map_err(Into::into)
    }

    async fn list_jobs(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        limit: i64,
    ) -> Result<Vec<Job>, RepositoryError> {
        PostgresStore::list_jobs(self, workspace_id, environment_id, limit)
            .await
            .map_err(Into::into)
    }

    async fn list_job_events(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        job_id: JobId,
    ) -> Result<Vec<JobEvent>, RepositoryError> {
        PostgresStore::list_job_events(self, workspace_id, environment_id, job_id)
            .await
            .map_err(Into::into)
    }

    async fn transition_job(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        job_id: JobId,
        state: JobState,
        reason: Option<JobFailureReason>,
        message: Option<String>,
        agent_id: Option<AgentId>,
        native_job_id: Option<String>,
    ) -> Result<Job, RepositoryError> {
        let sequence = self
            .get_job_sequence(workspace_id, environment_id, job_id)
            .await?
            + 1;
        let event = JobEvent {
            id: EventId::new(),
            job_id,
            sequence,
            state,
            reason,
            message,
            agent_id,
            native_job_id,
            occurred_at: Utc::now(),
        };
        self.append_event(workspace_id, environment_id, &event)
            .await
            .map_err(Into::into)
    }

    async fn claim_jobs(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        owner: &str,
        limit: i64,
    ) -> Result<Vec<JobLease>, RepositoryError> {
        PostgresStore::claim_jobs(self, workspace_id, environment_id, agent_id, owner, limit)
            .await
            .map_err(Into::into)
    }

    async fn renew_lease(
        &self,
        job_id: JobId,
        owner: &str,
    ) -> Result<DateTime<Utc>, RepositoryError> {
        PostgresStore::renew_lease(self, job_id, owner)
            .await
            .map_err(Into::into)
    }

    async fn renew_agent_lease(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        job_id: JobId,
        lease_id: Uuid,
        lease_token: &str,
    ) -> Result<DateTime<Utc>, RepositoryError> {
        Self::renew_agent_lease(
            self,
            workspace_id,
            environment_id,
            agent_id,
            job_id,
            lease_id,
            lease_token,
        )
        .await
        .map_err(Into::into)
    }

    async fn release_agent_lease(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        job_id: JobId,
        lease_id: Uuid,
        lease_token: &str,
    ) -> Result<(), RepositoryError> {
        Self::release_agent_lease(
            self,
            workspace_id,
            environment_id,
            agent_id,
            job_id,
            lease_id,
            lease_token,
        )
        .await
        .map_err(Into::into)
    }

    async fn accept_agent_job(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        job_id: JobId,
        lease_id: Uuid,
        lease_token: &str,
        content_sha256: Option<&str>,
        local_sequence: u64,
    ) -> Result<Job, RepositoryError> {
        Self::accept_agent_job(
            self,
            workspace_id,
            environment_id,
            agent_id,
            job_id,
            lease_id,
            lease_token,
            content_sha256,
            local_sequence,
        )
        .await
        .map_err(Into::into)
    }

    async fn compatibility_id(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        resource_type: &str,
        resource_id: &str,
    ) -> Result<i64, RepositoryError> {
        PostgresStore::compatibility_id(
            self,
            workspace_id,
            environment_id,
            resource_type,
            resource_id,
        )
        .await
        .map_err(Into::into)
    }

    async fn resolve_compatibility_id(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        resource_type: &str,
        compatibility_id: i64,
    ) -> Result<String, RepositoryError> {
        PostgresStore::resolve_compatibility_id(
            self,
            workspace_id,
            environment_id,
            resource_type,
            compatibility_id,
        )
        .await
        .map_err(Into::into)
    }
}

#[derive(Clone, Debug)]
struct MemoryJob {
    job: Job,
    agent_id: AgentId,
    sequence: u64,
    events: Vec<JobEvent>,
}

#[derive(Debug, Default)]
struct MemoryState {
    jobs: HashMap<JobId, MemoryJob>,
    printers: HashMap<PrinterId, (WorkspaceId, EnvironmentId, StoredPrinter)>,
    agents: HashMap<AgentId, (WorkspaceId, EnvironmentId, StoredAgent)>,
    agent_public_keys: HashMap<AgentId, Vec<u8>>,
    enrolments: HashMap<String, (WorkspaceId, EnvironmentId, String, DateTime<Utc>)>,
    webhooks: HashMap<String, (WorkspaceId, EnvironmentId, StoredWebhook, Vec<u8>)>,
    webhook_deliveries: HashMap<String, (WorkspaceId, EnvironmentId, StoredWebhookDelivery)>,
    webhook_work: HashMap<String, WebhookDeliveryWork>,
    uploads: HashMap<String, (WorkspaceId, EnvironmentId, StoredUpload)>,
    agent_nonces: HashMap<(AgentId, String), DateTime<Utc>>,
    leases: HashMap<JobId, (AgentId, Uuid, String, DateTime<Utc>)>,
    idempotency: HashMap<(WorkspaceId, EnvironmentId, String), (Vec<u8>, JobId)>,
    compatibility: HashMap<(WorkspaceId, EnvironmentId, String, String), i64>,
    reverse_compatibility: HashMap<(WorkspaceId, EnvironmentId, String, i64), String>,
    next_compatibility_id: i64,
}

#[derive(Clone, Debug, Default)]
pub struct MemoryRepository {
    state: Arc<RwLock<MemoryState>>,
}

impl MemoryRepository {
    pub async fn add_printer(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        printer_id: PrinterId,
        agent_id: AgentId,
    ) {
        self.state.write().await.printers.insert(
            printer_id,
            (
                workspace_id,
                environment_id,
                StoredPrinter {
                    id: printer_id,
                    agent_id,
                    name: "Test printer".into(),
                    state: PrinterState::Online,
                    capabilities: PrinterCapabilities::default(),
                    updated_at: Utc::now(),
                },
            ),
        );
        self.state.write().await.agents.insert(
            agent_id,
            (
                workspace_id,
                environment_id,
                StoredAgent {
                    id: agent_id,
                    name: "Test agent".into(),
                    platform: "test".into(),
                    state: "connected".into(),
                    version: env!("CARGO_PKG_VERSION").into(),
                    last_seen_at: Utc::now(),
                },
            ),
        );
    }

    pub async fn set_agent_public_key(&self, agent_id: AgentId, public_key: Vec<u8>) {
        self.state
            .write()
            .await
            .agent_public_keys
            .insert(agent_id, public_key);
    }
}

#[async_trait]
impl Repository for MemoryRepository {
    async fn ready(&self) -> Result<(), RepositoryError> {
        Ok(())
    }

    async fn agent_for_authentication(
        &self,
        agent_id: AgentId,
    ) -> Result<AgentAuthenticationRecord, RepositoryError> {
        let state = self.state.read().await;
        state
            .agents
            .get(&agent_id)
            .map(|(workspace, environment, _)| AgentAuthenticationRecord {
                workspace_id: *workspace,
                environment_id: *environment,
                public_key: state
                    .agent_public_keys
                    .get(&agent_id)
                    .cloned()
                    .unwrap_or_default(),
            })
            .ok_or(RepositoryError::NotFound)
    }

    async fn reserve_agent_nonce(
        &self,
        agent_id: AgentId,
        nonce: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        let mut state = self.state.write().await;
        if state
            .agent_nonces
            .insert((agent_id, nonce.to_owned()), expires_at)
            .is_some()
        {
            return Err(RepositoryError::ConcurrentStateChange);
        }
        Ok(())
    }

    async fn list_agents(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
    ) -> Result<Vec<StoredAgent>, RepositoryError> {
        Ok(self
            .state
            .read()
            .await
            .agents
            .values()
            .filter(|(workspace, environment, _)| {
                *workspace == workspace_id && *environment == environment_id
            })
            .map(|(_, _, agent)| agent.clone())
            .collect())
    }

    async fn list_printers(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        limit: i64,
    ) -> Result<Vec<StoredPrinter>, RepositoryError> {
        Ok(self
            .state
            .read()
            .await
            .printers
            .values()
            .filter(|(workspace, environment, _)| {
                *workspace == workspace_id && *environment == environment_id
            })
            .map(|(_, _, printer)| printer.clone())
            .take(usize::try_from(limit.clamp(1, 500)).unwrap_or(500))
            .collect())
    }

    async fn create_enrolment(
        &self,
        id: &str,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        secret_hash: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        self.state.write().await.enrolments.insert(
            id.to_owned(),
            (
                workspace_id,
                environment_id,
                secret_hash.to_owned(),
                expires_at,
            ),
        );
        Ok(())
    }

    async fn enrol_agent(
        &self,
        secret_hash: &str,
        public_key: &[u8],
        name: &str,
        _hostname: &str,
        platform: &str,
        _architecture: &str,
        version: &str,
        _protocol_version: u16,
    ) -> Result<EnrolledAgent, RepositoryError> {
        let mut state = self.state.write().await;
        let (token_id, (workspace_id, environment_id, _, _)) = state
            .enrolments
            .iter()
            .find(|(_, (_, _, stored_hash, expires))| {
                stored_hash == secret_hash && *expires > Utc::now()
            })
            .map(|(id, value)| (id.clone(), value.clone()))
            .ok_or(RepositoryError::NotFound)?;
        state.enrolments.remove(&token_id);
        let agent_id = AgentId::new();
        state.agents.insert(
            agent_id,
            (
                workspace_id,
                environment_id,
                StoredAgent {
                    id: agent_id,
                    name: name.into(),
                    platform: platform.into(),
                    state: "connected".into(),
                    version: version.into(),
                    last_seen_at: Utc::now(),
                },
            ),
        );
        state
            .agent_public_keys
            .insert(agent_id, public_key.to_vec());
        Ok(EnrolledAgent {
            agent_id,
            workspace_id,
            environment_id,
        })
    }

    async fn list_webhooks(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
    ) -> Result<Vec<StoredWebhook>, RepositoryError> {
        Ok(self
            .state
            .read()
            .await
            .webhooks
            .values()
            .filter(|(workspace, environment, _, _)| {
                *workspace == workspace_id && *environment == environment_id
            })
            .map(|(_, _, webhook, _)| webhook.clone())
            .collect())
    }

    async fn create_webhook(
        &self,
        id: &str,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        url: &str,
        events: &[String],
        secret_ciphertext: &[u8],
    ) -> Result<StoredWebhook, RepositoryError> {
        let webhook = StoredWebhook {
            id: id.to_owned(),
            url: url.to_owned(),
            events: events.to_vec(),
            enabled: true,
            created_at: Utc::now(),
        };
        self.state.write().await.webhooks.insert(
            id.to_owned(),
            (
                workspace_id,
                environment_id,
                webhook.clone(),
                secret_ciphertext.to_vec(),
            ),
        );
        Ok(webhook)
    }

    async fn delete_webhook(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        id: &str,
    ) -> Result<(), RepositoryError> {
        let mut state = self.state.write().await;
        match state.webhooks.get(id) {
            Some((workspace, environment, _, _))
                if *workspace == workspace_id && *environment == environment_id =>
            {
                state.webhooks.remove(id);
                Ok(())
            }
            _ => Err(RepositoryError::NotFound),
        }
    }

    async fn list_webhook_deliveries(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        _webhook_id: &str,
    ) -> Result<Vec<StoredWebhookDelivery>, RepositoryError> {
        Ok(self
            .state
            .read()
            .await
            .webhook_deliveries
            .values()
            .filter(|(workspace, environment, _)| {
                *workspace == workspace_id && *environment == environment_id
            })
            .map(|(_, _, delivery)| delivery.clone())
            .collect())
    }

    async fn replay_webhook_delivery(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        delivery_id: &str,
    ) -> Result<(), RepositoryError> {
        let mut state = self.state.write().await;
        let (_, _, delivery) = state
            .webhook_deliveries
            .get_mut(delivery_id)
            .filter(|(workspace, environment, _)| {
                *workspace == workspace_id && *environment == environment_id
            })
            .ok_or(RepositoryError::NotFound)?;
        delivery.next_attempt_at = Utc::now();
        delivery.dead_lettered_at = None;
        delivery.delivered_at = None;
        delivery.response_status = None;
        Ok(())
    }

    async fn enqueue_webhook_event(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        event_type: &str,
        payload: &serde_json::Value,
    ) -> Result<String, RepositoryError> {
        let mut state = self.state.write().await;
        let event_id = EventId::new().to_string();
        let endpoints = state
            .webhooks
            .values()
            .filter(|(workspace, environment, endpoint, _)| {
                *workspace == workspace_id
                    && *environment == environment_id
                    && endpoint.enabled
                    && endpoint.events.iter().any(|event| event == event_type)
            })
            .map(|(_, _, endpoint, secret)| (endpoint.url.clone(), secret.clone()))
            .collect::<Vec<_>>();
        for (url, secret_ciphertext) in endpoints {
            let id = format!("whd_{}", ulid::Ulid::new());
            state.webhook_work.insert(
                id.clone(),
                WebhookDeliveryWork {
                    id,
                    event_id: event_id.clone(),
                    event_type: event_type.into(),
                    url,
                    secret_ciphertext,
                    payload: payload.clone(),
                    attempt: 0,
                },
            );
        }
        Ok(event_id)
    }

    async fn claim_webhook_deliveries(
        &self,
        limit: i64,
    ) -> Result<Vec<WebhookDeliveryWork>, RepositoryError> {
        Ok(self
            .state
            .read()
            .await
            .webhook_work
            .values()
            .take(usize::try_from(limit.clamp(1, 100)).unwrap_or(100))
            .cloned()
            .collect())
    }

    async fn record_webhook_attempt(
        &self,
        delivery_id: &str,
        _status: Option<i32>,
        _response_excerpt: Option<&str>,
        next_attempt_at: Option<DateTime<Utc>>,
        delivered: bool,
    ) -> Result<(), RepositoryError> {
        let mut state = self.state.write().await;
        if delivered || next_attempt_at.is_none() {
            state
                .webhook_work
                .remove(delivery_id)
                .ok_or(RepositoryError::NotFound)?;
        } else {
            state
                .webhook_work
                .get_mut(delivery_id)
                .ok_or(RepositoryError::NotFound)?
                .attempt += 1;
        }
        Ok(())
    }

    async fn create_upload(
        &self,
        upload: &StoredUpload,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
    ) -> Result<(), RepositoryError> {
        self.state.write().await.uploads.insert(
            upload.id.clone(),
            (workspace_id, environment_id, upload.clone()),
        );
        Ok(())
    }

    async fn get_upload(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        upload_id: &str,
    ) -> Result<StoredUpload, RepositoryError> {
        self.state
            .read()
            .await
            .uploads
            .get(upload_id)
            .filter(|(workspace, environment, _)| {
                *workspace == workspace_id && *environment == environment_id
            })
            .map(|(_, _, upload)| upload.clone())
            .ok_or(RepositoryError::NotFound)
    }

    async fn complete_upload(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        upload_id: &str,
        actual_sha256: &str,
        actual_bytes: i64,
    ) -> Result<StoredUpload, RepositoryError> {
        let mut state = self.state.write().await;
        let (_, _, upload) = state
            .uploads
            .get_mut(upload_id)
            .filter(|(workspace, environment, upload)| {
                *workspace == workspace_id
                    && *environment == environment_id
                    && upload.state == "pending"
                    && upload.expires_at > Utc::now()
                    && upload.expected_sha256 == actual_sha256
                    && upload.expected_bytes == actual_bytes
            })
            .ok_or(RepositoryError::NotFound)?;
        upload.state = "complete".into();
        Ok(upload.clone())
    }

    async fn resolve_printer_agent(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        printer_id: PrinterId,
    ) -> Result<AgentId, RepositoryError> {
        self.state
            .read()
            .await
            .printers
            .get(&printer_id)
            .filter(|(workspace, environment, _printer)| {
                *workspace == workspace_id && *environment == environment_id
            })
            .map(|(_, _, printer)| printer.agent_id)
            .ok_or(RepositoryError::NotFound)
    }

    async fn create_job(
        &self,
        job: &Job,
        agent_id: AgentId,
        idempotency_key: Option<&str>,
        request_bytes: &[u8],
    ) -> Result<CreateResult, RepositoryError> {
        let mut state = self.state.write().await;
        if let Some(key) = idempotency_key {
            let index = (job.workspace_id, job.environment_id, key.to_owned());
            if let Some((existing_bytes, job_id)) = state.idempotency.get(&index) {
                if existing_bytes != request_bytes {
                    return Err(RepositoryError::IdempotencyConflict);
                }
                return state
                    .jobs
                    .get(job_id)
                    .map(|record| CreateResult::Existing(record.job.clone()))
                    .ok_or(RepositoryError::NotFound);
            }
            state
                .idempotency
                .insert(index, (request_bytes.to_vec(), job.id));
        }
        state.jobs.insert(
            job.id,
            MemoryJob {
                job: job.clone(),
                agent_id,
                sequence: 1,
                events: vec![JobEvent {
                    id: EventId::new(),
                    job_id: job.id,
                    sequence: 1,
                    state: job.state,
                    reason: None,
                    message: Some("Job durably registered".into()),
                    agent_id: None,
                    native_job_id: None,
                    occurred_at: job.created_at,
                }],
            },
        );
        Ok(CreateResult::Created(job.clone()))
    }

    async fn get_job(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        job_id: JobId,
    ) -> Result<Job, RepositoryError> {
        self.state
            .read()
            .await
            .jobs
            .get(&job_id)
            .filter(|record| {
                record.job.workspace_id == workspace_id
                    && record.job.environment_id == environment_id
            })
            .map(|record| record.job.clone())
            .ok_or(RepositoryError::NotFound)
    }

    async fn list_jobs(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        limit: i64,
    ) -> Result<Vec<Job>, RepositoryError> {
        let mut jobs = self
            .state
            .read()
            .await
            .jobs
            .values()
            .filter(|record| {
                record.job.workspace_id == workspace_id
                    && record.job.environment_id == environment_id
            })
            .map(|record| record.job.clone())
            .collect::<Vec<_>>();
        jobs.sort_by_key(|job| std::cmp::Reverse((job.created_at, job.id)));
        jobs.truncate(usize::try_from(limit.clamp(1, 500)).unwrap_or(500));
        Ok(jobs)
    }

    async fn list_job_events(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        job_id: JobId,
    ) -> Result<Vec<JobEvent>, RepositoryError> {
        self.state
            .read()
            .await
            .jobs
            .get(&job_id)
            .filter(|record| {
                record.job.workspace_id == workspace_id
                    && record.job.environment_id == environment_id
            })
            .map(|record| record.events.clone())
            .ok_or(RepositoryError::NotFound)
    }

    async fn transition_job(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        job_id: JobId,
        next: JobState,
        reason: Option<JobFailureReason>,
        message: Option<String>,
        agent_id: Option<AgentId>,
        native_job_id: Option<String>,
    ) -> Result<Job, RepositoryError> {
        let mut state = self.state.write().await;
        let record = state
            .jobs
            .get_mut(&job_id)
            .ok_or(RepositoryError::NotFound)?;
        if record.job.workspace_id != workspace_id || record.job.environment_id != environment_id {
            return Err(RepositoryError::NotFound);
        }
        validate_transition(record.job.state, next)
            .map_err(|_| RepositoryError::InvalidTransition)?;
        record.job.state = next;
        record.sequence += 1;
        record.events.push(JobEvent {
            id: EventId::new(),
            job_id,
            sequence: record.sequence,
            state: next,
            reason,
            message,
            agent_id,
            native_job_id,
            occurred_at: Utc::now(),
        });
        Ok(record.job.clone())
    }

    async fn claim_jobs(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        _owner: &str,
        limit: i64,
    ) -> Result<Vec<JobLease>, RepositoryError> {
        let lease_until = Utc::now() + chrono::Duration::seconds(30);
        let mut state = self.state.write().await;
        let jobs = state
            .jobs
            .values()
            .filter(|record| {
                record.agent_id == agent_id
                    && record.job.workspace_id == workspace_id
                    && record.job.environment_id == environment_id
                    && matches!(
                        record.job.state,
                        JobState::WaitingForAgent | JobState::FailedRetryable
                    )
            })
            .take(usize::try_from(limit.clamp(1, 100)).unwrap_or(100))
            .map(|record| record.job.clone())
            .collect::<Vec<_>>();
        let mut leases = Vec::with_capacity(jobs.len());
        for job in jobs {
            let lease_id = Uuid::new_v4();
            let lease_token = Uuid::new_v4().to_string();
            state.leases.insert(
                job.id,
                (agent_id, lease_id, lease_token.clone(), lease_until),
            );
            leases.push(JobLease {
                job,
                lease_id,
                lease_token,
                lease_until,
            });
        }
        Ok(leases)
    }

    async fn renew_lease(
        &self,
        job_id: JobId,
        _owner: &str,
    ) -> Result<DateTime<Utc>, RepositoryError> {
        self.state
            .read()
            .await
            .jobs
            .contains_key(&job_id)
            .then(|| Utc::now() + chrono::Duration::seconds(30))
            .ok_or(RepositoryError::NotFound)
    }

    async fn renew_agent_lease(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        job_id: JobId,
        lease_id: Uuid,
        lease_token: &str,
    ) -> Result<DateTime<Utc>, RepositoryError> {
        let mut state = self.state.write().await;
        let record = state.jobs.get(&job_id).ok_or(RepositoryError::NotFound)?;
        if record.job.workspace_id != workspace_id || record.job.environment_id != environment_id {
            return Err(RepositoryError::NotFound);
        }
        let lease = state
            .leases
            .get_mut(&job_id)
            .filter(|(stored_agent, stored_id, stored_token, expires)| {
                *stored_agent == agent_id
                    && *stored_id == lease_id
                    && stored_token == lease_token
                    && *expires > Utc::now()
            })
            .ok_or(RepositoryError::ConcurrentStateChange)?;
        lease.3 = Utc::now() + chrono::Duration::seconds(30);
        Ok(lease.3)
    }

    async fn release_agent_lease(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        job_id: JobId,
        lease_id: Uuid,
        lease_token: &str,
    ) -> Result<(), RepositoryError> {
        self.renew_agent_lease(
            workspace_id,
            environment_id,
            agent_id,
            job_id,
            lease_id,
            lease_token,
        )
        .await?;
        self.state.write().await.leases.remove(&job_id);
        Ok(())
    }

    async fn accept_agent_job(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        job_id: JobId,
        lease_id: Uuid,
        lease_token: &str,
        _content_sha256: Option<&str>,
        _local_sequence: u64,
    ) -> Result<Job, RepositoryError> {
        self.renew_agent_lease(
            workspace_id,
            environment_id,
            agent_id,
            job_id,
            lease_id,
            lease_token,
        )
        .await?;
        let mut state = self.state.write().await;
        state.leases.remove(&job_id);
        let record = state
            .jobs
            .get_mut(&job_id)
            .ok_or(RepositoryError::NotFound)?;
        record.job.state = JobState::AgentAccepted;
        record.sequence += 1;
        record.events.push(JobEvent {
            id: EventId::new(),
            job_id,
            sequence: record.sequence,
            state: JobState::AgentAccepted,
            reason: None,
            message: Some("Agent durably accepted the job".into()),
            agent_id: Some(agent_id),
            native_job_id: None,
            occurred_at: Utc::now(),
        });
        Ok(record.job.clone())
    }

    async fn compatibility_id(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        resource_type: &str,
        resource_id: &str,
    ) -> Result<i64, RepositoryError> {
        let mut state = self.state.write().await;
        let key = (
            workspace_id,
            environment_id,
            resource_type.to_owned(),
            resource_id.to_owned(),
        );
        if let Some(id) = state.compatibility.get(&key) {
            return Ok(*id);
        }
        state.next_compatibility_id += 1;
        let id = state.next_compatibility_id;
        state.compatibility.insert(key, id);
        state.reverse_compatibility.insert(
            (workspace_id, environment_id, resource_type.to_owned(), id),
            resource_id.to_owned(),
        );
        Ok(id)
    }

    async fn resolve_compatibility_id(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        resource_type: &str,
        compatibility_id: i64,
    ) -> Result<String, RepositoryError> {
        self.state
            .read()
            .await
            .reverse_compatibility
            .get(&(
                workspace_id,
                environment_id,
                resource_type.to_owned(),
                compatibility_id,
            ))
            .cloned()
            .ok_or(RepositoryError::NotFound)
    }
}
