#![allow(clippy::missing_errors_doc)]

use crate::{
    AppState, api::authenticate_native, error::AppError, repository::RepositoryError, request_id,
};
use axum::{
    Json,
    body::Bytes,
    extract::{Query, State},
    http::HeaderMap,
};
use chrono::{DateTime, Datelike, NaiveDate, TimeZone, Utc};
use hmac::{Hmac, Mac};
use piqae_auth::Scope;
use piqae_storage_postgres::{
    StoredBillingSummary, StoredUsageSummary, StripeBillingEvent, StripeProjectionResult,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;
const STRIPE_SIGNATURE_TOLERANCE_SECONDS: i64 = 300;

#[derive(Debug, Deserialize)]
pub struct UsageQuery {
    month: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StripeEnvelope {
    id: String,
    #[serde(rename = "type")]
    event_type: String,
    created: i64,
    data: StripeData,
}

#[derive(Debug, Deserialize)]
struct StripeData {
    object: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct StripeWebhookReceipt {
    received: bool,
    duplicate: bool,
}

pub async fn usage(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<UsageQuery>,
) -> Result<Json<StoredUsageSummary>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::UsageRead).await?;
    let (period_start, period_end) = usage_period(query.month.as_deref())?;
    Ok(Json(
        state
            .repository
            .usage_summary(tenant.workspace_id, period_start, period_end)
            .await?,
    ))
}

pub async fn summary(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<StoredBillingSummary>, AppError> {
    let tenant = authenticate_native(&state, &headers, Scope::UsageRead).await?;
    let (period_start, period_end) = usage_period(None)?;
    if state.capabilities.billing.enabled {
        return Ok(Json(
            state
                .repository
                .billing_summary(tenant.workspace_id, period_start, period_end)
                .await?,
        ));
    }
    let usage = state
        .repository
        .usage_summary(tenant.workspace_id, period_start, period_end)
        .await?;
    Ok(Json(StoredBillingSummary {
        enabled: false,
        managed_by_platform: false,
        plan: None,
        billing_interval: None,
        subscription_status: None,
        grace_ends_at: None,
        accept_new_cloud_jobs: true,
        entitlement: None,
        usage,
        overage_live_jobs: 0,
    }))
}

pub async fn stripe_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<StripeWebhookReceipt>, AppError> {
    if !state.capabilities.billing.enabled {
        return Err(AppError::billing_disabled());
    }
    let secret = state
        .stripe_webhook_secret
        .as_deref()
        .ok_or_else(|| AppError::service_unavailable("stripe_webhook_unavailable"))?;
    verify_stripe_signature(&headers, &body, secret)?;
    let envelope: StripeEnvelope = serde_json::from_slice(&body)?;
    if envelope.id.is_empty() || envelope.event_type.len() > 160 {
        return Err(AppError::invalid(
            "invalid_stripe_event",
            "The Stripe event is invalid.",
        ));
    }
    let Some(event) = stripe_projection(envelope, &body)? else {
        return Ok(Json(StripeWebhookReceipt {
            received: true,
            duplicate: false,
        }));
    };
    let result = state
        .repository
        .project_stripe_billing_event(&event, &request_id::current())
        .await
        .map_err(|error| match error {
            RepositoryError::IdempotencyConflict => AppError::conflict(
                "stripe_event_conflict",
                "The Stripe event ID was already used with different content.",
            ),
            other => other.into(),
        })?;
    Ok(Json(StripeWebhookReceipt {
        received: true,
        duplicate: result == StripeProjectionResult::Duplicate,
    }))
}

fn usage_period(month: Option<&str>) -> Result<(DateTime<Utc>, DateTime<Utc>), AppError> {
    let start_date = if let Some(month) = month {
        if month.len() != 7 {
            return Err(invalid_usage_month());
        }
        NaiveDate::parse_from_str(&format!("{month}-01"), "%Y-%m-%d")
            .map_err(|_| invalid_usage_month())?
    } else {
        Utc::now()
            .date_naive()
            .with_day(1)
            .ok_or_else(invalid_usage_month)?
    };
    let (next_year, next_month) = if start_date.month() == 12 {
        (start_date.year() + 1, 1)
    } else {
        (start_date.year(), start_date.month() + 1)
    };
    let end_date =
        NaiveDate::from_ymd_opt(next_year, next_month, 1).ok_or_else(invalid_usage_month)?;
    let start = Utc.from_utc_datetime(
        &start_date
            .and_hms_opt(0, 0, 0)
            .ok_or_else(invalid_usage_month)?,
    );
    let end = Utc.from_utc_datetime(
        &end_date
            .and_hms_opt(0, 0, 0)
            .ok_or_else(invalid_usage_month)?,
    );
    Ok((start, end))
}

fn invalid_usage_month() -> AppError {
    AppError::invalid(
        "invalid_usage_month",
        "Usage month must use the YYYY-MM UTC format.",
    )
}

fn verify_stripe_signature(headers: &HeaderMap, body: &[u8], secret: &str) -> Result<(), AppError> {
    let value = headers
        .get("stripe-signature")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(AppError::unauthorized)?;
    let mut timestamp = None;
    let mut signatures = Vec::new();
    for component in value.split(',') {
        if let Some(value) = component.strip_prefix("t=") {
            timestamp = value.parse::<i64>().ok();
        } else if let Some(value) = component.strip_prefix("v1=")
            && let Ok(decoded) = hex::decode(value)
        {
            signatures.push(decoded);
        }
    }
    let timestamp = timestamp.ok_or_else(AppError::unauthorized)?;
    if (Utc::now().timestamp() - timestamp).abs() > STRIPE_SIGNATURE_TOLERANCE_SECONDS {
        return Err(AppError::unauthorized());
    }
    let mut signed = timestamp.to_string().into_bytes();
    signed.push(b'.');
    signed.extend_from_slice(body);
    let verified = signatures.iter().any(|signature| {
        HmacSha256::new_from_slice(secret.as_bytes())
            .map(|mut mac| {
                mac.update(&signed);
                mac.verify_slice(signature).is_ok()
            })
            .unwrap_or(false)
    });
    if !verified {
        return Err(AppError::unauthorized());
    }
    Ok(())
}

fn stripe_projection(
    envelope: StripeEnvelope,
    body: &[u8],
) -> Result<Option<StripeBillingEvent>, AppError> {
    if !matches!(
        envelope.event_type.as_str(),
        "checkout.session.completed"
            | "customer.subscription.created"
            | "customer.subscription.updated"
            | "customer.subscription.deleted"
            | "invoice.paid"
            | "invoice.payment_failed"
    ) {
        return Ok(None);
    }
    let object = &envelope.data.object;
    let metadata = object
        .get("metadata")
        .and_then(serde_json::Value::as_object);
    let workspace_reference = metadata
        .and_then(|value| value.get("workspace_id"))
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            object
                .get("client_reference_id")
                .and_then(serde_json::Value::as_str)
        })
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let customer_id = object.get("customer").and_then(stripe_identifier);
    let subscription_event = envelope.event_type.starts_with("customer.subscription.");
    let subscription_id = if subscription_event {
        object.get("id").and_then(stripe_identifier)
    } else {
        object.get("subscription").and_then(stripe_identifier)
    };
    let plan = metadata
        .and_then(|value| value.get("plan"))
        .and_then(serde_json::Value::as_str)
        .filter(|plan| matches!(*plan, "free" | "pro"))
        .map(str::to_owned);
    let billing_interval = metadata
        .and_then(|value| value.get("interval"))
        .and_then(serde_json::Value::as_str)
        .filter(|interval| matches!(*interval, "monthly" | "annual"))
        .map(str::to_owned);
    let status = match envelope.event_type.as_str() {
        "customer.subscription.deleted" => Some("cancelled".into()),
        "invoice.payment_failed" => Some("past_due".into()),
        "invoice.paid" => Some("active".into()),
        value if value.starts_with("customer.subscription.") => object
            .get("status")
            .and_then(serde_json::Value::as_str)
            .and_then(normalize_stripe_status)
            .map(str::to_owned),
        _ => None,
    };
    let created_at = Utc
        .timestamp_opt(envelope.created, 0)
        .single()
        .ok_or_else(|| AppError::invalid("invalid_stripe_event", "Invalid event timestamp."))?;
    Ok(Some(StripeBillingEvent {
        id: envelope.id,
        event_type: envelope.event_type,
        created_at,
        payload_sha256: format!("{:x}", Sha256::digest(body)),
        workspace_reference,
        customer_id,
        subscription_id,
        plan,
        billing_interval,
        status,
        current_period_start: stripe_timestamp(object.get("current_period_start"))?,
        current_period_end: stripe_timestamp(object.get("current_period_end"))?,
        cancel_at_period_end: object
            .get("cancel_at_period_end")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
    }))
}

fn stripe_identifier(value: &serde_json::Value) -> Option<String> {
    value
        .as_str()
        .or_else(|| value.get("id").and_then(serde_json::Value::as_str))
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn normalize_stripe_status(status: &str) -> Option<&'static str> {
    match status {
        "active" => Some("active"),
        "trialing" => Some("trialing"),
        "past_due" | "incomplete" => Some("past_due"),
        "unpaid" | "incomplete_expired" => Some("unpaid"),
        "paused" => Some("paused"),
        "canceled" | "cancelled" => Some("cancelled"),
        _ => None,
    }
}

fn stripe_timestamp(value: Option<&serde_json::Value>) -> Result<Option<DateTime<Utc>>, AppError> {
    value
        .and_then(serde_json::Value::as_i64)
        .map(|timestamp| {
            Utc.timestamp_opt(timestamp, 0).single().ok_or_else(|| {
                AppError::invalid("invalid_stripe_event", "Invalid billing period timestamp.")
            })
        })
        .transpose()
}
