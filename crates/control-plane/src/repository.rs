#![allow(
    clippy::significant_drop_tightening,
    clippy::too_many_arguments,
    clippy::use_self
)]

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use piqae_domain::{
    AgentId, EnvironmentId, EventId, Job, JobEvent, JobFailureReason, JobId, JobState,
    PrinterCapabilities, PrinterId, PrinterState, WorkspaceId, validate_transition,
};
use piqae_protocol::agent::AgentCommand;
use piqae_storage_postgres::{
    AgentAuthenticationRecord, CreateJobResult as PgCreateJobResult, EnrolledAgent, JobLease,
    NewDeviceAuthorization, NodeUpdatePolicy, NodeUpdateState, PostgresStore, StorageError,
    StoredAgent, StoredAgentCommandBatch, StoredApiKey, StoredBillingSummary,
    StoredConnectSessionPreview, StoredContentEncryptionKey, StoredDeviceAuthorization,
    StoredNodeConnector, StoredNodeDiagnostic, StoredNodeUpdate, StoredPlatformAccount,
    StoredPrinter, StoredStock, StoredTarget, StoredTargetBinding, StoredTenantEvent, StoredUpload,
    StoredUsageSummary, StoredWebhook, StoredWebhookDelivery, StripeBillingEvent,
    StripeProjectionResult, SyncedPrinter, UpsertedPlatformAccount, WebhookDeliveryWork,
    WorkOsIdentityEvent, WorkOsProjectionResult,
};
use sha2::Digest as _;
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
    #[error("cloud free-plan quota exceeded")]
    QuotaExceeded,
    #[error("cloud billing blocks new jobs")]
    BillingBlocked,
    #[error("cloud node quota exceeded")]
    NodeQuotaExceeded,
    #[error("platform mode is already enabled")]
    PlatformAlreadyEnabled,
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
            StorageError::QuotaExceeded => Self::QuotaExceeded,
            StorageError::BillingBlocked => Self::BillingBlocked,
            StorageError::NodeQuotaExceeded => Self::NodeQuotaExceeded,
            StorageError::PlatformAlreadyEnabled => Self::PlatformAlreadyEnabled,
            other => Self::Persistence(other.to_string()),
        }
    }
}

