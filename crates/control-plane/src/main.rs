mod observability;

use anyhow::{Context, Result};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use piqae_control_plane::{
    AppState, AuthCapabilities, BillingCapabilities, DeploymentCapabilities, PlatformCapabilities,
    UpdateCapabilities,
    auth_maintenance_worker::AuthMaintenanceWorker,
    authentication::{
        CombinedAuthenticator, LocalSessionAuthenticator, OidcAuthenticator, OidcConfiguration,
        PostgresAuthenticator, StaticAuthenticator, TenantContext,
    },
    billing_usage_worker::BillingUsageWorker,
    identity::LocalIdentityState,
    repository::Repository,
    router,
    webhook_worker::WebhookWorker,
};
use piqae_domain::{EnvironmentId, WorkspaceId};
use piqae_object_store::{
    FileObjectStore, GcsConfiguration, GcsObjectStore, ObjectStore, S3Configuration, S3ObjectStore,
};
use piqae_storage_postgres::PostgresStore;
use std::{env, net::SocketAddr, str::FromStr, sync::Arc};
use tracing::Instrument;

#[tokio::main]
async fn main() -> Result<()> {
    if env::args().nth(1).as_deref() == Some("healthcheck") {
        return healthcheck().await;
    }
    if env::args().nth(1).as_deref() == Some("migrate") {
        return migrate_only().await;
    }
    let observability = observability::init()?;
    let run_span = tracing::info_span!(
        "service.run",
        otel.kind = "internal",
        otel.status_code = tracing::field::Empty,
        error.type = tracing::field::Empty,
    );
    let result = run().instrument(run_span.clone()).await;
    if result.is_err() {
        run_span.record("otel.status_code", "ERROR");
        run_span.record("error.type", "service_failure");
        let _entered = run_span.enter();
        tracing::error!(
            error.type = "service_failure",
            "control plane stopped with an error"
        );
    }
    let shutdown_result = observability.shutdown();
    result.and(shutdown_result)
}

