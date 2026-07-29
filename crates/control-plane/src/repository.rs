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
use spool_protocol::agent::AgentCommand;
use spool_storage_postgres::{
    AgentAuthenticationRecord, CreateJobResult as PgCreateJobResult, EnrolledAgent, JobLease,
    PostgresStore, StorageError, StoredAgent, StoredAgentCommandBatch, StoredApiKey, StoredPrinter,
    StoredStock, StoredTarget, StoredTargetBinding, StoredTenantEvent, StoredUpload, StoredWebhook,
    StoredWebhookDelivery, SyncedPrinter, WebhookDeliveryWork,
};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use thiserror::Error;
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub enum CreateResult {
    Created(Job),
    Existing(Job),
}

#[derive(Clone, Debug)]
pub struct AgentCommandBatch {
    pub cursor: Option<String>,
    pub commands: Vec<AgentCommand>,
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
    async fn environment_kind(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
    ) -> Result<String, RepositoryError>;
    async fn list_api_keys(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
    ) -> Result<Vec<StoredApiKey>, RepositoryError>;
    async fn create_api_key(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        id: &str,
        name: &str,
        lookup_prefix: &str,
        secret_hash: &str,
        scopes: &[String],
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<StoredApiKey, RepositoryError>;
    async fn revoke_api_key(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        id: &str,
    ) -> Result<StoredApiKey, RepositoryError>;
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
    async fn sync_agent_presence(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        version: &str,
        printers: Option<&[SyncedPrinter]>,
    ) -> Result<(), RepositoryError>;
    async fn enqueue_agent_command(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        command: &AgentCommand,
    ) -> Result<String, RepositoryError>;
    async fn sync_agent_commands(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        acknowledged_cursor: Option<&str>,
        limit: i64,
    ) -> Result<AgentCommandBatch, RepositoryError>;
    async fn list_agents(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
    ) -> Result<Vec<StoredAgent>, RepositoryError>;
    async fn list_printers(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        after: Option<PrinterId>,
        limit: i64,
    ) -> Result<Vec<StoredPrinter>, RepositoryError>;
    async fn get_printer(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        printer_id: PrinterId,
    ) -> Result<StoredPrinter, RepositoryError>;
    async fn list_stocks(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
    ) -> Result<Vec<StoredStock>, RepositoryError>;
    async fn get_stock(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        id: &str,
    ) -> Result<StoredStock, RepositoryError>;
    async fn create_stock(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        stock: &StoredStock,
    ) -> Result<StoredStock, RepositoryError>;
    async fn update_stock(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        stock: &StoredStock,
    ) -> Result<StoredStock, RepositoryError>;
    async fn list_targets(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
    ) -> Result<Vec<StoredTarget>, RepositoryError>;
    async fn get_target(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        id: &str,
    ) -> Result<StoredTarget, RepositoryError>;
    async fn create_target(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        target: &StoredTarget,
    ) -> Result<StoredTarget, RepositoryError>;
    async fn update_target(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        target: &StoredTarget,
    ) -> Result<StoredTarget, RepositoryError>;
    async fn list_target_bindings(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        target_id: &str,
    ) -> Result<Vec<StoredTargetBinding>, RepositoryError>;
    async fn create_target_binding(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        binding: &StoredTargetBinding,
    ) -> Result<StoredTargetBinding, RepositoryError>;
    async fn delete_target_binding(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        target_id: &str,
        binding_id: &str,
    ) -> Result<(), RepositoryError>;
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
    async fn list_tenant_events(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        after: Option<&str>,
        limit: i64,
    ) -> Result<Vec<StoredTenantEvent>, RepositoryError>;
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
        after: Option<JobId>,
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
    async fn request_job_cancellation(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        job_id: JobId,
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
    async fn validate_agent_lease(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        job_id: JobId,
        lease_id: Uuid,
        lease_token: &str,
    ) -> Result<(), RepositoryError>;
    async fn apply_agent_event(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        event: &JobEvent,
    ) -> Result<Option<Job>, RepositoryError>;
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

    async fn environment_kind(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
    ) -> Result<String, RepositoryError> {
        Self::environment_kind(self, workspace_id, environment_id)
            .await
            .map_err(Into::into)
    }

    async fn list_api_keys(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
    ) -> Result<Vec<StoredApiKey>, RepositoryError> {
        Self::list_api_keys(self, workspace_id, environment_id)
            .await
            .map_err(Into::into)
    }

    async fn create_api_key(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        id: &str,
        name: &str,
        lookup_prefix: &str,
        secret_hash: &str,
        scopes: &[String],
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<StoredApiKey, RepositoryError> {
        Self::create_api_key(
            self,
            workspace_id,
            environment_id,
            id,
            name,
            lookup_prefix,
            secret_hash,
            scopes,
            expires_at,
        )
        .await
        .map_err(Into::into)
    }

    async fn revoke_api_key(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        id: &str,
    ) -> Result<StoredApiKey, RepositoryError> {
        Self::revoke_api_key(self, workspace_id, environment_id, id)
            .await
            .map_err(Into::into)
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

    async fn sync_agent_presence(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        version: &str,
        printers: Option<&[SyncedPrinter]>,
    ) -> Result<(), RepositoryError> {
        Self::sync_agent_presence(
            self,
            workspace_id,
            environment_id,
            agent_id,
            version,
            printers,
        )
        .await
        .map_err(Into::into)
    }

    async fn enqueue_agent_command(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        command: &AgentCommand,
    ) -> Result<String, RepositoryError> {
        Self::enqueue_agent_command(
            self,
            workspace_id,
            environment_id,
            agent_id,
            &serde_json::to_value(command)
                .map_err(|error| RepositoryError::Persistence(error.to_string()))?,
        )
        .await
        .map_err(Into::into)
    }

    async fn sync_agent_commands(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        acknowledged_cursor: Option<&str>,
        limit: i64,
    ) -> Result<AgentCommandBatch, RepositoryError> {
        let acknowledged_cursor = acknowledged_cursor
            .map(str::parse::<i64>)
            .transpose()
            .map_err(|error| RepositoryError::Persistence(error.to_string()))?;
        let StoredAgentCommandBatch { cursor, commands } = Self::sync_agent_commands(
            self,
            workspace_id,
            environment_id,
            agent_id,
            acknowledged_cursor,
            limit,
        )
        .await?;
        Ok(AgentCommandBatch {
            cursor,
            commands: commands
                .into_iter()
                .map(serde_json::from_value)
                .collect::<Result<_, _>>()
                .map_err(|error| RepositoryError::Persistence(error.to_string()))?,
        })
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
        after: Option<PrinterId>,
        limit: i64,
    ) -> Result<Vec<StoredPrinter>, RepositoryError> {
        Self::list_printers(self, workspace_id, environment_id, after, limit)
            .await
            .map_err(Into::into)
    }

    async fn get_printer(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        printer_id: PrinterId,
    ) -> Result<StoredPrinter, RepositoryError> {
        Self::get_printer(self, workspace_id, environment_id, printer_id)
            .await
            .map_err(Into::into)
    }

    async fn list_stocks(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
    ) -> Result<Vec<StoredStock>, RepositoryError> {
        Self::list_stocks(self, workspace_id, environment_id)
            .await
            .map_err(Into::into)
    }

    async fn get_stock(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        id: &str,
    ) -> Result<StoredStock, RepositoryError> {
        Self::get_stock(self, workspace_id, environment_id, id)
            .await
            .map_err(Into::into)
    }

    async fn create_stock(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        stock: &StoredStock,
    ) -> Result<StoredStock, RepositoryError> {
        Self::create_stock(self, workspace_id, environment_id, stock)
            .await
            .map_err(Into::into)
    }

    async fn update_stock(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        stock: &StoredStock,
    ) -> Result<StoredStock, RepositoryError> {
        Self::update_stock(self, workspace_id, environment_id, stock)
            .await
            .map_err(Into::into)
    }

    async fn list_targets(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
    ) -> Result<Vec<StoredTarget>, RepositoryError> {
        Self::list_targets(self, workspace_id, environment_id)
            .await
            .map_err(Into::into)
    }

    async fn get_target(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        id: &str,
    ) -> Result<StoredTarget, RepositoryError> {
        Self::get_target(self, workspace_id, environment_id, id)
            .await
            .map_err(Into::into)
    }

    async fn create_target(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        target: &StoredTarget,
    ) -> Result<StoredTarget, RepositoryError> {
        Self::create_target(self, workspace_id, environment_id, target)
            .await
            .map_err(Into::into)
    }

    async fn update_target(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        target: &StoredTarget,
    ) -> Result<StoredTarget, RepositoryError> {
        Self::update_target(self, workspace_id, environment_id, target)
            .await
            .map_err(Into::into)
    }

    async fn list_target_bindings(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        target_id: &str,
    ) -> Result<Vec<StoredTargetBinding>, RepositoryError> {
        Self::list_target_bindings(self, workspace_id, environment_id, target_id)
            .await
            .map_err(Into::into)
    }

    async fn create_target_binding(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        binding: &StoredTargetBinding,
    ) -> Result<StoredTargetBinding, RepositoryError> {
        Self::create_target_binding(self, workspace_id, environment_id, binding)
            .await
            .map_err(Into::into)
    }

    async fn delete_target_binding(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        target_id: &str,
        binding_id: &str,
    ) -> Result<(), RepositoryError> {
        Self::delete_target_binding(self, workspace_id, environment_id, target_id, binding_id)
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

    async fn list_tenant_events(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        after: Option<&str>,
        limit: i64,
    ) -> Result<Vec<StoredTenantEvent>, RepositoryError> {
        Self::list_tenant_events(self, workspace_id, environment_id, after, limit)
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
        after: Option<JobId>,
        limit: i64,
    ) -> Result<Vec<Job>, RepositoryError> {
        PostgresStore::list_jobs(self, workspace_id, environment_id, after, limit)
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

    async fn request_job_cancellation(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        job_id: JobId,
    ) -> Result<Job, RepositoryError> {
        Self::request_job_cancellation(
            self,
            workspace_id,
            environment_id,
            job_id,
            &serde_json::to_value(AgentCommand::CancelJob { job_id })
                .map_err(|error| RepositoryError::Persistence(error.to_string()))?,
        )
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

    async fn validate_agent_lease(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        job_id: JobId,
        lease_id: Uuid,
        lease_token: &str,
    ) -> Result<(), RepositoryError> {
        Self::validate_agent_lease(
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

    async fn apply_agent_event(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        event: &JobEvent,
    ) -> Result<Option<Job>, RepositoryError> {
        Self::apply_agent_event(self, workspace_id, environment_id, agent_id, event)
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

#[derive(Clone, Debug)]
struct MemoryAgentCommand {
    cursor: u64,
    command: AgentCommand,
    delivered: bool,
    acknowledged: bool,
}

#[derive(Clone, Debug)]
struct MemoryJobAcceptance {
    agent_id: AgentId,
    lease_id: Uuid,
    lease_token: Option<String>,
    content_sha256: Option<String>,
    local_sequence: u64,
}

#[derive(Debug, Default)]
struct MemoryState {
    api_keys: HashMap<String, (WorkspaceId, EnvironmentId, StoredApiKey, String)>,
    jobs: HashMap<JobId, MemoryJob>,
    printers: HashMap<PrinterId, (WorkspaceId, EnvironmentId, StoredPrinter)>,
    stocks: HashMap<String, (WorkspaceId, EnvironmentId, StoredStock)>,
    targets: HashMap<String, (WorkspaceId, EnvironmentId, StoredTarget)>,
    target_bindings: HashMap<String, (WorkspaceId, EnvironmentId, StoredTargetBinding)>,
    agents: HashMap<AgentId, (WorkspaceId, EnvironmentId, StoredAgent)>,
    agent_public_keys: HashMap<AgentId, Vec<u8>>,
    enrolments: HashMap<String, (WorkspaceId, EnvironmentId, String, DateTime<Utc>)>,
    webhooks: HashMap<String, (WorkspaceId, EnvironmentId, StoredWebhook, Vec<u8>)>,
    webhook_deliveries: HashMap<String, (WorkspaceId, EnvironmentId, StoredWebhookDelivery)>,
    webhook_work: HashMap<String, WebhookDeliveryWork>,
    tenant_events: Vec<(WorkspaceId, EnvironmentId, StoredTenantEvent)>,
    uploads: HashMap<String, (WorkspaceId, EnvironmentId, StoredUpload)>,
    agent_nonces: HashMap<(AgentId, String), DateTime<Utc>>,
    agent_event_receipts: HashSet<(AgentId, EventId)>,
    agent_commands: HashMap<AgentId, Vec<MemoryAgentCommand>>,
    next_agent_command_cursor: u64,
    leases: HashMap<JobId, (AgentId, Uuid, String, DateTime<Utc>)>,
    job_acceptances: HashMap<JobId, MemoryJobAcceptance>,
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
                    capability_revision: 0,
                    native_options: std::collections::BTreeMap::default(),
                    profiles: Vec::new(),
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

    #[cfg(test)]
    pub async fn clear_acceptance_token(&self, job_id: JobId) {
        if let Some(acceptance) = self.state.write().await.job_acceptances.get_mut(&job_id) {
            acceptance.lease_token = None;
        }
    }
}

#[async_trait]
impl Repository for MemoryRepository {
    async fn ready(&self) -> Result<(), RepositoryError> {
        Ok(())
    }

    async fn environment_kind(
        &self,
        _workspace_id: WorkspaceId,
        _environment_id: EnvironmentId,
    ) -> Result<String, RepositoryError> {
        Ok("test".into())
    }

    async fn list_api_keys(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
    ) -> Result<Vec<StoredApiKey>, RepositoryError> {
        let mut keys = self
            .state
            .read()
            .await
            .api_keys
            .values()
            .filter(|(workspace, environment, _, _)| {
                *workspace == workspace_id && *environment == environment_id
            })
            .map(|(_, _, key, _)| key.clone())
            .collect::<Vec<_>>();
        keys.sort_by_key(|key| std::cmp::Reverse((key.created_at, key.id.clone())));
        Ok(keys)
    }

    async fn create_api_key(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        id: &str,
        name: &str,
        lookup_prefix: &str,
        secret_hash: &str,
        scopes: &[String],
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<StoredApiKey, RepositoryError> {
        let mut state = self.state.write().await;
        if state
            .api_keys
            .values()
            .any(|(_, _, key, _)| key.lookup_prefix == lookup_prefix)
        {
            return Err(RepositoryError::ConcurrentStateChange);
        }
        let key = StoredApiKey {
            id: id.into(),
            name: name.into(),
            lookup_prefix: lookup_prefix.into(),
            scopes: scopes.to_vec(),
            expires_at,
            last_used_at: None,
            revoked_at: None,
            created_at: Utc::now(),
        };
        state.api_keys.insert(
            id.into(),
            (
                workspace_id,
                environment_id,
                key.clone(),
                secret_hash.into(),
            ),
        );
        Ok(key)
    }

    async fn revoke_api_key(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        id: &str,
    ) -> Result<StoredApiKey, RepositoryError> {
        let mut state = self.state.write().await;
        let (_, _, key, _) = state
            .api_keys
            .get_mut(id)
            .filter(|(workspace, environment, _, _)| {
                *workspace == workspace_id && *environment == environment_id
            })
            .ok_or(RepositoryError::NotFound)?;
        if key.revoked_at.is_none() {
            key.revoked_at = Some(Utc::now());
        }
        Ok(key.clone())
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
        state.agent_nonces.retain(|_, expiry| *expiry > Utc::now());
        if state
            .agent_nonces
            .insert((agent_id, nonce.to_owned()), expires_at)
            .is_some()
        {
            return Err(RepositoryError::ConcurrentStateChange);
        }
        Ok(())
    }

    async fn sync_agent_presence(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        version: &str,
        printers: Option<&[SyncedPrinter]>,
    ) -> Result<(), RepositoryError> {
        let mut state = self.state.write().await;
        let (_, _, agent) = state
            .agents
            .get_mut(&agent_id)
            .filter(|(workspace, environment, _)| {
                *workspace == workspace_id && *environment == environment_id
            })
            .ok_or(RepositoryError::NotFound)?;
        agent.state = "connected".into();
        agent.version = version.into();
        agent.last_seen_at = Utc::now();
        if let Some(printers) = printers {
            state
                .printers
                .retain(|_, (_, _, printer)| printer.agent_id != agent_id);
            for printer in printers {
                state.printers.insert(
                    printer.id,
                    (
                        workspace_id,
                        environment_id,
                        StoredPrinter {
                            id: printer.id,
                            agent_id,
                            name: printer.name.clone(),
                            state: printer.state,
                            capabilities: printer.capabilities.clone(),
                            capability_revision: printer.capability_revision,
                            native_options: printer.native_options.clone(),
                            profiles: printer.profiles.clone(),
                            updated_at: Utc::now(),
                        },
                    ),
                );
            }
        }
        Ok(())
    }

    async fn enqueue_agent_command(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        command: &AgentCommand,
    ) -> Result<String, RepositoryError> {
        let mut state = self.state.write().await;
        state
            .agents
            .get(&agent_id)
            .filter(|(workspace, environment, _)| {
                *workspace == workspace_id && *environment == environment_id
            })
            .ok_or(RepositoryError::NotFound)?;
        state.next_agent_command_cursor = state.next_agent_command_cursor.saturating_add(1);
        let cursor = state.next_agent_command_cursor;
        state
            .agent_commands
            .entry(agent_id)
            .or_default()
            .push(MemoryAgentCommand {
                cursor,
                command: command.clone(),
                delivered: false,
                acknowledged: false,
            });
        Ok(cursor.to_string())
    }

    async fn sync_agent_commands(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        acknowledged_cursor: Option<&str>,
        limit: i64,
    ) -> Result<AgentCommandBatch, RepositoryError> {
        let mut state = self.state.write().await;
        state
            .agents
            .get(&agent_id)
            .filter(|(workspace, environment, _)| {
                *workspace == workspace_id && *environment == environment_id
            })
            .ok_or(RepositoryError::NotFound)?;
        let acknowledged_cursor = acknowledged_cursor
            .map(str::parse::<i64>)
            .transpose()
            .map_err(|error| RepositoryError::Persistence(error.to_string()))?;
        let commands = state.agent_commands.entry(agent_id).or_default();
        if let Some(cursor) = acknowledged_cursor {
            for command in commands.iter_mut().filter(|command| {
                command.delivered
                    && !command.acknowledged
                    && i64::try_from(command.cursor).is_ok_and(|value| value <= cursor)
            }) {
                command.acknowledged = true;
            }
        }
        let pending = commands
            .iter_mut()
            .filter(|command| !command.acknowledged)
            .take(usize::try_from(limit.clamp(1, 100)).unwrap_or(100))
            .map(|command| {
                command.delivered = true;
                (command.cursor, command.command.clone())
            })
            .collect::<Vec<_>>();
        Ok(AgentCommandBatch {
            cursor: pending.last().map(|(cursor, _)| cursor.to_string()),
            commands: pending.into_iter().map(|(_, command)| command).collect(),
        })
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
        after: Option<PrinterId>,
        limit: i64,
    ) -> Result<Vec<StoredPrinter>, RepositoryError> {
        let mut printers = self
            .state
            .read()
            .await
            .printers
            .values()
            .filter(|(workspace, environment, _)| {
                *workspace == workspace_id && *environment == environment_id
            })
            .map(|(_, _, printer)| printer.clone())
            .collect::<Vec<_>>();
        printers.sort_by_key(|printer| std::cmp::Reverse((printer.updated_at, printer.id)));
        if let Some(cursor) = after
            && let Some(position) = printers.iter().position(|printer| printer.id == cursor)
        {
            printers.drain(..=position);
        }
        printers.truncate(usize::try_from(limit.clamp(1, 500)).unwrap_or(500));
        Ok(printers)
    }

    async fn get_printer(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        printer_id: PrinterId,
    ) -> Result<StoredPrinter, RepositoryError> {
        self.state
            .read()
            .await
            .printers
            .get(&printer_id)
            .filter(|(workspace, environment, _)| {
                *workspace == workspace_id && *environment == environment_id
            })
            .map(|(_, _, printer)| printer.clone())
            .ok_or(RepositoryError::NotFound)
    }

    async fn list_stocks(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
    ) -> Result<Vec<StoredStock>, RepositoryError> {
        let mut values = self
            .state
            .read()
            .await
            .stocks
            .values()
            .filter(|(workspace, environment, _)| {
                *workspace == workspace_id && *environment == environment_id
            })
            .map(|(_, _, stock)| stock.clone())
            .collect::<Vec<_>>();
        values.sort_by_key(|stock| (stock.created_at, stock.id.clone()));
        Ok(values)
    }

    async fn get_stock(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        id: &str,
    ) -> Result<StoredStock, RepositoryError> {
        self.state
            .read()
            .await
            .stocks
            .get(id)
            .filter(|(workspace, environment, _)| {
                *workspace == workspace_id && *environment == environment_id
            })
            .map(|(_, _, stock)| stock.clone())
            .ok_or(RepositoryError::NotFound)
    }

    async fn create_stock(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        stock: &StoredStock,
    ) -> Result<StoredStock, RepositoryError> {
        let mut state = self.state.write().await;
        if state.stocks.contains_key(&stock.id)
            || state
                .stocks
                .values()
                .any(|(workspace, environment, existing)| {
                    *workspace == workspace_id
                        && *environment == environment_id
                        && existing.name == stock.name
                })
        {
            return Err(RepositoryError::ConcurrentStateChange);
        }
        state.stocks.insert(
            stock.id.clone(),
            (workspace_id, environment_id, stock.clone()),
        );
        Ok(stock.clone())
    }

    async fn update_stock(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        stock: &StoredStock,
    ) -> Result<StoredStock, RepositoryError> {
        let mut state = self.state.write().await;
        let (_, _, existing) = state
            .stocks
            .get_mut(&stock.id)
            .filter(|(workspace, environment, _)| {
                *workspace == workspace_id && *environment == environment_id
            })
            .ok_or(RepositoryError::NotFound)?;
        *existing = stock.clone();
        Ok(stock.clone())
    }

    async fn list_targets(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
    ) -> Result<Vec<StoredTarget>, RepositoryError> {
        let mut values = self
            .state
            .read()
            .await
            .targets
            .values()
            .filter(|(workspace, environment, _)| {
                *workspace == workspace_id && *environment == environment_id
            })
            .map(|(_, _, target)| target.clone())
            .collect::<Vec<_>>();
        values.sort_by_key(|target| (target.created_at, target.id.clone()));
        Ok(values)
    }

    async fn get_target(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        id: &str,
    ) -> Result<StoredTarget, RepositoryError> {
        self.state
            .read()
            .await
            .targets
            .get(id)
            .filter(|(workspace, environment, _)| {
                *workspace == workspace_id && *environment == environment_id
            })
            .map(|(_, _, target)| target.clone())
            .ok_or(RepositoryError::NotFound)
    }

    async fn create_target(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        target: &StoredTarget,
    ) -> Result<StoredTarget, RepositoryError> {
        let mut state = self.state.write().await;
        if let Some(stock_id) = target.stock_id.as_deref() {
            state
                .stocks
                .get(stock_id)
                .filter(|(workspace, environment, stock)| {
                    *workspace == workspace_id && *environment == environment_id && !stock.archived
                })
                .ok_or(RepositoryError::NotFound)?;
        }
        if state.targets.contains_key(&target.id)
            || state
                .targets
                .values()
                .any(|(workspace, environment, existing)| {
                    *workspace == workspace_id
                        && *environment == environment_id
                        && existing.name == target.name
                })
        {
            return Err(RepositoryError::ConcurrentStateChange);
        }
        state.targets.insert(
            target.id.clone(),
            (workspace_id, environment_id, target.clone()),
        );
        Ok(target.clone())
    }

    async fn update_target(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        target: &StoredTarget,
    ) -> Result<StoredTarget, RepositoryError> {
        let mut state = self.state.write().await;
        if let Some(stock_id) = target.stock_id.as_deref() {
            state
                .stocks
                .get(stock_id)
                .filter(|(workspace, environment, stock)| {
                    *workspace == workspace_id && *environment == environment_id && !stock.archived
                })
                .ok_or(RepositoryError::NotFound)?;
        }
        let (_, _, existing) = state
            .targets
            .get_mut(&target.id)
            .filter(|(workspace, environment, _)| {
                *workspace == workspace_id && *environment == environment_id
            })
            .ok_or(RepositoryError::NotFound)?;
        *existing = target.clone();
        Ok(target.clone())
    }

    async fn list_target_bindings(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        target_id: &str,
    ) -> Result<Vec<StoredTargetBinding>, RepositoryError> {
        self.get_target(workspace_id, environment_id, target_id)
            .await?;
        let mut values = self
            .state
            .read()
            .await
            .target_bindings
            .values()
            .filter(|(workspace, environment, binding)| {
                *workspace == workspace_id
                    && *environment == environment_id
                    && binding.target_id == target_id
            })
            .map(|(_, _, binding)| binding.clone())
            .collect::<Vec<_>>();
        values.sort_by_key(|binding| (i32::from(binding.role != "primary"), binding.created_at));
        Ok(values)
    }

    async fn create_target_binding(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        binding: &StoredTargetBinding,
    ) -> Result<StoredTargetBinding, RepositoryError> {
        let mut state = self.state.write().await;
        state
            .targets
            .get(&binding.target_id)
            .filter(|(workspace, environment, _)| {
                *workspace == workspace_id && *environment == environment_id
            })
            .ok_or(RepositoryError::NotFound)?;
        let (_, _, printer) = state
            .printers
            .get(&binding.printer_id)
            .filter(|(workspace, environment, _)| {
                *workspace == workspace_id && *environment == environment_id
            })
            .ok_or(RepositoryError::NotFound)?;
        let profile = printer
            .profiles
            .iter()
            .find(|profile| {
                (profile.profile_id.as_str(), profile.revision)
                    == (binding.profile_id.as_str(), binding.profile_revision)
            })
            .ok_or(RepositoryError::NotFound)?;
        if !profile.published || printer.agent_id != binding.agent_id {
            return Err(RepositoryError::InvalidTransition);
        }
        if state.target_bindings.contains_key(&binding.id)
            || state.target_bindings.values().any(|(_, _, existing)| {
                existing.target_id == binding.target_id && existing.role == binding.role
            })
        {
            return Err(RepositoryError::ConcurrentStateChange);
        }
        state.target_bindings.insert(
            binding.id.clone(),
            (workspace_id, environment_id, binding.clone()),
        );
        Ok(binding.clone())
    }

    async fn delete_target_binding(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        target_id: &str,
        binding_id: &str,
    ) -> Result<(), RepositoryError> {
        let mut state = self.state.write().await;
        let matches = state.target_bindings.get(binding_id).is_some_and(
            |(workspace, environment, binding)| {
                *workspace == workspace_id
                    && *environment == environment_id
                    && binding.target_id == target_id
            },
        );
        if !matches {
            return Err(RepositoryError::NotFound);
        }
        state.target_bindings.remove(binding_id);
        Ok(())
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
        let event_occurred_at = Utc::now();
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
                    event_occurred_at,
                    attempt: 0,
                },
            );
        }
        state.tenant_events.push((
            workspace_id,
            environment_id,
            StoredTenantEvent {
                id: event_id.clone(),
                event_type: event_type.into(),
                payload: payload.clone(),
            },
        ));
        Ok(event_id)
    }

    async fn list_tenant_events(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        after: Option<&str>,
        limit: i64,
    ) -> Result<Vec<StoredTenantEvent>, RepositoryError> {
        Ok(self
            .state
            .read()
            .await
            .tenant_events
            .iter()
            .filter(|(workspace, environment, event)| {
                *workspace == workspace_id
                    && *environment == environment_id
                    && after.is_none_or(|cursor| event.id.as_str() > cursor)
            })
            .map(|(_, _, event)| event.clone())
            .take(usize::try_from(limit.clamp(1, 500)).unwrap_or(500))
            .collect())
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
        after: Option<JobId>,
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
        if let Some(cursor) = after
            && let Some(position) = jobs.iter().position(|job| job.id == cursor)
        {
            jobs.drain(..=position);
        }
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

    async fn request_job_cancellation(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        job_id: JobId,
    ) -> Result<Job, RepositoryError> {
        let mut state = self.state.write().await;
        let (job, assigned_agent) = {
            let record = state
                .jobs
                .get_mut(&job_id)
                .ok_or(RepositoryError::NotFound)?;
            if record.job.workspace_id != workspace_id
                || record.job.environment_id != environment_id
            {
                return Err(RepositoryError::NotFound);
            }
            validate_transition(record.job.state, JobState::CancelRequested)
                .map_err(|_| RepositoryError::InvalidTransition)?;
            record.job.state = JobState::CancelRequested;
            record.sequence += 1;
            record.events.push(JobEvent {
                id: EventId::new(),
                job_id,
                sequence: record.sequence,
                state: JobState::CancelRequested,
                reason: None,
                message: Some("Cancellation requested by API caller".into()),
                agent_id: None,
                native_job_id: None,
                occurred_at: Utc::now(),
            });
            (record.job.clone(), record.agent_id)
        };
        state.next_agent_command_cursor = state.next_agent_command_cursor.saturating_add(1);
        let cursor = state.next_agent_command_cursor;
        state
            .agent_commands
            .entry(assigned_agent)
            .or_default()
            .push(MemoryAgentCommand {
                cursor,
                command: AgentCommand::CancelJob { job_id },
                delivered: false,
                acknowledged: false,
            });
        Ok(job)
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

    async fn validate_agent_lease(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        job_id: JobId,
        lease_id: Uuid,
        lease_token: &str,
    ) -> Result<(), RepositoryError> {
        let state = self.state.read().await;
        let record = state.jobs.get(&job_id).ok_or(RepositoryError::NotFound)?;
        let valid_tenant =
            record.job.workspace_id == workspace_id && record.job.environment_id == environment_id;
        let valid_lease = state.leases.get(&job_id).is_some_and(
            |(stored_agent, stored_id, stored_token, expiry)| {
                *stored_agent == agent_id
                    && *stored_id == lease_id
                    && stored_token == lease_token
                    && *expiry > Utc::now()
            },
        );
        if !valid_tenant || !valid_lease {
            return Err(RepositoryError::ConcurrentStateChange);
        }
        Ok(())
    }

    async fn apply_agent_event(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        event: &JobEvent,
    ) -> Result<Option<Job>, RepositoryError> {
        let mut state = self.state.write().await;
        let receipt = (agent_id, event.id);
        if state.agent_event_receipts.contains(&receipt) {
            return Ok(None);
        }
        let record = state
            .jobs
            .get_mut(&event.job_id)
            .filter(|record| {
                record.job.workspace_id == workspace_id
                    && record.job.environment_id == environment_id
                    && record.agent_id == agent_id
            })
            .ok_or(RepositoryError::NotFound)?;
        validate_transition(record.job.state, event.state)
            .map_err(|_| RepositoryError::InvalidTransition)?;
        record.sequence += 1;
        record.job.state = event.state;
        let mut stored = event.clone();
        stored.sequence = record.sequence;
        stored.agent_id = Some(agent_id);
        record.events.push(stored);
        let job = record.job.clone();
        state.agent_event_receipts.insert(receipt);
        Ok(Some(job))
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
        {
            let mut state = self.state.write().await;
            let job_is_accepted = state.jobs.get(&job_id).is_some_and(|record| {
                record.job.workspace_id == workspace_id
                    && record.job.environment_id == environment_id
                    && record.job.state == JobState::AgentAccepted
            });
            if let Some(acceptance) = state.job_acceptances.get_mut(&job_id) {
                if acceptance.agent_id != agent_id
                    || acceptance.lease_id != lease_id
                    || acceptance.content_sha256.as_deref() != content_sha256
                    || acceptance.local_sequence != local_sequence
                {
                    return Err(RepositoryError::IdempotencyConflict);
                }
                match acceptance.lease_token.as_deref() {
                    Some(stored) if stored == lease_token => {}
                    None if job_is_accepted => {
                        acceptance.lease_token = Some(lease_token.to_owned());
                    }
                    Some(_) | None => return Err(RepositoryError::IdempotencyConflict),
                }
                return state
                    .jobs
                    .get(&job_id)
                    .map(|record| record.job.clone())
                    .ok_or(RepositoryError::NotFound);
            }
        }
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
        let job = record.job.clone();
        state.job_acceptances.insert(
            job_id,
            MemoryJobAcceptance {
                agent_id,
                lease_id,
                lease_token: Some(lease_token.to_owned()),
                content_sha256: content_sha256.map(str::to_owned),
                local_sequence,
            },
        );
        Ok(job)
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

#[cfg(test)]
mod routing_repository_tests {
    use super::*;

    fn target(id: &str, name: &str) -> StoredTarget {
        let now = Utc::now();
        StoredTarget {
            id: id.into(),
            name: name.into(),
            description: None,
            stock_id: None,
            enabled: true,
            routing_policy: "primary_then_standby".into(),
            created_at: now,
            updated_at: now,
        }
    }

    #[tokio::test]
    async fn memory_targets_reject_duplicate_tenant_names() {
        let repository = MemoryRepository::default();
        let workspace_id = WorkspaceId::new();
        let environment_id = EnvironmentId::new();
        assert!(
            repository
                .create_target(
                    workspace_id,
                    environment_id,
                    &target("tgt_first", "Shipping labels"),
                )
                .await
                .is_ok()
        );

        let error = repository
            .create_target(
                workspace_id,
                environment_id,
                &target("tgt_second", "Shipping labels"),
            )
            .await;
        assert!(matches!(error, Err(RepositoryError::ConcurrentStateChange)));
    }

    #[tokio::test]
    async fn direct_printer_lookup_is_tenant_scoped() {
        let repository = MemoryRepository::default();
        let workspace_id = WorkspaceId::new();
        let environment_id = EnvironmentId::new();
        let printer_id = PrinterId::new();
        repository
            .add_printer(workspace_id, environment_id, printer_id, AgentId::new())
            .await;

        assert!(matches!(
            repository
                .get_printer(workspace_id, environment_id, printer_id)
                .await,
            Ok(printer) if printer.id == printer_id
        ));
        assert!(matches!(
            repository
                .get_printer(WorkspaceId::new(), environment_id, printer_id)
                .await,
            Err(RepositoryError::NotFound)
        ));
    }
}