#[async_trait]
pub trait Repository: Send + Sync + 'static {
    async fn ready(&self) -> Result<(), RepositoryError>;
    async fn has_platform_manager(
        &self,
        _owner_workspace_id: WorkspaceId,
    ) -> Result<bool, RepositoryError> {
        Ok(false)
    }
    async fn enable_platform_manager(
        &self,
        _id: &str,
        _name: &str,
        _secret_hash: &str,
        _owner_workspace_id: WorkspaceId,
        _request_id: &str,
    ) -> Result<(), RepositoryError> {
        Err(RepositoryError::NotFound)
    }
    async fn list_platform_accounts(
        &self,
        _service_account_id: &str,
    ) -> Result<Vec<StoredPlatformAccount>, RepositoryError> {
        Err(RepositoryError::NotFound)
    }
    async fn get_platform_account(
        &self,
        _service_account_id: &str,
        _external_id: &str,
    ) -> Result<StoredPlatformAccount, RepositoryError> {
        Err(RepositoryError::NotFound)
    }
    async fn upsert_platform_account(
        &self,
        _service_account_id: &str,
        _external_id: &str,
        _name: &str,
        _metadata: &std::collections::BTreeMap<String, String>,
        _request_id: &str,
    ) -> Result<UpsertedPlatformAccount, RepositoryError> {
        Err(RepositoryError::NotFound)
    }
    async fn archive_platform_account(
        &self,
        _service_account_id: &str,
        _external_id: &str,
        _request_id: &str,
    ) -> Result<(), RepositoryError> {
        Err(RepositoryError::NotFound)
    }
    async fn usage_summary(
        &self,
        _workspace_id: WorkspaceId,
        _period_start: DateTime<Utc>,
        _period_end: DateTime<Utc>,
    ) -> Result<StoredUsageSummary, RepositoryError> {
        Err(RepositoryError::NotFound)
    }
    async fn billing_summary(
        &self,
        _workspace_id: WorkspaceId,
        _period_start: DateTime<Utc>,
        _period_end: DateTime<Utc>,
    ) -> Result<StoredBillingSummary, RepositoryError> {
        Err(RepositoryError::NotFound)
    }
    async fn project_stripe_billing_event(
        &self,
        _event: &StripeBillingEvent,
        _request_id: &str,
    ) -> Result<StripeProjectionResult, RepositoryError> {
        Err(RepositoryError::NotFound)
    }
    async fn project_workos_identity_event(
        &self,
        _event: &WorkOsIdentityEvent,
    ) -> Result<WorkOsProjectionResult, RepositoryError> {
        Err(RepositoryError::NotFound)
    }
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
        health: &piqae_protocol::agent::AgentHealth,
        printers: Option<&[SyncedPrinter]>,
    ) -> Result<(), RepositoryError>;
    async fn create_node_diagnostic(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        request_id: &str,
    ) -> Result<StoredNodeDiagnostic, RepositoryError>;
    async fn store_node_diagnostic(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        report: &piqae_protocol::agent::DiagnosticReport,
    ) -> Result<(), RepositoryError>;
    async fn list_node_diagnostics(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
    ) -> Result<Vec<StoredNodeDiagnostic>, RepositoryError>;
    async fn get_node_diagnostic(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        request_id: &str,
    ) -> Result<StoredNodeDiagnostic, RepositoryError>;
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
    async fn get_agent(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
    ) -> Result<StoredAgent, RepositoryError>;
    async fn rename_agent(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        name: &str,
    ) -> Result<StoredAgent, RepositoryError>;
    async fn revoke_agent(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
    ) -> Result<(), RepositoryError>;
    async fn list_node_connectors(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
    ) -> Result<Vec<StoredNodeConnector>, RepositoryError>;
    async fn revoke_node_connector(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        connector_id: &str,
    ) -> Result<(), RepositoryError>;
    async fn get_node_update(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        node_id: AgentId,
        default_mode: &str,
    ) -> Result<StoredNodeUpdate, RepositoryError>;
    async fn update_node_policy(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        node_id: AgentId,
        policy: &NodeUpdatePolicy,
        default_mode: &str,
    ) -> Result<StoredNodeUpdate, RepositoryError>;
    async fn request_node_update(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        node_id: AgentId,
        version: &str,
        default_mode: &str,
    ) -> Result<StoredNodeUpdate, RepositoryError>;
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
    async fn create_connect_enrolment(
        &self,
        id: &str,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        secret_hash: &str,
        expires_at: DateTime<Utc>,
        return_url: Option<&str>,
        requesting_service_account_id: Option<&str>,
    ) -> Result<(), RepositoryError> {
        let _ = (return_url, requesting_service_account_id);
        self.create_enrolment(id, workspace_id, environment_id, secret_hash, expires_at)
            .await
    }
    async fn enrolment_status(
        &self,
        id: &str,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
    ) -> Result<(DateTime<Utc>, Option<AgentId>), RepositoryError>;
    async fn connect_session_preview(
        &self,
        secret_hash: &str,
    ) -> Result<StoredConnectSessionPreview, RepositoryError>;
    async fn node_installation_public_key(
        &self,
        _installation_key: &str,
    ) -> Result<Vec<u8>, RepositoryError> {
        Err(RepositoryError::NotFound)
    }
    async fn create_device_authorization(
        &self,
        authorization: &NewDeviceAuthorization<'_>,
    ) -> Result<StoredDeviceAuthorization, RepositoryError>;
    async fn device_authorization_by_hash(
        &self,
        device_code_hash: &str,
    ) -> Result<StoredDeviceAuthorization, RepositoryError>;
    async fn device_authorization_by_id(
        &self,
        id: &str,
    ) -> Result<StoredDeviceAuthorization, RepositoryError>;
    /// The node this pairing request would rebind, if it would rebind one.
    ///
    /// A pairing request reusing an installation ID replaces an existing node's
    /// device key rather than admitting a new node — that is how in-place key
    /// rotation works. An approver must be able to see which of the two they
    /// are being asked to authorize.
    async fn node_replaced_by_device_authorization(
        &self,
        id: &str,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
    ) -> Result<Option<AgentId>, RepositoryError>;
    async fn approve_device_authorization(
        &self,
        id: &str,
        user_code_hash: &str,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        approved_by: &str,
    ) -> Result<StoredDeviceAuthorization, RepositoryError>;
    async fn deny_device_authorization(
        &self,
        id: &str,
        user_code_hash: &str,
    ) -> Result<StoredDeviceAuthorization, RepositoryError>;
    async fn exchange_device_authorization(
        &self,
        device_code_hash: &str,
    ) -> Result<EnrolledAgent, RepositoryError>;
    async fn exchange_device_authorization_with_billing(
        &self,
        device_code_hash: &str,
        _enforce_cloud_billing: bool,
    ) -> Result<EnrolledAgent, RepositoryError> {
        self.exchange_device_authorization(device_code_hash).await
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
    ) -> Result<EnrolledAgent, RepositoryError>;
    #[allow(clippy::too_many_arguments)]
    async fn enrol_agent_with_billing(
        &self,
        secret_hash: &str,
        public_key: &[u8],
        name: &str,
        hostname: &str,
        platform: &str,
        architecture: &str,
        version: &str,
        protocol_version: u16,
        _enforce_cloud_billing: bool,
    ) -> Result<EnrolledAgent, RepositoryError> {
        self.enrol_agent(
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
    }
    #[allow(clippy::too_many_arguments)]
    async fn enrol_agent_connector_with_billing(
        &self,
        secret_hash: &str,
        public_key: &[u8],
        name: &str,
        hostname: &str,
        platform: &str,
        architecture: &str,
        version: &str,
        protocol_version: u16,
        enforce_cloud_billing: bool,
        _installation_id: &str,
        _printer_grant: &str,
        _allowed_printer_ids: &[String],
    ) -> Result<EnrolledAgent, RepositoryError> {
        self.enrol_agent_with_billing(
            secret_hash,
            public_key,
            name,
            hostname,
            platform,
            architecture,
            version,
            protocol_version,
            enforce_cloud_billing,
        )
        .await
    }
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
    async fn delete_upload(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        upload_id: &str,
    ) -> Result<(), RepositoryError>;
    async fn resolve_printer_agent(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        printer_id: PrinterId,
    ) -> Result<AgentId, RepositoryError>;
    async fn rotate_content_encryption_key(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        key_id: &str,
        algorithm: &str,
        public_key_spki: &str,
    ) -> Result<StoredContentEncryptionKey, RepositoryError>;
    async fn content_encryption_key_for_printer(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        printer_id: PrinterId,
    ) -> Result<StoredContentEncryptionKey, RepositoryError>;
    async fn revoke_content_encryption_key(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        key_id: &str,
    ) -> Result<(), RepositoryError>;
    async fn create_job(
        &self,
        job: &Job,
        agent_id: AgentId,
        idempotency_key: Option<&str>,
        request_bytes: &[u8],
    ) -> Result<CreateResult, RepositoryError>;
    async fn create_cloud_job(
        &self,
        job: &Job,
        agent_id: AgentId,
        idempotency_key: Option<&str>,
        request_bytes: &[u8],
        _enforce_cloud_billing: bool,
    ) -> Result<CreateResult, RepositoryError> {
        self.create_job(job, agent_id, idempotency_key, request_bytes)
            .await
    }

    #[allow(
        clippy::too_many_lines,
        reason = "atomic in-memory parity keeps every reroute fence under one write lock"
    )]
    async fn reroute_job_before_acceptance(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        job_id: JobId,
        target_id: &str,
        binding: &StoredTargetBinding,
        reason: &str,
    ) -> Result<Option<Job>, RepositoryError>;
    async fn list_reroutable_target_jobs(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        limit: i64,
    ) -> Result<Vec<Job>, RepositoryError>;
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

    async fn has_platform_manager(
        &self,
        owner_workspace_id: WorkspaceId,
    ) -> Result<bool, RepositoryError> {
        self.has_platform_manager_for_owner_workspace(owner_workspace_id)
            .await
            .map_err(Into::into)
    }

    async fn enable_platform_manager(
        &self,
        id: &str,
        name: &str,
        secret_hash: &str,
        owner_workspace_id: WorkspaceId,
        request_id: &str,
    ) -> Result<(), RepositoryError> {
        self.enable_platform_service_account(id, name, secret_hash, owner_workspace_id, request_id)
            .await
            .map_err(Into::into)
    }

    async fn list_platform_accounts(
        &self,
        service_account_id: &str,
    ) -> Result<Vec<StoredPlatformAccount>, RepositoryError> {
        PostgresStore::list_platform_accounts(self, service_account_id)
            .await
            .map_err(Into::into)
    }

    async fn get_platform_account(
        &self,
        service_account_id: &str,
        external_id: &str,
    ) -> Result<StoredPlatformAccount, RepositoryError> {
        PostgresStore::get_platform_account(self, service_account_id, external_id)
            .await
            .map_err(Into::into)
    }

    async fn upsert_platform_account(
        &self,
        service_account_id: &str,
        external_id: &str,
        name: &str,
        metadata: &std::collections::BTreeMap<String, String>,
        request_id: &str,
    ) -> Result<UpsertedPlatformAccount, RepositoryError> {
        PostgresStore::upsert_platform_account(
            self,
            service_account_id,
            external_id,
            name,
            metadata,
            request_id,
        )
        .await
        .map_err(Into::into)
    }

    async fn create_node_diagnostic(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        request_id: &str,
    ) -> Result<StoredNodeDiagnostic, RepositoryError> {
        Self::create_node_diagnostic(self, workspace_id, environment_id, agent_id, request_id)
            .await
            .map_err(Into::into)
    }

    async fn store_node_diagnostic(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        report: &piqae_protocol::agent::DiagnosticReport,
    ) -> Result<(), RepositoryError> {
        Self::store_node_diagnostic(self, workspace_id, environment_id, agent_id, report)
            .await
            .map_err(Into::into)
    }

    async fn list_node_diagnostics(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
    ) -> Result<Vec<StoredNodeDiagnostic>, RepositoryError> {
        Self::list_node_diagnostics(self, workspace_id, environment_id, agent_id)
            .await
            .map_err(Into::into)
    }

    async fn get_node_diagnostic(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        request_id: &str,
    ) -> Result<StoredNodeDiagnostic, RepositoryError> {
        Self::get_node_diagnostic(self, workspace_id, environment_id, agent_id, request_id)
            .await
            .map_err(Into::into)
    }

    async fn archive_platform_account(
        &self,
        service_account_id: &str,
        external_id: &str,
        request_id: &str,
    ) -> Result<(), RepositoryError> {
        PostgresStore::archive_platform_account(self, service_account_id, external_id, request_id)
            .await
            .map_err(Into::into)
    }

    async fn usage_summary(
        &self,
        workspace_id: WorkspaceId,
        period_start: DateTime<Utc>,
        period_end: DateTime<Utc>,
    ) -> Result<StoredUsageSummary, RepositoryError> {
        PostgresStore::usage_summary(self, workspace_id, period_start, period_end)
            .await
            .map_err(Into::into)
    }

    async fn billing_summary(
        &self,
        workspace_id: WorkspaceId,
        period_start: DateTime<Utc>,
        period_end: DateTime<Utc>,
    ) -> Result<StoredBillingSummary, RepositoryError> {
        PostgresStore::billing_summary(self, workspace_id, period_start, period_end)
            .await
            .map_err(Into::into)
    }

    async fn project_stripe_billing_event(
        &self,
        event: &StripeBillingEvent,
        request_id: &str,
    ) -> Result<StripeProjectionResult, RepositoryError> {
        PostgresStore::project_stripe_billing_event(self, event, request_id)
            .await
            .map_err(Into::into)
    }

    async fn project_workos_identity_event(
        &self,
        event: &WorkOsIdentityEvent,
    ) -> Result<WorkOsProjectionResult, RepositoryError> {
        PostgresStore::project_workos_identity_event(self, event)
            .await
            .map_err(Into::into)
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
        health: &piqae_protocol::agent::AgentHealth,
        printers: Option<&[SyncedPrinter]>,
    ) -> Result<(), RepositoryError> {
        Self::sync_agent_presence(
            self,
            workspace_id,
            environment_id,
            agent_id,
            version,
            health,
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

    async fn get_agent(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
    ) -> Result<StoredAgent, RepositoryError> {
        Self::get_agent(self, workspace_id, environment_id, agent_id)
            .await
            .map_err(Into::into)
    }

    async fn rename_agent(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        name: &str,
    ) -> Result<StoredAgent, RepositoryError> {
        Self::rename_agent(self, workspace_id, environment_id, agent_id, name)
            .await
            .map_err(Into::into)
    }

    async fn revoke_agent(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
    ) -> Result<(), RepositoryError> {
        Self::revoke_agent(self, workspace_id, environment_id, agent_id)
            .await
            .map_err(Into::into)
    }

    async fn list_node_connectors(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
    ) -> Result<Vec<StoredNodeConnector>, RepositoryError> {
        Self::list_node_connectors(self, workspace_id, environment_id, agent_id)
            .await
            .map_err(Into::into)
    }

    async fn revoke_node_connector(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        connector_id: &str,
    ) -> Result<(), RepositoryError> {
        Self::revoke_node_connector(self, workspace_id, environment_id, agent_id, connector_id)
            .await
            .map_err(Into::into)
    }

    async fn get_node_update(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        node_id: AgentId,
        default_mode: &str,
    ) -> Result<StoredNodeUpdate, RepositoryError> {
        Self::get_node_update(self, workspace_id, environment_id, node_id, default_mode)
            .await
            .map_err(Into::into)
    }

    async fn update_node_policy(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        node_id: AgentId,
        policy: &NodeUpdatePolicy,
        default_mode: &str,
    ) -> Result<StoredNodeUpdate, RepositoryError> {
        Self::update_node_policy(
            self,
            workspace_id,
            environment_id,
            node_id,
            policy,
            default_mode,
        )
        .await
        .map_err(Into::into)
    }

    async fn request_node_update(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        node_id: AgentId,
        version: &str,
        default_mode: &str,
    ) -> Result<StoredNodeUpdate, RepositoryError> {
        Self::request_node_update(
            self,
            workspace_id,
            environment_id,
            node_id,
            version,
            default_mode,
        )
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

    async fn create_connect_enrolment(
        &self,
        id: &str,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        secret_hash: &str,
        expires_at: DateTime<Utc>,
        return_url: Option<&str>,
        requesting_service_account_id: Option<&str>,
    ) -> Result<(), RepositoryError> {
        Self::create_connect_enrolment(
            self,
            id,
            workspace_id,
            environment_id,
            secret_hash,
            expires_at,
            return_url,
            requesting_service_account_id,
        )
        .await
        .map_err(Into::into)
    }

    async fn enrolment_status(
        &self,
        id: &str,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
    ) -> Result<(DateTime<Utc>, Option<AgentId>), RepositoryError> {
        Self::enrolment_status(self, id, workspace_id, environment_id)
            .await
            .map_err(Into::into)
    }

    async fn connect_session_preview(
        &self,
        secret_hash: &str,
    ) -> Result<StoredConnectSessionPreview, RepositoryError> {
        Self::connect_session_preview(self, secret_hash)
            .await
            .map_err(Into::into)
    }

    async fn node_installation_public_key(
        &self,
        installation_key: &str,
    ) -> Result<Vec<u8>, RepositoryError> {
        Self::node_installation_public_key(self, installation_key)
            .await
            .map_err(Into::into)
    }

    async fn create_device_authorization(
        &self,
        authorization: &NewDeviceAuthorization<'_>,
    ) -> Result<StoredDeviceAuthorization, RepositoryError> {
        Self::create_device_authorization(self, authorization)
            .await
            .map_err(Into::into)
    }

    async fn device_authorization_by_hash(
        &self,
        device_code_hash: &str,
    ) -> Result<StoredDeviceAuthorization, RepositoryError> {
        Self::device_authorization_by_hash(self, device_code_hash)
            .await
            .map_err(Into::into)
    }

    async fn device_authorization_by_id(
        &self,
        id: &str,
    ) -> Result<StoredDeviceAuthorization, RepositoryError> {
        Self::device_authorization_by_id(self, id)
            .await
            .map_err(Into::into)
    }

    async fn node_replaced_by_device_authorization(
        &self,
        id: &str,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
    ) -> Result<Option<AgentId>, RepositoryError> {
        Self::node_replaced_by_device_authorization(self, id, workspace_id, environment_id)
            .await
            .map_err(Into::into)
    }

    async fn approve_device_authorization(
        &self,
        id: &str,
        user_code_hash: &str,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        approved_by: &str,
    ) -> Result<StoredDeviceAuthorization, RepositoryError> {
        Self::approve_device_authorization(
            self,
            id,
            user_code_hash,
            workspace_id,
            environment_id,
            approved_by,
        )
        .await
        .map_err(Into::into)
    }

    async fn deny_device_authorization(
        &self,
        id: &str,
        user_code_hash: &str,
    ) -> Result<StoredDeviceAuthorization, RepositoryError> {
        Self::deny_device_authorization(self, id, user_code_hash)
            .await
            .map_err(Into::into)
    }

    async fn exchange_device_authorization(
        &self,
        device_code_hash: &str,
    ) -> Result<EnrolledAgent, RepositoryError> {
        Self::exchange_device_authorization(self, device_code_hash)
            .await
            .map_err(Into::into)
    }

    async fn exchange_device_authorization_with_billing(
        &self,
        device_code_hash: &str,
        enforce_cloud_billing: bool,
    ) -> Result<EnrolledAgent, RepositoryError> {
        Self::exchange_device_authorization_with_billing(
            self,
            device_code_hash,
            enforce_cloud_billing,
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

    async fn rotate_content_encryption_key(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        key_id: &str,
        algorithm: &str,
        public_key_spki: &str,
    ) -> Result<StoredContentEncryptionKey, RepositoryError> {
        PostgresStore::rotate_content_encryption_key(
            self,
            workspace_id,
            environment_id,
            agent_id,
            key_id,
            algorithm,
            public_key_spki,
        )
        .await
        .map_err(Into::into)
    }
    async fn content_encryption_key_for_printer(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        printer_id: PrinterId,
    ) -> Result<StoredContentEncryptionKey, RepositoryError> {
        PostgresStore::content_encryption_key_for_printer(
            self,
            workspace_id,
            environment_id,
            printer_id,
        )
        .await
        .map_err(Into::into)
    }
    async fn revoke_content_encryption_key(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        key_id: &str,
    ) -> Result<(), RepositoryError> {
        PostgresStore::revoke_content_encryption_key(
            self,
            workspace_id,
            environment_id,
            agent_id,
            key_id,
        )
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

    async fn enrol_agent_with_billing(
        &self,
        secret_hash: &str,
        public_key: &[u8],
        name: &str,
        hostname: &str,
        platform: &str,
        architecture: &str,
        version: &str,
        protocol_version: u16,
        enforce_cloud_billing: bool,
    ) -> Result<EnrolledAgent, RepositoryError> {
        Self::enrol_agent_with_billing(
            self,
            secret_hash,
            public_key,
            name,
            hostname,
            platform,
            architecture,
            version,
            protocol_version,
            enforce_cloud_billing,
        )
        .await
        .map_err(Into::into)
    }

    async fn enrol_agent_connector_with_billing(
        &self,
        secret_hash: &str,
        public_key: &[u8],
        name: &str,
        hostname: &str,
        platform: &str,
        architecture: &str,
        version: &str,
        protocol_version: u16,
        enforce_cloud_billing: bool,
        installation_id: &str,
        printer_grant: &str,
        allowed_printer_ids: &[String],
    ) -> Result<EnrolledAgent, RepositoryError> {
        Self::enrol_agent_connector_with_billing(
            self,
            secret_hash,
            public_key,
            name,
            hostname,
            platform,
            architecture,
            version,
            protocol_version,
            enforce_cloud_billing,
            Some(installation_id),
            printer_grant,
            allowed_printer_ids,
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

    async fn delete_upload(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        upload_id: &str,
    ) -> Result<(), RepositoryError> {
        PostgresStore::delete_upload(self, workspace_id, environment_id, upload_id)
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

    async fn create_cloud_job(
        &self,
        job: &Job,
        agent_id: AgentId,
        idempotency_key: Option<&str>,
        request_bytes: &[u8],
        enforce_cloud_billing: bool,
    ) -> Result<CreateResult, RepositoryError> {
        match PostgresStore::create_cloud_job(
            self,
            job,
            agent_id,
            idempotency_key,
            request_bytes,
            enforce_cloud_billing,
        )
        .await?
        {
            PgCreateJobResult::Created(job) => Ok(CreateResult::Created(job)),
            PgCreateJobResult::Existing(job) => Ok(CreateResult::Existing(job)),
        }
    }

    async fn reroute_job_before_acceptance(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        job_id: JobId,
        target_id: &str,
        binding: &StoredTargetBinding,
        reason: &str,
    ) -> Result<Option<Job>, RepositoryError> {
        PostgresStore::reroute_job_before_acceptance(
            self,
            workspace_id,
            environment_id,
            job_id,
            target_id,
            binding,
            reason,
        )
        .await
        .map_err(Into::into)
    }

    async fn list_reroutable_target_jobs(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        limit: i64,
    ) -> Result<Vec<Job>, RepositoryError> {
        PostgresStore::list_reroutable_target_jobs(self, workspace_id, environment_id, limit)
            .await
            .map_err(Into::into)
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

#[derive(Clone, Debug)]
struct MemoryDeviceAuthorization {
    record: StoredDeviceAuthorization,
    device_code_hash: String,
    user_code_hash: String,
    public_key: Vec<u8>,
    agent_version: String,
    installation_id: String,
}

/// Workspace, environment, installation identifier, expiry, and the node whose
/// device key the token rebinds, when it is a rotation rather than a new node.
type MemoryEnrolment = (
    WorkspaceId,
    EnvironmentId,
    String,
    DateTime<Utc>,
    Option<AgentId>,
);

#[derive(Debug, Default)]
struct MemoryState {
    platform_managers: HashMap<WorkspaceId, String>,
    api_keys: HashMap<String, (WorkspaceId, EnvironmentId, StoredApiKey, String)>,
    jobs: HashMap<JobId, MemoryJob>,
    printers: HashMap<PrinterId, (WorkspaceId, EnvironmentId, StoredPrinter)>,
    stocks: HashMap<String, (WorkspaceId, EnvironmentId, StoredStock)>,
    targets: HashMap<String, (WorkspaceId, EnvironmentId, StoredTarget)>,
    target_bindings: HashMap<String, (WorkspaceId, EnvironmentId, StoredTargetBinding)>,
    agents: HashMap<AgentId, (WorkspaceId, EnvironmentId, StoredAgent)>,
    /// Which node owns each installation, so pairing rebinds an existing node
    /// instead of admitting a duplicate — matching the `PostgreSQL` behaviour
    /// that in-place key rotation depends on.
    agent_installations: HashMap<(WorkspaceId, EnvironmentId, String), AgentId>,
    agent_public_keys: HashMap<AgentId, Vec<u8>>,
    content_encryption_keys: HashMap<AgentId, StoredContentEncryptionKey>,
    node_connectors: HashMap<(WorkspaceId, EnvironmentId, AgentId, String), StoredNodeConnector>,
    enrolments: HashMap<String, MemoryEnrolment>,
    device_authorizations: HashMap<String, MemoryDeviceAuthorization>,
    node_updates: HashMap<AgentId, StoredNodeUpdate>,
    webhooks: HashMap<String, (WorkspaceId, EnvironmentId, StoredWebhook, Vec<u8>)>,
    webhook_deliveries: HashMap<String, (WorkspaceId, EnvironmentId, StoredWebhookDelivery)>,
    webhook_work: HashMap<String, WebhookDeliveryWork>,
    tenant_events: Vec<(WorkspaceId, EnvironmentId, StoredTenantEvent)>,
    uploads: HashMap<String, (WorkspaceId, EnvironmentId, StoredUpload)>,
    agent_nonces: HashMap<(AgentId, String), DateTime<Utc>>,
    agent_event_receipts: HashSet<(AgentId, EventId)>,
    agent_commands: HashMap<AgentId, Vec<MemoryAgentCommand>>,
    node_diagnostics: HashMap<String, (WorkspaceId, EnvironmentId, StoredNodeDiagnostic)>,
    next_agent_command_cursor: u64,
    leases: HashMap<JobId, (AgentId, Uuid, String, DateTime<Utc>)>,
    job_acceptances: HashMap<JobId, MemoryJobAcceptance>,
    routing_attempts: Vec<(JobId, String, String)>,
    idempotency: HashMap<(WorkspaceId, EnvironmentId, String), (Vec<u8>, JobId)>,
    consumed_envelopes: HashMap<(WorkspaceId, EnvironmentId, String), (String, JobId)>,
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
                    health_started_at: None,
                    health_observed_at: None,
                    sqlite_integrity_ok: None,
                    executor_crashes: 0,
                    last_error_code: None,
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

    #[cfg(test)]
    pub async fn set_agent_offline(&self, agent_id: AgentId) {
        if let Some((_, _, agent)) = self.state.write().await.agents.get_mut(&agent_id) {
            agent.state = "offline".into();
            agent.last_seen_at = Utc::now() - chrono::Duration::minutes(5);
        }
    }
}

#[async_trait]
impl Repository for MemoryRepository {
    async fn ready(&self) -> Result<(), RepositoryError> {
        Ok(())
    }

    async fn has_platform_manager(
        &self,
        owner_workspace_id: WorkspaceId,
    ) -> Result<bool, RepositoryError> {
        Ok(self
            .state
            .read()
            .await
            .platform_managers
            .contains_key(&owner_workspace_id))
    }

    async fn enable_platform_manager(
        &self,
        id: &str,
        _name: &str,
        _secret_hash: &str,
        owner_workspace_id: WorkspaceId,
        _request_id: &str,
    ) -> Result<(), RepositoryError> {
        let mut state = self.state.write().await;
        if state.platform_managers.contains_key(&owner_workspace_id) {
            return Err(RepositoryError::PlatformAlreadyEnabled);
        }
        state
            .platform_managers
            .insert(owner_workspace_id, id.to_owned());
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

    async fn node_installation_public_key(
        &self,
        installation_key: &str,
    ) -> Result<Vec<u8>, RepositoryError> {
        let state = self.state.read().await;
        let agent_id = state
            .agent_installations
            .iter()
            .find(|((_, _, key), _)| key == installation_key)
            .map(|(_, agent_id)| *agent_id)
            .ok_or(RepositoryError::NotFound)?;
        state
            .agent_public_keys
            .get(&agent_id)
            .cloned()
            .ok_or(RepositoryError::NotFound)
    }

    async fn connect_session_preview(
        &self,
        secret_hash: &str,
    ) -> Result<StoredConnectSessionPreview, RepositoryError> {
        self.state
            .read()
            .await
            .enrolments
            .values()
            .find(|(_, _, hash, expires_at, agent_id)| {
                hash == secret_hash && *expires_at > Utc::now() && agent_id.is_none()
            })
            .map(
                |(workspace_id, environment_id, _, expires_at, _)| StoredConnectSessionPreview {
                    workspace_id: *workspace_id,
                    workspace_name: format!("Piqae workspace {workspace_id}"),
                    environment_id: *environment_id,
                    expires_at: *expires_at,
                    requesting_service_account_id: None,
                    requesting_service_name: None,
                    return_url: None,
                },
            )
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
        health: &piqae_protocol::agent::AgentHealth,
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
        agent.health_started_at = Some(health.started_at);
        agent.health_observed_at = Some(health.observed_at);
        agent.sqlite_integrity_ok = Some(health.sqlite_integrity_ok);
        agent.executor_crashes = health.executor_crashes;
        agent.last_error_code.clone_from(&health.last_error_code);
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

    async fn create_node_diagnostic(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        request_id: &str,
    ) -> Result<StoredNodeDiagnostic, RepositoryError> {
        let mut state = self.state.write().await;
        state
            .agents
            .get(&agent_id)
            .filter(|(workspace, environment, _)| {
                *workspace == workspace_id && *environment == environment_id
            })
            .ok_or(RepositoryError::NotFound)?;
        let requested_at = Utc::now();
        let diagnostic = StoredNodeDiagnostic {
            request_id: request_id.into(),
            node_id: agent_id,
            state: "requested".into(),
            report: None,
            requested_at,
            received_at: None,
            expires_at: requested_at + chrono::TimeDelta::days(14),
        };
        state.node_diagnostics.insert(
            request_id.into(),
            (workspace_id, environment_id, diagnostic.clone()),
        );
        Ok(diagnostic)
    }

    async fn store_node_diagnostic(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        report: &piqae_protocol::agent::DiagnosticReport,
    ) -> Result<(), RepositoryError> {
        let mut state = self.state.write().await;
        let (_, _, diagnostic) = state
            .node_diagnostics
            .get_mut(&report.request_id)
            .filter(|(workspace, environment, diagnostic)| {
                *workspace == workspace_id
                    && *environment == environment_id
                    && diagnostic.node_id == agent_id
            })
            .ok_or(RepositoryError::NotFound)?;
        diagnostic.state.clone_from(&report.state);
        diagnostic.report = Some(
            serde_json::to_value(report)
                .map_err(|error| RepositoryError::Persistence(error.to_string()))?,
        );
        diagnostic.received_at = Some(Utc::now());
        Ok(())
    }

    async fn list_node_diagnostics(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
    ) -> Result<Vec<StoredNodeDiagnostic>, RepositoryError> {
        let mut reports = self
            .state
            .read()
            .await
            .node_diagnostics
            .values()
            .filter(|(workspace, environment, diagnostic)| {
                *workspace == workspace_id
                    && *environment == environment_id
                    && diagnostic.node_id == agent_id
                    && diagnostic.expires_at > Utc::now()
            })
            .map(|(_, _, diagnostic)| diagnostic.clone())
            .collect::<Vec<_>>();
        reports.sort_by_key(|report| std::cmp::Reverse(report.requested_at));
        reports.truncate(50);
        Ok(reports)
    }

    async fn get_node_diagnostic(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        request_id: &str,
    ) -> Result<StoredNodeDiagnostic, RepositoryError> {
        self.state
            .read()
            .await
            .node_diagnostics
            .get(request_id)
            .filter(|(workspace, environment, diagnostic)| {
                *workspace == workspace_id
                    && *environment == environment_id
                    && diagnostic.node_id == agent_id
                    && diagnostic.expires_at > Utc::now()
            })
            .map(|(_, _, diagnostic)| diagnostic.clone())
            .ok_or(RepositoryError::NotFound)
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

    async fn get_agent(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
    ) -> Result<StoredAgent, RepositoryError> {
        self.state
            .read()
            .await
            .agents
            .get(&agent_id)
            .filter(|(workspace, environment, _)| {
                *workspace == workspace_id && *environment == environment_id
            })
            .map(|(_, _, agent)| agent.clone())
            .ok_or(RepositoryError::NotFound)
    }

    async fn rename_agent(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        name: &str,
    ) -> Result<StoredAgent, RepositoryError> {
        let mut state = self.state.write().await;
        let (_, _, agent) = state
            .agents
            .get_mut(&agent_id)
            .filter(|(workspace, environment, _)| {
                *workspace == workspace_id && *environment == environment_id
            })
            .ok_or(RepositoryError::NotFound)?;
        name.clone_into(&mut agent.name);
        Ok(agent.clone())
    }

    async fn revoke_agent(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
    ) -> Result<(), RepositoryError> {
        let mut state = self.state.write().await;
        let belongs_to_tenant =
            state
                .agents
                .get(&agent_id)
                .is_some_and(|(workspace, environment, _)| {
                    *workspace == workspace_id && *environment == environment_id
                });
        if !belongs_to_tenant {
            return Err(RepositoryError::NotFound);
        }
        state.agents.remove(&agent_id);
        Ok(())
    }

    async fn list_node_connectors(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
    ) -> Result<Vec<StoredNodeConnector>, RepositoryError> {
        let mut state = self.state.write().await;
        let (_, _, agent) = state
            .agents
            .get(&agent_id)
            .filter(|(workspace, environment, _)| {
                *workspace == workspace_id && *environment == environment_id
            })
            .ok_or(RepositoryError::NotFound)?;
        let connector_id = format!("ncon_{agent_id}");
        let key = (workspace_id, environment_id, agent_id, connector_id.clone());
        let created_at = agent.last_seen_at;
        Ok(
            vec![state.node_connectors.entry(key).or_insert_with(|| StoredNodeConnector {
            id: connector_id,
            node_id: agent_id,
            permissions: serde_json::json!({"printers":"all","print_jobs":"create_and_monitor"}),
            revoked_at: None,
            created_at,
        }).clone()],
        )
    }

    async fn revoke_node_connector(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        connector_id: &str,
    ) -> Result<(), RepositoryError> {
        let mut state = self.state.write().await;
        let created_at = state
            .agents
            .get(&agent_id)
            .filter(|(workspace, environment, _)| {
                *workspace == workspace_id && *environment == environment_id
            })
            .map(|(_, _, agent)| agent.last_seen_at)
            .ok_or(RepositoryError::NotFound)?;
        if connector_id != format!("ncon_{agent_id}") {
            return Err(RepositoryError::NotFound);
        }
        let key = (
            workspace_id,
            environment_id,
            agent_id,
            connector_id.to_owned(),
        );
        let connector = state.node_connectors.entry(key).or_insert_with(|| StoredNodeConnector {
            id: connector_id.to_owned(),
            node_id: agent_id,
            permissions: serde_json::json!({"printers":"all","print_jobs":"create_and_monitor"}),
            revoked_at: None,
            created_at,
        });
        if connector.revoked_at.is_some() {
            return Err(RepositoryError::NotFound);
        }
        connector.revoked_at = Some(Utc::now());
        Ok(())
    }

    async fn get_node_update(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        node_id: AgentId,
        default_mode: &str,
    ) -> Result<StoredNodeUpdate, RepositoryError> {
        let agent = self
            .get_agent(workspace_id, environment_id, node_id)
            .await?;
        let mut state = self.state.write().await;
        Ok(state
            .node_updates
            .entry(node_id)
            .or_insert_with(|| StoredNodeUpdate {
                node_id,
                policy: NodeUpdatePolicy {
                    channel: "stable".into(),
                    mode: default_mode.into(),
                    pinned_version: None,
                    maintenance_window: None,
                },
                status: NodeUpdateState {
                    current_version: agent.version,
                    available_version: None,
                    state: "idle".into(),
                    download_percent: None,
                    deferred_reason: None,
                    last_checked_at: None,
                    last_success_at: None,
                    last_error_code: None,
                    rollback_version: None,
                },
            })
            .clone())
    }

    async fn update_node_policy(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        node_id: AgentId,
        policy: &NodeUpdatePolicy,
        default_mode: &str,
    ) -> Result<StoredNodeUpdate, RepositoryError> {
        self.get_node_update(workspace_id, environment_id, node_id, default_mode)
            .await?;
        let mut state = self.state.write().await;
        let update = state
            .node_updates
            .get_mut(&node_id)
            .ok_or(RepositoryError::NotFound)?;
        update.policy = policy.clone();
        Ok(update.clone())
    }

    async fn request_node_update(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        node_id: AgentId,
        version: &str,
        default_mode: &str,
    ) -> Result<StoredNodeUpdate, RepositoryError> {
        self.get_node_update(workspace_id, environment_id, node_id, default_mode)
            .await?;
        let mut state = self.state.write().await;
        let update = state
            .node_updates
            .get_mut(&node_id)
            .ok_or(RepositoryError::NotFound)?;
        update.status.available_version = Some(version.into());
        update.status.state = "requested".into();
        update.status.last_checked_at = Some(Utc::now());
        Ok(update.clone())
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
                None,
            ),
        );
        Ok(())
    }

    async fn enrolment_status(
        &self,
        id: &str,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
    ) -> Result<(DateTime<Utc>, Option<AgentId>), RepositoryError> {
        self.state
            .read()
            .await
            .enrolments
            .get(id)
            .filter(|(workspace, environment, ..)| {
                *workspace == workspace_id && *environment == environment_id
            })
            .map(|(_, _, _, expires_at, agent_id)| (*expires_at, *agent_id))
            .ok_or(RepositoryError::NotFound)
    }

    async fn create_device_authorization(
        &self,
        authorization: &NewDeviceAuthorization<'_>,
    ) -> Result<StoredDeviceAuthorization, RepositoryError> {
        let record = StoredDeviceAuthorization {
            id: authorization.id.to_owned(),
            user_code: authorization.user_code_display.to_owned(),
            proposed_name: authorization.proposed_name.to_owned(),
            hostname: authorization.hostname.to_owned(),
            platform: authorization.platform.to_owned(),
            architecture: authorization.architecture.to_owned(),
            state: "pending".into(),
            expires_at: authorization.expires_at,
            workspace_id: None,
            environment_id: None,
        };
        self.state.write().await.device_authorizations.insert(
            authorization.id.to_owned(),
            MemoryDeviceAuthorization {
                record: record.clone(),
                device_code_hash: authorization.device_code_hash.to_owned(),
                user_code_hash: authorization.user_code_hash.to_owned(),
                public_key: authorization.device_public_key.to_vec(),
                agent_version: authorization.agent_version.to_owned(),
                installation_id: authorization.installation_id.to_owned(),
            },
        );
        Ok(record)
    }

    async fn device_authorization_by_hash(
        &self,
        device_code_hash: &str,
    ) -> Result<StoredDeviceAuthorization, RepositoryError> {
        let mut state = self.state.write().await;
        let authorization = state
            .device_authorizations
            .values_mut()
            .find(|authorization| authorization.device_code_hash == device_code_hash)
            .ok_or(RepositoryError::NotFound)?;
        if authorization.record.expires_at <= Utc::now()
            && matches!(authorization.record.state.as_str(), "pending" | "approved")
        {
            authorization.record.state = "expired".into();
        }
        Ok(authorization.record.clone())
    }

    async fn device_authorization_by_id(
        &self,
        id: &str,
    ) -> Result<StoredDeviceAuthorization, RepositoryError> {
        let mut state = self.state.write().await;
        let authorization = state
            .device_authorizations
            .get_mut(id)
            .ok_or(RepositoryError::NotFound)?;
        if authorization.record.expires_at <= Utc::now()
            && matches!(authorization.record.state.as_str(), "pending" | "approved")
        {
            authorization.record.state = "expired".into();
        }
        Ok(authorization.record.clone())
    }

    async fn approve_device_authorization(
        &self,
        id: &str,
        user_code_hash: &str,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        _approved_by: &str,
    ) -> Result<StoredDeviceAuthorization, RepositoryError> {
        let mut state = self.state.write().await;
        let authorization = state
            .device_authorizations
            .get_mut(id)
            .filter(|authorization| {
                authorization.record.state == "pending"
                    && authorization.user_code_hash == user_code_hash
                    && authorization.record.expires_at > Utc::now()
            })
            .ok_or(RepositoryError::NotFound)?;
        authorization.record.state = "approved".into();
        authorization.record.workspace_id = Some(workspace_id);
        authorization.record.environment_id = Some(environment_id);
        Ok(authorization.record.clone())
    }

    async fn deny_device_authorization(
        &self,
        id: &str,
        user_code_hash: &str,
    ) -> Result<StoredDeviceAuthorization, RepositoryError> {
        let mut state = self.state.write().await;
        let authorization = state
            .device_authorizations
            .get_mut(id)
            .filter(|authorization| {
                authorization.record.state == "pending"
                    && authorization.user_code_hash == user_code_hash
            })
            .ok_or(RepositoryError::NotFound)?;
        authorization.record.state = "denied".into();
        Ok(authorization.record.clone())
    }

    async fn exchange_device_authorization(
        &self,
        device_code_hash: &str,
    ) -> Result<EnrolledAgent, RepositoryError> {
        let mut state = self.state.write().await;
        let authorization = state
            .device_authorizations
            .values_mut()
            .find(|authorization| {
                authorization.device_code_hash == device_code_hash
                    && authorization.record.state == "approved"
                    && authorization.record.expires_at > Utc::now()
            })
            .ok_or(RepositoryError::NotFound)?;
        let workspace_id = authorization
            .record
            .workspace_id
            .ok_or(RepositoryError::NotFound)?;
        let environment_id = authorization
            .record
            .environment_id
            .ok_or(RepositoryError::NotFound)?;
        let installation = (
            workspace_id,
            environment_id,
            authorization.installation_id.clone(),
        );
        let proposed_name = authorization.record.proposed_name.clone();
        let platform = authorization.record.platform.clone();
        let version = authorization.agent_version.clone();
        let public_key = authorization.public_key.clone();
        authorization.record.state = "consumed".into();
        // Reusing an installation rebinds its existing node — this is what lets
        // a node rotate its device key without losing its ID or printers.
        let agent_id = state
            .agent_installations
            .get(&installation)
            .copied()
            .unwrap_or_else(AgentId::new);
        let agent = StoredAgent {
            id: agent_id,
            name: proposed_name,
            platform,
            state: "disconnected".into(),
            version,
            last_seen_at: Utc::now(),
            health_started_at: None,
            health_observed_at: None,
            sqlite_integrity_ok: None,
            executor_crashes: 0,
            last_error_code: None,
        };
        state
            .agents
            .insert(agent_id, (workspace_id, environment_id, agent));
        state.agent_installations.insert(installation, agent_id);
        state.agent_public_keys.insert(agent_id, public_key);
        Ok(EnrolledAgent {
            agent_id,
            workspace_id,
            environment_id,
            connector_id: None,
        })
    }

    async fn node_replaced_by_device_authorization(
        &self,
        id: &str,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
    ) -> Result<Option<AgentId>, RepositoryError> {
        let state = self.state.read().await;
        let authorization = state
            .device_authorizations
            .get(id)
            .ok_or(RepositoryError::NotFound)?;
        Ok(state
            .agent_installations
            .get(&(
                workspace_id,
                environment_id,
                authorization.installation_id.clone(),
            ))
            .copied())
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
        let (token_id, (workspace_id, environment_id, _, _, _)) = state
            .enrolments
            .iter()
            .find(|(_, (_, _, stored_hash, expires, agent_id))| {
                stored_hash == secret_hash && *expires > Utc::now() && agent_id.is_none()
            })
            .map(|(id, value)| (id.clone(), value.clone()))
            .ok_or(RepositoryError::NotFound)?;
        let agent_id = AgentId::new();
        if let Some(enrolment) = state.enrolments.get_mut(&token_id) {
            enrolment.4 = Some(agent_id);
        }
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
                    health_started_at: None,
                    health_observed_at: None,
                    sqlite_integrity_ok: None,
                    executor_crashes: 0,
                    last_error_code: None,
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
            connector_id: Some(format!("ncon_{agent_id}")),
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

    async fn delete_upload(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        upload_id: &str,
    ) -> Result<(), RepositoryError> {
        let mut state = self.state.write().await;
        let removable = state
            .uploads
            .get(upload_id)
            .is_some_and(|(workspace, environment, _)| {
                *workspace == workspace_id && *environment == environment_id
            });
        if !removable {
            return Err(RepositoryError::NotFound);
        }
        state.uploads.remove(upload_id);
        Ok(())
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

    async fn rotate_content_encryption_key(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        key_id: &str,
        algorithm: &str,
        public_key_spki: &str,
    ) -> Result<StoredContentEncryptionKey, RepositoryError> {
        if algorithm != "ECDH-P256-HKDF-SHA256" {
            return Err(RepositoryError::Persistence(
                "unsupported content encryption key algorithm".into(),
            ));
        }
        let mut state = self.state.write().await;
        if !state
            .agents
            .get(&agent_id)
            .is_some_and(|(w, e, _)| *w == workspace_id && *e == environment_id)
        {
            return Err(RepositoryError::NotFound);
        }
        let key = StoredContentEncryptionKey {
            agent_id,
            key_id: key_id.into(),
            algorithm: algorithm.into(),
            public_key_spki: public_key_spki.into(),
            created_at: Utc::now(),
        };
        state.content_encryption_keys.insert(agent_id, key.clone());
        Ok(key)
    }
    async fn content_encryption_key_for_printer(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        printer_id: PrinterId,
    ) -> Result<StoredContentEncryptionKey, RepositoryError> {
        let state = self.state.read().await;
        let agent_id = state
            .printers
            .get(&printer_id)
            .filter(|(w, e, _)| *w == workspace_id && *e == environment_id)
            .map(|(_, _, p)| p.agent_id)
            .ok_or(RepositoryError::NotFound)?;
        state
            .content_encryption_keys
            .get(&agent_id)
            .cloned()
            .ok_or(RepositoryError::NotFound)
    }
    async fn revoke_content_encryption_key(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        key_id: &str,
    ) -> Result<(), RepositoryError> {
        let mut state = self.state.write().await;
        if state
            .agents
            .get(&agent_id)
            .is_none_or(|(w, e, _)| *w != workspace_id || *e != environment_id)
            || state
                .content_encryption_keys
                .get(&agent_id)
                .is_none_or(|key| key.key_id != key_id)
        {
            return Err(RepositoryError::NotFound);
        }
        state.content_encryption_keys.remove(&agent_id);
        Ok(())
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
        if let piqae_domain::ContentSource::EncryptedUpload { manifest, .. } = &job.content {
            let encoded = serde_json::to_vec(manifest)
                .map_err(|error| RepositoryError::Persistence(error.to_string()))?;
            let digest = format!("{:x}", sha2::Sha256::digest(encoded));
            let index = (
                job.workspace_id,
                job.environment_id,
                manifest.binding.envelope_id.clone(),
            );
            if let Some((existing_digest, existing_job_id)) = state.consumed_envelopes.get(&index) {
                let existing_digest = existing_digest.clone();
                let existing_job_id = *existing_job_id;
                if let Some(key) = idempotency_key {
                    state.idempotency.remove(&(
                        job.workspace_id,
                        job.environment_id,
                        key.to_owned(),
                    ));
                }
                if existing_digest != digest {
                    return Err(RepositoryError::IdempotencyConflict);
                }
                return state
                    .jobs
                    .get(&existing_job_id)
                    .map(|record| CreateResult::Existing(record.job.clone()))
                    .ok_or(RepositoryError::NotFound);
            }
            state.consumed_envelopes.insert(index, (digest, job.id));
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

    #[allow(
        clippy::too_many_lines,
        reason = "atomic in-memory parity keeps every reroute fence under one write lock"
    )]
    async fn reroute_job_before_acceptance(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        job_id: JobId,
        target_id: &str,
        binding: &StoredTargetBinding,
        reason: &str,
    ) -> Result<Option<Job>, RepositoryError> {
        if !matches!(reason, "node_recovered" | "standby_recovery") {
            return Err(RepositoryError::Persistence(
                "unsupported routing attempt reason".into(),
            ));
        }
        let mut state = self.state.write().await;
        let has_acceptance = state.job_acceptances.contains_key(&job_id);
        let has_active_lease = state
            .leases
            .get(&job_id)
            .is_some_and(|(_, _, _, expiry)| *expiry > Utc::now());
        if has_acceptance || has_active_lease {
            return Ok(None);
        }
        let destination_is_valid =
            state
                .targets
                .get(target_id)
                .is_some_and(|(workspace, environment, target)| {
                    *workspace == workspace_id && *environment == environment_id && target.enabled
                })
                && state.target_bindings.get(&binding.id).is_some_and(
                    |(workspace, environment, stored)| {
                        *workspace == workspace_id
                            && *environment == environment_id
                            && stored.target_id == target_id
                            && stored.enabled
                            && stored.printer_id == binding.printer_id
                            && stored.agent_id == binding.agent_id
                            && stored.profile_id == binding.profile_id
                            && stored.profile_revision == binding.profile_revision
                    },
                );
        if !destination_is_valid {
            return Err(RepositoryError::ConcurrentStateChange);
        }
        let target_stock = state
            .targets
            .get(target_id)
            .and_then(|(_, _, target)| target.stock_id.clone());
        let intended_stock = state.jobs.get(&job_id).and_then(|record| {
            record
                .job
                .metadata
                .get("piqae.stock_id")
                .or_else(|| record.job.metadata.get("spool.stock_id"))
                .cloned()
        });
        if target_stock.is_some() && target_stock != intended_stock {
            return Ok(None);
        }
        let profile_is_valid = state.printers.get(&binding.printer_id).is_some_and(
            |(workspace, environment, printer)| {
                *workspace == workspace_id
                    && *environment == environment_id
                    && printer.agent_id == binding.agent_id
                    && printer.profiles.iter().any(|profile| {
                        profile.profile_id == binding.profile_id
                            && (profile.profile_id.as_str(), profile.revision)
                                == (binding.profile_id.as_str(), binding.profile_revision)
                            && profile.published
                            && matches!(profile.status.as_deref(), None | Some("ready"))
                            && profile.stock_id == intended_stock
                    })
            },
        );
        if !profile_is_valid {
            return Ok(None);
        }
        let (job, from_binding_id) = {
            let record = state
                .jobs
                .get_mut(&job_id)
                .ok_or(RepositoryError::NotFound)?;
            if record.job.workspace_id != workspace_id
                || record.job.environment_id != environment_id
                || !matches!(
                    record.job.state,
                    JobState::WaitingForAgent | JobState::FailedRetryable
                )
                || record
                    .job
                    .metadata
                    .get("piqae.target_id")
                    .or_else(|| record.job.metadata.get("spool.target_id"))
                    .map(String::as_str)
                    != Some(target_id)
                || record
                    .job
                    .metadata
                    .get("piqae.stock_id")
                    .or_else(|| record.job.metadata.get("spool.stock_id"))
                    != intended_stock.as_ref()
                || (record.agent_id == binding.agent_id
                    && record.job.printer_id == binding.printer_id)
            {
                return Ok(None);
            }
            let from_binding_id = record
                .job
                .metadata
                .get("piqae.binding_id")
                .or_else(|| record.job.metadata.get("spool.binding_id"))
                .cloned()
                .unwrap_or_default();
            for suffix in [
                "target_id",
                "binding_id",
                "profile_id",
                "profile_revision",
                "stock_id",
            ] {
                let legacy_key = format!("spool.{suffix}");
                if let Some(value) = record.job.metadata.remove(&legacy_key) {
                    record
                        .job
                        .metadata
                        .entry(format!("piqae.{suffix}"))
                        .or_insert(value);
                }
            }
            record.agent_id = binding.agent_id;
            record.job.printer_id = binding.printer_id;
            record
                .job
                .metadata
                .insert("piqae.binding_id".into(), binding.id.clone());
            record
                .job
                .metadata
                .insert("piqae.profile_id".into(), binding.profile_id.clone());
            record.job.metadata.insert(
                "piqae.profile_revision".into(),
                binding.profile_revision.to_string(),
            );
            (record.job.clone(), from_binding_id)
        };
        state.leases.remove(&job_id);
        state
            .routing_attempts
            .push((job_id, from_binding_id, binding.id.clone()));
        Ok(Some(job))
    }

    async fn list_reroutable_target_jobs(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        limit: i64,
    ) -> Result<Vec<Job>, RepositoryError> {
        let state = self.state.read().await;
        let mut jobs = state
            .jobs
            .values()
            .filter(|record| {
                record.job.workspace_id == workspace_id
                    && record.job.environment_id == environment_id
                    && matches!(
                        record.job.state,
                        JobState::WaitingForAgent | JobState::FailedRetryable
                    )
                    && record.job.expires_at > Utc::now()
                    && (record.job.metadata.contains_key("piqae.target_id")
                        || record.job.metadata.contains_key("spool.target_id"))
                    && !state.job_acceptances.contains_key(&record.job.id)
                    && state
                        .leases
                        .get(&record.job.id)
                        .is_none_or(|(_, _, _, expiry)| *expiry <= Utc::now())
            })
            .map(|record| record.job.clone())
            .collect::<Vec<_>>();
        jobs.sort_by_key(|job| (job.created_at, job.id));
        jobs.truncate(usize::try_from(limit.clamp(1, 100)).unwrap_or(100));
        Ok(jobs)
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
        let lease_until = lease.3;
        if let Some((_workspace, _environment, agent)) = state.agents.get_mut(&agent_id) {
            agent.state = "connected".into();
            agent.last_seen_at = Utc::now();
        }
        Ok(lease_until)
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
#[allow(clippy::expect_used, clippy::too_many_lines)]
mod routing_repository_tests {
    use super::*;
    use piqae_domain::JobOptions;
    use piqae_storage_postgres::PrinterProfileSnapshot;

    #[tokio::test]
    async fn encrypted_envelope_replay_returns_original_and_substitution_is_rejected() {
        let repository = MemoryRepository::default();
        let workspace = WorkspaceId::new();
        let environment = EnvironmentId::new();
        let printer = PrinterId::new();
        let agent = AgentId::new();
        let manifest = piqae_domain::EncryptedContentManifest {
            version: "piqae-encrypted-job-v3".into(),
            suite: "ECDH-P256+HKDF-SHA256+A256GCMKW+A256GCM".into(),
            binding: piqae_domain::EncryptedContentBinding {
                envelope_id: "env_012345678901234567890123".into(),
                workspace_id: workspace.to_string(),
                environment_id: environment.to_string(),
                content_type: piqae_domain::ContentKind::Pdf,
                printer_id: printer.to_string(),
                target_id: "tgt_test".into(),
                profile_revision: "prf_test:1".into(),
                options: JobOptions::default(),
                deliveries: 1,
                expires_at: "2099-01-01T00:00:00Z".into(),
                raw_authorized: false,
            },
            ciphertext_sha256: "A".repeat(43),
            iv: "A".repeat(16),
            recipients: vec![piqae_domain::EncryptedContentRecipient {
                key_id: "cek_test".into(),
                algorithm: "ECDH-P256-HKDF-SHA256".into(),
                ephemeral_public_key: format!("B{}", "A".repeat(86)),
                hkdf_salt: "A".repeat(43),
                key_wrap_iv: "A".repeat(16),
                encrypted_content_key: "A".repeat(64),
            }],
        };
        let make_job = |manifest: piqae_domain::EncryptedContentManifest| Job {
            id: JobId::new(),
            workspace_id: workspace,
            environment_id: environment,
            printer_id: printer,
            title: "private".into(),
            source: None,
            content_kind: piqae_domain::ContentKind::Pdf,
            content: piqae_domain::ContentSource::EncryptedUpload {
                upload_id: "upl_cipher".into(),
                manifest: Box::new(manifest),
            },
            options: JobOptions::default(),
            metadata: std::collections::BTreeMap::new(),
            deliveries: 1,
            state: JobState::Registered,
            created_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::hours(1),
        };
        let original = make_job(manifest.clone());
        assert!(matches!(
            repository
                .create_job(&original, agent, None, b"one")
                .await
                .expect("create"),
            CreateResult::Created(_)
        ));
        let replay = make_job(manifest.clone());
        match repository
            .create_job(&replay, agent, None, b"two")
            .await
            .expect("replay")
        {
            CreateResult::Existing(job) => assert_eq!(job.id, original.id),
            CreateResult::Created(_) => panic!("envelope replay created a duplicate"),
        }
        let mut changed = manifest;
        changed.binding.profile_revision = "prf_test:2".into();
        assert!(matches!(
            repository
                .create_job(&make_job(changed), agent, None, b"three")
                .await,
            Err(RepositoryError::IdempotencyConflict)
        ));
    }

    #[tokio::test]
    async fn connector_lookup_and_revocation_are_tenant_scoped() {
        let repository = MemoryRepository::default();
        let workspace = WorkspaceId::new();
        let environment = EnvironmentId::new();
        let other_workspace = WorkspaceId::new();
        let node = AgentId::new();
        repository.state.write().await.agents.insert(
            node,
            (
                workspace,
                environment,
                StoredAgent {
                    id: node,
                    name: "Shared PC".into(),
                    platform: "test".into(),
                    state: "connected".into(),
                    version: "1".into(),
                    last_seen_at: Utc::now(),
                    health_started_at: None,
                    health_observed_at: None,
                    sqlite_integrity_ok: None,
                    executor_crashes: 0,
                    last_error_code: None,
                },
            ),
        );

        assert_eq!(
            repository
                .list_node_connectors(workspace, environment, node)
                .await
                .expect("own connector")
                .len(),
            1
        );
        assert!(matches!(
            repository
                .list_node_connectors(other_workspace, environment, node)
                .await,
            Err(RepositoryError::NotFound)
        ));
        assert!(matches!(
            repository
                .revoke_node_connector(other_workspace, environment, node, &format!("ncon_{node}"))
                .await,
            Err(RepositoryError::NotFound)
        ));
        repository
            .revoke_node_connector(workspace, environment, node, &format!("ncon_{node}"))
            .await
            .expect("revoke own connector");
        assert!(
            repository
                .get_agent(workspace, environment, node)
                .await
                .is_ok()
        );
        let connectors = repository
            .list_node_connectors(workspace, environment, node)
            .await
            .expect("revoked connector remains visible");
        assert_eq!(connectors.len(), 1);
        assert!(connectors[0].revoked_at.is_some());
        assert!(matches!(
            repository
                .revoke_node_connector(workspace, environment, node, &format!("ncon_{node}"))
                .await,
            Err(RepositoryError::NotFound)
        ));
    }

    #[tokio::test]
    async fn memory_content_key_rotation_preserves_the_validated_algorithm() {
        let repository = MemoryRepository::default();
        let workspace = WorkspaceId::new();
        let environment = EnvironmentId::new();
        let node = AgentId::new();
        repository.state.write().await.agents.insert(
            node,
            (
                workspace,
                environment,
                StoredAgent {
                    id: node,
                    name: "Encryption node".into(),
                    platform: "test".into(),
                    state: "connected".into(),
                    version: "1".into(),
                    last_seen_at: Utc::now(),
                    health_started_at: None,
                    health_observed_at: None,
                    sqlite_integrity_ok: None,
                    executor_crashes: 0,
                    last_error_code: None,
                },
            ),
        );

        let stored = repository
            .rotate_content_encryption_key(
                workspace,
                environment,
                node,
                "cek_test",
                "ECDH-P256-HKDF-SHA256",
                "test-spki",
            )
            .await
            .expect("supported algorithm");
        assert_eq!(stored.algorithm, "ECDH-P256-HKDF-SHA256");
        assert!(matches!(
            repository
                .rotate_content_encryption_key(
                    workspace,
                    environment,
                    node,
                    "cek_legacy",
                    "RSA-OAEP-256",
                    "test-spki",
                )
                .await,
            Err(RepositoryError::Persistence(_))
        ));
        assert_eq!(
            repository
                .state
                .read()
                .await
                .content_encryption_keys
                .get(&node)
                .expect("original key remains")
                .key_id,
            "cek_test"
        );
    }

    struct RecoveryFixture {
        repository: MemoryRepository,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        primary_agent: AgentId,
        standby_printer: PrinterId,
        standby_binding: StoredTargetBinding,
        job_id: JobId,
    }

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

    async fn recovery_fixture() -> RecoveryFixture {
        let repository = MemoryRepository::default();
        let workspace_id = WorkspaceId::new();
        let environment_id = EnvironmentId::new();
        let primary_agent = AgentId::new();
        let standby_agent = AgentId::new();
        let primary_printer = PrinterId::new();
        let standby_printer = PrinterId::new();
        repository
            .add_printer(workspace_id, environment_id, primary_printer, primary_agent)
            .await;
        repository
            .add_printer(workspace_id, environment_id, standby_printer, standby_agent)
            .await;
        {
            let mut state = repository.state.write().await;
            for printer_id in [primary_printer, standby_printer] {
                state
                    .printers
                    .get_mut(&printer_id)
                    .expect("fixture printer")
                    .2
                    .profiles = vec![PrinterProfileSnapshot {
                    profile_id: "profile_shipping".into(),
                    revision: 4,
                    name: "Shipping".into(),
                    is_default: true,
                    options: JobOptions::default(),
                    status: Some("ready".into()),
                    native_kind: None,
                    native_digest: Some("sha256:fixture".into()),
                    driver_fingerprint: None,
                    summary: None,
                    stock_id: None,
                    safe_overrides: Vec::new(),
                    last_validated_at: None,
                    last_test_job_id: None,
                    published: true,
                }];
            }
        }
        repository
            .create_target(
                workspace_id,
                environment_id,
                &target("tgt_recovery", "Recovery target"),
            )
            .await
            .expect("create target");
        let now = Utc::now();
        let primary_binding = StoredTargetBinding {
            id: "tgb_primary".into(),
            target_id: "tgt_recovery".into(),
            printer_id: primary_printer,
            agent_id: primary_agent,
            profile_id: "profile_shipping".into(),
            profile_revision: 4,
            role: "primary".into(),
            enabled: true,
            created_at: now,
            updated_at: now,
        };
        let standby_binding = StoredTargetBinding {
            id: "tgb_standby".into(),
            target_id: "tgt_recovery".into(),
            printer_id: standby_printer,
            agent_id: standby_agent,
            profile_id: "profile_shipping".into(),
            profile_revision: 4,
            role: "standby".into(),
            enabled: true,
            created_at: now,
            updated_at: now,
        };
        repository
            .create_target_binding(workspace_id, environment_id, &primary_binding)
            .await
            .expect("create primary binding");
        repository
            .create_target_binding(workspace_id, environment_id, &standby_binding)
            .await
            .expect("create standby binding");
        let job_id = JobId::new();
        let job = Job {
            id: job_id,
            workspace_id,
            environment_id,
            printer_id: primary_printer,
            title: "Recovery fixture".into(),
            source: None,
            content_kind: piqae_domain::ContentKind::Pdf,
            content: piqae_domain::ContentSource::Base64 {
                data: "JVBERi0=".into(),
            },
            options: JobOptions::default(),
            metadata: std::collections::BTreeMap::from([
                ("spool.target_id".into(), "tgt_recovery".into()),
                ("spool.binding_id".into(), primary_binding.id),
                ("spool.profile_id".into(), "profile_shipping".into()),
                ("spool.profile_revision".into(), "4".into()),
            ]),
            deliveries: 1,
            state: JobState::WaitingForAgent,
            created_at: now,
            expires_at: now + chrono::Duration::hours(1),
        };
        repository
            .create_job(&job, primary_agent, None, b"recovery fixture")
            .await
            .expect("create recovery job");
        RecoveryFixture {
            repository,
            workspace_id,
            environment_id,
            primary_agent,
            standby_printer,
            standby_binding,
            job_id,
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

    #[tokio::test]
    async fn concurrent_standby_recovery_routes_once_and_records_one_attempt() {
        let fixture = recovery_fixture().await;
        let first_repository = fixture.repository.clone();
        let second_repository = fixture.repository.clone();
        let first_binding = fixture.standby_binding.clone();
        let second_binding = fixture.standby_binding.clone();
        let first = first_repository.reroute_job_before_acceptance(
            fixture.workspace_id,
            fixture.environment_id,
            fixture.job_id,
            "tgt_recovery",
            &first_binding,
            "standby_recovery",
        );
        let second = second_repository.reroute_job_before_acceptance(
            fixture.workspace_id,
            fixture.environment_id,
            fixture.job_id,
            "tgt_recovery",
            &second_binding,
            "standby_recovery",
        );
        let (first, second) = tokio::join!(first, second);
        assert_eq!(
            usize::from(first.expect("first attempt").is_some())
                + usize::from(second.expect("second attempt").is_some()),
            1
        );
        let rerouted = fixture
            .repository
            .get_job(fixture.workspace_id, fixture.environment_id, fixture.job_id)
            .await
            .expect("rerouted job");
        assert_eq!(rerouted.printer_id, fixture.standby_printer);
        assert_eq!(
            rerouted.metadata.get("piqae.target_id").map(String::as_str),
            Some("tgt_recovery")
        );
        assert!(!rerouted.metadata.contains_key("spool.target_id"));
        let state = fixture.repository.state.read().await;
        assert_eq!(state.routing_attempts.len(), 1);
        assert_eq!(
            state.routing_attempts[0],
            (fixture.job_id, "tgb_primary".into(), "tgb_standby".into())
        );
    }

    #[tokio::test]
    async fn actively_leased_job_is_never_rerouted() {
        let fixture = recovery_fixture().await;
        let leases = fixture
            .repository
            .claim_jobs(
                fixture.workspace_id,
                fixture.environment_id,
                fixture.primary_agent,
                "test-owner",
                1,
            )
            .await
            .expect("claim job");
        assert_eq!(leases.len(), 1);
        let result = fixture
            .repository
            .reroute_job_before_acceptance(
                fixture.workspace_id,
                fixture.environment_id,
                fixture.job_id,
                "tgt_recovery",
                &fixture.standby_binding,
                "standby_recovery",
            )
            .await
            .expect("reroute result");
        assert!(result.is_none());
        assert_eq!(
            fixture
                .repository
                .get_job(fixture.workspace_id, fixture.environment_id, fixture.job_id)
                .await
                .expect("leased job")
                .printer_id,
            leases[0].job.printer_id
        );
        assert!(
            fixture
                .repository
                .state
                .read()
                .await
                .routing_attempts
                .is_empty()
        );
    }

    #[tokio::test]
    async fn durably_accepted_job_is_never_rerouted() {
        let fixture = recovery_fixture().await;
        let lease = fixture
            .repository
            .claim_jobs(
                fixture.workspace_id,
                fixture.environment_id,
                fixture.primary_agent,
                "test-owner",
                1,
            )
            .await
            .expect("claim job")
            .pop()
            .expect("job lease");
        fixture
            .repository
            .accept_agent_job(
                fixture.workspace_id,
                fixture.environment_id,
                fixture.primary_agent,
                fixture.job_id,
                lease.lease_id,
                &lease.lease_token,
                None,
                1,
            )
            .await
            .expect("durable acceptance");
        let result = fixture
            .repository
            .reroute_job_before_acceptance(
                fixture.workspace_id,
                fixture.environment_id,
                fixture.job_id,
                "tgt_recovery",
                &fixture.standby_binding,
                "standby_recovery",
            )
            .await
            .expect("reroute result");
        assert!(result.is_none());
        assert_eq!(
            fixture
                .repository
                .get_job(fixture.workspace_id, fixture.environment_id, fixture.job_id)
                .await
                .expect("accepted job")
                .state,
            JobState::AgentAccepted
        );
        assert!(
            fixture
                .repository
                .state
                .read()
                .await
                .routing_attempts
                .is_empty()
        );
    }

    #[tokio::test]
    async fn diagnostic_reports_are_durable_tenant_scoped_projections() {
        let repository = MemoryRepository::default();
        let workspace = WorkspaceId::new();
        let environment = EnvironmentId::new();
        let other_workspace = WorkspaceId::new();
        let node = AgentId::new();
        repository
            .add_printer(workspace, environment, PrinterId::new(), node)
            .await;
        repository
            .create_node_diagnostic(workspace, environment, node, "diag_test")
            .await
            .expect("request diagnostic");
        let report = piqae_protocol::agent::DiagnosticReport {
            request_id: "diag_test".into(),
            observed_at: Utc::now(),
            state: "complete".into(),
            agent_version: "test".into(),
            platform: "test".into(),
            architecture: "test".into(),
            queued_jobs: 1,
            active_jobs: 0,
            sqlite_integrity_ok: true,
            executor_crashes: 2,
            last_error_code: Some("executor_crashed".into()),
            collection_error_code: None,
        };
        repository
            .store_node_diagnostic(workspace, environment, node, &report)
            .await
            .expect("store report");
        let stored = repository
            .get_node_diagnostic(workspace, environment, node, "diag_test")
            .await
            .expect("get report");
        assert_eq!(stored.state, "complete");
        assert!(stored.report.is_some());
        assert!(
            repository
                .get_node_diagnostic(other_workspace, environment, node, "diag_test")
                .await
                .is_err()
        );
    }
}
