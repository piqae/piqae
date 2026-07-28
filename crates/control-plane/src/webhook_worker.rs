use crate::{AppState, repository::RepositoryError};
use chrono::{Duration as ChronoDuration, Utc};
use rand::Rng;
use reqwest::{Client, redirect::Policy};
use spool_webhooks::{retry_delay, signature};
use std::{
    net::{IpAddr, SocketAddr},
    time::Duration,
};
use thiserror::Error;
use url::Url;

#[derive(Debug, Error)]
pub enum WebhookWorkerError {
    #[error("webhook repository failed: {0}")]
    Repository(#[from] RepositoryError),
    #[error("webhook destination is invalid or blocked")]
    DestinationBlocked,
    #[error("webhook secret cannot be decrypted")]
    Secret,
    #[error("webhook request failed: {0}")]
    Request(#[from] reqwest::Error),
}

#[derive(Clone, Debug)]
pub struct WebhookWorker {
    state: AppState,
}

impl WebhookWorker {
    #[must_use]
    pub const fn new(state: AppState) -> Self {
        Self { state }
    }

    /// Claims and processes at most `limit` due deliveries.
    ///
    /// # Errors
    ///
    /// Returns an error only when claiming work fails. Individual destination
    /// failures are persisted with their next retry and do not abort the batch.
    pub async fn run_once(&self, limit: i64) -> Result<usize, WebhookWorkerError> {
        let deliveries = self
            .state
            .repository
            .claim_webhook_deliveries(limit)
            .await?;
        let count = deliveries.len();
        for delivery in deliveries {
            let result = self.deliver(&delivery).await;
            let (status, delivered) = match result {
                Ok(status) => (Some(i32::from(status.as_u16())), status.is_success()),
                Err(error) => {
                    tracing::warn!(
                        delivery_id = %delivery.id,
                        event_id = %delivery.event_id,
                        %error,
                        "webhook delivery failed"
                    );
                    (None, false)
                }
            };
            let next_attempt_at = if delivered {
                None
            } else {
                retry_delay(usize::try_from(delivery.attempt + 1).unwrap_or(usize::MAX))
                    .map(jittered)
                    .and_then(|delay| ChronoDuration::from_std(delay).ok())
                    .map(|delay| Utc::now() + delay)
            };
            self.state
                .repository
                .record_webhook_attempt(&delivery.id, status, None, next_attempt_at, delivered)
                .await?;
        }
        Ok(count)
    }

    async fn deliver(
        &self,
        delivery: &spool_storage_postgres::WebhookDeliveryWork,
    ) -> Result<reqwest::StatusCode, WebhookWorkerError> {
        let (url, client) = pinned_client(&delivery.url).await?;
        let secret = self
            .state
            .webhook_secrets
            .decrypt(&delivery.secret_ciphertext)
            .map_err(|_| WebhookWorkerError::Secret)?;
        let timestamp = Utc::now().timestamp();
        let body = serde_json::to_vec(&serde_json::json!({
            "id": delivery.event_id,
            "type": delivery.event_type,
            "created_at": Utc::now(),
            "data": delivery.payload,
        }))
        .map_err(|_| WebhookWorkerError::DestinationBlocked)?;
        let signed = signature(&secret, timestamp, &body);
        Ok(client
            .post(url)
            .header("content-type", "application/json")
            .header("user-agent", "Spool-Webhook/1.0")
            .header("spool-event-id", &delivery.event_id)
            .header("spool-timestamp", timestamp)
            .header("spool-signature", format!("v1={signed}"))
            .header("spool-attempt", delivery.attempt + 1)
            .body(body)
            .send()
            .await?
            .status())
    }
}

async fn pinned_client(value: &str) -> Result<(Url, Client), WebhookWorkerError> {
    let url = Url::parse(value).map_err(|_| WebhookWorkerError::DestinationBlocked)?;
    if !matches!(url.scheme(), "http" | "https") || url.username() != "" || url.password().is_some()
    {
        return Err(WebhookWorkerError::DestinationBlocked);
    }
    let host = url
        .host_str()
        .filter(|host| !host.eq_ignore_ascii_case("localhost") && !host.ends_with(".localhost"))
        .ok_or(WebhookWorkerError::DestinationBlocked)?;
    let port = url
        .port_or_known_default()
        .ok_or(WebhookWorkerError::DestinationBlocked)?;
    let addresses = tokio::net::lookup_host((host, port))
        .await
        .map_err(|_| WebhookWorkerError::DestinationBlocked)?
        .collect::<Vec<_>>();
    if addresses.is_empty()
        || addresses
            .iter()
            .any(|address| !public_address(address.ip()))
    {
        return Err(WebhookWorkerError::DestinationBlocked);
    }
    let pinned = SocketAddr::new(addresses[0].ip(), port);
    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(Policy::none())
        .resolve(host, pinned)
        .build()?;
    Ok((url, client))
}

const fn public_address(address: IpAddr) -> bool {
    !address.is_loopback()
        && !address.is_unspecified()
        && !address.is_multicast()
        && match address {
            IpAddr::V4(address) => {
                !address.is_private()
                    && !address.is_link_local()
                    && !address.is_broadcast()
                    && !address.is_documentation()
            }
            IpAddr::V6(address) => {
                !address.is_unique_local()
                    && !address.is_unicast_link_local()
                    && (address.segments()[0] & 0xffc0) != 0xfec0
            }
        }
}

fn jittered(duration: Duration) -> Duration {
    let milliseconds = duration.as_millis();
    if milliseconds == 0 {
        return duration;
    }
    let jitter = (milliseconds / 10).max(1);
    let offset = rand::thread_rng().gen_range(0..=jitter * 2);
    let adjusted = milliseconds.saturating_sub(jitter).saturating_add(offset);
    Duration::from_millis(u64::try_from(adjusted).unwrap_or(u64::MAX))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn private_and_metadata_addresses_are_blocked() {
        assert!(!public_address("127.0.0.1".parse().unwrap()));
        assert!(!public_address("10.0.0.1".parse().unwrap()));
        assert!(!public_address("169.254.169.254".parse().unwrap()));
        assert!(!public_address("::1".parse().unwrap()));
        assert!(public_address("1.1.1.1".parse().unwrap()));
    }
}
