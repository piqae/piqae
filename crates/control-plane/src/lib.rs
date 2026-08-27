// Blacksmith runner validation: remove with this branch.
//! Authoritative Piqae HTTP control plane.

pub mod api;
pub mod auth_maintenance_worker;
pub mod authentication;
pub mod billing;
pub mod billing_usage_worker;
pub mod compatibility;
pub mod destination_topology;
pub mod device_auth;
pub mod document_crypto;
pub mod document_render_worker;
pub mod documents;
pub mod error;
pub mod identity;
pub mod pairing;
pub mod platform;
pub mod print_intents;
pub mod rate_limit;
pub mod repository;
pub mod request_id;
pub mod routing;
pub mod updates;
pub mod wake_hint_worker;
pub mod webhook_worker;
pub mod workos_identity;

use authentication::{Authenticator, TenantContext};
use axum::{
    Router,
    extract::DefaultBodyLimit,
    middleware,
    routing::{get, post},
};
use piqae_object_store::{MemoryObjectStore, ObjectStore};
use piqae_storage_postgres::destination_topology::{
    DestinationTopologyRepository, MemoryDestinationTopologyRepository,
};
use piqae_webhooks::WebhookSecretBox;
use repository::Repository;
use serde::Serialize;
use std::{fmt, sync::Arc};
use tokio::sync::{Semaphore, broadcast};
use tower_http::compression::CompressionLayer;

#[derive(Clone, Debug)]
pub struct PublishedEvent {
    pub id: String,
    pub tenant: TenantContext,
    pub event_type: String,
    pub data: serde_json::Value,
}

#[derive(Clone)]
pub struct AppState {
    pub repository: Arc<dyn Repository>,
    /// Destination/route topology is deliberately separate from the legacy
    /// job repository while the public target aliases delegate into it. The
    /// production builder replaces the in-memory default with `PostgreSQL`.
    pub destination_topology: Arc<dyn DestinationTopologyRepository>,
    /// Server-secret key used only to tenant-pseudonymise already-normalised
    /// node identity evidence before persistence.
    pub(crate) destination_identity_key: [u8; 32],
    pub authenticator: Arc<dyn Authenticator>,
    pub events: broadcast::Sender<PublishedEvent>,
    pub webhook_secrets: Arc<WebhookSecretBox>,
    pub document_secrets: Arc<document_crypto::DocumentSecretBox>,
    pub object_store: Arc<dyn ObjectStore>,
    pub document_artifact_downloads: Arc<Semaphore>,
    pub capabilities: DeploymentCapabilities,
    pub local_identity: Option<identity::LocalIdentityState>,
    pub stripe_webhook_secret: Option<Arc<str>>,
    pub workos_webhook_secret: Option<Arc<str>>,
    pub public_control_plane_url: Arc<str>,
}

impl fmt::Debug for AppState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AppState")
            .field("event_receivers", &self.events.receiver_count())
            .finish_non_exhaustive()
    }
}

impl AppState {
    /// Builds an in-memory state with deterministic zero-valued test keys.
    /// Production entrypoints must use [`Self::new_with_resources`].
    #[doc(hidden)]
    #[must_use]
    pub fn new_for_tests(
        repository: Arc<dyn Repository>,
        authenticator: Arc<dyn Authenticator>,
    ) -> Self {
        Self::new_with_resources(
            repository,
            authenticator,
            [0; 32],
            document_crypto::DocumentSecretBox::new([0; 32]),
            Arc::new(MemoryObjectStore::default()),
        )
    }

    #[must_use]
    /// Test-only convenience constructor with an explicit webhook fixture key.
    #[doc(hidden)]
    pub fn new_with_webhook_key_for_tests(
        repository: Arc<dyn Repository>,
        authenticator: Arc<dyn Authenticator>,
        webhook_key: [u8; 32],
    ) -> Self {
        Self::new_with_resources(
            repository,
            authenticator,
            webhook_key,
            document_crypto::DocumentSecretBox::new([0; 32]),
            Arc::new(MemoryObjectStore::default()),
        )
    }

    #[must_use]
    pub fn new_with_resources(
        repository: Arc<dyn Repository>,
        authenticator: Arc<dyn Authenticator>,
        webhook_key: [u8; 32],
        document_secrets: document_crypto::DocumentSecretBox,
        object_store: Arc<dyn ObjectStore>,
    ) -> Self {
        let (events, _) = broadcast::channel(1_024);
        let destination_topology = repository
            .memory_destination_topology()
            .unwrap_or_else(|| Arc::new(MemoryDestinationTopologyRepository::default()));
        Self {
            repository,
            destination_topology,
            // Tests deliberately use an explicit non-production fixture key.
            // Production replaces it from PIQAE_DESTINATION_IDENTITY_KEY.
            destination_identity_key: [0; 32],
            authenticator,
            events,
            webhook_secrets: Arc::new(WebhookSecretBox::new(webhook_key)),
            document_secrets: Arc::new(document_secrets),
            object_store,
            document_artifact_downloads: Arc::new(Semaphore::new(4)),
            capabilities: DeploymentCapabilities::default(),
            local_identity: None,
            stripe_webhook_secret: None,
            workos_webhook_secret: None,
            public_control_plane_url: Arc::from("http://127.0.0.1:8080"),
        }
    }

    #[must_use]
    pub fn with_capabilities(mut self, capabilities: DeploymentCapabilities) -> Self {
        self.capabilities = capabilities;
        self
    }

    #[must_use]
    pub fn with_destination_topology(
        mut self,
        repository: Arc<dyn DestinationTopologyRepository>,
    ) -> Self {
        self.destination_topology = repository;
        self
    }

    /// Configures the stable key used to tenant-pseudonymise physical-device
    /// evidence. It is intentionally independent of webhook and document keys
    /// so unrelated secret rotation cannot split destination identities.
    #[must_use]
    pub const fn with_destination_identity_key(mut self, key: [u8; 32]) -> Self {
        self.destination_identity_key = key;
        self
    }

    #[must_use]
    pub fn with_document_key(mut self, key: [u8; 32]) -> Self {
        self.document_secrets = Arc::new(document_crypto::DocumentSecretBox::new(key));
        self
    }

    #[must_use]
    pub fn with_document_keyring(mut self, keyring: document_crypto::DocumentSecretBox) -> Self {
        self.document_secrets = Arc::new(keyring);
        self
    }

    #[must_use]
    pub fn with_document_artifact_download_concurrency(mut self, concurrency: usize) -> Self {
        self.document_artifact_downloads = Arc::new(Semaphore::new(concurrency.clamp(1, 32)));
        self
    }

    #[must_use]
    pub fn with_local_identity(mut self, identity: identity::LocalIdentityState) -> Self {
        self.local_identity = Some(identity);
        self
    }

    #[must_use]
    pub fn with_stripe_webhook_secret(mut self, secret: impl Into<Arc<str>>) -> Self {
        self.stripe_webhook_secret = Some(secret.into());
        self
    }

    #[must_use]
    pub fn with_workos_webhook_secret(mut self, secret: impl Into<Arc<str>>) -> Self {
        self.workos_webhook_secret = Some(secret.into());
        self
    }

    #[must_use]
    pub fn with_public_control_plane_url(mut self, url: impl Into<Arc<str>>) -> Self {
        self.public_control_plane_url = url.into();
        self
    }

    /// Persists a tenant webhook event and broadcasts it to live subscribers.
    ///
    /// # Errors
    ///
    /// Returns an error when the event cannot be serialized or persisted.
    pub async fn publish(
        &self,
        tenant: TenantContext,
        event_type: &str,
        data: &(impl Serialize + Sync),
    ) -> Result<(), repository::RepositoryError> {
        let data = serde_json::to_value(data)
            .map_err(|error| repository::RepositoryError::Persistence(error.to_string()))?;
        let id = self
            .repository
            .enqueue_webhook_event(
                tenant.workspace_id,
                tenant.environment_id,
                event_type,
                &data,
            )
            .await?;
        let _ = self.events.send(PublishedEvent {
            id,
            tenant,
            event_type: event_type.into(),
            data,
        });
        Ok(())
    }

