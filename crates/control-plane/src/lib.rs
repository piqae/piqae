//! Authoritative Spool HTTP control plane.

pub mod api;
pub mod authentication;
pub mod billing;
pub mod billing_usage_worker;
pub mod compatibility;
pub mod device_auth;
pub mod error;
pub mod identity;
pub mod pairing;
pub mod platform;
pub mod repository;
pub mod request_id;
pub mod routing;
pub mod updates;
pub mod webhook_worker;
pub mod workos_identity;

use authentication::{Authenticator, TenantContext};
use axum::{
    Router,
    extract::DefaultBodyLimit,
    middleware,
    routing::{get, post},
};
use repository::Repository;
use serde::Serialize;
use spool_object_store::{MemoryObjectStore, ObjectStore};
use spool_webhooks::WebhookSecretBox;
use std::{fmt, sync::Arc};
use tokio::sync::broadcast;
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
    pub authenticator: Arc<dyn Authenticator>,
    pub events: broadcast::Sender<PublishedEvent>,
    pub webhook_secrets: Arc<WebhookSecretBox>,
    pub object_store: Arc<dyn ObjectStore>,
    pub capabilities: DeploymentCapabilities,
    pub local_identity: Option<identity::LocalIdentityState>,
    pub stripe_webhook_secret: Option<Arc<str>>,
    pub workos_webhook_secret: Option<Arc<str>>,
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
    #[must_use]
    pub fn new(repository: Arc<dyn Repository>, authenticator: Arc<dyn Authenticator>) -> Self {
        Self::new_with_resources(
            repository,
            authenticator,
            [0; 32],
            Arc::new(MemoryObjectStore::default()),
        )
    }

    #[must_use]
    pub fn new_with_webhook_key(
        repository: Arc<dyn Repository>,
        authenticator: Arc<dyn Authenticator>,
        webhook_key: [u8; 32],
    ) -> Self {
        Self::new_with_resources(
            repository,
            authenticator,
            webhook_key,
            Arc::new(MemoryObjectStore::default()),
        )
    }

    #[must_use]
    pub fn new_with_resources(
        repository: Arc<dyn Repository>,
        authenticator: Arc<dyn Authenticator>,
        webhook_key: [u8; 32],
        object_store: Arc<dyn ObjectStore>,
    ) -> Self {
        let (events, _) = broadcast::channel(1_024);
        Self {
            repository,
            authenticator,
            events,
            webhook_secrets: Arc::new(WebhookSecretBox::new(webhook_key)),
            object_store,
            capabilities: DeploymentCapabilities::default(),
            local_identity: None,
            stripe_webhook_secret: None,
            workos_webhook_secret: None,
        }
    }

    #[must_use]
    pub fn with_capabilities(mut self, capabilities: DeploymentCapabilities) -> Self {
        self.capabilities = capabilities;
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
        .route("/v1/platform/accounts", get(platform::list))
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
            "/v1/stocks",
            get(routing::list_stocks).post(routing::create_stock),
        )
        .route(
            "/v1/stocks/{stock_id}",
            axum::routing::patch(routing::patch_stock),
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
        .route("/v1/agent-enrolments", post(api::create_agent_enrolment))
        .route("/v1/agents/enrol", post(api::enrol_agent))
        .route("/v1/uploads", post(api::create_upload))
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
        .route("/v1/jobs/{job_id}/cancel", post(api::cancel_job))
        .route("/v1/events/stream", get(api::stream_events))
        .route("/v1/agent/sync", post(api::agent_sync))
        .route(
            "/v1/agent/jobs/{job_id}/accept",
            post(api::accept_agent_job),
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
    Router::new()
        .route("/v1/device-authorizations", post(pairing::create))
        .route(
            "/v1/device-authorizations/{device_code}",
            get(pairing::status),
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
        .route(
            "/v1/device-authorizations/{device_code}/exchange",
            post(pairing::exchange),
        )
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
            "/v1/nodes/{node_id}",
            get(api::get_node)
                .patch(api::patch_node)
                .delete(api::delete_node),
        )
        .route("/v1/nodes/{node_id}/pause", post(api::pause_node))
        .route("/v1/nodes/{node_id}/resume", post(api::resume_node))
        .route(
            "/v1/nodes/{node_id}/diagnostics",
            post(api::request_node_diagnostics),
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
    use rand::rngs::OsRng;
    use sha2::{Digest, Sha256};
    use spool_auth::Scope;
    use spool_domain::{
        AgentId, DriverFingerprint, EnvironmentId, JobId, JobOptions, JobState,
        NativePrinterChoice, NativePrinterOption, NativeProfileKind, PrinterCapabilities,
        PrinterId, PrinterState, ProfileStatus, ProfileSummary, SafeProfileOverride, WorkspaceId,
    };
    use spool_object_store::{ObjectStoreError, StoredObject};
    use spool_protocol::agent::{
        AgentAcceptJobRequest, AgentCommand, AgentHealth, AgentSyncRequest, AgentSyncResponse,
        PrinterProfileSnapshot, PrinterSnapshot, QueueSnapshot,
    };
    use std::{collections::BTreeMap, str::FromStr};
    use tower::ServiceExt;

    struct TestApplication {
        router: Router,
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
            _content: spool_object_store::ObjectByteStream,
            _expected_sha256: &str,
            _expected_bytes: u64,
        ) -> Result<StoredObject, ObjectStoreError> {
            Err(ObjectStoreError::S3("unavailable".into()))
        }

        async fn get_stream(
            &self,
            _key: &str,
        ) -> Result<spool_object_store::ObjectByteStream, ObjectStoreError> {
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
        authenticator.insert("spl_test_integration", tenant).await;
        authenticator
            .insert(
                "spl_test_other",
                TenantContext::unrestricted(WorkspaceId::new(), EnvironmentId::new()),
            )
            .await;
        TestApplication {
            router: router(AppState::new(
                Arc::new(repository.clone()),
                Arc::new(authenticator),
            )),
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
            router(AppState::new(
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

    fn compatibility_request(method: &str, path: &str, body: Option<String>) -> Request<Body> {
        let credentials = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            "spl_test_integration:",
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
        let timestamp = Utc::now().timestamp_millis();
        let nonce = uuid::Uuid::new_v4();
        let digest = format!("{:x}", Sha256::digest(&body));
        let canonical = format!("{method}\n{path}\n{timestamp}\n{nonce}\n{digest}");
        let signature = application.signing_key.sign(canonical.as_bytes());
        Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json")
            .header("x-spool-agent-id", application.agent_id.to_string())
            .header("x-spool-timestamp", timestamp.to_string())
            .header("x-spool-nonce", nonce.to_string())
            .header("x-spool-body-sha256", digest)
            .header(
                "x-spool-signature",
                STANDARD_NO_PAD.encode(signature.to_bytes()),
            )
            .body(Body::from(body))
            .expect("valid signed request")
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
    async fn object_store_failure_blocks_readiness_but_not_liveness() {
        let state = AppState::new_with_resources(
            Arc::new(MemoryRepository::default()),
            Arc::new(StaticAuthenticator::default()),
            [0; 32],
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

    async fn sync_test_agent(
        application: &TestApplication,
        acknowledged_command_cursor: Option<String>,
    ) -> AgentSyncResponse {
        let now = Utc::now();
        let request = AgentSyncRequest {
            agent_id: application.agent_id,
            protocol_version: 1,
            agent_version: "test".into(),
            printer_revision: 0,
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
            printers: None,
            events: Vec::new(),
        };
        let body = serde_json::to_vec(&request).expect("sync JSON");
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
            profiles: vec![PrinterProfileSnapshot {
                profile_id: "profile_shipping".into(),
                revision: 4,
                name: "A4 shipping".into(),
                is_default: true,
                options: JobOptions {
                    paper: Some("A4".into()),
                    duplex: Some(spool_domain::Duplex::LongEdge),
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
        }
    }

    fn stored_profiled_printer(printer_id: PrinterId) -> spool_storage_postgres::SyncedPrinter {
        let printer = profiled_printer_snapshot(printer_id);
        spool_storage_postgres::SyncedPrinter {
            id: printer.id,
            native_id: printer.native_id,
            name: printer.name,
            state: printer.state,
            is_default: printer.is_default,
            capabilities: printer.capabilities,
            capability_revision: printer.capability_revision,
            native_options: printer.native_options,
            profiles: printer
                .profiles
                .into_iter()
                .map(|profile| spool_storage_postgres::PrinterProfileSnapshot {
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
                "spl_test_integration",
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
                "spl_test_integration",
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
                "spl_test_integration",
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
                "spl_test_integration",
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
                "spl_test_integration",
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
                "spl_test_integration",
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
                .get("spool.target_id")
                .map(String::as_str),
            Some(target_id)
        );
        assert_eq!(
            routed_job
                .metadata
                .get("spool.profile_revision")
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
                "spl_test_other",
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
                "spl_test_integration",
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
                "spl_test_integration",
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
                "spl_test_integration",
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
                .get("spool.target_id")
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
            .header("authorization", "Bearer spl_test_integration")
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
        assert_eq!(stored[0].state, spool_domain::JobState::WaitingForAgent);
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
                    .header("authorization", "Bearer spl_test_integration")
                    .header("x-spool-workspace-id", WorkspaceId::new().to_string())
                    .header("x-spool-environment-id", EnvironmentId::new().to_string())
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
            ("spl_test_manager", manager),
            ("spl_test_reader", read_only),
            ("spl_test_other", other_tenant),
        ])
        .await;

        let denied = application
            .clone()
            .oneshot(api_request(
                "POST",
                "/v1/api-keys",
                "spl_test_reader",
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
                "spl_test_manager",
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
                "spl_test_manager",
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
        assert!(secret.starts_with("spl_test_"));
        assert!(created.get("secret_hash").is_none());

        let listed = application
            .clone()
            .oneshot(api_request("GET", "/v1/api-keys", "spl_test_manager", None))
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
            .oneshot(api_request("GET", "/v1/api-keys", "spl_test_other", None))
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
                "spl_test_other",
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
                    "spl_test_manager",
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
            "spl_test_integration:",
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
    async fn cancellation_is_redelivered_until_the_agent_acknowledges_its_cursor() {
        let application = application().await;
        let created = application
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/jobs")
                    .header("authorization", "Bearer spl_test_integration")
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
        let rerouted_agent = AgentId::new();
        application
            .repository
            .add_printer(
                application.tenant.workspace_id,
                application.tenant.environment_id,
                application.printer_id,
                rerouted_agent,
            )
            .await;

        let cancelled = application
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/v1/jobs/{job_id}/cancel"))
                    .header("authorization", "Bearer spl_test_integration")
                    .body(Body::empty())
                    .expect("valid request"),
            )
            .await
            .expect("cancel response");
        assert_eq!(cancelled.status(), StatusCode::ACCEPTED);

        let first = sync_test_agent(&application, None).await;
        assert!(matches!(
            first.commands.as_slice(),
            [AgentCommand::CancelJob { job_id: command_job_id }] if *command_job_id == job_id
        ));
        let cursor = first.command_cursor.expect("command cursor");
        let rerouted = application
            .repository
            .sync_agent_commands(
                application.tenant.workspace_id,
                application.tenant.environment_id,
                rerouted_agent,
                None,
                100,
            )
            .await
            .expect("rerouted agent command batch");
        assert!(rerouted.commands.is_empty());

        let retry = sync_test_agent(&application, None).await;
        assert_eq!(retry.command_cursor.as_deref(), Some(cursor.as_str()));
        assert_eq!(retry.commands.len(), 1);

        let acknowledged = sync_test_agent(&application, Some(cursor.clone())).await;
        assert!(acknowledged.commands.is_empty());
        assert!(acknowledged.command_cursor.is_none());

        application
            .repository
            .transition_job(
                application.tenant.workspace_id,
                application.tenant.environment_id,
                job_id,
                JobState::Cancelled,
                None,
                Some("Cancellation completed".into()),
                Some(application.agent_id),
                None,
            )
            .await
            .expect("terminal cancellation");
        assert!(matches!(
            application
                .repository
                .request_job_cancellation(
                    application.tenant.workspace_id,
                    application.tenant.environment_id,
                    job_id,
                )
                .await,
            Err(crate::repository::RepositoryError::InvalidTransition)
        ));
        let after_rejected_cancel = application
            .repository
            .sync_agent_commands(
                application.tenant.workspace_id,
                application.tenant.environment_id,
                application.agent_id,
                Some(&cursor),
                100,
            )
            .await
            .expect("no command after rejected cancellation");
        assert!(after_rejected_cancel.commands.is_empty());
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
            .header("authorization", "Bearer spl_test_integration")
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
            printer_revision: 0,
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
            printers: None,
            events: Vec::new(),
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
        let accept = AgentAcceptJobRequest {
            lease_id: offer.lease_id,
            lease_token: offer.lease_token.clone(),
            content_sha256: match &offer.content {
                spool_protocol::agent::ContentDescriptor::Download { sha256, .. } => sha256.clone(),
                _ => panic!("expected materialized download content"),
            },
            local_sequence: 1,
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
        assert_eq!(stored.state, spool_domain::JobState::AgentAccepted);
    }
}
