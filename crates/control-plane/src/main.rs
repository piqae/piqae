mod observability;

use anyhow::{Context, Result};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use spool_control_plane::{
    AppState, AuthCapabilities, BillingCapabilities, DeploymentCapabilities, PlatformCapabilities,
    UpdateCapabilities,
    authentication::{
        CombinedAuthenticator, LocalSessionAuthenticator, OidcAuthenticator, OidcConfiguration,
        PostgresAuthenticator, StaticAuthenticator, TenantContext,
    },
    identity::LocalIdentityState,
    repository::Repository,
    router,
    webhook_worker::WebhookWorker,
};
use spool_domain::{EnvironmentId, WorkspaceId};
use spool_object_store::{FileObjectStore, ObjectStore, S3Configuration, S3ObjectStore};
use spool_storage_postgres::PostgresStore;
use std::{env, net::SocketAddr, str::FromStr, sync::Arc};
use tracing::Instrument;

#[tokio::main]
async fn main() -> Result<()> {
    if env::args().nth(1).as_deref() == Some("healthcheck") {
        return healthcheck().await;
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

async fn run() -> Result<()> {
    let database_url = env::var("SPOOL_DATABASE_URL")
        .or_else(|_| env::var("DATABASE_URL"))
        .context("SPOOL_DATABASE_URL or DATABASE_URL is required")?;
    let listen = env::var("SPOOL_BIND")
        .or_else(|_| env::var("SPOOL_LISTEN"))
        .unwrap_or_else(|_| "0.0.0.0:8080".into());
    let webhook_key = parse_webhook_key(
        &env::var("SPOOL_WEBHOOK_MASTER_KEY").context("SPOOL_WEBHOOK_MASTER_KEY is required")?,
    )?;
    let bootstrap_key = env::var("SPOOL_BOOTSTRAP_API_KEY")
        .ok()
        .filter(|value| !value.is_empty());

    let store = PostgresStore::connect(&database_url, 20)
        .await
        .context("connect to PostgreSQL")?;
    store.migrate().await.context("run PostgreSQL migrations")?;
    let repository: Arc<dyn Repository> = Arc::new(store.clone());
    let object_store = build_object_store().await?;
    let bootstrap = if let Some(bootstrap_key) = bootstrap_key {
        let workspace_id = WorkspaceId::from_str(
            &env::var("SPOOL_BOOTSTRAP_WORKSPACE_ID")
                .context("SPOOL_BOOTSTRAP_WORKSPACE_ID is required with bootstrap auth")?,
        )
        .context("invalid SPOOL_BOOTSTRAP_WORKSPACE_ID")?;
        let environment_id = EnvironmentId::from_str(
            &env::var("SPOOL_BOOTSTRAP_ENVIRONMENT_ID")
                .context("SPOOL_BOOTSTRAP_ENVIRONMENT_ID is required with bootstrap auth")?,
        )
        .context("invalid SPOOL_BOOTSTRAP_ENVIRONMENT_ID")?;
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
            env::var("SPOOL_LOCAL_OWNER_BOOTSTRAP_TOKEN")
                .ok()
                .as_deref(),
            env::var("SPOOL_LOCAL_OWNER_SESSION_SECONDS")
                .ok()
                .and_then(|value| value.parse().ok()),
        )
    });
    let authenticator = CombinedAuthenticator::new(
        PostgresAuthenticator::new(store.clone()),
        local_identity
            .as_ref()
            .map(|_| LocalSessionAuthenticator::new(store)),
        bootstrap,
        oidc,
    );
    let mut application = AppState::new_with_resources(
        repository,
        Arc::new(authenticator),
        webhook_key,
        object_store,
    )
    .with_capabilities(deployment_capabilities());
    if let Some(local_identity) = local_identity {
        application = application.with_local_identity(local_identity);
    }
    let webhook_worker = WebhookWorker::new(application.clone());
    let _webhook_worker = tokio::spawn(async move {
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
    });
    let address: SocketAddr = listen
        .parse()
        .context("invalid SPOOL_BIND or SPOOL_LISTEN")?;
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .context("bind HTTP listener")?;
    tracing::info!(%address, "spool server listening");
    axum::serve(listener, router(application))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("serve HTTP")
}

fn local_identity_enabled() -> bool {
    let cloud = env::var("SPOOL_DEPLOYMENT").as_deref() == Ok("cloud");
    env::var("SPOOL_IDENTITY_PROVIDER").unwrap_or_else(|_| {
        if cloud {
            "workos".into()
        } else {
            "local_owner".into()
        }
    }) == "local_owner"
}