    /// Persists an idempotent tenant event and broadcasts its time-sortable ID
    /// to live subscribers. Transactional repository paths use the same key to
    /// replay an already-committed outbox event without a duplicate delivery.
    ///
    /// # Errors
    ///
    /// Returns an error when the event cannot be serialized or persisted.
    pub async fn publish_idempotently(
        &self,
        idempotency_key: &str,
        tenant: TenantContext,
        event_type: &str,
        data: &(impl Serialize + Sync),
    ) -> Result<(), repository::RepositoryError> {
        let data = serde_json::to_value(data)
            .map_err(|error| repository::RepositoryError::Persistence(error.to_string()))?;
        let id = self
            .repository
            .enqueue_webhook_event_idempotently(
                idempotency_key,
                tenant.workspace_id,
                tenant.environment_id,
                event_type,
                &data,
            )
            .await?;
        let _ = self.events.send(PublishedEvent {
            id,
            tenant,
            event_type: event_type.into(),
            data,
        });
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct DeploymentCapabilities {
    pub deployment: String,
    pub version: &'static str,
    pub auth: AuthCapabilities,
    pub billing: BillingCapabilities,
    pub updates: UpdateCapabilities,
    pub platform: PlatformCapabilities,
}

#[derive(Clone, Debug, Serialize)]
pub struct AuthCapabilities {
    pub provider: String,
    pub workspace_switching: bool,
    pub invitations: bool,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct BillingCapabilities {
    pub enabled: bool,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct UpdateCapabilities {
    pub official_feed: bool,
    pub custom_feed: bool,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct PlatformCapabilities {
    pub accounts: bool,
}

impl Default for DeploymentCapabilities {
    fn default() -> Self {
        Self {
            deployment: "self_hosted".into(),
            version: env!("CARGO_PKG_VERSION"),
            auth: AuthCapabilities {
                provider: "local_owner".into(),
                workspace_switching: false,
                invitations: false,
            },
            billing: BillingCapabilities { enabled: false },
            updates: UpdateCapabilities {
                official_feed: true,
                custom_feed: true,
            },
            platform: PlatformCapabilities { accounts: true },
        }
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "the V1 modular-monolith route table is intentionally centralized"
)]
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1/health", get(api::health))
        .route("/v1/ready", get(api::ready))
        .route("/v1/meta", get(api::meta))
        .merge(identity::router())
        .merge(workos_identity_router())
        .merge(pairing_router())
        .merge(documents::router())
        .route("/v1/platform/status", get(platform::status))
        .route("/v1/platform/enable", post(platform::enable))
        .route(
            "/v1/platform/credential",
            get(platform::credential)
                .post(platform::rotate_credential)
                .delete(platform::revoke_credential),
        )
        .route("/v1/platform/accounts", get(platform::list))
        .route("/v1/platform/operations", get(platform::operations))
        .route(
            "/v1/platform/accounts/{external_id}",
            get(platform::get)
                .put(platform::upsert)
                .delete(platform::archive),
        )
        .route("/v1/billing/summary", get(billing::summary))
        .route("/v1/usage", get(billing::usage))
        .route(
            "/v1/integrations/stripe/webhook",
            post(billing::stripe_webhook),
        )
        .route(
            "/v1/api-keys",
            get(api::list_api_keys).post(api::create_api_key),
        )
        .route(
            "/v1/api-keys/{key_id}",
            axum::routing::delete(api::revoke_api_key),
        )
        .route("/v1/agents", get(api::list_agents))
        .merge(node_operator_router())
        .route("/v1/printers", get(api::list_printers))
        .route("/v1/printers/{printer_id}", get(api::get_printer))
        .route(
            "/v1/physical-destinations",
            get(destination_topology::list_destinations),
        )
        .route(
            "/v1/physical-destinations/{destination_id}",
            get(destination_topology::get_destination),
        )
        .route(
            "/v1/physical-destinations/{destination_id}/routes",
            get(destination_topology::list_destination_routes),
        )
        .route(
            "/v1/physical-destinations/{destination_id}/identity-evidence",
            get(destination_topology::list_identity_evidence),
        )
        .route(
            "/v1/physical-destinations/{destination_id}/identity-decisions",
            get(destination_topology::list_identity_decisions)
                .post(destination_topology::create_identity_decision),
        )
        .route(
            "/v1/physical-destinations/{destination_id}/identity-decisions/{decision_id}/reverse",
            post(destination_topology::reverse_identity_decision),
        )
        .route("/v1/printer-routes", get(destination_topology::list_routes))
        .route(
            "/v1/printer-routes/{route_id}",
            get(destination_topology::get_route),
        )
        .route(
            "/v1/printer-routes/{route_id}/observations",
            get(destination_topology::list_route_observations),
        )
        .route(
            "/v1/route-reservations",
            get(destination_topology::list_route_reservations),
        )
        .route(
            "/v1/printers/{printer_id}/capabilities",
            get(print_intents::capability_document),
        )
        .route(
            "/v1/printers/{printer_id}/loaded-media",
            get(print_intents::loaded_media).put(print_intents::upsert_loaded_media),
        )
        .route("/v1/print-intents/validate", post(print_intents::validate))
        .route("/v1/print-intents/resolve", post(print_intents::resolve))
        .route(
            "/v1/printers/{printer_id}/content-encryption-key",
            get(api::printer_content_encryption_key),
        )
        .route(
            "/v1/stocks",
            get(routing::list_stocks).post(routing::create_stock),
        )
        .route(
            "/v1/stocks/{stock_id}",
            axum::routing::patch(routing::patch_stock),
        )
        .route(
            "/v1/print-workflows",
            get(routing::list_print_workflows).post(routing::create_print_workflow),
        )
        .route(
            "/v1/targets",
            get(routing::list_targets).post(routing::create_target),
        )
        .route(
            "/v1/targets/{target_id}",
            axum::routing::patch(routing::patch_target),
        )
        .route(
            "/v1/targets/{target_id}/bindings",
            get(routing::list_bindings).post(routing::create_binding),
        )
        .route(
            "/v1/targets/{target_id}/bindings/{binding_id}",
            axum::routing::delete(routing::delete_binding),
        )
        .route(
            "/v1/targets/{target_id}/readiness",
            get(routing::target_readiness),
        )
        .route(
            "/v1/targets/{target_id}/design-specification",
            get(routing::design_specification),
        )
        .route("/v1/agent-enrolments", post(api::create_agent_enrolment))
        .route(
            "/v1/node-connect-sessions",
            post(api::create_node_connect_session),
        )
        .route(
            "/v1/node-connect-sessions/{session_id}",
            get(api::get_node_connect_session),
        )
        .route(
            "/v1/nodes/{node_id}/connectors",
            get(api::list_node_connectors),
        )
        .route(
            "/v1/nodes/{node_id}/connectors/{connector_id}",
            axum::routing::delete(api::revoke_node_connector),
        )
        .merge(enrolment_router())
        .route("/v1/uploads", post(api::create_upload))
        .route(
            "/v1/agent/content-encryption-key",
            axum::routing::put(api::register_agent_content_encryption_key),
        )
        .route(
            "/v1/agent/identity",
            axum::routing::put(api::update_agent_identity),
        )
        .route(
            "/v1/agent/content-encryption-key/{key_id}",
            axum::routing::delete(api::revoke_agent_content_encryption_key),
        )
        .route("/v1/uploads/{upload_id}", get(api::get_upload))
        .route(
            "/v1/uploads/{upload_id}/content",
            axum::routing::put(api::upload_content),
        )
        .route(
            "/v1/uploads/{upload_id}/complete",
            post(api::complete_upload),
        )
        .route(
            "/v1/webhooks",
            get(api::list_webhooks).post(api::create_webhook),
        )
        .route(
            "/v1/webhooks/{webhook_id}",
            axum::routing::delete(api::delete_webhook),
        )
        .route(
            "/v1/webhooks/{webhook_id}/deliveries",
            get(api::list_webhook_deliveries),
        )
        .route(
            "/v1/webhook-deliveries/{delivery_id}/replay",
            post(api::replay_webhook_delivery),
        )
        .route("/v1/jobs", post(api::create_job).get(api::list_jobs))
        .route("/v1/jobs/{job_id}", get(api::get_job))
        .route("/v1/jobs/{job_id}/events", get(api::list_job_events))
        .route(
            "/v1/jobs/{job_id}/delivery-attempts",
            get(destination_topology::list_delivery_attempts),
        )
        .route(
            "/v1/jobs/{job_id}/resolve-uncertain",
            post(destination_topology::resolve_uncertain_delivery),
        )
        .route("/v1/jobs/{job_id}/cancel", post(api::cancel_job))
        .route("/v1/events/stream", get(api::stream_events))
        .route("/v1/agent/sync", post(api::agent_sync))
        .route(
            "/v1/agent/connectors/{connector_id}/revoke",
            post(api::revoke_agent_connector),
        )
        .route(
            "/v1/agent/jobs/{job_id}/accept",
            post(api::accept_agent_job),
        )
        .route(
            "/v1/agent/jobs/{job_id}/acceptance/reconcile",
            post(api::reconcile_agent_acceptance),
        )
        .route(
            "/v1/agent/jobs/{job_id}/acceptance/abandon",
            post(api::abandon_agent_acceptance),
        )
        .route(
            "/v1/agent/jobs/{job_id}/lease",
            post(api::renew_agent_lease),
        )
        .route(
            "/v1/agent/jobs/{job_id}/release",
            post(api::release_agent_lease),
        )
        .route(
            "/v1/agent/jobs/{job_id}/content",
            get(api::get_agent_content),
        )
        .route(
            "/v1/agent/jobs/{job_id}/resources/{digest}",
            get(api::get_agent_document_resource),
        )
        .merge(compatibility_router())
        // A 50 MiB binary payload expands to roughly 66.7 MiB when Base64 is
        // carried in JSON. Direct uploads remain preferred, but compatibility
        // clients must be able to submit the documented maximum.
        .layer(DefaultBodyLimit::max(72 * 1024 * 1024))
        .layer(CompressionLayer::new())
        .layer(middleware::from_fn(request_id::middleware))
        .with_state(state)
}

fn pairing_router() -> Router<AppState> {
    // Creating an authorization writes a durable row for an unauthenticated
    // caller, so it gets the tightest budget. Polling and exchange are read
    // paths a legitimately pairing node hits every two seconds for up to ten
    // minutes, so they get room for a plausible fleet pairing at once.
    let creation = rate_limit::RateLimiter::new(10, 60, 60);
    let polling = rate_limit::RateLimiter::new(300, 3_000, 60);
    Router::new()
        .route("/v1/device-authorizations", post(pairing::create))
        .layer(middleware::from_fn_with_state(
            creation,
            rate_limit::middleware,
        ))
        .merge(
            Router::new()
                .route("/v1/device-authorizations/status", post(pairing::status))
                .route(
                    "/v1/device-authorizations/exchange",
                    post(pairing::exchange),
                )
                .route(
                    "/v1/device-authorizations/{device_code}",
                    get(pairing::status_by_path),
                )
                .route(
                    "/v1/device-authorizations/{device_code}/exchange",
                    post(pairing::exchange_by_path),
                )
                .layer(middleware::from_fn_with_state(
                    polling,
                    rate_limit::middleware,
                )),
        )
        .route(
            "/v1/device-authorizations/{authorization_id}/review",
            get(pairing::review),
        )
        .route(
            "/v1/device-authorizations/{authorization_id}/approve",
            post(pairing::approve),
        )
        .route(
            "/v1/device-authorizations/{authorization_id}/deny",
            post(pairing::deny),
        )
}

fn enrolment_router() -> Router<AppState> {
    // A one-time enrolment token is consumed by an otherwise unauthenticated
    // caller. The budget leaves ample room for retries around a real install
    // while bounding automated attempts against the token space.
    let limiter = rate_limit::RateLimiter::new(20, 120, 60);
    Router::new()
        .route("/v1/agents/enrol", post(api::enrol_agent))
        .route(
            "/v1/node-connect-sessions/preview",
            post(api::preview_node_connect_session),
        )
        .layer(middleware::from_fn_with_state(
            limiter,
            rate_limit::middleware,
        ))
}

fn workos_identity_router() -> Router<AppState> {
    Router::new()
        .route(
            "/v1/integrations/workos/webhook",
            post(workos_identity::webhook),
        )
        .layer(DefaultBodyLimit::max(1024 * 1024))
}

fn node_operator_router() -> Router<AppState> {
    Router::new()
        .route("/v1/nodes", get(api::list_agents))
        .route(
            "/v1/nodes/runtime-observations",
            get(destination_topology::list_node_runtime_observations),
        )
        .route(
            "/v1/nodes/{node_id}",
            get(api::get_node)
                .patch(api::patch_node)
                .delete(api::delete_node),
        )
        .route("/v1/nodes/{node_id}/pause", post(api::pause_node))
        .route("/v1/nodes/{node_id}/resume", post(api::resume_node))
        .route(
            "/v1/nodes/{node_id}/runtime",
            get(destination_topology::get_node_runtime),
        )
        .route(
            "/v1/nodes/{node_id}/wake-hints",
            get(destination_topology::list_node_wake_hints)
                .post(destination_topology::create_node_wake_hint),
        )
        .route(
            "/v1/nodes/{node_id}/diagnostics",
            get(api::list_node_diagnostics).post(api::request_node_diagnostics),
        )
        .route(
            "/v1/nodes/{node_id}/diagnostics/{request_id}",
            get(api::get_node_diagnostic),
        )
        .route(
            "/v1/nodes/{node_id}/update",
            get(updates::get).post(updates::request),
        )
        .route(
            "/v1/nodes/{node_id}/update-policy",
            axum::routing::patch(updates::patch_policy),
        )
        .route("/v1/nodes/{node_id}/rollback", post(updates::rollback))
}

fn compatibility_router() -> Router<AppState> {
    Router::new()
        .route("/whoami", get(compatibility::whoami))
        .route("/ping", get(compatibility::ping))
        .route("/noop", get(compatibility::noop))
        .route(
            "/printjobs",
            post(compatibility::create_print_job)
                .get(compatibility::list_print_jobs)
                .delete(compatibility::cancel_print_jobs),
        )
        .route(
            "/printjobs/states",
            get(compatibility::get_print_job_states),
        )
        .route(
            "/printjobs/{set}",
            get(compatibility::get_print_jobs).delete(compatibility::cancel_print_job_set),
        )
        .route(
            "/printjobs/{set}/states",
            get(compatibility::get_print_job_states),
        )
        .route("/computers", get(compatibility::list_computers))
        .route("/computers/{set}", get(compatibility::get_computers))
        .route(
            "/computers/{computer_set}/printers",
            get(compatibility::get_computer_printers),
        )
        .route(
            "/computers/{computer_set}/printers/{printer_set}",
            get(compatibility::get_computer_printer_set),
        )
        .route("/printers", get(compatibility::list_printers))
        .route("/printers/{set}", get(compatibility::get_printers))
        .route(
            "/printers/{printer_set}/printjobs",
            get(compatibility::get_printer_print_jobs)
                .delete(compatibility::cancel_printer_print_jobs),
        )
        .route(
            "/printers/{printer_set}/printjobs/{job_set}",
            get(compatibility::get_printer_print_job_set)
                .delete(compatibility::cancel_printer_print_job_set),
        )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;
    use crate::{
        authentication::{StaticAuthenticator, TenantContext},
        repository::{MemoryRepository, Repository},
    };
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use base64::{Engine, engine::general_purpose::STANDARD_NO_PAD};
    use bytes::Bytes;
    use chrono::Utc;
    use ed25519_dalek::{Signer, SigningKey};
    use http_body_util::BodyExt;
    use piqae_auth::Scope;
    use piqae_domain::{
        AgentId, DriverFingerprint, EnvironmentId, JobId, JobOptions, JobState,
        NativePrinterChoice, NativePrinterOption, NativeProfileKind, PrinterCapabilities,
        PrinterId, PrinterState, ProfileStatus, ProfileSummary, SafeProfileOverride, WorkspaceId,
    };
    use piqae_object_store::{ObjectStoreError, StoredObject};
    use piqae_protocol::agent::{
        AgentAcceptJobRequest, AgentCommand, AgentHealth, AgentSyncRequest, AgentSyncResponse,
        NodeAvailability, NodeAvailabilityClass, NodeHostMode, NodeRuntimeObservation,
        PrinterProfileSnapshot, PrinterRouteSnapshot, PrinterSnapshot, PrivacySafeQueueObservation,
        QueueSnapshot, RouteObservation as AgentRouteObservation,
    };
    use rand::rngs::OsRng;
    use sha2::{Digest, Sha256};
    use std::{collections::BTreeMap, str::FromStr};
    use tower::ServiceExt;

    struct TestApplication {
        router: Router,
        state: AppState,
        repository: MemoryRepository,
        printer_id: PrinterId,
        agent_id: AgentId,
        signing_key: SigningKey,
        tenant: TenantContext,
    }

    #[derive(Debug)]
    struct UnavailableObjectStore;

    #[async_trait::async_trait]
    impl ObjectStore for UnavailableObjectStore {
        async fn put(
            &self,
            _key: &str,
            _content: Bytes,
            _expected_sha256: Option<&str>,
        ) -> Result<StoredObject, ObjectStoreError> {
            Err(ObjectStoreError::S3("unavailable".into()))
        }

        async fn get(&self, _key: &str) -> Result<Bytes, ObjectStoreError> {
            Err(ObjectStoreError::S3("unavailable".into()))
        }

        async fn put_stream(
            &self,
            _key: &str,
            _content: piqae_object_store::ObjectByteStream,
            _expected_sha256: &str,
            _expected_bytes: u64,
        ) -> Result<StoredObject, ObjectStoreError> {
            Err(ObjectStoreError::S3("unavailable".into()))
        }

        async fn get_stream(
            &self,
            _key: &str,
        ) -> Result<piqae_object_store::ObjectByteStream, ObjectStoreError> {
            Err(ObjectStoreError::S3("unavailable".into()))
        }

        async fn delete(&self, _key: &str) -> Result<(), ObjectStoreError> {
            Err(ObjectStoreError::S3("unavailable".into()))
        }

        async fn exists(&self, _key: &str) -> Result<bool, ObjectStoreError> {
            Err(ObjectStoreError::S3("unavailable".into()))
        }
    }

    async fn application() -> TestApplication {
        let repository = MemoryRepository::default();
        let authenticator = StaticAuthenticator::default();
        let tenant = TenantContext::unrestricted(WorkspaceId::new(), EnvironmentId::new());
        let printer_id = PrinterId::new();
        let agent_id = AgentId::new();
        let signing_key = SigningKey::generate(&mut OsRng);
        repository
            .add_printer(
                tenant.workspace_id,
                tenant.environment_id,
                printer_id,
                agent_id,
            )
            .await;
        repository
            .set_agent_public_key(agent_id, signing_key.verifying_key().to_bytes().to_vec())
            .await;
        authenticator.insert("piq_test_integration", tenant).await;
        authenticator
            .insert(
                "piq_test_other",
                TenantContext::unrestricted(WorkspaceId::new(), EnvironmentId::new()),
            )
            .await;
        let state = AppState::new_for_tests(Arc::new(repository.clone()), Arc::new(authenticator));
        TestApplication {
            router: router(state.clone()),
            state,
            repository,
            printer_id,
            agent_id,
            signing_key,
            tenant,
        }
    }

    async fn api_key_application(
        principals: &[(&str, TenantContext)],
    ) -> (Router, MemoryRepository) {
        let repository = MemoryRepository::default();
        let authenticator = StaticAuthenticator::default();
        for (token, tenant) in principals {
            authenticator.insert(token, *tenant).await;
        }
        (
            router(AppState::new_for_tests(
                Arc::new(repository.clone()),
                Arc::new(authenticator),
            )),
            repository,
        )
    }

    fn api_request(method: &str, path: &str, token: &str, body: Option<&str>) -> Request<Body> {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("authorization", format!("Bearer {token}"));
        if body.is_some() {
            request = request.header("content-type", "application/json");
        }
        request
            .body(body.map_or_else(Body::empty, |value| Body::from(value.to_owned())))
            .expect("valid API request")
    }

    fn idempotent_api_request(
        method: &str,
        path: &str,
        token: &str,
        key: &str,
        body: Option<&str>,
    ) -> Request<Body> {
        let mut request = api_request(method, path, token, body);
        request
            .headers_mut()
            .insert("idempotency-key", key.parse().expect("idempotency header"));
        request
    }

    fn compatibility_request(method: &str, path: &str, body: Option<String>) -> Request<Body> {
        let credentials = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            "piq_test_integration:",
        );
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("authorization", format!("Basic {credentials}"));
        if body.is_some() {
            request = request.header("content-type", "application/json");
        }
        request
            .body(body.map_or_else(Body::empty, Body::from))
            .expect("valid compatibility request")
    }

    async fn compatibility_json(
        router: &Router,
        request: Request<Body>,
    ) -> (StatusCode, serde_json::Value) {
        let response = router
            .clone()
            .oneshot(request)
            .await
            .expect("compatibility response");
        let status = response.status();
        let body = response
            .into_body()
            .collect()
            .await
            .expect("compatibility body")
            .to_bytes();
        let json = serde_json::from_slice(&body).expect("compatibility JSON");
        (status, json)
    }

    fn signed_request(
        application: &TestApplication,
        method: &str,
        path: &str,
        body: Vec<u8>,
    ) -> Request<Body> {
        signed_request_at(
            application,
            method,
            path,
            body,
            Utc::now().timestamp_millis(),
        )
    }

    fn signed_request_at(
        application: &TestApplication,
        method: &str,
        path: &str,
        body: Vec<u8>,
        timestamp: i64,
    ) -> Request<Body> {
        let nonce = uuid::Uuid::new_v4();
        let digest = format!("{:x}", Sha256::digest(&body));
        let canonical = format!("{method}\n{path}\n{timestamp}\n{nonce}\n{digest}");
        let signature = application.signing_key.sign(canonical.as_bytes());
        Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json")
            .header("x-piqae-agent-id", application.agent_id.to_string())
            .header("x-piqae-timestamp", timestamp.to_string())
            .header("x-piqae-nonce", nonce.to_string())
            .header("x-piqae-body-sha256", digest)
            .header(
                "x-piqae-signature",
                STANDARD_NO_PAD.encode(signature.to_bytes()),
            )
            .body(Body::from(body))
            .expect("valid signed request")
    }

    #[tokio::test]
    #[allow(
        clippy::too_many_lines,
        reason = "one route test proves validation, CAS, idempotency, tenant isolation, and event repair"
    )]
    async fn connector_identity_update_is_revision_fenced_idempotent_and_operator_safe() {
        let application = application().await;
        let oversized = serde_json::to_vec(&serde_json::json!({
            "expected_revision": 1,
            "display_name": "Dispatch Mac",
            "site": "é".repeat(61),
            "location": null,
            "labels": []
        }))
        .expect("oversized identity body");
        let rejected = application
            .router
            .clone()
            .oneshot(signed_request(
                &application,
                "PUT",
                "/v1/agent/identity",
                oversized,
            ))
            .await
            .expect("invalid identity response");
        assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);
        let rejected: serde_json::Value = serde_json::from_slice(
            &rejected
                .into_body()
                .collect()
                .await
                .expect("body")
                .to_bytes(),
        )
        .expect("invalid identity JSON");
        assert_eq!(rejected["error"]["code"], "invalid_node_identity");

        let first = serde_json::json!({
            "expected_revision": 1,
            "display_name": "Dispatch Mac",
            "site": "Warehouse",
            "location": "Desk 2",
            "labels": ["shipping"]
        });
        let first_body = serde_json::to_vec(&first).expect("identity body");
        let response = application
            .router
            .clone()
            .oneshot(signed_request(
                &application,
                "PUT",
                "/v1/agent/identity",
                first_body.clone(),
            ))
            .await
            .expect("identity response");
        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value = serde_json::from_slice(
            &response
                .into_body()
                .collect()
                .await
                .expect("body")
                .to_bytes(),
        )
        .expect("identity JSON");
        assert_eq!(body["revision"], 2);

        let replay = application
            .router
            .clone()
            .oneshot(signed_request(
                &application,
                "PUT",
                "/v1/agent/identity",
                first_body,
            ))
            .await
            .expect("identity replay response");
        assert_eq!(replay.status(), StatusCode::OK);
        let persisted = application
            .repository
            .get_agent(
                application.tenant.workspace_id,
                application.tenant.environment_id,
                application.agent_id,
            )
            .await
            .expect("agent");
        assert_eq!(persisted.identity_revision, 2);
        let events = application
            .repository
            .list_tenant_events(
                application.tenant.workspace_id,
                application.tenant.environment_id,
                None,
                100,
            )
            .await
            .expect("tenant events");
        assert_eq!(
            events
                .iter()
                .filter(|event| {
                    event.event_type == "node.updated"
                        && event.payload["identity_revision"] == serde_json::json!(2)
                })
                .count(),
            1,
            "an exact connector replay repairs a missing publish without duplicating the event"
        );

        let operator = application
            .router
            .clone()
            .oneshot(api_request(
                "PATCH",
                &format!("/v1/nodes/{}", application.agent_id),
                "piq_test_integration",
                Some(
                    &serde_json::json!({
                        "name": "Operator override",
                        "site": "Warehouse",
                        "location": "Desk 3",
                        "labels": ["shipping"],
                        "expected_revision": 2
                    })
                    .to_string(),
                ),
            ))
            .await
            .expect("operator response");
        let operator_status = operator.status();
        let operator_body = operator.into_body().collect().await.expect("operator body");
        assert_eq!(
            operator_status,
            StatusCode::OK,
            "{}",
            String::from_utf8_lossy(&operator_body.to_bytes())
        );

        let stale = serde_json::to_vec(&serde_json::json!({
            "expected_revision": 2,
            "display_name": "Stale local name",
            "site": null,
            "location": null,
            "labels": []
        }))
        .expect("stale body");
        let conflict = application
            .router
            .clone()
            .oneshot(signed_request(
                &application,
                "PUT",
                "/v1/agent/identity",
                stale,
            ))
            .await
            .expect("conflict response");
        assert_eq!(conflict.status(), StatusCode::CONFLICT);
        let conflict: serde_json::Value = serde_json::from_slice(
            &conflict
                .into_body()
                .collect()
                .await
                .expect("body")
                .to_bytes(),
        )
        .expect("conflict JSON");
        assert_eq!(conflict["error"]["code"], "node_identity_revision_conflict");
        assert_eq!(conflict["error"]["details"]["current_revision"], 3);
    }

