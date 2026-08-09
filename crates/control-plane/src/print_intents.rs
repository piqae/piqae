//! Pure capability-aware validation and display-safe ticket resolution.
//! No handler in this module submits, spools, or mutates printer defaults.

#![allow(clippy::missing_errors_doc)]

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
use piqae_storage_postgres::{StoredLoadedMedia, StoredResolvedPrintTicket};
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpsertLoadedMediaRequest {
    source: String,
    stock: Option<ResourceRevision>,
    calibration_state: String,
    remaining_amount: Option<Value>,
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
        &printer.semantic_capabilities,
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
    append_semantic_facets(&printer, &mut facets);
    Ok(Json(
        json!({"schema_version":1,"printer_id":id,"revision":printer.capability_revision,"driver_fingerprint_sha256":fingerprint,"facets":facets,"created_at":printer.updated_at}),
    ))
}

fn append_semantic_facets(
    printer: &piqae_storage_postgres::StoredPrinter,
    facets: &mut serde_json::Map<String, Value>,
) {
    let pack = &printer.semantic_capabilities.support_pack;
    for (name, values) in &printer.semantic_capabilities.facets {
        let writable_values = printer.semantic_capabilities.native_resolutions.get(name);
        let writable_values = values
            .iter()
            .filter(|value| {
                writable_values
                    .and_then(|choices| choices.get(*value))
                    .is_some_and(|resolution| {
                        printer
                            .native_options
                            .get(&resolution.native_option)
                            .is_some_and(|option| {
                                option
                                    .choices
                                    .iter()
                                    .any(|choice| choice.value == resolution.native_choice)
                            })
                    })
            })
            .collect::<Vec<_>>();
        facets.insert(
            name.clone(),
            json!({
                "type":"enum",
                "mutability":if writable_values.is_empty() {"read_only"} else {"job_override"},
                "supported":true,
                "unit":null,
                "values":values,
                "writable_values":writable_values,
                "minimum":null,
                "maximum":null,
                "dependencies":[],
                "conflicts":[],
                "evidence":{
                    "level":pack.as_ref().map_or("discovered", |value| value.evidence.as_str()),
                    "source":"trusted_support_pack",
                    "support_pack_id":pack.as_ref().map(|value| &value.pack_id),
                    "support_pack_digest":pack.as_ref().map(|value| &value.digest_sha256),
                }
            }),
        );
    }
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
        "semantic_options": intent["semantic_options"],
        "provenance": {
            "portable_options":"installed_driver_capability_revision",
            "semantic_options":"trusted_support_pack_exact_native_resolution"
        },
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

pub async fn upsert_loaded_media(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(request): Json<UpsertLoadedMediaRequest>,
) -> Result<Json<Value>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::PrintersWrite).await?;
    let printer_id = PrinterId::from_str(&id)
        .map_err(|_| AppError::invalid("invalid_printer_id", "Printer ID is invalid."))?;
    state
        .repository
        .get_printer(tenant.workspace_id, tenant.environment_id, printer_id)
        .await?;
    let source = request.source.trim();
    if source.is_empty() || source.chars().count() > 255 || source.chars().any(char::is_control) {
        return Err(AppError::invalid(
            "invalid_loaded_media",
            "Source must be a bounded printable identifier.",
        ));
    }
    if !matches!(
        request.calibration_state.as_str(),
        "current" | "required" | "unknown"
    ) {
        return Err(AppError::invalid(
            "invalid_loaded_media",
            "Calibration state is invalid.",
        ));
    }
    if request.remaining_amount.as_ref().is_some_and(|value| {
        value.as_object().is_none_or(|object| object.len() > 16)
            || serde_json::to_vec(value).map_or(true, |bytes| bytes.len() > 4096)
    }) {
        return Err(AppError::invalid(
            "invalid_loaded_media",
            "Remaining amount must be an object no larger than 4096 bytes.",
        ));
    }
    let (stock_id, stock_revision, confidence, calibration_state, remaining_amount) =
        if let Some(stock) = request.stock {
            let current = state
                .repository
                .get_stock(tenant.workspace_id, tenant.environment_id, &stock.id)
                .await?;
            if current.archived || current.revision != stock.revision {
                return Err(AppError::conflict(
                    "stock_revision_unavailable",
                    "The exact active stock revision is unavailable.",
                ));
            }
            (
                Some(stock.id),
                Some(stock.revision),
                "operator_confirmed".to_owned(),
                request.calibration_state,
                request.remaining_amount,
            )
        } else {
            (None, None, "unknown".to_owned(), "unknown".to_owned(), None)
        };
    let now = Utc::now();
    let observation = state
        .repository
        .upsert_loaded_media(
            tenant.workspace_id,
            tenant.environment_id,
            &StoredLoadedMedia {
                printer_id,
                source: source.to_owned(),
                stock_id,
                stock_revision,
                confidence,
                calibration_state,
                remaining_amount,
                observed_at: now,
                updated_at: now,
            },
        )
        .await?;
    state
        .publish(tenant, "printer.loaded_media.updated", &observation)
        .await?;
    Ok(Json(json!({
        "printer_id": observation.printer_id, "source": observation.source,
        "stock": observation.stock_id.zip(observation.stock_revision).map(|(id, revision)| json!({"id":id,"revision":revision})),
        "confidence": observation.confidence, "calibration_state": observation.calibration_state,
        "remaining_amount": observation.remaining_amount, "observed_at": observation.observed_at,
        "updated_at": observation.updated_at,
    })))
}

