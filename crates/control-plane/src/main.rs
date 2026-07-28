use anyhow::{Context, Result};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use spool_control_plane::{
    AppState,
    authentication::{
        CombinedAuthenticator, PostgresAuthenticator, StaticAuthenticator, TenantContext,
    },
    repository::Repository,
    router,
};
use spool_domain::{EnvironmentId, WorkspaceId};
use spool_object_store::{FileObjectStore, ObjectStore, S3Configuration, S3ObjectStore};
use spool_storage_postgres::PostgresStore;
use std::{env, net::SocketAddr, str::FromStr, sync::Arc};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    if env::args().nth(1).as_deref() == Some("healthcheck") {
        return healthcheck().await;
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .json()
        .init();

    let database_url = env::var("SPOOL_DATABASE_URL")
        .or_else(|_| env::var("DATABASE_URL"))
        .context("SPOOL_DATABASE_URL or DATABASE_URL is required")?;
    let listen = env::var("SPOOL_BIND")
        .or_else(|_| env::var("SPOOL_LISTEN"))
        .unwrap_or_else(|_| "0.0.0.0:8080".into());
    let webhook_key = parse_webhook_key(
        &env::var("SPOOL_WEBHOOK_MASTER_KEY").context("SPOOL_WEBHOOK_MASTER_KEY is required")?,
    )?;
    let bootstrap_key = env::var("SPOOL_BOOTSTRAP_API_KEY").ok();
    let workspace_id = env::var("SPOOL_BOOTSTRAP_WORKSPACE_ID")
        .ok()
        .map(|value| WorkspaceId::from_str(&value))
        .transpose()
        .context("invalid SPOOL_BOOTSTRAP_WORKSPACE_ID")?
        .unwrap_or_default();
    let environment_id = env::var("SPOOL_BOOTSTRAP_ENVIRONMENT_ID")
        .ok()
        .map(|value| EnvironmentId::from_str(&value))
        .transpose()
        .context("invalid SPOOL_BOOTSTRAP_ENVIRONMENT_ID")?
        .unwrap_or_default();

    let store = PostgresStore::connect(&database_url, 20)
        .await
        .context("connect to PostgreSQL")?;
    store.migrate().await.context("run PostgreSQL migrations")?;
    let repository: Arc<dyn Repository> = Arc::new(store.clone());
    let object_store = build_object_store().await?;
    let bootstrap = if let Some(bootstrap_key) = bootstrap_key {
        let authenticator = StaticAuthenticator::default();
        authenticator
            .insert(
                &bootstrap_key,
                TenantContext {
                    workspace_id,
                    environment_id,
                },
            )
            .await;
        Some(authenticator)
    } else {
        None
    };
    let authenticator = CombinedAuthenticator::new(PostgresAuthenticator::new(store), bootstrap);
    let address: SocketAddr = listen.parse().context("invalid SPOOL_LISTEN")?;
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .context("bind HTTP listener")?;
    tracing::info!(%address, %workspace_id, %environment_id, "spool server listening");
    axum::serve(
        listener,
        router(AppState::new_with_resources(
            repository,
            Arc::new(authenticator),
            webhook_key,
            object_store,
        )),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .context("serve HTTP")
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
