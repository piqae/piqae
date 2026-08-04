//! Fixed-window rate limiting for unauthenticated node-onboarding endpoints.
//!
//! Pairing and enrolment accept requests from callers that have not yet proved
//! anything, and pairing writes a durable row per attempt. Edge WAF limits
//! remain the operator's primary defence — see `deploy/README.md` — but the
//! control plane must not depend on them to keep its own tables bounded.
//!
//! Two buckets are enforced per request. The per-client bucket is keyed on the
//! forwarded client address and stops one caller from monopolizing the
//! endpoint. That key is spoofable by anything that can set `x-forwarded-for`,
//! so a global bucket bounds the total admission rate regardless of the key,
//! and it is the global bucket that actually caps table growth.

use crate::error::AppError;
use axum::{extract::Request, http::HeaderMap, middleware::Next, response::Response};
use chrono::{DateTime, Duration, Utc};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

/// Bucket key standing in for every caller whose address is unknown or shared.
const SHARED_CLIENT_KEY: &str = "shared";
/// Bucket key for the ceiling applied across all callers.
const GLOBAL_KEY: &str = "*";
/// Upper bound on tracked buckets, so the limiter cannot itself be flooded.
const MAXIMUM_TRACKED_KEYS: usize = 8_192;

#[derive(Clone, Copy, Debug)]
struct Window {
    started_at: DateTime<Utc>,
    count: u32,
}

#[derive(Debug)]
struct Buckets {
    windows: HashMap<String, Window>,
}

/// A fixed-window limiter shared by every request to a guarded route.
#[derive(Clone, Debug)]
pub struct RateLimiter {
    buckets: Arc<Mutex<Buckets>>,
    per_client: u32,
    global: u32,
    window: Duration,
}

impl RateLimiter {
    /// Builds a limiter admitting `per_client` requests per key and `global`
    /// requests in total within each `window_seconds` window.
    #[must_use]
    pub fn new(per_client: u32, global: u32, window_seconds: i64) -> Self {
        Self {
            buckets: Arc::new(Mutex::new(Buckets {
                windows: HashMap::new(),
            })),
            per_client,
            global,
            window: Duration::seconds(window_seconds),
        }
    }

    /// Records one request and reports whether it stays inside both budgets.
    ///
    /// Both buckets are charged even when the first one rejects, so a caller
    /// cannot use rejected requests to avoid its share of the global budget.
    #[must_use]
    pub fn admits(&self, client_key: &str, now: DateTime<Utc>) -> bool {
        let Ok(mut buckets) = self.buckets.lock() else {
            // A poisoned lock means a previous holder panicked mid-update. Fail
            // open rather than locking every node out of onboarding; the global
            // ceiling is a safeguard, not an authorization decision.
            return true;
        };
        if buckets.windows.len() > MAXIMUM_TRACKED_KEYS {
            buckets
                .windows
                .retain(|_, window| now - window.started_at < self.window);
            buckets.windows.shrink_to_fit();
        }
        let within_client = self.charge(&mut buckets, client_key, self.per_client, now);
        let within_global = self.charge(&mut buckets, GLOBAL_KEY, self.global, now);
        within_client && within_global
    }

    fn charge(&self, buckets: &mut Buckets, key: &str, limit: u32, now: DateTime<Utc>) -> bool {
        let window = buckets.windows.entry(key.to_owned()).or_insert(Window {
            started_at: now,
            count: 0,
        });
        if now - window.started_at >= self.window {
            window.started_at = now;
            window.count = 0;
        }
        window.count = window.count.saturating_add(1);
        window.count <= limit
    }
}

/// Derives the per-client bucket key from the forwarded client address.
///
/// The value is a bucket label only. It never grants access, so a spoofed
/// header can at worst move a caller between buckets that are all bounded.
fn client_key(headers: &HeaderMap) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.len() <= 64)
        .map_or_else(|| SHARED_CLIENT_KEY.to_owned(), str::to_owned)
}

/// Rejects requests to a guarded route once either budget is exhausted.
///
/// # Errors
///
/// Returns a retryable `too_many_requests` error when the caller or the
/// deployment as a whole is over budget.
pub async fn middleware(
    limiter: axum::extract::State<RateLimiter>,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    if limiter.admits(&client_key(request.headers()), Utc::now()) {
        return Ok(next.run(request).await);
    }
    Err(AppError::too_many_requests())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn at(seconds: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + seconds, 0).unwrap_or_default()
    }

    #[test]
    fn a_client_is_limited_within_one_window_and_recovers_in_the_next() {
        let limiter = RateLimiter::new(2, 100, 60);
        assert!(limiter.admits("10.0.0.1", at(0)));
        assert!(limiter.admits("10.0.0.1", at(1)));
        assert!(!limiter.admits("10.0.0.1", at(2)));
        assert!(limiter.admits("10.0.0.1", at(61)));
    }

    #[test]
    fn one_noisy_client_does_not_exhaust_another_clients_budget() {
        let limiter = RateLimiter::new(1, 100, 60);
        assert!(limiter.admits("10.0.0.1", at(0)));
        assert!(!limiter.admits("10.0.0.1", at(1)));
        assert!(limiter.admits("10.0.0.2", at(1)));
    }

    #[test]
    fn the_global_ceiling_bounds_callers_that_rotate_their_forwarded_address() {
        let limiter = RateLimiter::new(100, 3, 60);
        for index in 0..3 {
            assert!(limiter.admits(&format!("10.0.0.{index}"), at(0)));
        }
        assert!(!limiter.admits("10.0.0.99", at(0)));
    }

    #[test]
    fn rejected_requests_still_consume_the_global_budget() {
        let limiter = RateLimiter::new(1, 3, 60);
        assert!(limiter.admits("10.0.0.1", at(0)));
        assert!(!limiter.admits("10.0.0.1", at(0)));
        assert!(!limiter.admits("10.0.0.1", at(0)));
        // The global bucket is now spent, so a fresh client is rejected too.
        assert!(!limiter.admits("10.0.0.2", at(0)));
    }

    #[test]
    fn tracked_keys_stay_bounded_when_addresses_are_rotated() {
        let limiter = RateLimiter::new(1, u32::MAX, 60);
        for index in 0..(MAXIMUM_TRACKED_KEYS + 200) {
            let _ = limiter.admits(&format!("10.0.{}.{}", index / 256, index % 256), at(0));
        }
        let tracked = limiter
            .buckets
            .lock()
            .map(|buckets| buckets.windows.len())
            .unwrap_or_default();
        assert!(tracked <= MAXIMUM_TRACKED_KEYS + 201, "tracked {tracked}");
    }

    #[test]
    fn unproxied_and_malformed_forwarded_addresses_share_one_bucket() {
        let mut headers = HeaderMap::new();
        assert_eq!(client_key(&headers), SHARED_CLIENT_KEY);
        headers.insert("x-forwarded-for", HeaderValue::from_static("   "));
        assert_eq!(client_key(&headers), SHARED_CLIENT_KEY);
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("203.0.113.7, 10.0.0.1"),
        );
        assert_eq!(client_key(&headers), "203.0.113.7");
    }
}
