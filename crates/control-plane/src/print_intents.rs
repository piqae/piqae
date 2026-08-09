//! Pure capability-aware validation and display-safe ticket resolution.
//! No handler in this module submits, spools, or mutates printer defaults.

use crate::{AppState, api::authenticate_native, error::AppError};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use chrono::{Duration, Utc};
use piqae_auth::Scope;
use piqae_domain::{JobOptions, PrinterId};
use piqae_storage_postgres::StoredResolvedPrintTicket;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, str::FromStr};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PrintIntent {
    schema_version: u8,
    printer_id: String,
    capability_revision: u64,
    workflow: Option<ResourceRevision>,
    stock: Option<ResourceRevision>,
    #[serde(default)]
    portable_options: JobOptions,
    #[serde(default)]
    semantic_options: BTreeMap<String, Value>,
    document_manifest: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ResourceRevision {
    id: String,
    revision: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationRequest {
    intent: PrintIntent,
}

pub async fn capability_document(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::PrintersRead).await?;
    let printer_id = PrinterId::from_str(&id)
        .map_err(|_| AppError::invalid("invalid_printer_id", "Printer ID is invalid."))?;
    let printer = state
        .repository
        .get_printer(tenant.workspace_id, tenant.environment_id, printer_id)
        .await?;
    if printer.capability_revision == 0 {
        return Err(AppError::conflict(
            "capabilities_unavailable",
            "The node has not published a capability revision.",
        ));
    }
    let fingerprint_input = serde_json::to_vec(&(
        printer.capability_revision,
        &printer.capabilities,
        &printer.native_options,
    ))?;
    let fingerprint = hex::encode(Sha256::digest(fingerprint_input));
    let evidence = json!({"level":"discovered","source":"installed_driver","support_pack_id":null,"support_pack_digest":null});
    let mut facets = serde_json::Map::new();
    let mut facet = |name: &str, kind: &str, supported: bool, values: Value| {
        facets.insert(name.into(), json!({"type":kind,"mutability":if supported {"job_override"} else {"unsupported"},"supported":supported,"unit":null,"values":values,"minimum":null,"maximum":null,"dependencies":[],"conflicts":[],"evidence":evidence}));
    };
    facet(
        "print.color",
        "boolean",
        printer.capabilities.color,
        json!([false, true]),
    );
    facet(
        "print.duplex",
        "enum",
        printer.capabilities.duplex,
        json!(["one-sided", "long-edge", "short-edge"]),
    );
    facet(
        "print.resolution",
        "enum",
        !printer.capabilities.dpis.is_empty(),
        json!(printer.capabilities.dpis),
    );
    facet(
        "media.source",
        "enum",
        !printer.capabilities.bins.is_empty(),
        json!(printer.capabilities.bins),
    );
    facet(
        "media.name",
        "enum",
        !printer.capabilities.medias.is_empty(),
        json!(printer.capabilities.medias),
    );
    Ok(Json(
        json!({"schema_version":1,"printer_id":id,"revision":printer.capability_revision,"driver_fingerprint_sha256":fingerprint,"facets":facets,"created_at":printer.updated_at}),
    ))
}

pub async fn validate(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<ValidationRequest>,
) -> Result<Json<Value>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::JobsWrite).await?;
    let result = validate_for_tenant(
        &state,
        tenant.workspace_id,
        tenant.environment_id,
        request.intent,
    )
    .await?;
    Ok(Json(result))
}

pub async fn resolve(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<ValidationRequest>,
) -> Result<Response, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::JobsWrite).await?;
    let validation = validate_for_tenant(
        &state,
        tenant.workspace_id,
        tenant.environment_id,
        request.intent,
    )
    .await?;
    if validation["status"] != "valid" {
        return Err(AppError::conflict(
            "print_intent_invalid",
            "The print intent must validate before it can be resolved.",
        ));
    }
    let intent = validation
        .get("normalized_intent")
        .cloned()
        .ok_or_else(|| {
            AppError::conflict("print_intent_invalid", "Normalized intent is unavailable.")
        })?;
    let expires_at = Utc::now() + Duration::minutes(15);
    let mut display = json!({
        "printer_id": intent["printer_id"], "capability_revision": intent["capability_revision"],
        "workflow": intent["workflow"], "stock": intent["stock"],
        "resolved_options": intent["portable_options"],
        "provenance": {"portable_options":"installed_driver_capability_revision"},
        "expires_at": expires_at,
    });
    let canonical = serde_json::to_vec(&display)?;
    let digest = hex::encode(Sha256::digest(canonical));
    display
        .as_object_mut()
        .ok_or_else(|| AppError::invalid("invalid_print_intent", "Resolved ticket is invalid."))?
        .insert("digest".into(), json!(digest));
    let printer_id = PrinterId::from_str(intent["printer_id"].as_str().unwrap_or_default())
        .map_err(|_| AppError::invalid("invalid_printer_id", "Printer ID is invalid."))?;
    state
        .repository
        .store_resolved_print_ticket(
            tenant.workspace_id,
            tenant.environment_id,
            &StoredResolvedPrintTicket {
                digest,
                printer_id,
                capability_revision: intent["capability_revision"].as_u64().unwrap_or_default(),
                display_ticket: display.clone(),
                expires_at,
                created_at: Utc::now(),
            },
        )
        .await?;
    Ok((StatusCode::CREATED, Json(display)).into_response())
}