fn deployment_capabilities() -> DeploymentCapabilities {
    let deployment = env::var("SPOOL_DEPLOYMENT").unwrap_or_else(|_| "self_hosted".into());
    let cloud = deployment == "cloud";
    let configured_auth = env::var("SPOOL_IDENTITY_PROVIDER").unwrap_or_else(|_| {
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
            enabled: cloud && env::var("SPOOL_BILLING_ENABLED").as_deref() != Ok("false"),
        },
        updates: UpdateCapabilities {
            official_feed: env::var("SPOOL_OFFICIAL_UPDATE_FEED").as_deref() != Ok("false"),
            custom_feed: !cloud,
        },
        platform: PlatformCapabilities { accounts: true },
    }
}

fn build_oidc_authenticator(store: &PostgresStore) -> Result<Option<OidcAuthenticator>> {
    match env::var("SPOOL_AUTH_MODE")
        .unwrap_or_else(|_| "bootstrap".into())
        .as_str()
    {
        "bootstrap" | "api_key" => Ok(None),
        "oidc" | "hybrid" => Ok(Some(
            OidcAuthenticator::new(
                store.clone(),
                OidcConfiguration {
                    issuer: env::var("SPOOL_OIDC_ISSUER")
                        .context("SPOOL_OIDC_ISSUER is required for OIDC")?,
                    audience: env::var("SPOOL_OIDC_AUDIENCE")
                        .ok()
                        .filter(|value| !value.is_empty()),
                    binding_claim: env::var("SPOOL_OIDC_BINDING_CLAIM")
                        .ok()
                        .filter(|value| !value.is_empty())
                        .or_else(|| {
                            env::var("SPOOL_OIDC_CLIENT_ID")
                                .ok()
                                .filter(|value| !value.is_empty())
                                .map(|_| "client_id".into())
                        }),
                    binding_value: env::var("SPOOL_OIDC_BINDING_VALUE")
                        .ok()
                        .filter(|value| !value.is_empty())
                        .or_else(|| {
                            env::var("SPOOL_OIDC_CLIENT_ID")
                                .ok()
                                .filter(|value| !value.is_empty())
                        }),
                    jwks_url: env::var("SPOOL_OIDC_JWKS_URL")
                        .context("SPOOL_OIDC_JWKS_URL is required for OIDC")?,
                    organization_claim: env::var("SPOOL_OIDC_ORGANIZATION_CLAIM")
                        .unwrap_or_else(|_| "org_id".into()),
                    permissions_claim: env::var("SPOOL_OIDC_PERMISSIONS_CLAIM")
                        .unwrap_or_else(|_| "permissions".into()),
                    environment_kind: env::var("SPOOL_OIDC_ENVIRONMENT")
                        .unwrap_or_else(|_| "live".into()),
                    allow_unrestricted: env::var("SPOOL_OIDC_ALLOW_UNRESTRICTED").as_deref()
                        == Ok("true"),
                },
            )
            .map_err(|_| anyhow::anyhow!("invalid OIDC configuration"))?,
        )),
        other => anyhow::bail!("unsupported SPOOL_AUTH_MODE `{other}`"),
    }
}

async fn build_object_store() -> Result<Arc<dyn ObjectStore>> {
    match env::var("SPOOL_OBJECT_STORE")
        .unwrap_or_else(|_| "filesystem".into())
        .as_str()
    {
        "s3" => Ok(Arc::new(S3ObjectStore::new(S3Configuration {
            bucket: env::var("SPOOL_S3_BUCKET").context("SPOOL_S3_BUCKET is required")?,
            region: env::var("SPOOL_S3_REGION").unwrap_or_else(|_| "auto".into()),
            endpoint: env::var("SPOOL_S3_ENDPOINT").ok(),
            access_key_id: env::var("SPOOL_S3_ACCESS_KEY_ID")
                .context("SPOOL_S3_ACCESS_KEY_ID is required")?,
            secret_access_key: env::var("SPOOL_S3_SECRET_ACCESS_KEY")
                .context("SPOOL_S3_SECRET_ACCESS_KEY is required")?,
            allow_http: env::var("SPOOL_S3_ALLOW_HTTP").as_deref() == Ok("true"),
            virtual_hosted_style: env::var("SPOOL_S3_VIRTUAL_HOSTED_STYLE").as_deref()
                == Ok("true"),
        })?)),
        "filesystem" => Ok(Arc::new(
            FileObjectStore::new(
                env::var("SPOOL_OBJECT_STORE_PATH")
                    .unwrap_or_else(|_| "/var/lib/spool/objects".into()),
            )
            .await?,
        )),
        other => anyhow::bail!("unsupported SPOOL_OBJECT_STORE `{other}`"),
    }
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
        .context("SPOOL_WEBHOOK_MASTER_KEY must be base64")?;
    decoded
        .try_into()
        .map_err(|_| anyhow::anyhow!("SPOOL_WEBHOOK_MASTER_KEY must decode to exactly 32 bytes"))
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
