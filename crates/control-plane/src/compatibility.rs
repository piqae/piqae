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
use spool_domain::{AgentId, ContentKind, ContentSource, Job, JobOptions, JobState, PrinterId};
use spool_storage_postgres::{StoredAgent, StoredPrinter};
use std::{
    collections::{HashMap, HashSet},
    str::FromStr,
};

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
        metadata: std::collections::BTreeMap::new(),
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
    #[serde(default)]
    after: Option<i64>,
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

fn paginate_compatibility<T>(
    items: &mut Vec<T>,
    query: &CompatibilityListQuery,
    id: impl Fn(&T) -> i64,
) -> Result<(), AppError> {
    if query.after.is_some_and(|after| after <= 0) {
        return Err(
            AppError::invalid("InvalidAfter", "The pagination cursor is invalid.").compatibility(),
        );
    }
    items.sort_unstable_by_key(&id);
    if matches!(query.dir, Direction::Desc) {
        items.reverse();
    }
    if let Some(after) = query.after {
        items.retain(|item| match query.dir {
            Direction::Asc => id(item) > after,
            Direction::Desc => id(item) < after,
        });
    }
    items.truncate(usize::try_from(query.limit.clamp(1, 500)).unwrap_or(500));
    Ok(())
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

async fn compatibility_job(
    state: &AppState,
    tenant: crate::authentication::TenantContext,
    job: Job,
    id: i64,
) -> Result<CompatibilityJob, AppError> {
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
    Ok(CompatibilityJob {
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
    })
}

pub(crate) async fn list_print_jobs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<CompatibilityListQuery>,
) -> Result<Json<Vec<CompatibilityJob>>, AppError> {
    let tenant = authenticate_compatibility(&state, &headers, Scope::JobsRead).await?;
    let jobs = state
        .repository
        .list_jobs(tenant.workspace_id, tenant.environment_id, None, 500)
        .await
        .map_err(|error| AppError::from(error).compatibility())?;
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
        response.push(compatibility_job(&state, tenant, job, id).await?);
    }
    paginate_compatibility(&mut response, &query, |job| job.id)?;
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
        response.push(compatibility_job(&state, tenant, job, value).await?);
    }
    Ok(Json(response))
}

pub(crate) async fn get_printer_print_jobs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(printer_set): Path<String>,
    Query(query): Query<CompatibilityListQuery>,
) -> Result<Json<Vec<CompatibilityJob>>, AppError> {
    filtered_print_jobs(&state, &headers, &printer_set, None, query).await
}

pub(crate) async fn get_printer_print_job_set(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((printer_set, job_set)): Path<(String, String)>,
    Query(query): Query<CompatibilityListQuery>,
) -> Result<Json<Vec<CompatibilityJob>>, AppError> {
    filtered_print_jobs(&state, &headers, &printer_set, Some(&job_set), query).await
}