#[allow(clippy::too_many_lines)]
async fn run() -> Result<()> {
    let service_role = ServiceRole::from_environment()?;
    let database_url = product_env("PIQAE_DATABASE_URL")
        .or_else(|_| env::var("DATABASE_URL"))
        .context("PIQAE_DATABASE_URL or DATABASE_URL is required")?;
    let listen = product_env("PIQAE_BIND")
        .or_else(|_| product_env("PIQAE_LISTEN"))
        .unwrap_or_else(|_| "0.0.0.0:8080".into());
    let webhook_key = parse_webhook_key(
        &product_env("PIQAE_WEBHOOK_MASTER_KEY").context("PIQAE_WEBHOOK_MASTER_KEY is required")?,
    )?;
    let bootstrap_key = product_env("PIQAE_BOOTSTRAP_API_KEY")
        .ok()
        .filter(|value| !value.is_empty());

    let store = PostgresStore::connect(&database_url, 20)
        .await
        .context("connect to PostgreSQL")?;
    if startup_migrations_enabled() {
        store.migrate().await.context("run PostgreSQL migrations")?;
    }
    let repository: Arc<dyn Repository> = Arc::new(store.clone());
    let object_store = build_object_store().await?;
    let bootstrap = if let Some(bootstrap_key) = bootstrap_key {
        let workspace_id = WorkspaceId::from_str(
            &product_env("PIQAE_BOOTSTRAP_WORKSPACE_ID")
                .context("PIQAE_BOOTSTRAP_WORKSPACE_ID is required with bootstrap auth")?,
        )
        .context("invalid PIQAE_BOOTSTRAP_WORKSPACE_ID")?;
        let environment_id = EnvironmentId::from_str(
            &product_env("PIQAE_BOOTSTRAP_ENVIRONMENT_ID")
                .context("PIQAE_BOOTSTRAP_ENVIRONMENT_ID is required with bootstrap auth")?,
        )
        .context("invalid PIQAE_BOOTSTRAP_ENVIRONMENT_ID")?;
        store
            .ensure_bootstrap_tenant(workspace_id, environment_id)
            .await
            .context("seed bootstrap tenant")?;
        let authenticator = StaticAuthenticator::default();
        authenticator
            .insert(
                &bootstrap_key,
                TenantContext::unrestricted(workspace_id, environment_id),
            )
            .await;
        Some(authenticator)
    } else {
        None
    };
    let oidc = build_oidc_authenticator(&store)?;
    let local_identity = local_identity_enabled().then(|| {
        LocalIdentityState::new(
            store.clone(),
            product_env("PIQAE_LOCAL_OWNER_BOOTSTRAP_TOKEN")
                .ok()
                .as_deref(),
            product_env("PIQAE_LOCAL_OWNER_SESSION_SECONDS")
                .ok()
                .and_then(|value| value.parse().ok()),
        )
    });
    let authenticator = CombinedAuthenticator::new(
        PostgresAuthenticator::new(store.clone()),
        local_identity
            .as_ref()
            .map(|_| LocalSessionAuthenticator::new(store.clone())),
        bootstrap,
        oidc,
    );
    let capabilities = deployment_capabilities();
    let public_control_plane_url = product_env("PIQAE_PUBLIC_API_URL").or_else(|_| {
        if capabilities.deployment == "cloud" {
            Ok("https://api.piqae.com".to_owned())
        } else {
            Err(anyhow::anyhow!(
                "PIQAE_PUBLIC_API_URL is required for self-hosted node connections"
            ))
        }
    })?;
    let public_control_plane_url =
        piqae_control_plane::api::validated_control_plane_url(&public_control_plane_url)
            .context("PIQAE_PUBLIC_API_URL is invalid")?;
    let mut application = AppState::new_with_resources(
        repository,
        Arc::new(authenticator),
        webhook_key,
        object_store,
    )
    .with_capabilities(capabilities.clone())
    .with_public_control_plane_url(public_control_plane_url);
    if capabilities.auth.provider == "workos" && service_role.accepts_identity_webhooks() {
        application = application.with_workos_webhook_secret(
            env::var("WORKOS_WEBHOOK_SECRET")
                .or_else(|_| product_env("PIQAE_WORKOS_WEBHOOK_SECRET"))
                .context("WORKOS_WEBHOOK_SECRET is required with WorkOS identity")?,
        );
    }
    if capabilities.billing.enabled {
        application = application.with_stripe_webhook_secret(
            env::var("STRIPE_WEBHOOK_SECRET")
                .or_else(|_| product_env("PIQAE_STRIPE_WEBHOOK_SECRET"))
                .context("STRIPE_WEBHOOK_SECRET is required when Cloud billing is enabled")?,
        );
    }
    let billing_usage_worker = if capabilities.billing.enabled && service_role.runs_workers() {
        Some(
            BillingUsageWorker::new(
                store.clone(),
                env::var("STRIPE_SECRET_KEY")
                    .or_else(|_| product_env("PIQAE_STRIPE_SECRET_KEY"))
                    .context("STRIPE_SECRET_KEY is required by the Cloud billing worker")?,
                env::var("STRIPE_METER_EVENT_NAME")
                    .or_else(|_| product_env("PIQAE_STRIPE_METER_EVENT_NAME"))
                    .unwrap_or_else(|_| "piqae_print_overage_blocks".into()),
            )
            .context("build Stripe billing worker HTTP client")?,
        )
    } else {
        None
    };
    if let Some(local_identity) = local_identity {
        application = application.with_local_identity(local_identity);
    }
    let _webhook_worker = if service_role.runs_workers() {
        let webhook_worker = WebhookWorker::new(application.clone());
        Some(tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(500));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                match webhook_worker.run_once(25).await {
                    Ok(0) => {}
                    Ok(count) => tracing::debug!(count, "processed webhook deliveries"),
                    Err(error) => tracing::error!(%error, "webhook worker batch failed"),
                }
            }
        }))
    } else {
        None
    };
    let _billing_usage_worker = billing_usage_worker.map(spawn_billing_usage_worker);
    let _auth_maintenance_worker = service_role
        .runs_workers()
        .then(|| spawn_auth_maintenance_worker(AuthMaintenanceWorker::new(store)));
    let address: SocketAddr = listen
        .parse()
        .context("invalid PIQAE_BIND or PIQAE_LISTEN")?;
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .context("bind HTTP listener")?;
    tracing::info!(%address, role = service_role.as_str(), "piqae server listening");
    axum::serve(listener, router(application))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("serve HTTP")
}

fn spawn_auth_maintenance_worker(worker: AuthMaintenanceWorker) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            match worker.run_once().await {
                Ok(purged) if purged == piqae_storage_postgres::PurgedAuthState::default() => {}
                Ok(purged) => tracing::info!(
                    nonces = purged.nonces,
                    device_authorizations = purged.device_authorizations,
                    "purged expired node authentication state"
                ),
                Err(error) => {
                    tracing::error!(error.type = "auth_state_purge", %error);
                }
            }
        }
    })
}

fn spawn_billing_usage_worker(worker: BillingUsageWorker) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            match worker.run_once(25).await {
                Ok(0) => {}
                Ok(count) => tracing::info!(count, "submitted Stripe usage meter events"),
                Err(error) => {
                    tracing::error!(error.type = "stripe_usage_export", %error);
                }
            }
        }
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServiceRole {
    Api,
    Sync,
    Worker,
    All,
}