#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
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
    let mut intent = intent;
    if intent.capability_revision == 0 || intent.capability_revision != printer.capability_revision
    {
        errors.push(finding(
            "stale_capability_revision",
            "capability_revision",
            "The printer capability revision changed.",
        ));
    }
    if let Some(workflow_ref) = intent.workflow.clone() {
        let workflow = state
            .repository
            .list_print_workflows(workspace_id, environment_id)
            .await?
            .into_iter()
            .find(|current| {
                current.id == workflow_ref.id && current.revision == workflow_ref.revision
            });
        match workflow {
            Some(workflow)
                if workflow.published
                    && !workflow.archived
                    && workflow.printer_id == printer_id =>
            {
                if workflow.capability_revision != printer.capability_revision {
                    errors.push(finding(
                        "stale_workflow_capability",
                        "workflow",
                        "The workflow was approved against a different capability revision.",
                    ));
                }
                match (&workflow.profile_id, workflow.profile_revision) {
                    (Some(profile_id), Some(profile_revision))
                        if !printer.profiles.iter().any(|profile| {
                            profile.profile_id == *profile_id
                                && profile.revision == profile_revision
                                && profile.published
                        }) =>
                    {
                        errors.push(finding(
                            "workflow_profile_unavailable",
                            "workflow",
                            "The workflow's exact published profile revision is unavailable.",
                        ));
                    }
                    (Some(_), None) | (None, Some(_)) => errors.push(finding(
                        "workflow_profile_invalid",
                        "workflow",
                        "The workflow has an incomplete profile revision binding.",
                    )),
                    _ => {}
                }
                if workflow.stock_id.is_some() != workflow.stock_revision.is_some() {
                    errors.push(finding(
                        "workflow_stock_invalid",
                        "workflow",
                        "The workflow has an incomplete stock revision binding.",
                    ));
                }
                let pinned_stock = workflow
                    .stock_id
                    .clone()
                    .zip(workflow.stock_revision)
                    .map(|(id, revision)| ResourceRevision { id, revision });
                if intent.stock.is_some()
                    && !same_revision(intent.stock.as_ref(), pinned_stock.as_ref())
                {
                    errors.push(finding(
                        "workflow_stock_mismatch",
                        "stock",
                        "The requested stock differs from the workflow's pinned stock revision.",
                    ));
                } else if pinned_stock.is_some() {
                    intent.stock = pinned_stock;
                }
                match serde_json::from_value::<PrintIntent>(workflow.definition.clone()) {
                    Ok(base) => {
                        match merge_workflow_intent(base, &intent, &workflow.safe_overrides) {
                            Ok(merged) => intent = merged,
                            Err(mut findings) => errors.append(&mut findings),
                        }
                    }
                    Err(_) => errors.push(finding(
                        "workflow_definition_invalid",
                        "workflow",
                        "The stored workflow definition is invalid.",
                    )),
                }
            }
            Some(_) => errors.push(finding(
                "workflow_not_published",
                "workflow",
                "The exact workflow revision is not published for this printer.",
            )),
            None => errors.push(finding(
                "workflow_revision_unavailable",
                "workflow",
                "The exact workflow revision is unavailable for this printer.",
            )),
        }
    }
    resolve_semantic_options(&mut intent, &printer, &mut errors);
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

