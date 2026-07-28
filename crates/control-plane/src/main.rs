use anyhow::{Context, Result};
use spool_control_plane::{
    AppState,
    authentication::{StaticAuthenticator, TenantContext},
    repository::Repository,
    router,
};
use spool_domain::{EnvironmentId, WorkspaceId};
use spool_storage_postgres::PostgresStore;
use std::{env, net::SocketAddr, str::FromStr, sync::Arc};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .json()
        .init();

    let database_url = env::var("DATABASE_URL").context("DATABASE_URL is required")?;
    let listen = env::var("SPOOL_LISTEN").unwrap_or_else(|_| "0.0.0.0:8080".into());
    let bootstrap_key =
        env::var("SPOOL_BOOTSTRAP_API_KEY").context("SPOOL_BOOTSTRAP_API_KEY is required")?;
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
    let repository: Arc<dyn Repository> = Arc::new(store);
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
    let address: SocketAddr = listen.parse().context("invalid SPOOL_LISTEN")?;
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .context("bind HTTP listener")?;
    tracing::info!(%address, %workspace_id, %environment_id, "spool server listening");
    axum::serve(
        listener,
        router(AppState::new(repository, Arc::new(authenticator))),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .context("serve HTTP")
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