async fn filtered_print_jobs(
    state: &AppState,
    headers: &HeaderMap,
    printer_set: &str,
    job_set: Option<&str>,
    query: CompatibilityListQuery,
) -> Result<Json<Vec<CompatibilityJob>>, AppError> {
    let tenant = authenticate_compatibility(state, headers, Scope::JobsRead).await?;
    let printers = resolve_printer_set(state, tenant, printer_set).await?;
    let mut jobs = if let Some(set) = job_set {
        let mut jobs = Vec::new();
        for id in parse_integer_set(set)? {
            let native = state
                .repository
                .resolve_compatibility_id(tenant.workspace_id, tenant.environment_id, "job", id)
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
            jobs.push((id, job));
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
    jobs.retain(|(_, job)| printers.contains(&job.printer_id));
    let mut response = Vec::with_capacity(jobs.len());
    for (id, job) in jobs {
        response.push(compatibility_job(state, tenant, job, id).await?);
    }
    paginate_compatibility(&mut response, &query, |job| job.id)?;
    Ok(Json(response))
}

pub(crate) async fn cancel_print_jobs(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<i64>>, AppError> {
    cancel_matching_jobs(&state, &headers, None, None).await
}

pub(crate) async fn cancel_print_job_set(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(job_set): Path<String>,
) -> Result<Json<Vec<i64>>, AppError> {
    cancel_matching_jobs(&state, &headers, None, Some(&job_set)).await
}

pub(crate) async fn cancel_printer_print_jobs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(printer_set): Path<String>,
) -> Result<Json<Vec<i64>>, AppError> {
    cancel_matching_jobs(&state, &headers, Some(&printer_set), None).await
}

pub(crate) async fn cancel_printer_print_job_set(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((printer_set, job_set)): Path<(String, String)>,
) -> Result<Json<Vec<i64>>, AppError> {
    cancel_matching_jobs(&state, &headers, Some(&printer_set), Some(&job_set)).await
}

async fn cancel_matching_jobs(
    state: &AppState,
    headers: &HeaderMap,
    printer_set: Option<&str>,
    job_set: Option<&str>,
) -> Result<Json<Vec<i64>>, AppError> {
    let tenant = authenticate_compatibility(state, headers, Scope::JobsWrite).await?;
    let printers = match printer_set {
        Some(set) => Some(resolve_printer_set(state, tenant, set).await?),
        None => None,
    };
    let candidates = if let Some(set) = job_set {
        let mut jobs = Vec::new();
        for id in parse_integer_set(set)? {
            let native = state
                .repository
                .resolve_compatibility_id(tenant.workspace_id, tenant.environment_id, "job", id)
                .await
                .map_err(|error| AppError::from(error).compatibility())?;
            jobs.push((
                id,
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
    let mut cancelled = Vec::new();
    for (id, job) in candidates {
        if printers
            .as_ref()
            .is_some_and(|printers| !printers.contains(&job.printer_id))
            || !matches!(
                job.state,
                JobState::Registered | JobState::ContentPending | JobState::WaitingForAgent
            )
        {
            continue;
        }
        match state
            .repository
            .request_job_cancellation(tenant.workspace_id, tenant.environment_id, job.id)
            .await
        {
            Ok(job) => {
                state
                    .publish(tenant, "job.updated", &job)
                    .await
                    .map_err(|error| AppError::from(error).compatibility())?;
                cancelled.push(id);
            }
            Err(
                crate::repository::RepositoryError::ConcurrentStateChange
                | crate::repository::RepositoryError::InvalidTransition,
            ) => {}
            Err(error) => return Err(AppError::from(error).compatibility()),
        }
    }
    Ok(Json(cancelled))
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
    Query(query): Query<CompatibilityListQuery>,
) -> Result<Json<Vec<CompatibilityComputer>>, AppError> {
    let tenant = authenticate_compatibility(&state, &headers, Scope::AgentsRead).await?;
    let agents = state
        .repository
        .list_agents(tenant.workspace_id, tenant.environment_id)
        .await
        .map_err(|error| AppError::from(error).compatibility())?;
    let mut response = Vec::new();
    for agent in agents {
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
        response.push(compatibility_computer(agent, id));
    }
    paginate_compatibility(&mut response, &query, |computer| computer.id)?;
    Ok(Json(response))
}

pub(crate) async fn get_computers(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(set): Path<String>,
) -> Result<Json<Vec<CompatibilityComputer>>, AppError> {
    let tenant = authenticate_compatibility(&state, &headers, Scope::AgentsRead).await?;
    let agents = state
        .repository
        .list_agents(tenant.workspace_id, tenant.environment_id)
        .await
        .map_err(|error| AppError::from(error).compatibility())?
        .into_iter()
        .map(|agent| (agent.id, agent))
        .collect::<HashMap<_, _>>();
    let mut response = Vec::new();
    for id in parse_integer_set(&set)? {
        let native = state
            .repository
            .resolve_compatibility_id(tenant.workspace_id, tenant.environment_id, "computer", id)
            .await
            .map_err(|error| AppError::from(error).compatibility())?;
        let agent_id = AgentId::from_str(&native).map_err(|_| {
            AppError::invalid("InvalidComputer", "The computer does not exist.").compatibility()
        })?;
        let agent = agents.get(&agent_id).cloned().ok_or_else(|| {
            AppError::invalid("InvalidComputer", "The computer does not exist.").compatibility()
        })?;
        response.push(compatibility_computer(agent, id));
    }
    Ok(Json(response))
}

fn compatibility_computer(agent: StoredAgent, id: i64) -> CompatibilityComputer {
    CompatibilityComputer {
        id,
        name: agent.name,
        state: if agent.state == "connected" {
            "connected"
        } else {
            "disconnected"
        },
        version: agent.version,
    }
}

pub(crate) async fn list_printers(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<CompatibilityListQuery>,
) -> Result<Json<Vec<CompatibilityPrinter>>, AppError> {
    let tenant = authenticate_compatibility(&state, &headers, Scope::PrintersRead).await?;
    let printers = state
        .repository
        .list_printers(tenant.workspace_id, tenant.environment_id, None, 500)
        .await
        .map_err(|error| AppError::from(error).compatibility())?;
    let mut response = Vec::new();
    for printer in printers {
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
        response.push(compatibility_printer(&state, tenant, printer, id).await?);
    }
    paginate_compatibility(&mut response, &query, |printer| printer.id)?;
    Ok(Json(response))
}

pub(crate) async fn get_printers(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(set): Path<String>,
) -> Result<Json<Vec<CompatibilityPrinter>>, AppError> {
    let tenant = authenticate_compatibility(&state, &headers, Scope::PrintersRead).await?;
    let available = state
        .repository
        .list_printers(tenant.workspace_id, tenant.environment_id, None, 500)
        .await
        .map_err(|error| AppError::from(error).compatibility())?
        .into_iter()
        .map(|printer| (printer.id, printer))
        .collect::<HashMap<_, _>>();
    let mut response = Vec::new();
    for id in parse_integer_set(&set)? {
        let native = state
            .repository
            .resolve_compatibility_id(tenant.workspace_id, tenant.environment_id, "printer", id)
            .await
            .map_err(|error| AppError::from(error).compatibility())?;
        let printer_id = PrinterId::from_str(&native).map_err(|_| {
            AppError::invalid("InvalidPrinter", "The printer does not exist.").compatibility()
        })?;
        let printer = available.get(&printer_id).cloned().ok_or_else(|| {
            AppError::invalid("InvalidPrinter", "The printer does not exist.").compatibility()
        })?;
        response.push(compatibility_printer(&state, tenant, printer, id).await?);
    }
    Ok(Json(response))
}

pub(crate) async fn get_computer_printers(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(computer_set): Path<String>,
    Query(query): Query<CompatibilityListQuery>,
) -> Result<Json<Vec<CompatibilityPrinter>>, AppError> {
    filtered_computer_printers(&state, &headers, &computer_set, None, query).await
}

pub(crate) async fn get_computer_printer_set(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((computer_set, printer_set)): Path<(String, String)>,
    Query(query): Query<CompatibilityListQuery>,
) -> Result<Json<Vec<CompatibilityPrinter>>, AppError> {
    filtered_computer_printers(&state, &headers, &computer_set, Some(&printer_set), query).await
}

async fn filtered_computer_printers(
    state: &AppState,
    headers: &HeaderMap,
    computer_set: &str,
    printer_set: Option<&str>,
    query: CompatibilityListQuery,
) -> Result<Json<Vec<CompatibilityPrinter>>, AppError> {
    let tenant = authenticate_compatibility(state, headers, Scope::PrintersRead).await?;
    let computers = resolve_computer_set(state, tenant, computer_set).await?;
    let selected_printers = match printer_set {
        Some(set) => Some(resolve_printer_set(state, tenant, set).await?),
        None => None,
    };
    let mut printers = state
        .repository
        .list_printers(tenant.workspace_id, tenant.environment_id, None, 500)
        .await
        .map_err(|error| AppError::from(error).compatibility())?;
    printers.retain(|printer| {
        computers.contains(&printer.agent_id)
            && selected_printers
                .as_ref()
                .is_none_or(|selected| selected.contains(&printer.id))
    });
    let mut response = Vec::with_capacity(printers.len());
    for printer in printers {
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
        response.push(compatibility_printer(state, tenant, printer, id).await?);
    }
    paginate_compatibility(&mut response, &query, |printer| printer.id)?;
    Ok(Json(response))
}

async fn compatibility_printer(
    state: &AppState,
    tenant: crate::authentication::TenantContext,
    printer: StoredPrinter,
    id: i64,
) -> Result<CompatibilityPrinter, AppError> {
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
    Ok(CompatibilityPrinter {
        id,
        name: printer.name,
        computer: CompatibilityComputerReference { id: computer_id },
        is_default: false,
        state: match printer.state {
            spool_domain::PrinterState::Online | spool_domain::PrinterState::Busy => "online",
            _ => "offline",
        },
        capabilities: printer.capabilities,
    })
}

async fn resolve_computer_set(
    state: &AppState,
    tenant: crate::authentication::TenantContext,
    set: &str,
) -> Result<HashSet<AgentId>, AppError> {
    let available = state
        .repository
        .list_agents(tenant.workspace_id, tenant.environment_id)
        .await
        .map_err(|error| AppError::from(error).compatibility())?
        .into_iter()
        .map(|agent| agent.id)
        .collect::<HashSet<_>>();
    let mut result = HashSet::new();
    for id in parse_integer_set(set)? {
        let native = state
            .repository
            .resolve_compatibility_id(tenant.workspace_id, tenant.environment_id, "computer", id)
            .await
            .map_err(|error| AppError::from(error).compatibility())?;
        let agent_id = AgentId::from_str(&native).map_err(|_| {
            AppError::invalid("InvalidComputer", "The computer does not exist.").compatibility()
        })?;
        if !available.contains(&agent_id) {
            return Err(
                AppError::invalid("InvalidComputer", "The computer does not exist.")
                    .compatibility(),
            );
        }
        result.insert(agent_id);
    }
    Ok(result)
}

async fn resolve_printer_set(
    state: &AppState,
    tenant: crate::authentication::TenantContext,
    set: &str,
) -> Result<HashSet<PrinterId>, AppError> {
    let available = state
        .repository
        .list_printers(tenant.workspace_id, tenant.environment_id, None, 500)
        .await
        .map_err(|error| AppError::from(error).compatibility())?
        .into_iter()
        .map(|printer| printer.id)
        .collect::<HashSet<_>>();
    let mut result = HashSet::new();
    for id in parse_integer_set(set)? {
        let native = state
            .repository
            .resolve_compatibility_id(tenant.workspace_id, tenant.environment_id, "printer", id)
            .await
            .map_err(|error| AppError::from(error).compatibility())?;
        let printer_id = PrinterId::from_str(&native).map_err(|_| {
            AppError::invalid("InvalidPrinter", "The printer does not exist.").compatibility()
        })?;
        if !available.contains(&printer_id) {
            return Err(
                AppError::invalid("InvalidPrinter", "The printer does not exist.").compatibility(),
            );
        }
        result.insert(printer_id);
    }
    Ok(result)
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
    if result.is_empty() || result.iter().any(|id| *id <= 0) {
        return Err(
            AppError::invalid("InvalidSet", "The resource ID set is invalid.").compatibility(),
        );
    }
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
