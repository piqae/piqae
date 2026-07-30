use anyhow::{Context, Result, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use clap::{Parser, Subcommand, ValueEnum};
use piqae_auth::{generate_platform_service_account_key, rotate_platform_service_account_key};
use piqae_domain::{EnvironmentId, WorkspaceId};
use piqae_storage_postgres::PostgresStore;
use reqwest::{Client, Method, StatusCode};
use serde_json::{Value, json};
use std::{path::PathBuf, str::FromStr};
use uuid::Uuid;

#[derive(Debug, Parser)]
#[command(name = "piqaectl", version, about = "Operate a Piqae deployment")]
struct Arguments {
    /// Piqae API origin. Defaults to the local-only agent API.
    #[arg(
        long,
        env = "PIQAE_API_ORIGIN",
        default_value = "http://127.0.0.1:39100"
    )]
    api_origin: String,
    /// Native API token.
    #[arg(long, env = "PIQAE_API_KEY", hide_env_values = true)]
    api_key: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Health,
    Printers,
    Jobs {
        #[command(subcommand)]
        command: JobCommand,
    },
    Platform {
        #[command(subcommand)]
        command: PlatformCommand,
    },
}

#[derive(Debug, Subcommand)]
enum PlatformCommand {
    Create {
        #[arg(long)]
        name: String,
        #[arg(long)]
        workspace: String,
        #[arg(long)]
        environment: String,
        #[arg(long, value_delimiter = ',', required = true)]
        scopes: Vec<String>,
    },
    Grant {
        #[arg(long)]
        service_account: String,
        #[arg(long)]
        workspace: String,
        #[arg(long)]
        environment: String,
        #[arg(long, value_delimiter = ',', required = true)]
        scopes: Vec<String>,
    },
    RevokeGrant {
        #[arg(long)]
        service_account: String,
        #[arg(long)]
        workspace: String,
        #[arg(long)]
        environment: String,
    },
    Rotate {
        #[arg(long)]
        service_account: String,
    },
    Revoke {
        #[arg(long)]
        service_account: String,
    },
    Delete {
        #[arg(long)]
        service_account: String,
        /// Must exactly repeat the service-account ID.
        #[arg(long)]
        confirm: String,
    },
}

