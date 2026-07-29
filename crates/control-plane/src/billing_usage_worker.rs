use chrono::Utc;
use reqwest::Client;
use spool_storage_postgres::{ClaimedUsageExport, PostgresStore, StorageError};
use std::time::Duration;

#[derive(Clone)]
pub struct BillingUsageWorker {
    store: PostgresStore,
    client: Client,
    stripe_secret_key: String,
    stripe_meter_event_name: String,
    stripe_meter_events_url: String,
}

impl std::fmt::Debug for BillingUsageWorker {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BillingUsageWorker")
            .field("store", &self.store)
            .field("client", &self.client)
            .field("stripe_secret_key", &"[REDACTED]")
            .field("stripe_meter_event_name", &self.stripe_meter_event_name)
            .field("stripe_meter_events_url", &self.stripe_meter_events_url)
            .finish()
    }
}

impl BillingUsageWorker {
    #[must_use]
    pub fn new(
        store: PostgresStore,
        stripe_secret_key: impl Into<String>,
        stripe_meter_event_name: impl Into<String>,
    ) -> Result<Self, reqwest::Error> {
        Ok(Self {
            store,
            client: Client::builder().timeout(Duration::from_secs(15)).build()?,
            stripe_secret_key: stripe_secret_key.into(),
            stripe_meter_event_name: stripe_meter_event_name.into(),
            stripe_meter_events_url: "https://api.stripe.com/v1/billing/meter_events".into(),
        })
    }

    /// Prepares ended periods and submits at most `limit` Stripe meter events.
    ///
    /// # Errors
    ///
    /// Returns a storage error when durable export preparation, claiming, or
    /// acknowledgement fails. Stripe transport failures are durably retried.
    pub async fn run_once(&self, limit: usize) -> Result<usize, StorageError> {
        self.store.prepare_due_usage_exports(Utc::now()).await?;
        let mut submitted = 0;
        for _ in 0..limit {
            let Some(export) = self.store.claim_usage_export(Utc::now()).await? else {
                break;
            };
            if self.submit(&export).await {
                self.store
                    .complete_usage_export(&export.id, &export.claim_token)
                    .await?;
                submitted += 1;
            }
        }
        Ok(submitted)
    }

    async fn submit(&self, export: &ClaimedUsageExport) -> bool {
        let overage_blocks = export.overage_blocks.to_string();
        let timestamp = export.period_end.timestamp().to_string();
        let form = [
            ("event_name", self.stripe_meter_event_name.as_str()),
            (
                "payload[stripe_customer_id]",
                export.stripe_customer_id.as_str(),
            ),
            ("payload[value]", overage_blocks.as_str()),
            ("identifier", export.stripe_event_identifier.as_str()),
            ("timestamp", timestamp.as_str()),
        ];
        let Ok(encoded) = serde_urlencoded::to_string(form) else {
            let _ = self
                .store
                .fail_usage_export(&export.id, &export.claim_token, "request_encoding")
                .await;
            return false;
        };
        let response = self
            .client
            .post(&self.stripe_meter_events_url)
            .bearer_auth(&self.stripe_secret_key)
            .header("content-type", "application/x-www-form-urlencoded")
            .header("idempotency-key", &export.stripe_event_identifier)
            .body(encoded)
            .send()
            .await;
        match response {
            Ok(response) if response.status().is_success() => true,
            Ok(response) => {
                let error = format!("stripe_http_{}", response.status().as_u16());
                let _ = self
                    .store
                    .fail_usage_export(&export.id, &export.claim_token, &error)
                    .await;
                false
            }
            Err(_) => {
                let _ = self
                    .store
                    .fail_usage_export(&export.id, &export.claim_token, "stripe_transport")
                    .await;
                false
            }
        }
    }
}
