use anyhow::{Context, Result, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use clap::{Parser, Subcommand, ValueEnum};
use reqwest::{Client, Method, StatusCode};
use serde_json::{Value, json};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "spoolctl", version, about = "Operate a Spool deployment")]
struct Arguments {
    /// Spool API origin. Defaults to the local-only agent API.
    #[arg(
        long,
        env = "SPOOL_API_ORIGIN",
        default_value = "http://127.0.0.1:39100"
    )]
    api_origin: String,
    /// Native API token.
    #[arg(long, env = "SPOOL_API_KEY", hide_env_values = true)]
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
        .user_agent(concat!("spoolctl/", env!("CARGO_PKG_VERSION")))
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

    let response = request.send().await.context("Spool request failed")?;
    let status = response.status();
    let request_id = response
        .headers()
        .get("Request-Id")
        .and_then(|header| header.to_str().ok())
        .map(str::to_owned);
    let body = response
        .text()
        .await
        .context("failed to read Spool response")?;

    if !status.is_success() {
        print_failure(status, request_id.as_deref(), &body);
        bail!("Spool returned {status}");
    }

    match serde_json::from_str::<Value>(&body) {
        Ok(value) => println!("{}", serde_json::to_string_pretty(&value)?),
        Err(_) => println!("{body}"),
    }
    Ok(())
}

fn print_failure(status: StatusCode, request_id: Option<&str>, body: &str) {
    eprintln!("Spool request failed with {status}");
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