#[derive(Debug, Subcommand)]
enum JobCommand {
    List,
    Get {
        job_id: String,
    },
    Cancel {
        job_id: String,
    },
    Submit {
        #[arg(long)]
        printer: String,
        #[arg(long)]
        title: String,
        #[arg(long, value_enum, default_value_t = ContentType::Pdf)]
        content_type: ContentType,
        #[arg(long, conflicts_with = "uri", required_unless_present = "uri")]
        file: Option<PathBuf>,
        #[arg(long, conflicts_with = "file", required_unless_present = "file")]
        uri: Option<String>,
        #[arg(long, default_value_t = 1)]
        deliveries: u16,
        #[arg(long)]
        idempotency_key: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ContentType {
    Pdf,
    Raw,
}

impl ContentType {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::Raw => "raw",
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let arguments = Arguments::parse();
    let client = Client::builder()
        .user_agent(concat!("piqaectl/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("failed to construct HTTP client")?;

    let (method, path, body, idempotency_key) = match arguments.command {
        Command::Health => (Method::GET, "/v1/health".to_owned(), None, None),
        Command::Printers => (Method::GET, "/v1/printers".to_owned(), None, None),
        Command::Jobs {
            command: JobCommand::List,
        } => (Method::GET, "/v1/jobs".to_owned(), None, None),
        Command::Jobs {
            command: JobCommand::Get { job_id },
        } => (Method::GET, format!("/v1/jobs/{job_id}"), None, None),
        Command::Jobs {
            command: JobCommand::Cancel { job_id },
        } => (
            Method::POST,
            format!("/v1/jobs/{job_id}/cancel"),
            None,
            None,
        ),
        Command::Jobs {
            command:
                JobCommand::Submit {
                    printer,
                    title,
                    content_type,
                    file,
                    uri,
                    deliveries,
                    idempotency_key,
                },
        } => {
            let content = if let Some(path) = file {
                let bytes = tokio::fs::read(&path)
                    .await
                    .with_context(|| format!("failed to read {}", path.display()))?;
                json!({"type": "base64", "data": STANDARD.encode(bytes)})
            } else if let Some(uri) = uri {
                json!({"type": "uri", "uri": uri})
            } else {
                bail!("either --file or --uri is required");
            };
            (
                Method::POST,
                "/v1/jobs".to_owned(),
                Some(json!({
                    "printer_id": printer,
                    "title": title,
                    "content_type": content_type.as_str(),
                    "content": content,
                    "deliveries": deliveries
                })),
                idempotency_key,
            )
        }
        Command::Platform { command } => {
            run_platform_command(command).await?;
            return Ok(());
        }
    };

    let url = format!("{}{}", arguments.api_origin.trim_end_matches('/'), path);
    let mut request = client.request(method, url);
    if let Some(key) = arguments.api_key {
        request = request.bearer_auth(key);
    }
    if let Some(key) = idempotency_key {
        request = request.header("Idempotency-Key", key);
    }
    if let Some(body) = body {
        request = request.json(&body);
    }

    let response = request.send().await.context("Piqae request failed")?;
    let status = response.status();
    let request_id = response
        .headers()
        .get("Request-Id")
        .and_then(|header| header.to_str().ok())
        .map(str::to_owned);
    let body = response
        .text()
        .await
        .context("failed to read Piqae response")?;

    if !status.is_success() {
        print_failure(status, request_id.as_deref(), &body);
        bail!("Piqae returned {status}");
    }

    match serde_json::from_str::<Value>(&body) {
        Ok(value) => println!("{}", serde_json::to_string_pretty(&value)?),
        Err(_) => println!("{body}"),
    }
    Ok(())
}

async fn run_platform_command(command: PlatformCommand) -> Result<()> {
    let store = platform_store().await?;
    match command {
        PlatformCommand::Create {
            name,
            workspace,
            environment,
            scopes,
        } => {
            let workspace_id = parse_workspace(&workspace)?;
            let environment_id = parse_environment(&environment)?;
            let generated = generate_platform_service_account_key()
                .context("failed to generate platform credential")?;
            store
                .create_platform_service_account_with_grant(
                    &generated.id.to_string(),
                    &name,
                    &generated.password_hash,
                    workspace_id,
                    environment_id,
                    &scopes,
                    None,
                )
                .await
                .context("failed to create platform service account")?;
            println!("service_account_id={}", generated.id);
            println!("credential={}", generated.plaintext);
            eprintln!("Store the credential now; Piqae cannot display it again.");
        }
        PlatformCommand::Grant {
            service_account,
            workspace,
            environment,
            scopes,
        } => {
            store
                .upsert_platform_workspace_grant(
                    &service_account,
                    parse_workspace(&workspace)?,
                    parse_environment(&environment)?,
                    &scopes,
                    None,
                )
                .await
                .context("failed to grant platform workspace access")?;
        }
        PlatformCommand::RevokeGrant {
            service_account,
            workspace,
            environment,
        } => {
            store
                .revoke_platform_workspace_grant(
                    &service_account,
                    parse_workspace(&workspace)?,
                    parse_environment(&environment)?,
                )
                .await
                .context("failed to revoke platform workspace access")?;
        }
        PlatformCommand::Rotate { service_account } => {
            let account_id =
                Uuid::parse_str(&service_account).context("invalid platform service-account ID")?;
            let generated = rotate_platform_service_account_key(account_id)
                .context("failed to rotate platform credential")?;
            store
                .rotate_platform_service_account(&service_account, &generated.password_hash)
                .await
                .context("failed to rotate platform service account")?;
            println!("service_account_id={service_account}");
            println!("credential={}", generated.plaintext);
            eprintln!(
                "Store the credential now; the previous credential is invalid and Piqae cannot display the new one again."
            );
        }
        PlatformCommand::Revoke { service_account } => {
            store
                .revoke_platform_service_account(&service_account)
                .await
                .context("failed to revoke platform service account")?;
        }
        PlatformCommand::Delete {
            service_account,
            confirm,
        } => {
            if confirm != service_account {
                bail!("--confirm must exactly match --service-account");
            }
            store
                .delete_platform_service_account(&service_account)
                .await
                .context("failed to delete platform service account")?;
        }
    }
    Ok(())
}

async fn platform_store() -> Result<PostgresStore> {
    let database_url = std::env::var("PIQAE_DATABASE_URL")
        .context("PIQAE_DATABASE_URL is required for platform operator commands")?;
    let store = PostgresStore::connect(&database_url, 2)
        .await
        .context("failed to connect to the Piqae database")?;
    store.migrate().await.context("database migration failed")?;
    Ok(store)
}

fn parse_workspace(value: &str) -> Result<WorkspaceId> {
    WorkspaceId::from_str(value).context("invalid workspace ID")
}

fn parse_environment(value: &str) -> Result<EnvironmentId> {
    EnvironmentId::from_str(value).context("invalid environment ID")
}

fn print_failure(status: StatusCode, request_id: Option<&str>, body: &str) {
    eprintln!("Piqae request failed with {status}");
    if let Some(request_id) = request_id {
        eprintln!("Request-Id: {request_id}");
    }
    if let Ok(value) = serde_json::from_str::<Value>(body) {
        if let Ok(pretty) = serde_json::to_string_pretty(&value) {
            eprintln!("{pretty}");
            return;
        }
    }
    eprintln!("{body}");
}
