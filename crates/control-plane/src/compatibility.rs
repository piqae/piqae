#![allow(clippy::branches_sharing_code)]

use crate::{
    AppState,
    api::{authenticate_compatibility, parse_job_id, persist_job_content},
    error::AppError,
    repository::CreateResult,
};
use axum::{
    Json,
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header::CONTENT_TYPE},
    response::{IntoResponse, Response},
};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use spool_auth::Scope;
use spool_domain::{ContentKind, ContentSource, Job, JobOptions, JobState, PrinterId};
use std::{collections::HashMap, str::FromStr};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WhoAmI {
    id: i64,
    email: &'static str,
    can_create_sub_accounts: bool,
    credits: i64,
    num_computers: i64,
    total_prints: i64,
    versions: Vec<String>,
}

pub(crate) async fn whoami(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<WhoAmI>, AppError> {
    authenticate_compatibility(&state, &headers, Scope::JobsRead).await?;
    Ok(Json(WhoAmI {
        id: 1,
        email: "spool@self-hosted.invalid",
        can_create_sub_accounts: false,
        credits: 0,
        num_computers: 0,
        total_prints: 0,
        versions: vec![env!("CARGO_PKG_VERSION").into()],
    }))
}

pub(crate) async fn ping(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<&'static str, AppError> {
    authenticate_compatibility(&state, &headers, Scope::JobsRead).await?;
    Ok("pong")
}

pub(crate) async fn noop(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<StatusCode, AppError> {
    authenticate_compatibility(&state, &headers, Scope::JobsRead).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrintJobRequest {
    printer_id: i64,
    title: String,
    content_type: String,
    content: String,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    options: JobOptions,
    #[serde(default = "one")]
    qty: u16,
    #[serde(default = "compatibility_expiry")]
    expire_after: i64,
}

const fn one() -> u16 {
    1
}

const fn compatibility_expiry() -> i64 {
    1_209_600
}

pub(crate) async fn create_print_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, AppError> {
    let tenant = authenticate_compatibility(&state, &headers, Scope::JobsWrite).await?;
    let request = decode_request(&headers, &body)?;
    if request.title.trim().is_empty() || !(1..=100).contains(&request.qty) {
        return Err(
            AppError::invalid("InvalidRequest", "The print job request is invalid.")
                .compatibility(),
        );
    }
    let native_printer = state
        .repository
        .resolve_compatibility_id(
            tenant.workspace_id,
            tenant.environment_id,
            "printer",
            request.printer_id,
        )
        .await
        .map_err(|error| AppError::from(error).compatibility())?;
    let printer_id = PrinterId::from_str(&native_printer).map_err(|_| {
        AppError::invalid("InvalidPrinter", "The printer does not exist.").compatibility()
    })?;
    let agent_id = state
        .repository
        .resolve_printer_agent(tenant.workspace_id, tenant.environment_id, printer_id)
        .await
        .map_err(|error| AppError::from(error).compatibility())?;
    let (content_kind, content) = compatibility_content(&request)?;
    let content = persist_job_content(&state, tenant, content_kind, content)
        .await
        .map_err(AppError::compatibility)?;
    let now = Utc::now();
    let job = Job {
        id: spool_domain::JobId::new(),
        workspace_id: tenant.workspace_id,
        environment_id: tenant.environment_id,
        printer_id,
        title: request.title,
        source: request.source,
        content_kind,
        content,
        options: if content_kind == ContentKind::Raw {
            JobOptions::default()
        } else {
            request.options
        },
        deliveries: request.qty,
        state: JobState::Registered,
        created_at: now,
        expires_at: now + Duration::seconds(request.expire_after.clamp(1, 1_209_600)),
    };
    let idempotency = headers
        .get("x-idempotency-key")
        .and_then(|value| value.to_str().ok());
    let created = state
        .repository
        .create_job(&job, agent_id, idempotency, &body)
        .await
        .map_err(|error| AppError::from(error).compatibility())?;
    let created_job = match &created {
        CreateResult::Created(job) | CreateResult::Existing(job) => job,
    };
    if matches!(created, CreateResult::Created(_)) {
        state
            .repository
            .transition_job(
                tenant.workspace_id,
                tenant.environment_id,
                created_job.id,
                JobState::WaitingForAgent,
                None,
                Some("Waiting for PrintNode-compatible client delivery".into()),
                None,
                None,
            )
            .await
            .map_err(|error| AppError::from(error).compatibility())?;
    }
    let compatibility_id = state
        .repository
        .compatibility_id(
            tenant.workspace_id,
            tenant.environment_id,
            "job",
            &created_job.id.to_string(),
        )
        .await
        .map_err(|error| AppError::from(error).compatibility())?;
    Ok((StatusCode::CREATED, Json(compatibility_id)).into_response())
}

#[derive(Debug, Deserialize)]
pub(crate) struct CompatibilityListQuery {
    #[serde(default = "compatibility_limit")]
    limit: i64,
    #[serde(default)]
    dir: Direction,
}

const fn compatibility_limit() -> i64 {
    100
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Direction {
    Asc,
    #[default]
    Desc,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CompatibilityJob {
    id: i64,
    printer: CompatibilityPrinterReference,
    title: String,
    content_type: &'static str,
    source: Option<String>,
    state: &'static str,
    create_timestamp: i64,
}

#[derive(Debug, Serialize)]
struct CompatibilityPrinterReference {
    id: i64,
}

pub(crate) async fn list_print_jobs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<CompatibilityListQuery>,
) -> Result<Json<Vec<CompatibilityJob>>, AppError> {
    let tenant = authenticate_compatibility(&state, &headers, Scope::JobsRead).await?;
    let mut jobs = state
        .repository
        .list_jobs(
            tenant.workspace_id,
            tenant.environment_id,
            None,
            query.limit.clamp(1, 500),
        )
        .await
        .map_err(|error| AppError::from(error).compatibility())?;
    if matches!(query.dir, Direction::Asc) {
        jobs.reverse();
    }
    let mut response = Vec::with_capacity(jobs.len());
    for job in jobs {
        let id = state
            .repository
            .compatibility_id(
                tenant.workspace_id,
                tenant.environment_id,
                "job",
                &job.id.to_string(),
            )
            .await
            .map_err(|error| AppError::from(error).compatibility())?;
        let printer_id = state
            .repository
            .compatibility_id(
                tenant.workspace_id,
                tenant.environment_id,
                "printer",
                &job.printer_id.to_string(),
            )
            .await
            .map_err(|error| AppError::from(error).compatibility())?;
        response.push(CompatibilityJob {
            id,
            printer: CompatibilityPrinterReference { id: printer_id },
            title: job.title,
            content_type: match job.content_kind {
                ContentKind::Pdf => "pdf",
                ContentKind::Raw => "raw",
            },
            source: job.source,
            state: compatibility_state(job.state),
            create_timestamp: job.created_at.timestamp(),
        });
    }
    Ok(Json(response))
}

pub(crate) async fn get_print_jobs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(set): Path<String>,
) -> Result<Json<Vec<CompatibilityJob>>, AppError> {
    let tenant = authenticate_compatibility(&state, &headers, Scope::JobsRead).await?;
    let mut response = Vec::new();
    for value in parse_integer_set(&set)? {
        let native = state
            .repository
            .resolve_compatibility_id(tenant.workspace_id, tenant.environment_id, "job", value)
            .await
            .map_err(|error| AppError::from(error).compatibility())?;
        let job = state
            .repository
            .get_job(
                tenant.workspace_id,
                tenant.environment_id,
                parse_job_id(&native).map_err(AppError::compatibility)?,
            )
            .await
            .map_err(|error| AppError::from(error).compatibility())?;
        let printer_id = state
            .repository
            .compatibility_id(
                tenant.workspace_id,
                tenant.environment_id,
                "printer",
                &job.printer_id.to_string(),
            )
            .await
            .map_err(|error| AppError::from(error).compatibility())?;
        response.push(CompatibilityJob {
            id: value,
            printer: CompatibilityPrinterReference { id: printer_id },
            title: job.title,
            content_type: match job.content_kind {
                ContentKind::Pdf => "pdf",
                ContentKind::Raw => "raw",
            },
            source: job.source,
            state: compatibility_state(job.state),
            create_timestamp: job.created_at.timestamp(),
        });
    }
    Ok(Json(response))
}

#[derive(Debug, Serialize)]
pub(crate) struct CompatibilityState {
    id: i64,
    state: &'static str,
    age: i64,
}

pub(crate) async fn get_print_job_states(
    State(state): State<AppState>,
    headers: HeaderMap,
    set: Option<Path<String>>,
) -> Result<Json<Vec<CompatibilityState>>, AppError> {
    let tenant = authenticate_compatibility(&state, &headers, Scope::JobsRead).await?;
    let jobs = if let Some(Path(set)) = set {
        let mut jobs = Vec::new();
        for value in parse_integer_set(&set)? {
            let native = state
                .repository
                .resolve_compatibility_id(tenant.workspace_id, tenant.environment_id, "job", value)
                .await
                .map_err(|error| AppError::from(error).compatibility())?;
            jobs.push((
                value,
                state
                    .repository
                    .get_job(
                        tenant.workspace_id,
                        tenant.environment_id,
                        parse_job_id(&native).map_err(AppError::compatibility)?,
                    )
                    .await
                    .map_err(|error| AppError::from(error).compatibility())?,
            ));
        }
        jobs
    } else {
        let mut jobs = Vec::new();
        for job in state
            .repository
            .list_jobs(tenant.workspace_id, tenant.environment_id, None, 500)
            .await
            .map_err(|error| AppError::from(error).compatibility())?
        {
            let id = state
                .repository
                .compatibility_id(
                    tenant.workspace_id,
                    tenant.environment_id,
                    "job",
                    &job.id.to_string(),
                )
                .await
                .map_err(|error| AppError::from(error).compatibility())?;
            jobs.push((id, job));
        }
        jobs
    };
    Ok(Json(
        jobs.into_iter()
            .map(|(id, job)| CompatibilityState {
                id,
                state: compatibility_state(job.state),
                age: (Utc::now() - job.created_at).num_seconds().max(0),
            })
            .collect(),
    ))
}

#[derive(Debug, Serialize)]
pub(crate) struct CompatibilityComputer {
    id: i64,
    name: String,
    state: &'static str,
    version: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct CompatibilityPrinter {
    id: i64,
    name: String,
    computer: CompatibilityComputerReference,
    #[serde(rename = "default")]
    is_default: bool,
    state: &'static str,
    capabilities: spool_domain::PrinterCapabilities,
}

#[derive(Debug, Serialize)]
struct CompatibilityComputerReference {
    id: i64,
}

pub(crate) async fn list_computers(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<CompatibilityComputer>>, AppError> {
    let tenant = authenticate_compatibility(&state, &headers, Scope::AgentsRead).await?;
    let mut response = Vec::new();
    for agent in state
        .repository
        .list_agents(tenant.workspace_id, tenant.environment_id)
        .await
        .map_err(|error| AppError::from(error).compatibility())?
    {
        let id = state
            .repository
            .compatibility_id(
                tenant.workspace_id,
                tenant.environment_id,
                "computer",
                &agent.id.to_string(),
            )
            .await
            .map_err(|error| AppError::from(error).compatibility())?;
        response.push(CompatibilityComputer {
            id,
            name: agent.name,
            state: if agent.state == "connected" {
                "connected"
            } else {
                "disconnected"
            },
            version: agent.version,
        });
    }
    Ok(Json(response))
}

pub(crate) async fn list_printers(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<CompatibilityPrinter>>, AppError> {
    let tenant = authenticate_compatibility(&state, &headers, Scope::PrintersRead).await?;
    let mut response = Vec::new();
    for printer in state
        .repository
        .list_printers(tenant.workspace_id, tenant.environment_id, None, 500)
        .await
        .map_err(|error| AppError::from(error).compatibility())?
    {
        let id = state
            .repository
            .compatibility_id(
                tenant.workspace_id,
                tenant.environment_id,
                "printer",
                &printer.id.to_string(),
            )
            .await
            .map_err(|error| AppError::from(error).compatibility())?;
        let computer_id = state
            .repository
            .compatibility_id(
                tenant.workspace_id,
                tenant.environment_id,
                "computer",
                &printer.agent_id.to_string(),
            )
            .await
            .map_err(|error| AppError::from(error).compatibility())?;
        response.push(CompatibilityPrinter {
            id,
            name: printer.name,
            computer: CompatibilityComputerReference { id: computer_id },
            is_default: false,
            state: match printer.state {
                spool_domain::PrinterState::Online | spool_domain::PrinterState::Busy => "online",
                _ => "offline",
            },
            capabilities: printer.capabilities,
        });
    }
    Ok(Json(response))
}

fn decode_request(headers: &HeaderMap, body: &[u8]) -> Result<PrintJobRequest, AppError> {
    let content_type = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/json");
    if content_type.starts_with("application/x-www-form-urlencoded") {
        let values: HashMap<String, String> = serde_urlencoded::from_bytes(body).map_err(|_| {
            AppError::invalid("InvalidRequest", "The form body is invalid.").compatibility()
        })?;
        let printer_id = values
            .get("printerId")
            .and_then(|value| value.parse().ok())
            .ok_or_else(|| {
                AppError::invalid("InvalidRequest", "printerId is required.").compatibility()
            })?;
        let options = values
            .get("options")
            .map(|value| serde_json::from_str(value))
            .transpose()
            .map_err(|_| {
                AppError::invalid("InvalidRequest", "options is invalid.").compatibility()
            })?
            .unwrap_or_default();
        return Ok(PrintJobRequest {
            printer_id,
            title: values.get("title").cloned().unwrap_or_default(),
            content_type: values.get("contentType").cloned().unwrap_or_default(),
            content: values.get("content").cloned().unwrap_or_default(),
            source: values.get("source").cloned(),
            options,
            qty: values
                .get("qty")
                .and_then(|value| value.parse().ok())
                .unwrap_or(1),
            expire_after: values
                .get("expireAfter")
                .and_then(|value| value.parse().ok())
                .unwrap_or_else(compatibility_expiry),
        });
    }
    serde_json::from_slice(body).map_err(|_| {
        AppError::invalid("InvalidRequest", "The JSON body is invalid.").compatibility()
    })
}

fn compatibility_content(
    request: &PrintJobRequest,
) -> Result<(ContentKind, ContentSource), AppError> {
    match request.content_type.as_str() {
        "pdf_base64" => Ok((
            ContentKind::Pdf,
            ContentSource::Base64 {
                data: request.content.clone(),
            },
        )),
        "raw_base64" => Ok((
            ContentKind::Raw,
            ContentSource::Base64 {
                data: request.content.clone(),
            },
        )),
        "pdf_uri" => Ok((
            ContentKind::Pdf,
            ContentSource::Uri {
                uri: request.content.clone(),
                authentication: None,
            },
        )),
        "raw_uri" => Ok((
            ContentKind::Raw,
            ContentSource::Uri {
                uri: request.content.clone(),
                authentication: None,
            },
        )),
        _ => Err(
            AppError::invalid("InvalidContentType", "contentType is not supported.")
                .compatibility(),
        ),
    }
}

fn parse_integer_set(value: &str) -> Result<Vec<i64>, AppError> {
    let mut result = value
        .split(',')
        .map(str::parse::<i64>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| {
            AppError::invalid("InvalidSet", "The resource ID set is invalid.").compatibility()
        })?;
    result.sort_unstable();
    result.dedup();
    Ok(result)
}

const fn compatibility_state(state: JobState) -> &'static str {
    match state {
        JobState::Registered | JobState::ContentPending | JobState::WaitingForAgent => "new",
        JobState::AgentDownloading
        | JobState::AgentAccepted
        | JobState::QueuedLocal
        | JobState::Preparing
        | JobState::Rendering
        | JobState::SpoolIntent => "sent_to_client",
        JobState::AcceptedBySpooler
        | JobState::Spooling
        | JobState::Printing
        | JobState::Blocked
        | JobState::CompletedReported => "done",
        JobState::DeliveryUncertain | JobState::FailedRetryable | JobState::FailedTerminal => {
            "error"
        }
        JobState::CancelRequested | JobState::Cancelled | JobState::Expired => "expired",
    }
}
