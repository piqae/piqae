use crate::{AppState, repository::RepositoryError};
use async_trait::async_trait;
use chrono::{Duration as ChronoDuration, Utc};
use futures::{StreamExt, stream};
use piqae_storage_postgres::WebhookDeliveryWork;
use piqae_webhooks::{retry_delay, signature};
use rand::Rng;
use reqwest::{Client, redirect::Policy};
use std::{
    fmt::Debug,
    net::{IpAddr, SocketAddr},
    sync::Arc,
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

const WEBHOOK_DELIVERY_CONCURRENCY: usize = 8;
const WEBHOOK_DNS_TIMEOUT: Duration = Duration::from_secs(5);
const WEBHOOK_HTTP_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(test)]
const WEBHOOK_ATTEMPT_BUDGET_SECONDS: usize = 15;

#[async_trait]
trait DeliveryTransport: Debug + Send + Sync {
    async fn deliver(
        &self,
        state: &AppState,
        delivery: &WebhookDeliveryWork,
    ) -> Result<reqwest::StatusCode, WebhookWorkerError>;
}

#[derive(Debug)]
struct HttpDeliveryTransport;

#[derive(Clone, Debug)]
pub struct WebhookWorker {
    state: AppState,
    transport: Arc<dyn DeliveryTransport>,
}

impl WebhookWorker {
    #[must_use]
    pub fn new(state: AppState) -> Self {
        Self {
            state,
            transport: Arc::new(HttpDeliveryTransport),
        }
    }

    /// Claims and processes at most `limit` due deliveries.
    ///
    /// # Errors
    ///
    /// Returns an error when claiming or recording work fails. Every claimed
    /// delivery is processed before a persistence error is returned. Individual
    /// destination failures are persisted and do not abort sibling deliveries.
    pub async fn run_once(&self, limit: i64) -> Result<usize, WebhookWorkerError> {
        let deliveries = self
            .state
            .repository
            .claim_webhook_deliveries(limit)
            .await?;
        let count = deliveries.len();
        let results = stream::iter(deliveries)
            .map(|delivery| async move { self.process_delivery(delivery).await })
            .buffer_unordered(WEBHOOK_DELIVERY_CONCURRENCY)
            .collect::<Vec<_>>()
            .await;
        if let Some(error) = results.into_iter().find_map(Result::err) {
            return Err(error);
        }
        Ok(count)
    }

    async fn process_delivery(
        &self,
        delivery: WebhookDeliveryWork,
    ) -> Result<(), WebhookWorkerError> {
        let result = self.transport.deliver(&self.state, &delivery).await;
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
        Ok(())
    }

    #[cfg(test)]
    fn with_transport(state: AppState, transport: Arc<dyn DeliveryTransport>) -> Self {
        Self { state, transport }
    }
}

#[async_trait]
impl DeliveryTransport for HttpDeliveryTransport {
    async fn deliver(
        &self,
        state: &AppState,
        delivery: &WebhookDeliveryWork,
    ) -> Result<reqwest::StatusCode, WebhookWorkerError> {
        let (url, client) = pinned_client(&delivery.url).await?;
        let secret = state
            .webhook_secrets
            .decrypt(&delivery.secret_ciphertext)
            .map_err(|_| WebhookWorkerError::Secret)?;
        let timestamp = Utc::now().timestamp();
        let body = webhook_body(delivery)?;
        let signed = signature(&secret, timestamp, &body);
        Ok(client
            .post(url)
            .header("content-type", "application/json")
            .header("user-agent", "Piqae-Webhook/1.0")
            .header("piqae-event-id", &delivery.event_id)
            .header("piqae-timestamp", timestamp)
            .header("piqae-signature", format!("v1={signed}"))
            .header("piqae-attempt", delivery.attempt + 1)
            .body(body)
            .send()
            .await?
            .status())
    }
}