impl ServiceRole {
    fn from_environment() -> Result<Self> {
        match product_env("PIQAE_SERVICE_ROLE")
            .unwrap_or_else(|_| "all".into())
            .as_str()
        {
            "api" => Ok(Self::Api),
            "sync" => Ok(Self::Sync),
            "worker" => Ok(Self::Worker),
            "all" => Ok(Self::All),
            other => anyhow::bail!(
                "unsupported PIQAE_SERVICE_ROLE `{other}`; expected api, sync, worker, or all"
            ),
        }
    }

    const fn runs_workers(self) -> bool {
        matches!(self, Self::Worker | Self::All)
    }

    const fn accepts_identity_webhooks(self) -> bool {
        !matches!(self, Self::Worker)
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Api => "api",
            Self::Sync => "sync",
            Self::Worker => "worker",
            Self::All => "all",
        }
    }
}

fn startup_migrations_enabled() -> bool {
    if let Ok(value) = product_env("PIQAE_RUN_MIGRATIONS_ON_STARTUP") {
        return value == "true";
    }
    !(product_env("PIQAE_DEPLOYMENT").as_deref() == Ok("cloud")
        && product_env("PIQAE_ENVIRONMENT").as_deref() == Ok("production"))
}

async fn migrate_only() -> Result<()> {
    let database_url = product_env("PIQAE_DATABASE_URL")
        .or_else(|_| env::var("DATABASE_URL"))
        .context("PIQAE_DATABASE_URL or DATABASE_URL is required")?;
    let store = PostgresStore::connect(&database_url, 2)
        .await
        .context("connect to PostgreSQL")?;
    store.migrate().await.context("run PostgreSQL migrations")
}

fn local_identity_enabled() -> bool {
    let cloud = product_env("PIQAE_DEPLOYMENT").as_deref() == Ok("cloud");
    product_env("PIQAE_IDENTITY_PROVIDER").unwrap_or_else(|_| {
        if cloud {
            "workos".into()
        } else {
            "local_owner".into()
        }
    }) == "local_owner"
}

fn deployment_capabilities() -> DeploymentCapabilities {
    let deployment = product_env("PIQAE_DEPLOYMENT").unwrap_or_else(|_| "self_hosted".into());
    let cloud = deployment == "cloud";
    let configured_auth = product_env("PIQAE_IDENTITY_PROVIDER").unwrap_or_else(|_| {
        if cloud {
            "workos".into()
        } else {
            "local_owner".into()
        }
    });
    let auth_provider = match configured_auth.as_str() {
        "bootstrap" | "api_key" => "local_owner",
        other => other,
    };
    DeploymentCapabilities {
        deployment,
        version: env!("CARGO_PKG_VERSION"),
        auth: AuthCapabilities {
            provider: auth_provider.into(),
            workspace_switching: cloud,
            invitations: cloud,
        },
        billing: BillingCapabilities {
            enabled: cloud && product_env("PIQAE_BILLING_ENABLED").as_deref() != Ok("false"),
        },
        updates: UpdateCapabilities {
            official_feed: product_env("PIQAE_OFFICIAL_UPDATE_FEED").as_deref() != Ok("false"),
            custom_feed: !cloud,
        },
        platform: PlatformCapabilities { accounts: true },
    }
}

fn build_oidc_authenticator(store: &PostgresStore) -> Result<Option<OidcAuthenticator>> {
    match product_env("PIQAE_AUTH_MODE")
        .unwrap_or_else(|_| "bootstrap".into())
        .as_str()
    {
        "bootstrap" | "api_key" => Ok(None),
        "oidc" | "hybrid" => Ok(Some(
            OidcAuthenticator::new(
                store.clone(),
                OidcConfiguration {
                    provider: product_env("PIQAE_IDENTITY_PROVIDER")
                        .ok()
                        .filter(|value| matches!(value.as_str(), "workos" | "oidc"))
                        .unwrap_or_else(|| "oidc".into()),
                    issuer: product_env("PIQAE_OIDC_ISSUER")
                        .context("PIQAE_OIDC_ISSUER is required for OIDC")?,
                    audience: product_env("PIQAE_OIDC_AUDIENCE")
                        .ok()
                        .filter(|value| !value.is_empty()),
                    binding_claim: product_env("PIQAE_OIDC_BINDING_CLAIM")
                        .ok()
                        .filter(|value| !value.is_empty())
                        .or_else(|| {
                            product_env("PIQAE_OIDC_CLIENT_ID")
                                .ok()
                                .filter(|value| !value.is_empty())
                                .map(|_| "client_id".into())
                        }),
                    binding_value: product_env("PIQAE_OIDC_BINDING_VALUE")
                        .ok()
                        .filter(|value| !value.is_empty())
                        .or_else(|| {
                            product_env("PIQAE_OIDC_CLIENT_ID")
                                .ok()
                                .filter(|value| !value.is_empty())
                        }),
                    jwks_url: product_env("PIQAE_OIDC_JWKS_URL")
                        .context("PIQAE_OIDC_JWKS_URL is required for OIDC")?,
                    organization_claim: product_env("PIQAE_OIDC_ORGANIZATION_CLAIM")
                        .unwrap_or_else(|_| "org_id".into()),
                    permissions_claim: product_env("PIQAE_OIDC_PERMISSIONS_CLAIM")
                        .unwrap_or_else(|_| "permissions".into()),
                    environment_kind: product_env("PIQAE_OIDC_ENVIRONMENT")
                        .unwrap_or_else(|_| "live".into()),
                    allow_unrestricted: product_env("PIQAE_OIDC_ALLOW_UNRESTRICTED").as_deref()
                        == Ok("true"),
                },
            )
            .map_err(|_| anyhow::anyhow!("invalid OIDC configuration"))?,
        )),
        other => anyhow::bail!("unsupported PIQAE_AUTH_MODE `{other}`"),
    }
}

