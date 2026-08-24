//! Optional Sentry error reporting for the control plane.
//!
//! This is a sibling of the OTLP trace path, not a replacement for it. OTLP
//! stays the vendor-neutral pipe a self-hoster points at their own collector
//! (Sentry included, since Sentry ingests OTLP natively). This module adds
//! error grouping and panic capture for deployments that want it.
//!
//! It is compile-time optional and runtime opt-in: without a `SENTRY_DSN` the
//! client is never constructed, no layer is installed, and no background
//! transport thread is started.
//!
//! Every payload passes [`Redactor`] before the transport sees it.

use super::redaction::Redactor;
use anyhow::{Context as _, Result};
use sentry::{ClientInitGuard, ClientOptions, integrations::tracing as sentry_tracing, types::Dsn};
use std::{str::FromStr, sync::Arc, time::Duration};
use tracing::Subscriber;
use tracing_subscriber::registry::LookupSpan;

/// Deployment environment reported when `PIQAE_ENVIRONMENT` is unset. Matches
/// the OTLP resource default so both pipelines label a deployment the same way.
const DEFAULT_ENVIRONMENT: &str = "development";
/// Bounded breadcrumb ring. Breadcrumbs are scrubbed, but a smaller window
/// still means less to scrub and less to hold.
const MAX_BREADCRUMBS: usize = 32;
/// Matches the OTLP provider's flush budget on orderly shutdown.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
/// All errors are reported unless an operator dials it back.
const DEFAULT_ERROR_SAMPLE_RATE: f32 = 1.0;
/// Performance tracing belongs to the OTLP path.
const DEFAULT_TRACES_SAMPLE_RATE: f32 = 0.0;

/// Initializes Sentry when `SENTRY_DSN` is set.
///
/// Returns `Ok(None)` when no DSN is configured, which is the only state a
/// default deployment ever reaches.
pub fn init_from_env() -> Result<Option<ClientInitGuard>> {
    init(super::nonempty_env("SENTRY_DSN").as_deref())
}

/// Initializes Sentry for an explicit DSN.
///
/// A missing or blank DSN is the inert path: no client, no transport thread,
/// no layer, and no background work of any kind.
fn init(dsn: Option<&str>) -> Result<Option<ClientInitGuard>> {
    let Some(dsn) = dsn.map(str::trim).filter(|dsn| !dsn.is_empty()) else {
        return Ok(None);
    };
    // The DSN identifies the ingest project and is treated as a credential
    // everywhere else in this repository, so it is never echoed. `ParseDsnError`
    // reports only the structural problem, never the rejected value.
    let dsn = Dsn::from_str(dsn).context("SENTRY_DSN is not a valid Sentry DSN")?;

    Ok(Some(sentry::init(client_options(dsn)?)))
}

/// Builds the client configuration, including the redaction hooks every payload
/// must pass through.
fn client_options(dsn: Dsn) -> Result<ClientOptions> {
    let redactor = Arc::new(Redactor::new().context("compile Sentry redaction rules")?);
    let breadcrumb_redactor = Arc::clone(&redactor);
    let environment =
        super::nonempty_env("PIQAE_ENVIRONMENT").unwrap_or_else(|| DEFAULT_ENVIRONMENT.into());

    let mut options = ClientOptions::new()
        .release(concat!("piqae-control-plane@", env!("CARGO_PKG_VERSION")))
        .environment(environment)
        .send_default_pii(false)
        .attach_stacktrace(true)
        .max_breadcrumbs(MAX_BREADCRUMBS)
        .shutdown_timeout(SHUTDOWN_TIMEOUT)
        .before_send(move |event| Some(redactor.event(event)))
        .before_breadcrumb(move |breadcrumb| breadcrumb_redactor.breadcrumb(breadcrumb))
        .sample_rate(sample_rate(
            super::nonempty_env("SENTRY_SAMPLE_RATE").as_deref(),
            DEFAULT_ERROR_SAMPLE_RATE,
        ))
        .traces_sample_rate(sample_rate(
            super::nonempty_env("SENTRY_TRACES_SAMPLE_RATE").as_deref(),
            DEFAULT_TRACES_SAMPLE_RATE,
        ));
    options.dsn = Some(dsn);
    // Hostnames identify a self-hosted operator's infrastructure, and the
    // contexts integration would otherwise fill this in from the hostname.
    options.server_name = None;

    Ok(options)
}