fn webhook_body(delivery: &WebhookDeliveryWork) -> Result<Vec<u8>, WebhookWorkerError> {
    serde_json::to_vec(&serde_json::json!({
        "id": delivery.event_id,
        "type": delivery.event_type,
        "created_at": delivery.event_occurred_at,
        "data": delivery.payload,
    }))
    .map_err(|_| WebhookWorkerError::DestinationBlocked)
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
    let addresses =
        tokio::time::timeout(WEBHOOK_DNS_TIMEOUT, tokio::net::lookup_host((host, port)))
            .await
            .map_err(|_| WebhookWorkerError::DestinationBlocked)?
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
        .timeout(WEBHOOK_HTTP_TIMEOUT)
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
    use crate::{
        authentication::{StaticAuthenticator, TenantContext},
        repository::{MemoryRepository, Repository},
    };
    use piqae_domain::{EnvironmentId, WorkspaceId};
    use piqae_storage_postgres::{WEBHOOK_CLAIM_TTL_SECONDS, WEBHOOK_MAX_CLAIM_BATCH};
    use std::{
        collections::HashMap,
        sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering},
        },
    };

    type CapturedBodies = HashMap<String, Vec<(i32, Vec<u8>)>>;

    #[derive(Debug)]
    struct MockTransport {
        active: AtomicUsize,
        maximum_active: AtomicUsize,
        delay: Duration,
        bodies: Mutex<CapturedBodies>,
    }

    impl MockTransport {
        fn new(delay: Duration) -> Self {
            Self {
                active: AtomicUsize::new(0),
                maximum_active: AtomicUsize::new(0),
                delay,
                bodies: Mutex::new(HashMap::new()),
            }
        }
    }

    #[async_trait]
    impl DeliveryTransport for MockTransport {
        async fn deliver(
            &self,
            _state: &AppState,
            delivery: &WebhookDeliveryWork,
        ) -> Result<reqwest::StatusCode, WebhookWorkerError> {
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.maximum_active.fetch_max(active, Ordering::SeqCst);
            self.bodies
                .lock()
                .unwrap()
                .entry(delivery.id.clone())
                .or_default()
                .push((delivery.attempt, webhook_body(delivery)?));
            tokio::time::sleep(self.delay).await;
            self.active.fetch_sub(1, Ordering::SeqCst);
            if delivery.url.contains("/fail/") {
                Err(WebhookWorkerError::DestinationBlocked)
            } else {
                Ok(reqwest::StatusCode::OK)
            }
        }
    }

    async fn test_worker(
        count: usize,
        failed: &[usize],
        transport: Arc<MockTransport>,
    ) -> (WebhookWorker, MemoryRepository) {
        let repository = MemoryRepository::default();
        let state = AppState::new_with_webhook_key_for_tests(
            Arc::new(repository.clone()),
            Arc::new(StaticAuthenticator::default()),
            [7; 32],
        );
        let tenant = TenantContext::unrestricted(WorkspaceId::new(), EnvironmentId::new());
        let ciphertext = state.webhook_secrets.encrypt(b"test-secret").unwrap();
        for index in 0..count {
            let path = if failed.contains(&index) {
                "fail"
            } else {
                "success"
            };
            repository
                .create_webhook(
                    &format!("whk_{index}"),
                    tenant.workspace_id,
                    tenant.environment_id,
                    &format!("https://example.test/{path}/{index}"),
                    &["job.updated".into()],
                    &ciphertext,
                )
                .await
                .unwrap();
        }
        state
            .publish(
                tenant,
                "job.updated",
                &serde_json::json!({"job_id": "job_test"}),
            )
            .await
            .unwrap();
        (WebhookWorker::with_transport(state, transport), repository)
    }

    #[test]
    fn private_and_metadata_addresses_are_blocked() {
        assert!(!public_address("127.0.0.1".parse().unwrap()));
        assert!(!public_address("10.0.0.1".parse().unwrap()));
        assert!(!public_address("169.254.169.254".parse().unwrap()));
        assert!(!public_address("::1".parse().unwrap()));
        assert!(public_address("1.1.1.1".parse().unwrap()));
    }

    #[test]
    fn claim_ttl_exceeds_the_bounded_worst_case_batch_budget() {
        let max_batch = usize::try_from(WEBHOOK_MAX_CLAIM_BATCH).unwrap();
        let waves = max_batch.div_ceil(WEBHOOK_DELIVERY_CONCURRENCY);
        let worst_case_seconds = waves * WEBHOOK_ATTEMPT_BUDGET_SECONDS;
        assert!(usize::try_from(WEBHOOK_CLAIM_TTL_SECONDS).unwrap() > worst_case_seconds);
    }

    #[tokio::test]
    async fn deliveries_are_bounded_concurrent_and_fail_independently() {
        let transport = Arc::new(MockTransport::new(Duration::from_millis(25)));
        let (worker, repository) = test_worker(12, &[2, 9], transport.clone()).await;
        assert_eq!(worker.run_once(25).await.unwrap(), 12);
        let maximum_active = transport.maximum_active.load(Ordering::SeqCst);
        assert!(maximum_active > 1);
        assert!(maximum_active <= WEBHOOK_DELIVERY_CONCURRENCY);
        let retries = repository.claim_webhook_deliveries(100).await.unwrap();
        assert_eq!(retries.len(), 2);
        assert!(retries.iter().all(|delivery| delivery.attempt == 1));
    }

    #[tokio::test]
    async fn retry_body_is_byte_identical_for_the_same_event() {
        let transport = Arc::new(MockTransport::new(Duration::ZERO));
        let (worker, _) = test_worker(1, &[0], transport.clone()).await;
        assert_eq!(worker.run_once(1).await.unwrap(), 1);
        assert_eq!(worker.run_once(1).await.unwrap(), 1);
        let attempts = {
            let bodies = transport.bodies.lock().unwrap();
            bodies.values().next().unwrap().clone()
        };
        assert_eq!(attempts.len(), 2);
        assert_eq!(attempts[0].0, 0);
        assert_eq!(attempts[1].0, 1);
        assert_eq!(attempts[0].1, attempts[1].1);
    }
}