async fn build_object_store() -> Result<Arc<dyn ObjectStore>> {
    match product_env("PIQAE_OBJECT_STORE")
        .unwrap_or_else(|_| "filesystem".into())
        .as_str()
    {
        "s3" => Ok(Arc::new(S3ObjectStore::new(S3Configuration {
            bucket: product_env("PIQAE_S3_BUCKET").context("PIQAE_S3_BUCKET is required")?,
            region: product_env("PIQAE_S3_REGION").unwrap_or_else(|_| "auto".into()),
            endpoint: product_env("PIQAE_S3_ENDPOINT").ok(),
            access_key_id: product_env("PIQAE_S3_ACCESS_KEY_ID")
                .context("PIQAE_S3_ACCESS_KEY_ID is required")?,
            secret_access_key: product_env("PIQAE_S3_SECRET_ACCESS_KEY")
                .context("PIQAE_S3_SECRET_ACCESS_KEY is required")?,
            allow_http: product_env("PIQAE_S3_ALLOW_HTTP").as_deref() == Ok("true"),
            virtual_hosted_style: product_env("PIQAE_S3_VIRTUAL_HOSTED_STYLE").as_deref()
                == Ok("true"),
        })?)),
        "gcs" => Ok(Arc::new(GcsObjectStore::new_gcs(GcsConfiguration {
            bucket: product_env("PIQAE_GCS_BUCKET").context("PIQAE_GCS_BUCKET is required")?,
            service_account_path: env::var("GOOGLE_APPLICATION_CREDENTIALS")
                .ok()
                .filter(|value| !value.is_empty()),
        })?)),
        "filesystem" => Ok(Arc::new(
            FileObjectStore::new(
                product_env("PIQAE_OBJECT_STORE_PATH")
                    .unwrap_or_else(|_| "/var/lib/piqae/objects".into()),
            )
            .await?,
        )),
        other => anyhow::bail!("unsupported PIQAE_OBJECT_STORE `{other}`"),
    }
}

/// Reads the Piqae variable first and its pre-rebrand equivalent second.
///
/// New deployments expose only `PIQAE_*`. The fallback keeps existing
/// self-hosted and managed deployments upgradeable through V1.
fn product_env(name: &str) -> Result<String, env::VarError> {
    env::var(name).or_else(|canonical_error| {
        let Some(suffix) = name.strip_prefix("PIQAE_") else {
            return Err(canonical_error);
        };
        env::var(format!("SPOOL_{suffix}")).map_err(|_| canonical_error)
    })
}

async fn healthcheck() -> Result<()> {
    let arguments = env::args().collect::<Vec<_>>();
    let url = arguments
        .windows(2)
        .find(|pair| pair[0] == "--url")
        .map_or("http://127.0.0.1:8080/v1/health", |pair| pair[1].as_str());
    let response = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .context("build healthcheck client")?
        .get(url)
        .send()
        .await
        .context("healthcheck request failed")?;
    anyhow::ensure!(
        response.status().is_success(),
        "healthcheck returned {}",
        response.status()
    );
    Ok(())
}

fn parse_webhook_key(value: &str) -> Result<[u8; 32]> {
    let decoded = URL_SAFE_NO_PAD
        .decode(value)
        .or_else(|_| STANDARD.decode(value))
        .context("PIQAE_WEBHOOK_MASTER_KEY must be base64")?;
    decoded
        .try_into()
        .map_err(|_| anyhow::anyhow!("PIQAE_WEBHOOK_MASTER_KEY must decode to exactly 32 bytes"))
}

async fn shutdown_signal() {
    let control_c = async {
        if tokio::signal::ctrl_c().await.is_err() {
            tracing::error!("failed to install Ctrl+C handler");
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => tracing::error!(%error, "failed to install terminate handler"),
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = control_c => {},
        () = terminate => {},
    }
}