    #[tokio::test]
    async fn a_node_with_a_drifting_clock_is_tolerated_then_told_the_server_time() {
        let application = application().await;
        let now = Utc::now();
        let body = serde_json::to_vec(&AgentSyncRequest {
            agent_id: application.agent_id,
            protocol_version: 1,
            agent_version: "test-clock-skew".into(),
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
                started_at: now,
                observed_at: now,
                sqlite_integrity_ok: true,
                executor_crashes: 3,
                last_error_code: Some("executor_crashed".into()),
            },
            printers: Some(vec![profiled_printer_snapshot(application.printer_id)]),
            events: Vec::new(),
            diagnostics: Vec::new(),
            document_render: piqae_protocol::agent::DocumentRenderCapabilities::default(),
            capabilities: piqae_protocol::agent::AgentProtocolCapabilities::default(),
            route_observations: Vec::new(),
            topology_changes: Vec::new(),
            native_handoffs: Vec::new(),
            runtime: None,
        })
        .expect("sync body");

        // Four minutes of drift is ordinary on a machine without NTP and must
        // not stop it printing.
        let tolerated = application
            .router
            .clone()
            .oneshot(signed_request_at(
                &application,
                "POST",
                "/v1/agent/sync",
                body.clone(),
                Utc::now().timestamp_millis() - 240_000,
            ))
            .await
            .expect("tolerated skew response");
        assert_eq!(tolerated.status(), StatusCode::OK);
        let agent = application
            .repository
            .get_agent(
                application.tenant.workspace_id,
                application.tenant.environment_id,
                application.agent_id,
            )
            .await
            .expect("persisted agent health");
        assert_eq!(agent.health_started_at, Some(now));
        assert_eq!(agent.health_observed_at, Some(now));
        assert_eq!(agent.sqlite_integrity_ok, Some(true));
        assert_eq!(agent.executor_crashes, 3);
        assert_eq!(agent.last_error_code.as_deref(), Some("executor_crashed"));