fn resolve_semantic_options(
    intent: &mut PrintIntent,
    printer: &piqae_storage_postgres::StoredPrinter,
    errors: &mut Vec<Value>,
) {
    for (facet, requested) in &intent.semantic_options {
        let path = format!("semantic_options.{facet}");
        let Some(choice) = requested.as_str() else {
            errors.push(finding(
                "unsupported_semantic_option",
                &path,
                "Semantic choices must be advertised string values.",
            ));
            continue;
        };
        let Some(resolution) = printer
            .semantic_capabilities
            .native_resolutions
            .get(facet)
            .and_then(|choices| choices.get(choice))
        else {
            errors.push(finding(
                "semantic_option_read_only",
                &path,
                "This semantic choice has no exact writable native resolution.",
            ));
            continue;
        };
        let advertised = printer
            .native_options
            .get(&resolution.native_option)
            .is_some_and(|option| {
                option
                    .choices
                    .iter()
                    .any(|native| native.value == resolution.native_choice)
            });
        if !advertised {
            errors.push(finding(
                "stale_semantic_resolution",
                &path,
                "The mapped native choice is absent from the current driver snapshot.",
            ));
            continue;
        }
        match intent
            .portable_options
            .native_options
            .get(&resolution.native_option)
        {
            Some(existing) if existing != &resolution.native_choice => errors.push(finding(
                "conflicting_semantic_resolution",
                &path,
                "The semantic choice conflicts with another requested native choice.",
            )),
            _ => {
                intent.portable_options.native_options.insert(
                    resolution.native_option.clone(),
                    resolution.native_choice.clone(),
                );
            }
        }
    }
}

fn same_revision(left: Option<&ResourceRevision>, right: Option<&ResourceRevision>) -> bool {
    matches!((left, right), (None, None))
        || matches!((left, right), (Some(left), Some(right)) if left.id == right.id && left.revision == right.revision)
}