pub async fn loaded_media(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Vec<Value>>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::PrintersRead).await?;
    let printer_id = PrinterId::from_str(&id)
        .map_err(|_| AppError::invalid("invalid_printer_id", "Printer ID is invalid."))?;
    state
        .repository
        .get_printer(tenant.workspace_id, tenant.environment_id, printer_id)
        .await?;
    let observations = state
        .repository
        .list_loaded_media(tenant.workspace_id, tenant.environment_id, printer_id)
        .await?;
    let values = observations.into_iter().map(|observation| json!({
        "printer_id": observation.printer_id, "source": observation.source,
        "stock": observation.stock_id.zip(observation.stock_revision).map(|(id, revision)| json!({"id":id,"revision":revision})),
        "confidence": observation.confidence, "calibration_state": observation.calibration_state,
        "remaining_amount": observation.remaining_amount, "observed_at": observation.observed_at,
        "updated_at": observation.updated_at,
    })).collect();
    Ok(Json(values))
}

async fn validate_for_tenant(
    state: &AppState,
    workspace_id: piqae_domain::WorkspaceId,
    environment_id: piqae_domain::EnvironmentId,
    intent: PrintIntent,
) -> Result<Value, AppError> {
    if intent.schema_version != 1 || !intent.document_manifest.is_object() {
        return Err(AppError::invalid(
            "invalid_print_intent",
            "Only schema version 1 with an object document manifest is accepted.",
        ));
    }
    let printer_id = PrinterId::from_str(&intent.printer_id)
        .map_err(|_| AppError::invalid("invalid_printer_id", "Printer ID is invalid."))?;
    let printer = state
        .repository
        .get_printer(workspace_id, environment_id, printer_id)
        .await?;
    let mut errors = Vec::new();
    if intent.capability_revision == 0 || intent.capability_revision != printer.capability_revision
    {
        errors.push(finding(
            "stale_capability_revision",
            "capability_revision",
            "The printer capability revision changed.",
        ));
    }
    if !intent.semantic_options.is_empty() {
        errors.push(finding(
            "unsupported_semantic_option",
            "semantic_options",
            "No trusted support-pack mapping exists for these semantic options.",
        ));
    }
    let options = &intent.portable_options;
    if options.color == Some(true) && !printer.capabilities.color {
        errors.push(finding(
            "unsupported_color",
            "portable_options.color",
            "The printer does not report colour support.",
        ));
    }
    if options.duplex.is_some() && !printer.capabilities.duplex {
        errors.push(finding(
            "unsupported_duplex",
            "portable_options.duplex",
            "The printer does not report duplex support.",
        ));
    }
    if options
        .copies
        .is_some_and(|copies| copies == 0 || copies > printer.capabilities.copies)
    {
        errors.push(finding(
            "unsupported_copies",
            "portable_options.copies",
            "Copies exceed the driver-reported limit.",
        ));
    }
    if options
        .dpi
        .as_ref()
        .is_some_and(|value| !printer.capabilities.dpis.contains(value))
    {
        errors.push(finding(
            "unsupported_resolution",
            "portable_options.dpi",
            "Resolution is not in the driver capability snapshot.",
        ));
    }
    if options
        .bin
        .as_ref()
        .is_some_and(|value| !printer.capabilities.bins.contains(value))
    {
        errors.push(finding(
            "unsupported_media_source",
            "portable_options.bin",
            "Media source is not in the driver capability snapshot.",
        ));
    }
    for (key, value) in &options.native_options {
        if !printer
            .native_options
            .get(key)
            .is_some_and(|option| option.choices.iter().any(|choice| choice.value == *value))
        {
            errors.push(finding(
                "unsupported_native_option",
                &format!("portable_options.native_options.{key}"),
                "Native option is absent or not an allowed driver choice.",
            ));
        }
    }
    if let Some(stock) = &intent.stock {
        match state
            .repository
            .get_stock(workspace_id, environment_id, &stock.id)
            .await
        {
            Ok(current) if current.revision == stock.revision && !current.archived => {}
            _ => errors.push(finding(
                "stock_revision_unavailable",
                "stock",
                "The exact active stock revision is unavailable.",
            )),
        }
    }
    if let Some(workflow) = &intent.workflow {
        let found = state
            .repository
            .list_print_workflows(workspace_id, environment_id)
            .await?
            .into_iter()
            .any(|current| {
                current.id == workflow.id
                    && current.revision == workflow.revision
                    && !current.archived
                    && current.printer_id == printer_id
            });
        if !found {
            errors.push(finding(
                "workflow_revision_unavailable",
                "workflow",
                "The exact workflow revision is unavailable for this printer.",
            ));
        }
    }
    let status = if errors.is_empty() {
        "valid"
    } else {
        "invalid"
    };
    Ok(
        json!({"status":status,"capability_revision":printer.capability_revision,"errors":errors,"warnings":[],"normalized_intent":if errors.is_empty() { serde_json::to_value(intent)? } else { Value::Null }}),
    )
}

fn finding(code: &str, path: &str, message: &str) -> Value {
    json!({"code":code,"path":path,"message":message})
}