/// Bridges `tracing` into Sentry instead of running a parallel reporting API.
///
/// `error` events become Sentry issues; `warn` and `info` become breadcrumbs on
/// the next issue; `debug` and `trace` are ignored. Spans are deliberately not
/// converted into Sentry transactions: OTLP owns tracing, and span fields would
/// otherwise reach the vendor without passing `before_send`.
pub fn tracing_layer<S>() -> sentry_tracing::SentryLayer<S>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    sentry_tracing::layer()
        .event_filter(|metadata| match *metadata.level() {
            tracing::Level::ERROR => sentry_tracing::EventFilter::Event,
            tracing::Level::WARN | tracing::Level::INFO => sentry_tracing::EventFilter::Breadcrumb,
            tracing::Level::DEBUG | tracing::Level::TRACE => sentry_tracing::EventFilter::Ignore,
        })
        .span_filter(|_| false)
}

/// Parses a `0.0`–`1.0` sample rate, falling back to `default` for anything
/// missing, unparseable, or out of range.
fn sample_rate(value: Option<&str>, default: f32) -> f32 {
    value
        .and_then(|value| value.trim().parse::<f32>().ok())
        .filter(|rate| rate.is_finite() && (0.0..=1.0).contains(rate))
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::{MAX_BREADCRUMBS, SHUTDOWN_TIMEOUT, client_options, init, sample_rate};
    use anyhow::Result;
    use sentry::{protocol::Event, types::Dsn};
    use std::str::FromStr;

    /// Syntactically valid, deliberately unroutable. Nothing in these tests
    /// constructs a client, so no transport is ever created.
    const TEST_DSN: &str = "https://examplePublicKey@o0.ingest.piqae.invalid/0";

    fn options() -> Result<sentry::ClientOptions> {
        client_options(Dsn::from_str(TEST_DSN)?)
    }

    #[test]
    fn an_absent_or_blank_dsn_initializes_nothing() {
        assert!(matches!(init(None), Ok(None)));
        assert!(matches!(init(Some("")), Ok(None)));
        assert!(matches!(init(Some("   ")), Ok(None)));
    }

    #[test]
    fn a_malformed_dsn_is_rejected_without_echoing_it() {
        let secret = "piq_live_EXAMPLE_NOT_A_REAL_KEY";
        let Err(error) = init(Some(secret)) else {
            panic!("a malformed DSN must not be accepted");
        };
        let rendered = format!("{error:#}");

        assert!(
            rendered.contains("SENTRY_DSN"),
            "unhelpful error: {rendered}"
        );
        assert!(
            !rendered.contains(secret),
            "the rejected value was echoed: {rendered}"
        );
    }

    #[test]
    fn client_options_pin_the_privacy_defaults() -> Result<()> {
        let options = options()?;

        assert!(!options.send_default_pii);
        assert!(options.server_name.is_none());
        assert!(options.attach_stacktrace);
        assert_eq!(options.max_breadcrumbs, MAX_BREADCRUMBS);
        assert_eq!(options.shutdown_timeout, SHUTDOWN_TIMEOUT);
        assert_eq!(
            options.release.as_deref(),
            Some(concat!("piqae-control-plane@", env!("CARGO_PKG_VERSION")))
        );
        assert!(options.before_send.is_some(), "redaction hook is not wired");
        assert!(
            options.before_breadcrumb.is_some(),
            "breadcrumb redaction hook is not wired"
        );
        Ok(())
    }

    #[test]
    fn the_before_send_hook_scrubs_events() -> Result<()> {
        let options = options()?;
        let Some(before_send) = options.before_send.as_ref() else {
            panic!("redaction hook is not wired");
        };

        let event = Event {
            message: Some(
                "rejected piq_live_EXAMPLE_NOT_A_REAL_KEY for operator@example.com".into(),
            ),
            server_name: Some("piqae-prod-01.internal".into()),
            ..Default::default()
        };

        let Some(scrubbed) = before_send(event) else {
            panic!("the hook must not drop ordinary events");
        };

        assert_eq!(
            scrubbed.message.as_deref(),
            Some("rejected [redacted] for [redacted]")
        );
        assert!(scrubbed.server_name.is_none());
        Ok(())
    }

    #[test]
    fn sample_rate_falls_back_for_unusable_values() {
        assert!((sample_rate(None, 1.0) - 1.0).abs() < f32::EPSILON);
        assert!((sample_rate(Some("not-a-number"), 0.25) - 0.25).abs() < f32::EPSILON);
        assert!((sample_rate(Some("-0.5"), 1.0) - 1.0).abs() < f32::EPSILON);
        assert!((sample_rate(Some("1.5"), 1.0) - 1.0).abs() < f32::EPSILON);
        assert!((sample_rate(Some("NaN"), 1.0) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn sample_rate_accepts_the_documented_range() {
        assert!((sample_rate(Some("0"), 1.0)).abs() < f32::EPSILON);
        assert!((sample_rate(Some(" 0.1 "), 1.0) - 0.1).abs() < f32::EPSILON);
        assert!((sample_rate(Some("1"), 0.0) - 1.0).abs() < f32::EPSILON);
    }
}