#[allow(clippy::cognitive_complexity)]
fn merge_workflow_intent(
    mut base: PrintIntent,
    requested: &PrintIntent,
    safe_overrides: &[String],
) -> Result<PrintIntent, Vec<Value>> {
    let allowed = |path: &str| safe_overrides.iter().any(|candidate| candidate == path);
    let mut errors = Vec::new();
    macro_rules! apply {
        ($field:ident) => {
            if let Some(value) = requested.portable_options.$field.clone() {
                let path = concat!("portable_options.", stringify!($field));
                if allowed(path) {
                    base.portable_options.$field = Some(value);
                } else if base.portable_options.$field.as_ref() != Some(&value) {
                    errors.push(finding(
                        "workflow_override_not_allowed",
                        path,
                        "This workflow does not allow the requested override.",
                    ));
                }
            }
        };
    }
    apply!(bin);
    apply!(collate);
    apply!(color);
    apply!(copies);
    apply!(dpi);
    apply!(duplex);
    apply!(fit_to_page);
    apply!(media);
    apply!(nup);
    apply!(pages);
    apply!(paper);
    apply!(rotate);
    for (key, value) in &requested.portable_options.native_options {
        let path = format!("portable_options.native_options.{key}");
        if allowed(&path) {
            base.portable_options
                .native_options
                .insert(key.clone(), value.clone());
        } else if base.portable_options.native_options.get(key) != Some(value) {
            errors.push(finding(
                "workflow_override_not_allowed",
                &path,
                "This workflow does not allow the requested native override.",
            ));
        }
    }
    for (key, value) in &requested.semantic_options {
        let path = format!("semantic_options.{key}");
        if allowed(&path) {
            base.semantic_options.insert(key.clone(), value.clone());
        } else if base.semantic_options.get(key) != Some(value) {
            errors.push(finding(
                "workflow_override_not_allowed",
                &path,
                "This workflow does not allow the requested semantic override.",
            ));
        }
    }
    base.workflow.clone_from(&requested.workflow);
    base.stock = requested.stock.clone().or(base.stock);
    base.document_manifest = requested.document_manifest.clone();
    if base.printer_id != requested.printer_id
        || base.capability_revision != requested.capability_revision
    {
        errors.push(finding(
            "workflow_definition_mismatch",
            "workflow",
            "Workflow printer or capability revision does not match the request.",
        ));
    }
    if errors.is_empty() {
        Ok(base)
    } else {
        Err(errors)
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod workflow_tests {
    use super::*;
    use piqae_domain::{
        AgentId, NativePrinterChoice, NativePrinterOption, PrinterCapabilities, PrinterState,
        SemanticNativeResolution, SemanticPrinterCapabilities,
    };
    fn intent(copies: Option<u32>) -> PrintIntent {
        PrintIntent {
            schema_version: 1,
            printer_id: "prt_test".into(),
            capability_revision: 3,
            workflow: None,
            stock: None,
            portable_options: JobOptions {
                copies,
                ..JobOptions::default()
            },
            semantic_options: BTreeMap::new(),
            document_manifest: json!({}),
        }
    }
    #[test]
    fn workflow_safe_override_merges_over_pinned_base() {
        let merged = merge_workflow_intent(
            intent(Some(2)),
            &intent(Some(5)),
            &["portable_options.copies".into()],
        )
        .expect("allowed override");
        assert_eq!(merged.portable_options.copies, Some(5));
    }
    #[test]
    fn workflow_unsafe_override_fails_closed() {
        let findings = merge_workflow_intent(intent(Some(2)), &intent(Some(5)), &[])
            .expect_err("override rejected");
        assert_eq!(findings[0]["code"], "workflow_override_not_allowed");
    }

    #[test]
    fn workflow_matching_value_does_not_require_override_permission() {
        let merged = merge_workflow_intent(intent(Some(2)), &intent(Some(2)), &[])
            .expect("matching pinned value is not an override");
        assert_eq!(merged.portable_options.copies, Some(2));
    }

    #[test]
    fn workflow_identity_cannot_be_overridden() {
        let mut requested = intent(None);
        requested.capability_revision = 4;
        let findings = merge_workflow_intent(intent(None), &requested, &[])
            .expect_err("workflow identity must remain pinned");
        assert_eq!(findings[0]["code"], "workflow_definition_mismatch");
    }

    fn semantic_printer() -> piqae_storage_postgres::StoredPrinter {
        piqae_storage_postgres::StoredPrinter {
            id: PrinterId::new(),
            agent_id: AgentId::new(),
            name: "Mapped printer".into(),
            state: PrinterState::Online,
            capabilities: PrinterCapabilities::default(),
            capability_revision: 2,
            native_options: BTreeMap::from([(
                "SensingMode".into(),
                NativePrinterOption {
                    display_name: "Sensing".into(),
                    default_choice: Some("Gap".into()),
                    selected_choice: None,
                    choices: vec![NativePrinterChoice {
                        value: "Gap".into(),
                        display_name: "Gap".into(),
                    }],
                },
            )]),
            semantic_capabilities: SemanticPrinterCapabilities {
                facets: BTreeMap::from([("media.sensing".into(), vec!["gap".into()])]),
                native_resolutions: BTreeMap::from([(
                    "media.sensing".into(),
                    BTreeMap::from([(
                        "gap".into(),
                        SemanticNativeResolution {
                            native_option: "SensingMode".into(),
                            native_choice: "Gap".into(),
                        },
                    )]),
                )]),
                support_pack: None,
            },
            profiles: Vec::new(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn semantic_choice_resolves_only_to_current_advertised_native_choice() {
        let printer = semantic_printer();
        let mut requested = intent(None);
        requested
            .semantic_options
            .insert("media.sensing".into(), json!("gap"));
        let mut errors = Vec::new();
        resolve_semantic_options(&mut requested, &printer, &mut errors);
        assert!(errors.is_empty());
        assert_eq!(
            requested.portable_options.native_options["SensingMode"],
            "Gap"
        );
    }

    #[test]
    fn semantic_choice_without_exact_resolution_remains_read_only() {
        let mut printer = semantic_printer();
        printer.semantic_capabilities.native_resolutions.clear();
        let mut requested = intent(None);
        requested
            .semantic_options
            .insert("media.sensing".into(), json!("gap"));
        let mut errors = Vec::new();
        resolve_semantic_options(&mut requested, &printer, &mut errors);
        assert_eq!(errors[0]["code"], "semantic_option_read_only");
        assert!(requested.portable_options.native_options.is_empty());
    }
}