        // Beyond the window the request is refused — but the response still
        // carries the server clock, which is what lets the node self-correct
        // instead of failing forever.
        let rejected = application
            .router
            .clone()
            .oneshot(signed_request_at(
                &application,
                "POST",
                "/v1/agent/sync",
                body,
                Utc::now().timestamp_millis() - 600_000,
            ))
            .await
            .expect("rejected skew response");
        assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);
        let server_time = rejected
            .headers()
            .get("x-piqae-server-time")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<i64>().ok())
            .expect("server time on a rejected request");
        assert!(
            (server_time - Utc::now().timestamp_millis()).abs() < 60_000,
            "server time {server_time} is not the current server clock"
        );
    }

    #[tokio::test]
    #[allow(
        clippy::too_many_lines,
        reason = "the end-to-end assertion keeps wake, stale admission, fencing, and the public projection in one scenario"
    )]
    async fn wake_hint_never_leases_until_embedded_host_is_fresh_and_eligible() {
        let application = application().await;
        let created = application
            .router
            .clone()
            .oneshot(api_request(
                "POST",
                "/v1/jobs",
                "piq_test_integration",
                Some(
                    &serde_json::json!({
                        "printer_id": application.printer_id,
                        "title": "Wake-gated job",
                        "content_type": "pdf",
                        "content": {"type": "base64", "data": "cHJpbnQ="}
                    })
                    .to_string(),
                ),
            ))
            .await
            .expect("create wake-gated job");
        assert_eq!(created.status(), StatusCode::CREATED);
        let wake_worker = crate::wake_hint_worker::WakeHintWorker::new(application.state.clone());
        assert_eq!(
            wake_worker
                .run_once(10)
                .await
                .expect("publish automatic external wake"),
            1
        );

        let sync = |sequence, lifecycle_state, accepts_cloud_jobs| {
            let now = Utc::now();
            AgentSyncRequest {
                agent_id: application.agent_id,
                protocol_version: 1,
                agent_version: "embedded-test".into(),
                printer_revision: sequence,
                acknowledged_command_cursor: None,
                event_cursor: None,
                queue: QueueSnapshot {
                    queued_jobs: 0,
                    active_jobs: 0,
                    content_bytes: 0,
                    accepts_jobs: true,
                },
                health: AgentHealth {
                    started_at: now,
                    observed_at: now,
                    sqlite_integrity_ok: true,
                    executor_crashes: 0,
                    last_error_code: None,
                },
                printers: Some(vec![profiled_printer_snapshot(application.printer_id)]),
                events: Vec::new(),
                diagnostics: Vec::new(),
                document_render: piqae_protocol::agent::DocumentRenderCapabilities::default(),
                capabilities: piqae_protocol::agent::AgentProtocolCapabilities::default(),
                route_observations: vec![live_route_observation(application.printer_id, sequence)],
                topology_changes: Vec::new(),
                native_handoffs: Vec::new(),
                runtime: Some(NodeRuntimeObservation {
                    sequence,
                    host_mode: NodeHostMode::EmbeddedApplication,
                    availability_class: NodeAvailabilityClass::ForegroundOnly,
                    lifecycle_state,
                    accepts_cloud_jobs,
                    observed_at: now,
                    fresh_until: now + chrono::Duration::minutes(1),
                    execution_budget_ms: None,
                    wake_mechanisms: Vec::new(),
                }),
            }
        };
        let suspended = sync(1, NodeAvailability::Suspended, false);
        let suspended_response = sync_agent_request(&application, &suspended).await;
        assert!(suspended_response.candidate_jobs.is_empty());
        assert!(suspended_response.wake_hints.is_empty());

        let mut stale = sync(2, NodeAvailability::Foreground, true);
        if let Some(runtime) = &mut stale.runtime {
            runtime.observed_at = Utc::now() - chrono::Duration::minutes(2);
            runtime.fresh_until = Utc::now() - chrono::Duration::minutes(1);
        }
        let stale_response = sync_agent_request(&application, &stale).await;
        assert!(stale_response.candidate_jobs.is_empty());
        assert_eq!(stale_response.wake_hints.len(), 1);
        assert_eq!(
            stale_response.wake_hints[0].delivery_channel,
            piqae_protocol::agent::WakeDeliveryChannel::ExternalPush
        );

        let unsafe_suspended = sync(3, NodeAvailability::Suspended, true);
        let unsafe_suspended_response = sync_agent_request(&application, &unsafe_suspended).await;
        assert!(unsafe_suspended_response.candidate_jobs.is_empty());

        let mut foreground = sync(4, NodeAvailability::Foreground, true);
        foreground
            .runtime
            .as_mut()
            .expect("runtime fixture")
            .availability_class = NodeAvailabilityClass::WakeRelayCapable;
        let foreground_response = sync_agent_request(&application, &foreground).await;
        assert_eq!(foreground_response.candidate_jobs.len(), 1);
        assert!(foreground_response.wake_hints.is_empty());

        let runtime = application
            .router
            .clone()
            .oneshot(api_request(
                "GET",
                &format!("/v1/nodes/{}/runtime", application.agent_id),
                "piq_test_integration",
                None,
            ))
            .await
            .expect("read runtime projection");
        assert_eq!(runtime.status(), StatusCode::OK);
        let runtime_page = json_response(
            &application.router,
            api_request(
                "GET",
                "/v1/nodes/runtime-observations?limit=1",
                "piq_test_integration",
                None,
            ),
        )
        .await;
        assert_eq!(runtime_page["data"].as_array().map(Vec::len), Some(1));
        assert_eq!(
            runtime_page["data"][0]["node_id"],
            application.agent_id.to_string()
        );
        assert_eq!(
            runtime_page["data"][0]["availability_class"], "wake_relay_capable",
            "relay capability is persisted as telemetry without creating a trusted relay"
        );
        let cross_tenant_runtime = application
            .router
            .clone()
            .oneshot(api_request(
                "GET",
                &format!("/v1/nodes/{}/runtime", application.agent_id),
                "piq_test_other",
                None,
            ))
            .await
            .expect("cross-tenant runtime probe");
        assert_eq!(cross_tenant_runtime.status(), StatusCode::NOT_FOUND);
        let cross_tenant_page = json_response(
            &application.router,
            api_request(
                "GET",
                "/v1/nodes/runtime-observations",
                "piq_test_other",
                None,
            ),
        )
        .await;
        assert!(
            cross_tenant_page["data"]
                .as_array()
                .is_some_and(Vec::is_empty)
        );
    }

    #[tokio::test]
    #[allow(
        clippy::too_many_lines,
        reason = "the HTTP idempotency and content-free event assertions share one durable job fixture"
    )]
    async fn waiting_job_dispatches_one_content_free_wake_hint_across_idempotent_retries() {
        let application = application().await;
        let body = serde_json::json!({
            "printer_id": application.printer_id,
            "title": "private wake title",
            "content_type": "pdf",
            "content": {"type": "base64", "data": "cHJpbnQ="}
        })
        .to_string();
        let create = || {
            idempotent_api_request(
                "POST",
                "/v1/jobs",
                "piq_test_integration",
                "automatic-wake-test-0001",
                Some(&body),
            )
        };
        let first = application
            .router
            .clone()
            .oneshot(create())
            .await
            .expect("first create response");
        assert_eq!(first.status(), StatusCode::CREATED);

        let worker = crate::wake_hint_worker::WakeHintWorker::new(application.state.clone());
        assert_eq!(worker.run_once(10).await.expect("first wake dispatch"), 1);
        let events = application
            .repository
            .list_tenant_events(
                application.tenant.workspace_id,
                application.tenant.environment_id,
                None,
                100,
            )
            .await
            .expect("tenant events");
        let wakes = events
            .iter()
            .filter(|event| event.event_type == "node.wake_hint.requested")
            .collect::<Vec<_>>();
        assert_eq!(wakes.len(), 1);
        let wake = wakes[0].payload.as_object().expect("wake object");
        assert_eq!(
            wake.get("reason"),
            Some(&serde_json::json!("job_available"))
        );
        assert_eq!(
            wake.get("delivery_channel"),
            Some(&serde_json::json!("external_push"))
        );
        assert_eq!(
            wake.get("node_id"),
            Some(&serde_json::json!(application.agent_id.to_string()))
        );
        assert_eq!(
            wake.keys()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>(),
            [
                "delivery_channel",
                "expires_at",
                "id",
                "node_id",
                "observed_at",
                "reason",
                "requested_at",
                "status",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect()
        );
        assert!(!wakes[0].payload.to_string().contains("private wake title"));
        assert!(!wake.contains_key("job_id"));
        assert!(!wake.contains_key("title"));
        assert!(!wake.contains_key("content"));

        let retry = application
            .router
            .clone()
            .oneshot(create())
            .await
            .expect("idempotent retry response");
        assert_eq!(retry.status(), StatusCode::OK);
        assert_eq!(worker.run_once(10).await.expect("retry wake dispatch"), 0);
        let events = application
            .repository
            .list_tenant_events(
                application.tenant.workspace_id,
                application.tenant.environment_id,
                None,
                100,
            )
            .await
            .expect("tenant events after retry");
        assert_eq!(
            events
                .iter()
                .filter(|event| event.event_type == "node.wake_hint.requested")
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn pairing_completes_without_the_device_code_entering_a_request_path() {
        let application = application().await;
        let created = json_response(
            &application.router,
            Request::builder()
                .method("POST")
                .uri("/v1/device-authorizations")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "public_key": STANDARD_NO_PAD.encode([7_u8; 32]),
                        "installation_id": "installation-1",
                        "proposed_name": "Packing room",
                        "hostname": "packing-1",
                        "platform": "linux",
                        "architecture": "x86_64",
                        "installation_mode": "user",
                        "agent_version": "0.1.0",
                        "protocol_version": 1,
                    })
                    .to_string(),
                ))
                .expect("create request"),
        )
        .await;
        let device_code = created["device_code"].as_str().expect("device code");
        let authorization_id = created["id"].as_str().expect("authorization id");
        let user_code = created["user_code"].as_str().expect("user code");

        let pending = json_response(
            &application.router,
            api_request(
                "POST",
                "/v1/device-authorizations/status",
                "",
                Some(&serde_json::json!({ "device_code": device_code }).to_string()),
            ),
        )
        .await;
        assert_eq!(pending["state"], "pending");

        let approved = application
            .router
            .clone()
            .oneshot(api_request(
                "POST",
                &format!("/v1/device-authorizations/{authorization_id}/approve"),
                "piq_test_integration",
                Some(&serde_json::json!({ "user_code": user_code }).to_string()),
            ))
            .await
            .expect("approve response");
        assert_eq!(approved.status(), StatusCode::OK);

        let exchanged = json_response(
            &application.router,
            api_request(
                "POST",
                "/v1/device-authorizations/exchange",
                "",
                Some(&serde_json::json!({ "device_code": device_code }).to_string()),
            ),
        )
        .await;
        assert!(exchanged["node_id"].is_string(), "{exchanged}");
    }

    #[tokio::test]
    async fn a_rotation_tells_the_approver_which_node_it_replaces() {
        let application = application().await;
        let pair = async |installation: &str| {
            let created = json_response(
                &application.router,
                Request::builder()
                    .method("POST")
                    .uri("/v1/device-authorizations")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "public_key": STANDARD_NO_PAD.encode([7_u8; 32]),
                            "installation_id": installation,
                            "proposed_name": "Packing room",
                            "hostname": "packing-1",
                            "platform": "linux",
                            "architecture": "x86_64",
                            "installation_mode": "user",
                            "agent_version": "0.1.0",
                            "protocol_version": 1,
                        })
                        .to_string(),
                    ))
                    .expect("create request"),
            )
            .await;
            (
                created["id"].as_str().unwrap_or_default().to_owned(),
                created["user_code"].as_str().unwrap_or_default().to_owned(),
                created["device_code"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
            )
        };
        let review = async |authorization_id: &str| {
            json_response(
                &application.router,
                api_request(
                    "GET",
                    &format!("/v1/device-authorizations/{authorization_id}/review"),
                    "piq_test_integration",
                    None,
                ),
            )
            .await
        };

        // First pairing of an installation admits a new node.
        let (first_id, first_code, first_device_code) = pair("installation-rotating").await;
        assert!(review(&first_id).await["replaces_node_id"].is_null());
        let approved = application
            .router
            .clone()
            .oneshot(api_request(
                "POST",
                &format!("/v1/device-authorizations/{first_id}/approve"),
                "piq_test_integration",
                Some(&serde_json::json!({ "user_code": first_code }).to_string()),
            ))
            .await
            .expect("approve response");
        assert_eq!(approved.status(), StatusCode::OK);
        let node = json_response(
            &application.router,
            api_request(
                "POST",
                "/v1/device-authorizations/exchange",
                "",
                Some(&serde_json::json!({ "device_code": first_device_code }).to_string()),
            ),
        )
        .await;
        let node_id = node["node_id"].as_str().expect("node id").to_owned();

        // Pairing the same installation again is a key rotation, and the
        // approver must be told whose key they are retiring.
        let (second_id, _, _) = pair("installation-rotating").await;
        assert_eq!(
            review(&second_id).await["replaces_node_id"]
                .as_str()
                .expect("replaced node"),
            node_id
        );
    }

    #[tokio::test]
    async fn unauthenticated_pairing_creation_is_rate_limited() {
        let application = application().await;
        let create = || {
            Request::builder()
                .method("POST")
                .uri("/v1/device-authorizations")
                .header("content-type", "application/json")
                .header("x-forwarded-for", "203.0.113.9")
                .body(Body::from(
                    serde_json::json!({
                        "public_key": STANDARD_NO_PAD.encode([7_u8; 32]),
                        "installation_id": "installation-1",
                        "proposed_name": "Packing room",
                        "hostname": "packing-1",
                        "platform": "linux",
                        "architecture": "x86_64",
                        "installation_mode": "user",
                        "agent_version": "0.1.0",
                        "protocol_version": 1,
                    })
                    .to_string(),
                ))
                .expect("create request")
        };
        let mut statuses = Vec::new();
        for _ in 0..12 {
            statuses.push(
                application
                    .router
                    .clone()
                    .oneshot(create())
                    .await
                    .expect("create response")
                    .status(),
            );
        }
        assert!(
            statuses.contains(&StatusCode::TOO_MANY_REQUESTS),
            "unauthenticated row creation was never throttled: {statuses:?}"
        );
    }

    async fn json_response(router: &Router, request: Request<Body>) -> serde_json::Value {
        let response = router.clone().oneshot(request).await.expect("response");
        let status = response.status();
        let body = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        assert!(
            status.is_success(),
            "unexpected {status}: {}",
            String::from_utf8_lossy(&body)
        );
        serde_json::from_slice(&body).expect("JSON body")
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn document_template_publish_and_render_is_tenant_scoped_end_to_end() {
        let application = application().await;
        let template = json_response(
            &application.router,
            idempotent_api_request(
                "POST",
                "/v1/business-document-templates",
                "piq_test_integration",
                "template-receipt-v1",
                Some(
                    &serde_json::json!({
                        "name": "Receipt",
                        "specification": {"format":"piqae.business-document/v1","media":{"kind":"paged","size":"a4"},
                            "body":[{"type":"paragraph","content":[{"type":"value","value":{"type":"path","path":["number"]}}]}]}
                    })
                    .to_string(),
                ),
            ),
        )
        .await;
        let template_id = template["id"].as_str().expect("template id");
        let revision = json_response(
            &application.router,
            idempotent_api_request(
                "POST",
                &format!("/v1/business-document-templates/{template_id}/publish"),
                "piq_test_integration",
                "publish-receipt-v1",
                Some(&serde_json::json!({"specification": template["specification"]}).to_string()),
            ),
        )
        .await;
        let revision_id = revision["id"].as_str().expect("revision id");
        let replay = json_response(
            &application.router,
            idempotent_api_request(
                "POST",
                &format!("/v1/business-document-templates/{template_id}/publish"),
                "piq_test_integration",
                "publish-receipt-v1",
                Some(&serde_json::json!({"specification": template["specification"]}).to_string()),
            ),
        )
        .await;
        assert_eq!(replay["specification"], revision["specification"]);
        let mismatch = application
            .router
            .clone()
            .oneshot(idempotent_api_request(
                "POST",
                &format!("/v1/business-document-templates/{template_id}/publish"),
                "piq_test_integration",
                "publish-receipt-v1",
                Some(
                    &serde_json::json!({"specification": {
                        "format":"piqae.business-document/v1","media":{"kind":"paged","size":"a4"},
                        "body":[{"type":"paragraph","content":[{"type":"text","value":"different"}]}]
                    }})
                    .to_string(),
                ),
            ))
            .await
            .expect("mismatched replay response");
        assert_eq!(mismatch.status(), StatusCode::CONFLICT);
        let render = json_response(
            &application.router,
            idempotent_api_request(
                "POST",
                "/v1/business-document-renders",
                "piq_test_integration",
                "render-receipt-1042",
                Some(
                    &serde_json::json!({
                        "template_revision_id": revision_id, "input":{"number":"R-1042"},
                    })
                    .to_string(),
                ),
            ),
        )
        .await;
        assert_eq!(render["state"], "registered");
        for private_field in [
            "input_ciphertext",
            "input_sha256",
            "artifact_object_key_ciphertext",
            "attempt",
            "max_attempts",
            "lease_token",
            "lease_expires_at",
        ] {
            assert!(render.get(private_field).is_none());
        }
        let render_replay = application
            .router
            .clone()
            .oneshot(idempotent_api_request(
                "POST",
                "/v1/business-document-renders",
                "piq_test_integration",
                "render-receipt-1042",
                Some(
                    &serde_json::json!({
                        "template_revision_id": revision_id, "input":{"number":"R-1042"},
                    })
                    .to_string(),
                ),
            ))
            .await
            .expect("render replay response");
        assert_eq!(render_replay.status(), StatusCode::OK);
        let not_ready = application
            .router
            .clone()
            .oneshot(api_request(
                "GET",
                &format!(
                    "/v1/business-document-renders/{}/artifact",
                    render["id"].as_str().expect("render id")
                ),
                "piq_test_integration",
                None,
            ))
            .await
            .expect("not-ready artifact response");
        assert_eq!(not_ready.status(), StatusCode::CONFLICT);
        let worker = crate::document_render_worker::DocumentRenderWorker::new(
            application.state.clone(),
            "test-worker",
        );
        assert_eq!(worker.run_once(1).await.expect("render batch"), 1);
        let render = json_response(
            &application.router,
            api_request(
                "GET",
                &format!(
                    "/v1/business-document-renders/{}",
                    render["id"].as_str().expect("render id")
                ),
                "piq_test_integration",
                None,
            ),
        )
        .await;
        assert_eq!(render["state"], "completed");
        assert_eq!(render["artifact_media_type"], "application/pdf");
        assert!(render.get("artifact_object_key_ciphertext").is_none());
        assert!(render.get("input_ciphertext").is_none());
        assert!(
            render["artifact_byte_length"]
                .as_i64()
                .is_some_and(|bytes| bytes > 0)
        );
        let held_download_permits = application
            .state
            .document_artifact_downloads
            .clone()
            .try_acquire_many_owned(4)
            .expect("hold bounded artifact buffers");
        let busy = application
            .router
            .clone()
            .oneshot(api_request(
                "GET",
                &format!(
                    "/v1/business-document-renders/{}/artifact",
                    render["id"].as_str().expect("render id")
                ),
                "piq_test_integration",
                None,
            ))
            .await
            .expect("busy artifact response");
        assert_eq!(busy.status(), StatusCode::SERVICE_UNAVAILABLE);
        drop(held_download_permits);
        let artifact = application
            .router
            .clone()
            .oneshot(api_request(
                "GET",
                &format!(
                    "/v1/business-document-renders/{}/artifact",
                    render["id"].as_str().expect("render id")
                ),
                "piq_test_integration",
                None,
            ))
            .await
            .expect("artifact response");
        assert_eq!(artifact.status(), StatusCode::OK);
        assert_eq!(
            artifact
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok()),
            Some("application/pdf")
        );
        assert_eq!(
            artifact
                .headers()
                .get("content-disposition")
                .and_then(|v| v.to_str().ok()),
            Some("attachment; filename=\"document.pdf\"")
        );
        let digest_header = artifact
            .headers()
            .get("digest")
            .and_then(|value| value.to_str().ok())
            .expect("standards-compatible digest")
            .to_owned();
        let artifact_body = artifact
            .into_body()
            .collect()
            .await
            .expect("PDF body")
            .to_bytes();
        assert!(artifact_body.starts_with(b"%PDF-"));
        assert_eq!(
            digest_header,
            format!(
                "sha-256={}",
                base64::engine::general_purpose::STANDARD
                    .encode(sha2::Sha256::digest(&artifact_body))
            )
        );
        let object_key = format!(
            "{}/{}/documents/{}.pdf",
            application.tenant.workspace_id,
            application.tenant.environment_id,
            render["id"].as_str().expect("render id")
        );
        let preview = json_response(
            &application.router,
            idempotent_api_request(
                "POST",
                &format!(
                    "/v1/business-document-renders/{}/previews",
                    render["id"].as_str().expect("render id")
                ),
                "piq_test_integration",
                "preview-receipt-1042",
                Some(r#"{"expires_in_seconds":600}"#),
            ),
        )
        .await;
        assert_eq!(preview["state"], "awaiting_approval");
        let preview_id = preview["id"].as_str().expect("preview id");
        let preview_artifact = application
            .router
            .clone()
            .oneshot(api_request(
                "GET",
                &format!("/v1/business-document-previews/{preview_id}/artifact"),
                "piq_test_integration",
                None,
            ))
            .await
            .expect("preview artifact response");
        assert_eq!(preview_artifact.status(), StatusCode::OK);
        let preview_bytes = preview_artifact
            .into_body()
            .collect()
            .await
            .expect("preview PDF body")
            .to_bytes();
        assert_eq!(preview_bytes, artifact_body);

        // Construct the public idempotency fixture from low-entropy components
        // so secret scanners do not mistake the non-secret example for a key.
        let approval_key = ["approve", "receipt", "1042"].join("-");
        let approval = json_response(
            &application.router,
            idempotent_api_request(
                "POST",
                &format!("/v1/business-document-previews/{preview_id}/approve"),
                "piq_test_integration",
                &approval_key,
                Some(
                    &serde_json::json!({
                        "printer_id": application.printer_id.to_string(),
                        "title": "Receipt R-1042"
                    })
                    .to_string(),
                ),
            ),
        )
        .await;
        assert_eq!(approval["preview"]["state"], "approved");
        let job = &approval["job"];
        assert_eq!(job["content_type"], "pdf");
        assert_eq!(job["state"], "waiting_for_agent");
        let approval_replay = json_response(
            &application.router,
            idempotent_api_request(
                "POST",
                &format!("/v1/business-document-previews/{preview_id}/approve"),
                "piq_test_integration",
                &approval_key,
                Some(
                    &serde_json::json!({
                        "printer_id": application.printer_id.to_string(),
                        "title": "Receipt R-1042"
                    })
                    .to_string(),
                ),
            ),
        )
        .await;
        assert_eq!(approval_replay["job"]["id"], job["id"]);

        // Printing acquires a durable upload reference to the exact immutable
        // preview object; it must not copy or regenerate the PDF.
        let acquisition_sha256 = hex::encode(sha2::Sha256::digest(
            [
                render["id"].as_str().expect("render id").as_bytes(),
                b"\0",
                approval_key.as_bytes(),
            ]
            .concat(),
        ));
        let artifact_upload = application
            .repository
            .get_upload(
                application.tenant.workspace_id,
                application.tenant.environment_id,
                &format!("dua_{acquisition_sha256}"),
            )
            .await
            .expect("zero-copy artifact upload");
        assert_eq!(artifact_upload.object_key, object_key);
        assert_eq!(
            artifact_upload.expected_sha256,
            render["artifact_sha256"]
                .as_str()
                .expect("render artifact digest")
        );
        assert_eq!(
            artifact_upload.expected_bytes,
            i64::try_from(artifact_body.len()).expect("bounded artifact bytes")
        );

        let now = Utc::now();
        let sync = AgentSyncRequest {
            agent_id: application.agent_id,
            protocol_version: 1,
            agent_version: "virtual-document-node".into(),
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
                started_at: now,
                observed_at: now,
                sqlite_integrity_ok: true,
                executor_crashes: 0,
                last_error_code: None,
            },
            printers: Some(vec![profiled_printer_snapshot(application.printer_id)]),
            events: Vec::new(),
            diagnostics: Vec::new(),
            document_render: piqae_protocol::agent::DocumentRenderCapabilities::default(),
            capabilities: piqae_protocol::agent::AgentProtocolCapabilities::default(),
            route_observations: vec![live_route_observation(application.printer_id, 1)],
            topology_changes: Vec::new(),
            native_handoffs: Vec::new(),
            runtime: None,
        };
        let sync_response = application
            .router
            .clone()
            .oneshot(signed_request(
                &application,
                "POST",
                "/v1/agent/sync",
                serde_json::to_vec(&sync).expect("virtual node sync JSON"),
            ))
            .await
            .expect("virtual node sync response");
        assert_eq!(sync_response.status(), StatusCode::OK);
        let sync: AgentSyncResponse = serde_json::from_slice(
            &sync_response
                .into_body()
                .collect()
                .await
                .expect("virtual node sync body")
                .to_bytes(),
        )
        .expect("virtual node sync response JSON");
        let approved_job_id = job["id"]
            .as_str()
            .expect("approved job id")
            .parse::<piqae_domain::JobId>()
            .expect("typed approved job id");
        let offer = sync
            .candidate_jobs
            .iter()
            .find(|offer| offer.job.id == approved_job_id)
            .unwrap_or_else(|| {
                panic!(
                    "approved document {} was not offered; candidates={:?}",
                    job["id"],
                    sync.candidate_jobs
                        .iter()
                        .map(|offer| offer.job.id)
                        .collect::<Vec<_>>()
                )
            });
        let piqae_protocol::agent::ContentDescriptor::Download {
            sha256: offered_sha256,
            ..
        } = &offer.content
        else {
            panic!("business-document print must use immutable download content");
        };
        assert_eq!(offered_sha256, &artifact_upload.expected_sha256);
        let reservation = offer
            .route_reservation
            .as_ref()
            .expect("document offer is destination fenced");
        let accept = AgentAcceptJobRequest {
            lease_id: offer.lease_id,
            lease_token: offer.lease_token.clone(),
            content_sha256: offered_sha256.clone(),
            local_sequence: 1,
            route_reservation_id: Some(reservation.reservation_id),
            route_generation: Some(reservation.generation),
            route_fencing_token: Some(reservation.fencing_token.clone()),
        };
        let accepted = application
            .router
            .clone()
            .oneshot(signed_request(
                &application,
                "POST",
                &format!("/v1/agent/jobs/{}/accept", offer.job.id),
                serde_json::to_vec(&accept).expect("virtual acceptance JSON"),
            ))
            .await
            .expect("virtual acceptance response");
        assert_eq!(accepted.status(), StatusCode::OK);
        assert_eq!(
            application
                .repository
                .get_job(
                    application.tenant.workspace_id,
                    application.tenant.environment_id,
                    offer.job.id,
                )
                .await
                .expect("accepted virtual document job")
                .state,
            piqae_domain::JobState::AgentAccepted
        );
        let probe = application
            .router
            .clone()
            .oneshot(api_request(
                "GET",
                &format!(
                    "/v1/business-document-renders/{}",
                    render["id"].as_str().expect("render id")
                ),
                "piq_test_other",
                None,
            ))
            .await
            .expect("cross tenant probe");
        assert_eq!(probe.status(), StatusCode::NOT_FOUND);
        let artifact_probe = application
            .router
            .clone()
            .oneshot(api_request(
                "GET",
                &format!(
                    "/v1/business-document-renders/{}/artifact",
                    render["id"].as_str().expect("render id")
                ),
                "piq_test_other",
                None,
            ))
            .await
            .expect("cross tenant artifact probe");
        assert_eq!(artifact_probe.status(), StatusCode::NOT_FOUND);
        application
            .state
            .object_store
            .put(
                &object_key,
                Bytes::from(vec![0_u8; artifact_body.len()]),
                None,
            )
            .await
            .expect("replace fixture with same-length corrupt artifact");
        let corrupt = application
            .router
            .clone()
            .oneshot(api_request(
                "GET",
                &format!(
                    "/v1/business-document-renders/{}/artifact",
                    render["id"].as_str().expect("render id")
                ),
                "piq_test_integration",
                None,
            ))
            .await
            .expect("corrupt artifact response");
        assert_eq!(corrupt.status(), StatusCode::SERVICE_UNAVAILABLE);
        application
            .state
            .object_store
            .delete(&object_key)
            .await
            .expect("delete fixture artifact");
        let missing = application
            .router
            .clone()
            .oneshot(api_request(
                "GET",
                &format!(
                    "/v1/business-document-renders/{}/artifact",
                    render["id"].as_str().expect("render id")
                ),
                "piq_test_integration",
                None,
            ))
            .await
            .expect("missing artifact response");
        assert_eq!(missing.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn readiness_accepts_a_missing_health_object() {
        let application = application().await;
        let response = application
            .router
            .oneshot(api_request("GET", "/v1/ready", "", None))
            .await
            .expect("readiness response");
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn platform_enablement_is_one_time_workspace_scoped_and_not_cacheable() {
        let application = application().await;
        let before = json_response(
            &application.router,
            api_request("GET", "/v1/platform/status", "piq_test_integration", None),
        )
        .await;
        assert_eq!(before["enabled"], false);

        let response = application
            .router
            .clone()
            .oneshot(api_request(
                "POST",
                "/v1/platform/enable",
                "piq_test_integration",
                None,
            ))
            .await
            .expect("enablement response");
        assert_eq!(response.status(), StatusCode::CREATED);
        assert_eq!(
            response
                .headers()
                .get("cache-control")
                .and_then(|value| value.to_str().ok()),
            Some("no-store")
        );
        assert_eq!(
            response
                .headers()
                .get("pragma")
                .and_then(|value| value.to_str().ok()),
            Some("no-cache")
        );
        let body = response
            .into_body()
            .collect()
            .await
            .expect("enablement body")
            .to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&body).expect("enablement JSON");
        assert_eq!(body["enabled"], true);
        assert!(
            body["secret"]
                .as_str()
                .is_some_and(|secret| secret.starts_with("piq_platform_"))
        );

        let after = json_response(
            &application.router,
            api_request("GET", "/v1/platform/status", "piq_test_integration", None),
        )
        .await;
        assert_eq!(after["enabled"], true);

        let repeated = application
            .router
            .clone()
            .oneshot(api_request(
                "POST",
                "/v1/platform/enable",
                "piq_test_integration",
                None,
            ))
            .await
            .expect("repeat enablement response");
        assert_eq!(repeated.status(), StatusCode::CONFLICT);

        let metadata = json_response(
            &application.router,
            api_request(
                "GET",
                "/v1/platform/credential",
                "piq_test_integration",
                None,
            ),
        )
        .await;
        assert_eq!(metadata["name"], "Piqae platform integration");
        assert!(metadata.get("secret").is_none());
        assert!(
            metadata["lookup_prefix"]
                .as_str()
                .is_some_and(|prefix| prefix.starts_with("piq_platform_"))
        );

        let rotated = application
            .router
            .clone()
            .oneshot(api_request(
                "POST",
                "/v1/platform/credential",
                "piq_test_integration",
                None,
            ))
            .await
            .expect("rotation response");
        assert_eq!(rotated.status(), StatusCode::OK);
        assert_eq!(
            rotated
                .headers()
                .get("cache-control")
                .and_then(|value| value.to_str().ok()),
            Some("no-store")
        );
        let rotated_body = rotated
            .into_body()
            .collect()
            .await
            .expect("rotation body")
            .to_bytes();
        let rotated_body: serde_json::Value =
            serde_json::from_slice(&rotated_body).expect("rotation JSON");
        assert!(
            rotated_body["secret"]
                .as_str()
                .is_some_and(|secret| secret.starts_with("piq_platform_"))
        );

        let revoked = application
            .router
            .clone()
            .oneshot(api_request(
                "DELETE",
                "/v1/platform/credential",
                "piq_test_integration",
                None,
            ))
            .await
            .expect("revoke response");
        assert_eq!(revoked.status(), StatusCode::NO_CONTENT);
        let after_revoke = json_response(
            &application.router,
            api_request("GET", "/v1/platform/status", "piq_test_integration", None),
        )
        .await;
        assert_eq!(after_revoke["enabled"], false);
    }

    #[tokio::test]
    async fn object_store_failure_blocks_readiness_but_not_liveness() {
        let state = AppState::new_with_resources(
            Arc::new(MemoryRepository::default()),
            Arc::new(StaticAuthenticator::default()),
            [0; 32],
            document_crypto::DocumentSecretBox::new([0; 32]),
            Arc::new(UnavailableObjectStore),
        );
        let application = router(state);
        let health = application
            .clone()
            .oneshot(api_request("GET", "/v1/health", "", None))
            .await
            .expect("health response");
        assert_eq!(health.status(), StatusCode::OK);
        let ready = application
            .oneshot(api_request("GET", "/v1/ready", "", None))
            .await
            .expect("readiness response");
        assert_eq!(ready.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    async fn sync_agent_request(
        application: &TestApplication,
        request: &AgentSyncRequest,
    ) -> AgentSyncResponse {
        let body = serde_json::to_vec(request).expect("sync JSON");
        let response = application
            .router
            .clone()
            .oneshot(signed_request(application, "POST", "/v1/agent/sync", body))
            .await
            .expect("sync response");
        assert_eq!(response.status(), StatusCode::OK);
        serde_json::from_slice(
            &response
                .into_body()
                .collect()
                .await
                .expect("sync body")
                .to_bytes(),
        )
        .expect("sync response JSON")
    }

    async fn sync_test_agent(
        application: &TestApplication,
        acknowledged_command_cursor: Option<String>,
    ) -> AgentSyncResponse {
        let now = Utc::now();
        let request = AgentSyncRequest {
            agent_id: application.agent_id,
            protocol_version: 1,
            agent_version: "test".into(),
            printer_revision: 1,
            acknowledged_command_cursor,
            event_cursor: None,
            queue: QueueSnapshot {
                queued_jobs: 0,
                active_jobs: 0,
                content_bytes: 0,
                accepts_jobs: false,
            },
            health: AgentHealth {
                started_at: now,
                observed_at: now,
                sqlite_integrity_ok: true,
                executor_crashes: 0,
                last_error_code: None,
            },
            printers: Some(vec![profiled_printer_snapshot(application.printer_id)]),
            events: Vec::new(),
            diagnostics: Vec::new(),
            document_render: piqae_protocol::agent::DocumentRenderCapabilities::default(),
            capabilities: piqae_protocol::agent::AgentProtocolCapabilities::default(),
            route_observations: Vec::new(),
            topology_changes: Vec::new(),
            native_handoffs: Vec::new(),
            runtime: None,
        };
        sync_agent_request(application, &request).await
    }

    #[allow(clippy::too_many_lines)]
    fn profiled_printer_snapshot(printer_id: PrinterId) -> PrinterSnapshot {
        let mut native_options = BTreeMap::new();
        native_options.insert(
            "StapleLocation".into(),
            NativePrinterOption {
                display_name: "Staple".into(),
                default_choice: Some("None".into()),
                selected_choice: Some("None".into()),
                choices: vec![
                    NativePrinterChoice {
                        value: "None".into(),
                        display_name: "Off".into(),
                    },
                    NativePrinterChoice {
                        value: "UpperLeft".into(),
                        display_name: "Upper left".into(),
                    },
                ],
            },
        );
        let mut selected_native_options = BTreeMap::new();
        selected_native_options.insert("StapleLocation".into(), "UpperLeft".into());
        PrinterSnapshot {
            id: printer_id,
            native_id: "cups-office".into(),
            name: "Office Laser".into(),
            state: PrinterState::Online,
            is_default: true,
            capabilities: PrinterCapabilities {
                color: true,
                duplex: true,
                copies: 99,
                dpis: vec!["600".into()],
                papers: BTreeMap::from([("A4".into(), [Some(210_000), Some(297_000)])]),
                ..PrinterCapabilities::default()
            },
            exposed: true,
            capability_revision: 7,
            native_options,
            semantic_capabilities: piqae_domain::SemanticPrinterCapabilities {
                facets: BTreeMap::from([(
                    "finishing.staple".into(),
                    vec!["none".into(), "upper_left".into()],
                )]),
                native_resolutions: BTreeMap::from([(
                    "finishing.staple".into(),
                    BTreeMap::from([
                        (
                            "none".into(),
                            piqae_domain::SemanticNativeResolution {
                                native_option: "StapleLocation".into(),
                                native_choice: "None".into(),
                            },
                        ),
                        (
                            "upper_left".into(),
                            piqae_domain::SemanticNativeResolution {
                                native_option: "StapleLocation".into(),
                                native_choice: "UpperLeft".into(),
                            },
                        ),
                    ]),
                )]),
                support_pack: Some(piqae_domain::SupportPackProvenance {
                    pack_id: "pack.test.staple".into(),
                    digest_sha256: "a".repeat(64),
                    evidence: "replay_tested".into(),
                }),
            },
            profiles: vec![PrinterProfileSnapshot {
                profile_id: "profile_shipping".into(),
                revision: 4,
                name: "A4 shipping".into(),
                is_default: true,
                options: JobOptions {
                    paper: Some("A4".into()),
                    duplex: Some(piqae_domain::Duplex::LongEdge),
                    native_options: selected_native_options,
                    ..JobOptions::default()
                },
                status: ProfileStatus::Ready,
                native_kind: Some(NativeProfileKind::CupsOptions),
                native_digest: Some("sha256:test-profile".into()),
                driver_fingerprint: DriverFingerprint::default(),
                summary: ProfileSummary {
                    paper: Some("A4".into()),
                    ..ProfileSummary::default()
                },
                stock_id: Some("stk_shipping".into()),
                safe_overrides: vec![SafeProfileOverride::Copies, SafeProfileOverride::Pages],
                last_validated_unix_ms: None,
                last_test_job_id: None,
                published: true,
            }],
            route: Some(PrinterRouteSnapshot {
                local_route_key: format!(
                    "rte_{}",
                    &format!("{:x}", Sha256::digest(printer_id.to_string().as_bytes()))[..32]
                ),
                inventory_revision: 1,
                topology_revision: 1,
                observed_at: Utc::now(),
                identity_evidence: Vec::new(),
                identity_confidence: piqae_protocol::agent::IdentityConfidence::Unknown,
                topology_change: None,
                profile_observed_at: Some(Utc::now()),
                stock_observed_at: Some(Utc::now()),
            }),
        }
    }

    fn live_route_observation(printer_id: PrinterId, sequence: u64) -> AgentRouteObservation {
        AgentRouteObservation {
            local_route_key: format!(
                "rte_{}",
                &format!("{:x}", Sha256::digest(printer_id.to_string().as_bytes()))[..32]
            ),
            sequence,
            observed_at: Utc::now(),
            inventory_revision: 1,
            state: PrinterState::Online,
            accepts_jobs: true,
            state_reasons: Vec::new(),
            queue: Some(PrivacySafeQueueObservation::default()),
            profile_observed_at: Some(Utc::now()),
            stock_observed_at: Some(Utc::now()),
        }
    }

    fn stored_profiled_printer(printer_id: PrinterId) -> piqae_storage_postgres::SyncedPrinter {
        let printer = profiled_printer_snapshot(printer_id);
        piqae_storage_postgres::SyncedPrinter {
            id: printer.id,
            native_id: printer.native_id,
            name: printer.name,
            state: printer.state,
            is_default: printer.is_default,
            capabilities: printer.capabilities,
            capability_revision: printer.capability_revision,
            native_options: printer.native_options,
            semantic_capabilities: printer.semantic_capabilities,
            profiles: printer
                .profiles
                .into_iter()
                .map(|profile| piqae_storage_postgres::PrinterProfileSnapshot {
                    profile_id: profile.profile_id,
                    revision: profile.revision,
                    name: profile.name,
                    is_default: profile.is_default,
                    options: profile.options,
                    status: Some("ready".into()),
                    native_kind: Some("cups_options".into()),
                    native_digest: profile.native_digest,
                    driver_fingerprint: None,
                    summary: Some(serde_json::to_value(profile.summary).expect("profile summary")),
                    stock_id: profile.stock_id,
                    safe_overrides: vec!["copies".into(), "pages".into()],
                    last_validated_at: None,
                    last_test_job_id: None,
                    published: profile.published,
                })
                .collect(),
        }
    }

    #[tokio::test]
    #[allow(
        clippy::too_many_lines,
        reason = "one end-to-end test exercises profile sync, compatibility projection, and rejection"
    )]
    async fn synced_printer_profiles_are_visible_through_the_canonical_api() {
        let application = application().await;
        let now = Utc::now();
        let printer_id = PrinterId::new();
        let request = AgentSyncRequest {
            agent_id: application.agent_id,
            protocol_version: 1,
            agent_version: "test-profile-sync".into(),
            printer_revision: 12,
            acknowledged_command_cursor: None,
            event_cursor: None,
            queue: QueueSnapshot {
                queued_jobs: 0,
                active_jobs: 0,
                content_bytes: 0,
                accepts_jobs: true,
            },
            health: AgentHealth {
                started_at: now,
                observed_at: now,
                sqlite_integrity_ok: true,
                executor_crashes: 0,
                last_error_code: None,
            },
            printers: Some(vec![profiled_printer_snapshot(printer_id)]),
            events: Vec::new(),
            diagnostics: Vec::new(),
            document_render: piqae_protocol::agent::DocumentRenderCapabilities::default(),
            capabilities: piqae_protocol::agent::AgentProtocolCapabilities::default(),
            route_observations: vec![live_route_observation(printer_id, 1)],
            topology_changes: Vec::new(),
            native_handoffs: Vec::new(),
            runtime: None,
        };
        let body = serde_json::to_vec(&request).expect("sync JSON");
        let sync = application
            .router
            .clone()
            .oneshot(signed_request(&application, "POST", "/v1/agent/sync", body))
            .await
            .expect("sync response");
        assert_eq!(sync.status(), StatusCode::OK);

        let response = application
            .router
            .clone()
            .oneshot(api_request(
                "GET",
                "/v1/printers",
                "piq_test_integration",
                None,
            ))
            .await
            .expect("printer list response");
        assert_eq!(response.status(), StatusCode::OK);
        let json: serde_json::Value = serde_json::from_slice(
            &response
                .into_body()
                .collect()
                .await
                .expect("printer list body")
                .to_bytes(),
        )
        .expect("printer list JSON");
        let printer = &json["data"][0];
        assert_eq!(printer["id"], printer_id.as_ulid().to_string());
        assert_eq!(printer["capability_revision"], 7);
        assert_eq!(printer["capabilities"]["color"], true);
        assert_eq!(
            printer["native_options"]["StapleLocation"]["selected_choice"],
            "None"
        );
        assert_eq!(
            printer["native_options"]["StapleLocation"]["choices"][1]["value"],
            "UpperLeft"
        );
        assert_eq!(
            printer["semantic_capabilities"]["facets"]["finishing.staple"][1],
            "upper_left"
        );
        assert_eq!(printer["profiles"][0]["profile_id"], "profile_shipping");
        assert_eq!(printer["profiles"][0]["revision"], 4);
        assert_eq!(
            printer["profiles"][0]["options"]["native_options"]["StapleLocation"],
            "UpperLeft"
        );
        assert_eq!(printer["profiles"][0]["status"], "ready");
        assert_eq!(printer["profiles"][0]["native_kind"], "cups_options");
        assert_eq!(printer["profiles"][0]["stock_id"], "stk_shipping");
        assert_eq!(printer["profiles"][0]["published"], true);

        let capabilities = application
            .router
            .clone()
            .oneshot(api_request(
                "GET",
                &format!("/v1/printers/{printer_id}/capabilities"),
                "piq_test_integration",
                None,
            ))
            .await
            .expect("capability document response");
        assert_eq!(capabilities.status(), StatusCode::OK);
        let capabilities: serde_json::Value = serde_json::from_slice(
            &capabilities
                .into_body()
                .collect()
                .await
                .expect("capability document body")
                .to_bytes(),
        )
        .expect("capability document JSON");
        assert_eq!(
            capabilities["facets"]["finishing.staple"]["mutability"],
            "job_override"
        );
        assert_eq!(
            capabilities["facets"]["finishing.staple"]["evidence"]["support_pack_id"],
            "pack.test.staple"
        );

        let (status, compatibility_printers) = compatibility_json(
            &application.router,
            compatibility_request("GET", "/printers", None),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let compatibility_printers = compatibility_printers
            .as_array()
            .expect("compatibility printers");
        assert_eq!(compatibility_printers.len(), 2);
        let profile_printer = compatibility_printers
            .iter()
            .find(|printer| printer["name"] == "Office Laser — A4 shipping")
            .expect("virtual profile printer");
        assert_eq!(
            profile_printer["capabilities"]["papers"]
                .as_object()
                .expect("profile papers object")
                .len(),
            1
        );
        let virtual_id = profile_printer["id"].as_i64().expect("virtual printer ID");

        let (status, created) = compatibility_json(
            &application.router,
            compatibility_request(
                "POST",
                "/printjobs",
                Some(format!(
                    r#"{{"printerId":{virtual_id},"title":"Profile job","contentType":"pdf_base64","content":"JVBERi0=","options":{{"copies":2}}}}"#
                )),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert!(created.is_i64());

        let rejected = application
            .router
            .clone()
            .oneshot(compatibility_request(
                "POST",
                "/printjobs",
                Some(format!(
                    r#"{{"printerId":{virtual_id},"title":"Unsafe profile job","contentType":"pdf_base64","content":"JVBERi0=","options":{{"paper":"Letter"}}}}"#
                )),
            ))
            .await
            .expect("profile override response");
        assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn stocks_targets_and_bindings_form_a_ready_tenant_scoped_route() {
        let application = application().await;
        let now = Utc::now();
        let printer_id = PrinterId::new();
        let sync_request = AgentSyncRequest {
            agent_id: application.agent_id,
            protocol_version: 1,
            agent_version: "test-routing".into(),
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
                started_at: now,
                observed_at: now,
                sqlite_integrity_ok: true,
                executor_crashes: 0,
                last_error_code: None,
            },
            printers: Some(vec![profiled_printer_snapshot(printer_id)]),
            events: Vec::new(),
            diagnostics: Vec::new(),
            document_render: piqae_protocol::agent::DocumentRenderCapabilities::default(),
            capabilities: piqae_protocol::agent::AgentProtocolCapabilities::default(),
            route_observations: vec![live_route_observation(printer_id, 1)],
            topology_changes: Vec::new(),
            native_handoffs: Vec::new(),
            runtime: None,
        };
        let body = serde_json::to_vec(&sync_request).expect("sync JSON");
        let response = application
            .router
            .clone()
            .oneshot(signed_request(&application, "POST", "/v1/agent/sync", body))
            .await
            .expect("sync response");
        assert_eq!(response.status(), StatusCode::OK);

        let stock_response = application
            .router
            .clone()
            .oneshot(api_request(
                "POST",
                "/v1/stocks",
                "piq_test_integration",
                Some(r#"{"name":"Shipping A4","sku":"A4-SHIP","attributes":{"width_mm":210}}"#),
            ))
            .await
            .expect("stock response");
        assert_eq!(stock_response.status(), StatusCode::CREATED);
        let stock: serde_json::Value = serde_json::from_slice(
            &stock_response
                .into_body()
                .collect()
                .await
                .expect("stock body")
                .to_bytes(),
        )
        .expect("stock JSON");
        assert!(stock["id"].as_str().expect("stock id").starts_with("stk_"));

        let target_response = application
            .router
            .clone()
            .oneshot(api_request(
                "POST",
                "/v1/targets",
                "piq_test_integration",
                Some(r#"{"name":"Shipping labels"}"#),
            ))
            .await
            .expect("target response");
        assert_eq!(target_response.status(), StatusCode::CREATED);
        let target: serde_json::Value = serde_json::from_slice(
            &target_response
                .into_body()
                .collect()
                .await
                .expect("target body")
                .to_bytes(),
        )
        .expect("target JSON");
        let target_id = target["id"].as_str().expect("target id");

        let binding_response = application
            .router
            .clone()
            .oneshot(api_request(
                "POST",
                &format!("/v1/targets/{target_id}/bindings"),
                "piq_test_integration",
                Some(&format!(
                    r#"{{"printer_id":"{printer_id}","profile_id":"profile_shipping","profile_revision":4,"role":"primary"}}"#
                )),
            ))
            .await
            .expect("binding response");
        assert_eq!(binding_response.status(), StatusCode::CREATED);

        let readiness_response = application
            .router
            .clone()
            .oneshot(api_request(
                "GET",
                &format!("/v1/targets/{target_id}/readiness"),
                "piq_test_integration",
                None,
            ))
            .await
            .expect("readiness response");
        assert_eq!(readiness_response.status(), StatusCode::OK);
        let readiness: serde_json::Value = serde_json::from_slice(
            &readiness_response
                .into_body()
                .collect()
                .await
                .expect("readiness body")
                .to_bytes(),
        )
        .expect("readiness JSON");
        assert_eq!(readiness["status"], "ready");
        assert_eq!(readiness["bindings"][0]["status"], "ready");

        application
            .repository
            .set_agent_offline(application.agent_id)
            .await;
        let target_job_response = application
            .router
            .clone()
            .oneshot(api_request(
                "POST",
                "/v1/jobs",
                "piq_test_integration",
                Some(&format!(
                    r#"{{"target_id":"{target_id}","title":"Routed shipping label","content_type":"pdf","content":{{"type":"base64","data":"JVBERi0="}}}}"#
                )),
            ))
            .await
            .expect("target job response");
        assert_eq!(target_job_response.status(), StatusCode::CREATED);
        let target_job: serde_json::Value = serde_json::from_slice(
            &target_job_response
                .into_body()
                .collect()
                .await
                .expect("target job body")
                .to_bytes(),
        )
        .expect("target job JSON");
        let routed_job = application
            .repository
            .get_job(
                application.tenant.workspace_id,
                application.tenant.environment_id,
                target_job["id"]
                    .as_str()
                    .expect("target job id")
                    .parse()
                    .expect("typed target job id"),
            )
            .await
            .expect("stored target job");
        assert_eq!(routed_job.printer_id, printer_id);
        assert_eq!(
            routed_job
                .metadata
                .get("piqae.target_id")
                .map(String::as_str),
            Some(target_id)
        );
        assert_eq!(
            routed_job
                .metadata
                .get("piqae.profile_revision")
                .map(String::as_str),
            Some("4")
        );

        let reconnect_time = Utc::now();
        let reconnect = AgentSyncRequest {
            agent_id: application.agent_id,
            protocol_version: 1,
            agent_version: "test-routing".into(),
            printer_revision: 2,
            acknowledged_command_cursor: None,
            event_cursor: None,
            queue: QueueSnapshot {
                queued_jobs: 0,
                active_jobs: 0,
                content_bytes: 0,
                accepts_jobs: true,
            },
            health: AgentHealth {
                started_at: reconnect_time,
                observed_at: reconnect_time,
                sqlite_integrity_ok: true,
                executor_crashes: 0,
                last_error_code: None,
            },
            printers: Some(vec![profiled_printer_snapshot(printer_id)]),
            events: Vec::new(),
            diagnostics: Vec::new(),
            document_render: piqae_protocol::agent::DocumentRenderCapabilities::default(),
            capabilities: piqae_protocol::agent::AgentProtocolCapabilities::default(),
            route_observations: vec![live_route_observation(printer_id, 2)],
            topology_changes: Vec::new(),
            native_handoffs: Vec::new(),
            runtime: None,
        };
        let reconnect_body = serde_json::to_vec(&reconnect).expect("reconnect JSON");
        let reconnect_response = application
            .router
            .clone()
            .oneshot(signed_request(
                &application,
                "POST",
                "/v1/agent/sync",
                reconnect_body,
            ))
            .await
            .expect("reconnect response");
        assert_eq!(reconnect_response.status(), StatusCode::OK);
        let reconnect_sync: AgentSyncResponse = serde_json::from_slice(
            &reconnect_response
                .into_body()
                .collect()
                .await
                .expect("reconnect body")
                .to_bytes(),
        )
        .expect("reconnect response JSON");
        assert_eq!(reconnect_sync.candidate_jobs.len(), 1);
        assert_eq!(
            reconnect_sync.candidate_jobs[0].job.id,
            target_job["id"]
                .as_str()
                .expect("target job id")
                .parse()
                .expect("typed target job id")
        );

        let cross_tenant = application
            .router
            .clone()
            .oneshot(api_request(
                "GET",
                &format!("/v1/targets/{target_id}/readiness"),
                "piq_test_other",
                None,
            ))
            .await
            .expect("cross-tenant response");
        assert_eq!(cross_tenant.status(), StatusCode::NOT_FOUND);

        let wrong_revision = application
            .router
            .clone()
            .oneshot(api_request(
                "POST",
                &format!("/v1/targets/{target_id}/bindings"),
                "piq_test_integration",
                Some(&format!(
                    r#"{{"printer_id":"{printer_id}","profile_id":"profile_shipping","profile_revision":99,"role":"standby"}}"#
                )),
            ))
            .await
            .expect("invalid binding response");
        assert_eq!(wrong_revision.status(), StatusCode::NOT_FOUND);

        let standby_agent = AgentId::new();
        let standby_printer = PrinterId::new();
        application
            .repository
            .add_printer(
                application.tenant.workspace_id,
                application.tenant.environment_id,
                standby_printer,
                standby_agent,
            )
            .await;
        let standby_snapshot = stored_profiled_printer(standby_printer);
        application
            .repository
            .sync_agent_presence(
                application.tenant.workspace_id,
                application.tenant.environment_id,
                standby_agent,
                "test-standby",
                &AgentHealth {
                    started_at: Utc::now(),
                    observed_at: Utc::now(),
                    sqlite_integrity_ok: true,
                    executor_crashes: 0,
                    last_error_code: None,
                },
                &piqae_protocol::agent::DocumentRenderCapabilities::default(),
                Some(std::slice::from_ref(&standby_snapshot)),
            )
            .await
            .expect("standby presence");
        let standby_binding = application
            .router
            .clone()
            .oneshot(api_request(
                "POST",
                &format!("/v1/targets/{target_id}/bindings"),
                "piq_test_integration",
                Some(&format!(
                    r#"{{"printer_id":"{standby_printer}","profile_id":"profile_shipping","profile_revision":4,"role":"standby"}}"#
                )),
            ))
            .await
            .expect("standby binding response");
        assert_eq!(standby_binding.status(), StatusCode::CREATED);
        application
            .repository
            .revoke_agent(
                application.tenant.workspace_id,
                application.tenant.environment_id,
                application.agent_id,
            )
            .await
            .expect("primary node offline");

        let standby_job_response = application
            .router
            .clone()
            .oneshot(api_request(
                "POST",
                "/v1/jobs",
                "piq_test_integration",
                Some(&format!(
                    r#"{{"target_id":"{target_id}","title":"Standby shipping label","content_type":"pdf","content":{{"type":"base64","data":"JVBERi0="}}}}"#
                )),
            ))
            .await
            .expect("standby job response");
        assert_eq!(standby_job_response.status(), StatusCode::CREATED);
        let standby_job: serde_json::Value = serde_json::from_slice(
            &standby_job_response
                .into_body()
                .collect()
                .await
                .expect("standby job body")
                .to_bytes(),
        )
        .expect("standby job JSON");
        let stored_standby_job = application
            .repository
            .get_job(
                application.tenant.workspace_id,
                application.tenant.environment_id,
                standby_job["id"]
                    .as_str()
                    .expect("standby job id")
                    .parse()
                    .expect("typed standby job id"),
            )
            .await
            .expect("stored standby job");
        assert_eq!(stored_standby_job.printer_id, standby_printer);
        assert_eq!(
            stored_standby_job
                .metadata
                .get("piqae.target_id")
                .map(String::as_str),
            Some(target_id)
        );
    }

    #[tokio::test]
    async fn native_create_is_durable_and_redacts_content() {
        let application = application().await;
        let request = Request::builder()
            .method("POST")
            .uri("/v1/jobs")
            .header("authorization", "Bearer piq_test_integration")
            .header("content-type", "application/json")
            .header("idempotency-key", "order-481")
            .body(Body::from(format!(
                r#"{{"printer_id":"{}","title":"Order 481","content_type":"pdf","content":{{"type":"base64","data":"c2VjcmV0"}}}}"#,
                application.printer_id
            )))
            .expect("valid test request");
        let response = application
            .router
            .clone()
            .oneshot(request)
            .await
            .expect("router response");
        assert_eq!(response.status(), StatusCode::CREATED);
        let body = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).expect("JSON");
        assert_eq!(json["state"], "waiting_for_agent");
        assert!(json.get("content").is_none());
        let stored = application
            .repository
            .list_jobs(
                application.tenant.workspace_id,
                application.tenant.environment_id,
                None,
                10,
            )
            .await
            .expect("stored jobs");
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].state, piqae_domain::JobState::WaitingForAgent);
    }

    #[tokio::test]
    async fn native_api_requires_bearer_authentication() {
        let application = application().await;
        let response = application
            .router
            .oneshot(
                Request::builder()
                    .uri("/v1/jobs")
                    .body(Body::empty())
                    .expect("valid request"),
            )
            .await
            .expect("router response");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn ordinary_api_keys_cannot_select_another_workspace() {
        let application = application().await;
        let response = application
            .router
            .oneshot(
                Request::builder()
                    .uri("/v1/jobs")
                    .header("authorization", "Bearer piq_test_integration")
                    .header("x-piqae-workspace-id", WorkspaceId::new().to_string())
                    .header("x-piqae-environment-id", EnvironmentId::new().to_string())
                    .body(Body::empty())
                    .expect("valid request"),
            )
            .await
            .expect("router response");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn native_and_compatibility_error_ids_match_response_headers() {
        let application = application().await;
        let native = application
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/jobs")
                    .body(Body::empty())
                    .expect("native request"),
            )
            .await
            .expect("native response");
        let native_header = native
            .headers()
            .get(request_id::HEADER_NAME)
            .expect("native request ID")
            .to_str()
            .expect("ASCII request ID")
            .to_owned();
        let native_body: serde_json::Value = serde_json::from_slice(
            &native
                .into_body()
                .collect()
                .await
                .expect("native body")
                .to_bytes(),
        )
        .expect("native JSON");
        assert_eq!(native_body["error"]["request_id"], native_header);

        let compatibility = application
            .router
            .oneshot(
                Request::builder()
                    .uri("/whoami")
                    .body(Body::empty())
                    .expect("compatibility request"),
            )
            .await
            .expect("compatibility response");
        let compatibility_header = compatibility
            .headers()
            .get(request_id::HEADER_NAME)
            .expect("compatibility request ID")
            .to_str()
            .expect("ASCII request ID")
            .to_owned();
        let compatibility_body: serde_json::Value = serde_json::from_slice(
            &compatibility
                .into_body()
                .collect()
                .await
                .expect("compatibility body")
                .to_bytes(),
        )
        .expect("compatibility JSON");
        assert_eq!(compatibility_body["uid"], compatibility_header);
    }

    #[tokio::test]
    async fn request_id_acceptance_replacement_and_success_are_consistent() {
        let application = application().await;
        let trusted = "client.trace-123:abc";
        let accepted = application
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/health")
                    .header(request_id::HEADER_NAME, trusted)
                    .body(Body::empty())
                    .expect("trusted request"),
            )
            .await
            .expect("trusted response");
        assert_eq!(accepted.status(), StatusCode::OK);
        assert_eq!(
            accepted
                .headers()
                .get(request_id::HEADER_NAME)
                .expect("trusted response ID"),
            trusted
        );

        let replaced = application
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/health")
                    .header(request_id::HEADER_NAME, "bad id!")
                    .body(Body::empty())
                    .expect("invalid request ID request"),
            )
            .await
            .expect("invalid request ID response");
        let replacement = replaced
            .headers()
            .get(request_id::HEADER_NAME)
            .expect("replacement request ID")
            .to_str()
            .expect("ASCII replacement");
        assert!(replacement.starts_with("req_"));
        assert_ne!(replacement, "bad id!");

        let generated = application
            .router
            .oneshot(
                Request::builder()
                    .uri("/v1/health")
                    .body(Body::empty())
                    .expect("health request"),
            )
            .await
            .expect("health response");
        assert_eq!(generated.status(), StatusCode::OK);
        assert!(
            generated
                .headers()
                .get(request_id::HEADER_NAME)
                .expect("generated success ID")
                .to_str()
                .expect("ASCII generated ID")
                .starts_with("req_")
        );
    }

    #[tokio::test]
    async fn framework_not_found_and_method_errors_have_request_ids() {
        let application = application().await;
        for request in [
            Request::builder()
                .uri("/does-not-exist")
                .body(Body::empty())
                .expect("not found request"),
            Request::builder()
                .method("POST")
                .uri("/v1/health")
                .body(Body::empty())
                .expect("method request"),
        ] {
            let response = application
                .router
                .clone()
                .oneshot(request)
                .await
                .expect("framework response");
            assert!(matches!(
                response.status(),
                StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED
            ));
            assert!(
                response
                    .headers()
                    .get(request_id::HEADER_NAME)
                    .expect("framework request ID")
                    .to_str()
                    .expect("ASCII framework ID")
                    .starts_with("req_")
            );
        }
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn api_keys_are_one_time_scoped_tenant_isolated_and_revocable() {
        let workspace = WorkspaceId::new();
        let first_environment = EnvironmentId::new();
        let second_environment = EnvironmentId::new();
        let manager = TenantContext::with_scopes(
            workspace,
            first_environment,
            &[Scope::ApiKeysRead, Scope::ApiKeysWrite, Scope::JobsRead],
        );
        let read_only =
            TenantContext::with_scopes(workspace, first_environment, &[Scope::ApiKeysRead]);
        let other_tenant = TenantContext::with_scopes(
            workspace,
            second_environment,
            &[Scope::ApiKeysRead, Scope::ApiKeysWrite, Scope::JobsRead],
        );
        let (application, _) = api_key_application(&[
            ("piq_test_manager", manager),
            ("piq_test_reader", read_only),
            ("piq_test_other", other_tenant),
        ])
        .await;

        let denied = application
            .clone()
            .oneshot(api_request(
                "POST",
                "/v1/api-keys",
                "piq_test_reader",
                Some(r#"{"name":"Denied","scopes":["jobs_read"]}"#),
            ))
            .await
            .expect("read-only response");
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);

        let overscoped = application
            .clone()
            .oneshot(api_request(
                "POST",
                "/v1/api-keys",
                "piq_test_manager",
                Some(r#"{"name":"Too broad","scopes":["jobs_write"]}"#),
            ))
            .await
            .expect("overscope response");
        assert_eq!(overscoped.status(), StatusCode::FORBIDDEN);

        let created = application
            .clone()
            .oneshot(api_request(
                "POST",
                "/v1/api-keys",
                "piq_test_manager",
                Some(r#"{"name":"Read jobs","scopes":["jobs_read"]}"#),
            ))
            .await
            .expect("create response");
        assert_eq!(created.status(), StatusCode::CREATED);
        let created: serde_json::Value = serde_json::from_slice(
            &created
                .into_body()
                .collect()
                .await
                .expect("created body")
                .to_bytes(),
        )
        .expect("created JSON");
        let key_id = created["id"].as_str().expect("key id");
        let secret = created["secret"].as_str().expect("one-time secret");
        assert!(secret.starts_with("piq_test_"));
        assert!(created.get("secret_hash").is_none());

        let listed = application
            .clone()
            .oneshot(api_request("GET", "/v1/api-keys", "piq_test_manager", None))
            .await
            .expect("list response");
        let listed: serde_json::Value = serde_json::from_slice(
            &listed
                .into_body()
                .collect()
                .await
                .expect("list body")
                .to_bytes(),
        )
        .expect("list JSON");
        assert_eq!(listed.as_array().expect("key list").len(), 1);
        assert!(listed[0].get("secret").is_none());
        assert!(listed[0].get("secret_hash").is_none());

        let isolated = application
            .clone()
            .oneshot(api_request("GET", "/v1/api-keys", "piq_test_other", None))
            .await
            .expect("isolated list response");
        let isolated: serde_json::Value = serde_json::from_slice(
            &isolated
                .into_body()
                .collect()
                .await
                .expect("isolated body")
                .to_bytes(),
        )
        .expect("isolated JSON");
        assert!(isolated.as_array().expect("isolated key list").is_empty());

        let missing = application
            .clone()
            .oneshot(api_request(
                "DELETE",
                &format!("/v1/api-keys/{key_id}"),
                "piq_test_other",
                None,
            ))
            .await
            .expect("cross-tenant revoke response");
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);

        for _ in 0..2 {
            let revoked = application
                .clone()
                .oneshot(api_request(
                    "DELETE",
                    &format!("/v1/api-keys/{key_id}"),
                    "piq_test_manager",
                    None,
                ))
                .await
                .expect("revoke response");
            assert_eq!(revoked.status(), StatusCode::OK);
            let revoked: serde_json::Value = serde_json::from_slice(
                &revoked
                    .into_body()
                    .collect()
                    .await
                    .expect("revoke body")
                    .to_bytes(),
            )
            .expect("revoke JSON");
            assert!(revoked["revoked_at"].is_string());
        }
    }

    #[tokio::test]
    async fn compatibility_create_returns_numeric_identifier() {
        let application = application().await;
        let compatibility_printer_id = application
            .repository
            .compatibility_id(
                application.tenant.workspace_id,
                application.tenant.environment_id,
                "printer",
                &application.printer_id.to_string(),
            )
            .await
            .expect("compatibility printer ID");
        let credentials = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            "piq_test_integration:",
        );
        let response = application
            .router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/printjobs")
                    .header("authorization", format!("Basic {credentials}"))
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        r#"{{"printerId":{compatibility_printer_id},"title":"Label","contentType":"pdf_base64","content":"JVBERi0="}}"#
                    )))
                    .expect("valid request"),
            )
            .await
            .expect("router response");
        assert_eq!(response.status(), StatusCode::CREATED);
        let body = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        let id: i64 = serde_json::from_slice(&body).expect("numeric ID");
        assert!(id > 0);
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn compatibility_filtered_routes_and_cancellation_are_tenant_scoped() {
        let application = application().await;
        let second_printer = PrinterId::new();
        application
            .repository
            .add_printer(
                application.tenant.workspace_id,
                application.tenant.environment_id,
                second_printer,
                application.agent_id,
            )
            .await;
        let computer_id = application
            .repository
            .compatibility_id(
                application.tenant.workspace_id,
                application.tenant.environment_id,
                "computer",
                &application.agent_id.to_string(),
            )
            .await
            .expect("computer compatibility ID");
        let printer_id = application
            .repository
            .compatibility_id(
                application.tenant.workspace_id,
                application.tenant.environment_id,
                "printer",
                &application.printer_id.to_string(),
            )
            .await
            .expect("printer compatibility ID");
        let second_printer_id = application
            .repository
            .compatibility_id(
                application.tenant.workspace_id,
                application.tenant.environment_id,
                "printer",
                &second_printer.to_string(),
            )
            .await
            .expect("second printer compatibility ID");

        let (_, computers) = compatibility_json(
            &application.router,
            compatibility_request(
                "GET",
                &format!("/computers/{computer_id},{computer_id}"),
                None,
            ),
        )
        .await;
        assert_eq!(computers.as_array().expect("computers").len(), 1);
        assert_eq!(computers[0]["id"], computer_id);

        let (_, printers) = compatibility_json(
            &application.router,
            compatibility_request(
                "GET",
                &format!("/printers/{second_printer_id},{printer_id}"),
                None,
            ),
        )
        .await;
        assert_eq!(printers.as_array().expect("printers").len(), 2);
        assert_eq!(
            printers
                .as_array()
                .expect("printers")
                .iter()
                .map(|printer| printer["id"].as_i64().expect("printer ID"))
                .collect::<Vec<_>>(),
            {
                let mut expected = vec![printer_id, second_printer_id];
                expected.sort_unstable();
                expected
            }
        );
        let (_, paged) = compatibility_json(
            &application.router,
            compatibility_request("GET", "/printers?limit=1&dir=asc", None),
        )
        .await;
        assert_eq!(paged.as_array().expect("paged printers").len(), 1);
        let first_page_id = paged[0]["id"].as_i64().expect("first page printer ID");
        assert_eq!(first_page_id, printer_id.min(second_printer_id));
        let (_, next_page) = compatibility_json(
            &application.router,
            compatibility_request(
                "GET",
                &format!("/printers?limit=1&dir=asc&after={first_page_id}"),
                None,
            ),
        )
        .await;
        assert_eq!(
            next_page[0]["id"].as_i64().expect("next page printer ID"),
            printer_id.max(second_printer_id)
        );
        let invalid_after = application
            .router
            .clone()
            .oneshot(compatibility_request("GET", "/printers?after=0", None))
            .await
            .expect("invalid after response");
        assert_eq!(invalid_after.status(), StatusCode::BAD_REQUEST);

        let (_, nested) = compatibility_json(
            &application.router,
            compatibility_request("GET", &format!("/computers/{computer_id}/printers"), None),
        )
        .await;
        assert_eq!(nested.as_array().expect("nested printers").len(), 2);
        let (_, nested_set) = compatibility_json(
            &application.router,
            compatibility_request(
                "GET",
                &format!("/computers/{computer_id}/printers/{printer_id}"),
                None,
            ),
        )
        .await;
        assert_eq!(nested_set.as_array().expect("nested printer set").len(), 1);

        let (_, created) = compatibility_json(
            &application.router,
            compatibility_request(
                "POST",
                "/printjobs",
                Some(format!(
                    r#"{{"printerId":{printer_id},"title":"Filtered","contentType":"pdf_base64","content":"JVBERi0="}}"#
                )),
            ),
        )
        .await;
        let job_id = created.as_i64().expect("job compatibility ID");
        let (_, printer_jobs) = compatibility_json(
            &application.router,
            compatibility_request("GET", &format!("/printers/{printer_id}/printjobs"), None),
        )
        .await;
        assert_eq!(printer_jobs[0]["id"], job_id);
        let (_, other_jobs) = compatibility_json(
            &application.router,
            compatibility_request(
                "GET",
                &format!("/printers/{second_printer_id}/printjobs"),
                None,
            ),
        )
        .await;
        assert!(other_jobs.as_array().expect("other jobs").is_empty());
        let (_, selected_job) = compatibility_json(
            &application.router,
            compatibility_request(
                "GET",
                &format!("/printers/{printer_id}/printjobs/{job_id}"),
                None,
            ),
        )
        .await;
        assert_eq!(selected_job[0]["id"], job_id);

        let (cancel_status, cancelled) = compatibility_json(
            &application.router,
            compatibility_request(
                "DELETE",
                &format!("/printers/{printer_id}/printjobs/{job_id}"),
                None,
            ),
        )
        .await;
        assert_eq!(cancel_status, StatusCode::OK);
        assert_eq!(cancelled, serde_json::json!([job_id]));
        let (_, repeated) = compatibility_json(
            &application.router,
            compatibility_request("DELETE", &format!("/printjobs/{job_id}"), None),
        )
        .await;
        assert_eq!(repeated, serde_json::json!([]));

        let foreign_tenant = TenantContext::unrestricted(WorkspaceId::new(), EnvironmentId::new());
        let foreign_printer = PrinterId::new();
        application
            .repository
            .add_printer(
                foreign_tenant.workspace_id,
                foreign_tenant.environment_id,
                foreign_printer,
                AgentId::new(),
            )
            .await;
        let foreign_id = application
            .repository
            .compatibility_id(
                foreign_tenant.workspace_id,
                foreign_tenant.environment_id,
                "printer",
                &foreign_printer.to_string(),
            )
            .await
            .expect("foreign printer ID");
        let foreign = application
            .router
            .clone()
            .oneshot(compatibility_request(
                "GET",
                &format!("/printers/{foreign_id}"),
                None,
            ))
            .await
            .expect("foreign response");
        assert_eq!(foreign.status(), StatusCode::NOT_FOUND);
        let invalid = application
            .router
            .clone()
            .oneshot(compatibility_request("GET", "/printers/not-a-set", None))
            .await
            .expect("invalid set response");
        assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn preaccept_cancellation_is_terminal_and_stale_n_minus_one_command_is_retired() {
        let application = application().await;
        let created = application
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/jobs")
                    .header("authorization", "Bearer piq_test_integration")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        r#"{{"printer_id":"{}","title":"Cancel me","content_type":"pdf","content":{{"type":"base64","data":"cHJpbnQ="}}}}"#,
                        application.printer_id
                    )))
                    .expect("valid request"),
            )
            .await
            .expect("create response");
        assert_eq!(created.status(), StatusCode::CREATED);
        let created: serde_json::Value = serde_json::from_slice(
            &created
                .into_body()
                .collect()
                .await
                .expect("create body")
                .to_bytes(),
        )
        .expect("create JSON");
        let job_id =
            JobId::from_str(created["id"].as_str().expect("job ID")).expect("typed job ID");

        assert!(matches!(
            application
                .repository
                .request_job_cancellation(
                    application.tenant.workspace_id,
                    EnvironmentId::new(),
                    job_id,
                )
                .await,
            Err(crate::repository::RepositoryError::NotFound)
        ));
        let direct_job_updated_before = application
            .repository
            .list_tenant_events(
                application.tenant.workspace_id,
                application.tenant.environment_id,
                None,
                500,
            )
            .await
            .expect("tenant events before direct cancellation")
            .into_iter()
            .filter(|event| event.event_type == "job.updated")
            .count();
        let cancelled = application
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/v1/jobs/{job_id}/cancel"))
                    .header("authorization", "Bearer piq_test_integration")
                    .body(Body::empty())
                    .expect("valid request"),
            )
            .await
            .expect("cancel response");
        assert_eq!(cancelled.status(), StatusCode::ACCEPTED);
        let direct_job_updated_after = application
            .repository
            .list_tenant_events(
                application.tenant.workspace_id,
                application.tenant.environment_id,
                None,
                500,
            )
            .await
            .expect("tenant events after direct cancellation")
            .into_iter()
            .filter(|event| event.event_type == "job.updated")
            .count();
        assert_eq!(direct_job_updated_after, direct_job_updated_before + 1);

        let first = sync_test_agent(&application, None).await;
        assert!(first.commands.is_empty());
        assert_eq!(
            application
                .repository
                .get_job(
                    application.tenant.workspace_id,
                    application.tenant.environment_id,
                    job_id,
                )
                .await
                .expect("cancelled job")
                .state,
            JobState::Cancelled
        );

        // Simulate a cancellation stored by an N-1 server before the direct
        // pre-accept terminalization existed. The current server proves the
        // exact tenant/node job was never accepted and retires it without
        // relying on a new command shape the old node would ignore.
        application
            .repository
            .enqueue_agent_command(
                application.tenant.workspace_id,
                application.tenant.environment_id,
                application.agent_id,
                &AgentCommand::CancelJob { job_id },
            )
            .await
            .expect("legacy command");
        let repaired = sync_test_agent(&application, None).await;
        assert!(repaired.commands.is_empty());
        assert!(repaired.command_cursor.is_some());
        let after_repair = application
            .repository
            .sync_agent_commands(
                application.tenant.workspace_id,
                application.tenant.environment_id,
                application.agent_id,
                None,
                100,
            )
            .await
            .expect("retired command");
        assert!(after_repair.commands.is_empty());

        // Model the narrower N-1 crash window: CancelRequested and its command
        // committed, but the server did not yet complete the pre-accept
        // cancellation. Retrying the repair must create exactly one durable
        // tenant event while still retiring the command on every replay.
        let repair_created = application
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/jobs")
                    .header("authorization", "Bearer piq_test_integration")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        r#"{{"printer_id":"{}","title":"Repair cancel","content_type":"pdf","content":{{"type":"base64","data":"cHJpbnQ="}}}}"#,
                        application.printer_id
                    )))
                    .expect("valid request"),
            )
            .await
            .expect("repair create response");
        let repair_body: serde_json::Value = serde_json::from_slice(
            &repair_created
                .into_body()
                .collect()
                .await
                .expect("repair create body")
                .to_bytes(),
        )
        .expect("repair create JSON");
        let repair_job_id = JobId::from_str(repair_body["id"].as_str().expect("repair job ID"))
            .expect("typed repair job ID");
        application
            .repository
            .transition_job(
                application.tenant.workspace_id,
                application.tenant.environment_id,
                repair_job_id,
                JobState::CancelRequested,
                None,
                None,
                None,
                None,
            )
            .await
            .expect("legacy cancel-requested state");
        application
            .repository
            .enqueue_agent_command(
                application.tenant.workspace_id,
                application.tenant.environment_id,
                application.agent_id,
                &AgentCommand::CancelJob {
                    job_id: repair_job_id,
                },
            )
            .await
            .expect("legacy repair command");
        let job_updated_before = application
            .repository
            .list_tenant_events(
                application.tenant.workspace_id,
                application.tenant.environment_id,
                None,
                500,
            )
            .await
            .expect("tenant events before repair")
            .into_iter()
            .filter(|event| event.event_type == "job.updated")
            .count();
        for _ in 0..2 {
            assert!(
                application
                    .repository
                    .retire_terminal_absent_local_cancellation(
                        application.tenant.workspace_id,
                        application.tenant.environment_id,
                        application.agent_id,
                        repair_job_id,
                    )
                    .await
                    .expect("idempotent repair")
            );
        }
        let job_updated_after = application
            .repository
            .list_tenant_events(
                application.tenant.workspace_id,
                application.tenant.environment_id,
                None,
                500,
            )
            .await
            .expect("tenant events")
            .into_iter()
            .filter(|event| event.event_type == "job.updated")
            .count();
        assert_eq!(job_updated_after, job_updated_before + 1);

        let accepted_created = application
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/jobs")
                    .header("authorization", "Bearer piq_test_integration")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        r#"{{"printer_id":"{}","title":"Accepted cancel","content_type":"pdf","content":{{"type":"base64","data":"cHJpbnQ="}}}}"#,
                        application.printer_id
                    )))
                    .expect("valid request"),
            )
            .await
            .expect("create accepted response");
        let accepted_body: serde_json::Value = serde_json::from_slice(
            &accepted_created
                .into_body()
                .collect()
                .await
                .expect("accepted create body")
                .to_bytes(),
        )
        .expect("accepted create JSON");
        let accepted_job_id =
            JobId::from_str(accepted_body["id"].as_str().expect("accepted job ID"))
                .expect("typed accepted job ID");
        let lease = application
            .repository
            .claim_jobs(
                application.tenant.workspace_id,
                application.tenant.environment_id,
                application.agent_id,
                "accepted-cancel-test",
                1,
            )
            .await
            .expect("claim accepted job")
            .into_iter()
            .find(|lease| lease.job.id == accepted_job_id)
            .expect("accepted job lease");
        application
            .repository
            .accept_agent_job(
                application.tenant.workspace_id,
                application.tenant.environment_id,
                application.agent_id,
                accepted_job_id,
                lease.lease_id,
                &lease.lease_token,
                None,
                1,
            )
            .await
            .expect("durable acceptance");
        application
            .repository
            .request_job_cancellation(
                application.tenant.workspace_id,
                application.tenant.environment_id,
                accepted_job_id,
            )
            .await
            .expect("accepted cancellation request");
        assert!(
            !application
                .repository
                .retire_terminal_absent_local_cancellation(
                    application.tenant.workspace_id,
                    application.tenant.environment_id,
                    application.agent_id,
                    accepted_job_id,
                )
                .await
                .expect("accepted proof")
        );
        assert!(
            !application
                .repository
                .retire_terminal_absent_local_cancellation(
                    application.tenant.workspace_id,
                    application.tenant.environment_id,
                    AgentId::new(),
                    accepted_job_id,
                )
                .await
                .expect("wrong-agent proof")
        );
        let accepted_command = sync_test_agent(&application, None).await;
        assert!(matches!(
            accepted_command.commands.as_slice(),
            [AgentCommand::CancelJob { job_id: candidate }] if *candidate == accepted_job_id
        ));
    }

    #[tokio::test]
    async fn command_acknowledgement_cannot_skip_an_undelivered_command() {
        let application = application().await;
        for command in [AgentCommand::Pause, AgentCommand::Resume] {
            application
                .repository
                .enqueue_agent_command(
                    application.tenant.workspace_id,
                    application.tenant.environment_id,
                    application.agent_id,
                    &command,
                )
                .await
                .expect("queued command");
        }
        let first = application
            .repository
            .sync_agent_commands(
                application.tenant.workspace_id,
                application.tenant.environment_id,
                application.agent_id,
                None,
                1,
            )
            .await
            .expect("first bounded batch");
        assert!(matches!(first.commands.as_slice(), [AgentCommand::Pause]));

        let remaining = application
            .repository
            .sync_agent_commands(
                application.tenant.workspace_id,
                application.tenant.environment_id,
                application.agent_id,
                Some("9223372036854775807"),
                100,
            )
            .await
            .expect("bounded acknowledgement");
        assert!(matches!(
            remaining.commands.as_slice(),
            [AgentCommand::Resume]
        ));
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn signed_agent_claim_and_durable_accept_flow() {
        let application = application().await;
        let create = Request::builder()
            .method("POST")
            .uri("/v1/jobs")
            .header("authorization", "Bearer piq_test_integration")
            .header("content-type", "application/json")
            .body(Body::from(format!(
                r#"{{"printer_id":"{}","title":"Agent lease","content_type":"pdf","content":{{"type":"base64","data":"cHJpbnQ="}}}}"#,
                application.printer_id
            )))
            .expect("valid request");
        let created = application
            .router
            .clone()
            .oneshot(create)
            .await
            .expect("create response");
        assert_eq!(created.status(), StatusCode::CREATED);

        let now = Utc::now();
        let sync = AgentSyncRequest {
            agent_id: application.agent_id,
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
                started_at: now,
                observed_at: now,
                sqlite_integrity_ok: true,
                executor_crashes: 0,
                last_error_code: None,
            },
            printers: Some(vec![profiled_printer_snapshot(application.printer_id)]),
            events: Vec::new(),
            diagnostics: Vec::new(),
            document_render: piqae_protocol::agent::DocumentRenderCapabilities::default(),
            capabilities: piqae_protocol::agent::AgentProtocolCapabilities::default(),
            route_observations: vec![live_route_observation(application.printer_id, 1)],
            topology_changes: Vec::new(),
            native_handoffs: Vec::new(),
            runtime: None,
        };
        let sync_body = serde_json::to_vec(&sync).expect("sync JSON");
        let sync_response = application
            .router
            .clone()
            .oneshot(signed_request(
                &application,
                "POST",
                "/v1/agent/sync",
                sync_body,
            ))
            .await
            .expect("sync response");
        assert_eq!(sync_response.status(), StatusCode::OK);
        let sync_body = sync_response
            .into_body()
            .collect()
            .await
            .expect("sync body")
            .to_bytes();
        let sync: AgentSyncResponse =
            serde_json::from_slice(&sync_body).expect("sync response JSON");
        assert_eq!(sync.candidate_jobs.len(), 1);
        let offer = &sync.candidate_jobs[0];
        let reservation = offer
            .route_reservation
            .as_ref()
            .expect("job offer is destination fenced");
        let accept = AgentAcceptJobRequest {
            lease_id: offer.lease_id,
            lease_token: offer.lease_token.clone(),
            content_sha256: match &offer.content {
                piqae_protocol::agent::ContentDescriptor::Download { sha256, .. } => sha256.clone(),
                _ => panic!("expected materialized download content"),
            },
            local_sequence: 1,
            route_reservation_id: Some(reservation.reservation_id),
            route_generation: Some(reservation.generation),
            route_fencing_token: Some(reservation.fencing_token.clone()),
        };
        let path = format!("/v1/agent/jobs/{}/accept", offer.job.id);
        let response = application
            .router
            .clone()
            .oneshot(signed_request(
                &application,
                "POST",
                &path,
                serde_json::to_vec(&accept).expect("accept JSON"),
            ))
            .await
            .expect("accept response");
        assert_eq!(response.status(), StatusCode::OK);
        application
            .repository
            .clear_acceptance_token(offer.job.id)
            .await;
        let retry = application
            .router
            .clone()
            .oneshot(signed_request(
                &application,
                "POST",
                &path,
                serde_json::to_vec(&accept).expect("retry accept JSON"),
            ))
            .await
            .expect("retry accept response");
        assert_eq!(retry.status(), StatusCode::OK);
        let mut mismatched_route = accept.clone();
        mismatched_route.route_reservation_id = Some(uuid::Uuid::new_v4());
        let route_rejected = application
            .router
            .clone()
            .oneshot(signed_request(
                &application,
                "POST",
                &path,
                serde_json::to_vec(&mismatched_route).expect("mismatched route accept JSON"),
            ))
            .await
            .expect("mismatched route response");
        assert_eq!(route_rejected.status(), StatusCode::CONFLICT);
        let exact_after_route_conflict = application
            .router
            .clone()
            .oneshot(signed_request(
                &application,
                "POST",
                &path,
                serde_json::to_vec(&accept).expect("exact accept JSON"),
            ))
            .await
            .expect("exact accept after route conflict");
        assert_eq!(exact_after_route_conflict.status(), StatusCode::OK);
        let mut mismatched = accept.clone();
        mismatched.lease_token.push('x');
        let rejected = application
            .router
            .clone()
            .oneshot(signed_request(
                &application,
                "POST",
                &path,
                serde_json::to_vec(&mismatched).expect("mismatched accept JSON"),
            ))
            .await
            .expect("mismatched accept response");
        assert_eq!(rejected.status(), StatusCode::CONFLICT);
        assert!(matches!(
            application
                .repository
                .accept_agent_job(
                    application.tenant.workspace_id,
                    application.tenant.environment_id,
                    AgentId::new(),
                    offer.job.id,
                    accept.lease_id,
                    &accept.lease_token,
                    Some(&accept.content_sha256),
                    accept.local_sequence,
                )
                .await,
            Err(crate::repository::RepositoryError::IdempotencyConflict)
        ));
        let stored = application
            .repository
            .get_job(
                application.tenant.workspace_id,
                application.tenant.environment_id,
                offer.job.id,
            )
            .await
            .expect("accepted job");
        assert_eq!(stored.state, piqae_domain::JobState::AgentAccepted);
        let reservation_id = accept
            .route_reservation_id
            .expect("accepted route reservation")
            .to_string();
        let generation = accept.route_generation.expect("accepted route generation");
        let fencing_token = accept
            .route_fencing_token
            .as_deref()
            .expect("accepted route fence");
        let before_revoke = application
            .repository
            .reconcile_agent_acceptance(
                application.tenant.workspace_id,
                application.tenant.environment_id,
                application.agent_id,
                offer.job.id,
                accept.lease_id,
                &accept.lease_token,
                &accept.content_sha256,
                accept.local_sequence,
                piqae_storage_postgres::DeliveryAttemptProof {
                    reservation_id: &reservation_id,
                    generation,
                    fencing_token,
                },
            )
            .await
            .expect("memory acceptance reconciliation");
        assert_eq!(before_revoke, (true, false, false));
        application
            .repository
            .revoke_node_connector(
                application.tenant.workspace_id,
                application.tenant.environment_id,
                application.agent_id,
                &format!("ncon_{}", application.agent_id),
            )
            .await
            .expect("memory connector revoke");
        application
            .repository
            .reactivate_connector_for_test(
                application.tenant.workspace_id,
                application.tenant.environment_id,
                application.agent_id,
            )
            .await;
        let after_reactivation = application
            .repository
            .reconcile_agent_acceptance(
                application.tenant.workspace_id,
                application.tenant.environment_id,
                application.agent_id,
                offer.job.id,
                accept.lease_id,
                &accept.lease_token,
                &accept.content_sha256,
                accept.local_sequence,
                piqae_storage_postgres::DeliveryAttemptProof {
                    reservation_id: &reservation_id,
                    generation,
                    fencing_token,
                },
            )
            .await
            .expect("memory reactivated reconciliation");
        assert_eq!(after_reactivation, (false, false, true));
        let foreign_scope = application
            .repository
            .reconcile_agent_acceptance(
                WorkspaceId::new(),
                EnvironmentId::new(),
                application.agent_id,
                offer.job.id,
                accept.lease_id,
                &accept.lease_token,
                &accept.content_sha256,
                accept.local_sequence,
                piqae_storage_postgres::DeliveryAttemptProof {
                    reservation_id: &reservation_id,
                    generation,
                    fencing_token,
                },
            )
            .await
            .expect("cross-tenant reconciliation is privacy safe");
        assert_eq!(foreign_scope, (false, false, false));
    }
}
