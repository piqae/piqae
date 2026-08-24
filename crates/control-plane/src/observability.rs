#[cfg(feature = "sentry")]
mod error_reporting;
#[cfg(feature = "sentry")]
mod redaction;

use anyhow::{Context, Result};
use tracing_subscriber::{
    EnvFilter, Layer, Registry, layer::Layered, layer::SubscriberExt, util::SubscriberInitExt,
};

const DEFAULT_FILTER: &str = "info";

#[cfg(feature = "otlp")]
const SERVICE_NAME: &str = "piqae-control-plane";

/// The subscriber every optional layer is composed onto.
type FilteredRegistry = Layered<EnvFilter, Registry>;
type BoxedLayer = Box<dyn Layer<FilteredRegistry> + Send + Sync + 'static>;

#[must_use = "the guard flushes pending trace spans when it is shut down or dropped"]
pub struct ObservabilityGuard {
    #[cfg(feature = "otlp")]
    provider: Option<opentelemetry_sdk::trace::SdkTracerProvider>,
    // `sentry::ClientInitGuard` drains the send queue when it is dropped.
    #[cfg(feature = "sentry")]
    error_reporting: Option<sentry::ClientInitGuard>,
}

impl std::fmt::Debug for ObservabilityGuard {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = formatter.debug_struct("ObservabilityGuard");
        #[cfg(feature = "otlp")]
        debug.field("otlp", &self.provider.is_some());
        #[cfg(feature = "sentry")]
        debug.field("error_reporting", &self.error_reporting.is_some());
        debug.finish()
    }
}

pub fn init() -> Result<ObservabilityGuard> {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER));

    // Both optional pipelines are constructed before the subscriber is
    // installed so a configuration failure is reported by `main` rather than
    // half-initializing telemetry.
    #[cfg(feature = "sentry")]
    let error_reporting = error_reporting::init_from_env()?;

    #[cfg(feature = "otlp")]
    let otlp = if otlp_requested_from_env() {
        Some(build_otlp_pipeline()?)
    } else {
        None
    };

    #[cfg_attr(not(any(feature = "otlp", feature = "sentry")), allow(unused_mut))]
    let mut layers: Vec<BoxedLayer> = vec![tracing_subscriber::fmt::layer().json().boxed()];

    #[cfg(feature = "otlp")]
    let provider = otlp.map(|(provider, layer)| {
        layers.push(layer);
        provider
    });

    #[cfg(feature = "sentry")]
    if error_reporting.is_some() {
        layers.push(error_reporting::tracing_layer().boxed());
    }

    tracing_subscriber::registry()
        .with(filter)
        .with(layers)
        .try_init()
        .context("initialize structured tracing")?;

    #[cfg(feature = "otlp")]
    if provider.is_some() {
        tracing::info!(
            service.name = SERVICE_NAME,
            service.version = env!("CARGO_PKG_VERSION"),
            transport = "http/protobuf",
            "OTLP trace export enabled"
        );
    }

    #[cfg(feature = "sentry")]
    if error_reporting.is_some() {
        // The DSN is never logged; it identifies the ingest project.
        tracing::info!(
            service.version = env!("CARGO_PKG_VERSION"),
            "Sentry error reporting enabled"
        );
    }

    Ok(ObservabilityGuard {
        #[cfg(feature = "otlp")]
        provider,
        #[cfg(feature = "sentry")]
        error_reporting,
    })
}

impl ObservabilityGuard {
    // The Result is meaningful in OTLP builds, where flushing can fail. Keep
    // one call shape for both feature sets so main has no telemetry-specific
    // control flow.
    #[allow(clippy::missing_const_for_fn, clippy::unnecessary_wraps)]
    pub fn shutdown(self) -> Result<()> {
        #[cfg(any(feature = "otlp", feature = "sentry"))]
        let mut this = self;

        #[cfg(feature = "otlp")]
        let flushed = this.provider.take().map_or(Ok(()), |provider| {
            provider
                .shutdown()
                .context("flush OpenTelemetry trace provider")
        });

        // Drained last so a failing trace flush is still reported.
        #[cfg(feature = "sentry")]
        drop(this.error_reporting.take());

        #[cfg(feature = "otlp")]
        flushed?;

        #[cfg(not(any(feature = "otlp", feature = "sentry")))]
        let _ = self;

        Ok(())
    }
}

#[cfg(any(feature = "otlp", feature = "sentry"))]
impl Drop for ObservabilityGuard {
    fn drop(&mut self) {
        #[cfg(feature = "otlp")]
        if let Some(provider) = self.provider.take() {
            let _ = provider.shutdown();
        }
        #[cfg(feature = "sentry")]
        drop(self.error_reporting.take());
    }
}

#[cfg(feature = "otlp")]
fn build_otlp_pipeline() -> Result<(opentelemetry_sdk::trace::SdkTracerProvider, BoxedLayer)> {
    use opentelemetry::{KeyValue, global, trace::TracerProvider};
    use opentelemetry_otlp::WithHttpConfig;
    use opentelemetry_sdk::{Resource, propagation::TraceContextPropagator};
    use tracing_subscriber::filter::filter_fn;

    let http_client = OtlpHttpClient(
        reqwest::Client::builder()
            .build()
            .context("configure OTLP HTTP client")?,
    );
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .with_http_client(http_client)
        .build()
        .context("configure OTLP trace exporter")?;
    let service_name = nonempty_env("OTEL_SERVICE_NAME").unwrap_or_else(|| SERVICE_NAME.into());
    let deployment_environment =
        nonempty_env("PIQAE_ENVIRONMENT").unwrap_or_else(|| "development".into());
    let resource = Resource::builder()
        .with_service_name(service_name)
        .with_attributes([
            KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
            KeyValue::new("deployment.environment.name", deployment_environment),
        ])
        .build();
    let processor =
        opentelemetry_sdk::trace::span_processor_with_async_runtime::BatchSpanProcessor::builder(
            exporter,
            opentelemetry_sdk::runtime::Tokio,
        )
        .build();
    let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
        .with_span_processor(processor)
        .with_resource(resource)
        .build();
    let tracer = provider.tracer(SERVICE_NAME);
    let otlp_layer = tracing_opentelemetry::layer()
        .with_tracer(tracer)
        // Exporter diagnostics remain in JSON logs but cannot recursively
        // generate more telemetry if an export fails.
        .with_filter(filter_fn(|metadata| {
            !metadata.target().starts_with("opentelemetry")
        }));

    global::set_tracer_provider(provider.clone());
    global::set_text_map_propagator(TraceContextPropagator::new());

    Ok((provider, Box::new(otlp_layer)))
}

#[cfg(feature = "otlp")]
#[derive(Debug)]
struct OtlpHttpClient(reqwest::Client);

#[cfg(feature = "otlp")]
#[async_trait::async_trait]
impl opentelemetry_http::HttpClient for OtlpHttpClient {
    async fn send_bytes(
        &self,
        request: opentelemetry_http::Request<opentelemetry_http::Bytes>,
    ) -> Result<
        opentelemetry_http::Response<opentelemetry_http::Bytes>,
        opentelemetry_http::HttpError,
    > {
        let mut response = self
            .0
            .execute(request.try_into()?)
            .await?
            .error_for_status()?;
        let headers = std::mem::take(response.headers_mut());
        let mut response = opentelemetry_http::Response::builder()
            .status(response.status())
            .body(response.bytes().await?)?;
        *response.headers_mut() = headers;
        Ok(response)
    }
}

#[cfg(feature = "otlp")]
fn otlp_requested_from_env() -> bool {
    otlp_requested(
        nonempty_env("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT").as_deref(),
        nonempty_env("OTEL_EXPORTER_OTLP_ENDPOINT").as_deref(),
        nonempty_env("OTEL_TRACES_EXPORTER").as_deref(),
        nonempty_env("OTEL_SDK_DISABLED").as_deref(),
    )
}

#[cfg(any(feature = "otlp", feature = "sentry"))]
fn nonempty_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

#[cfg(feature = "otlp")]
fn otlp_requested(
    traces_endpoint: Option<&str>,
    generic_endpoint: Option<&str>,
    traces_exporter: Option<&str>,
    sdk_disabled: Option<&str>,
) -> bool {
    if sdk_disabled.is_some_and(|value| value.eq_ignore_ascii_case("true")) {
        return false;
    }
    if traces_exporter.is_some_and(|value| value.eq_ignore_ascii_case("none")) {
        return false;
    }

    traces_endpoint.is_some()
        || generic_endpoint.is_some()
        || traces_exporter.is_some_and(|value| {
            value
                .split(',')
                .any(|exporter| exporter.trim().eq_ignore_ascii_case("otlp"))
        })
}

#[cfg(all(test, feature = "otlp"))]
mod tests {
    use super::otlp_requested;

    #[test]
    fn endpoint_or_otlp_exporter_enables_export() {
        assert!(otlp_requested(
            Some("https://collector.example/v1/traces"),
            None,
            None,
            None
        ));
        assert!(otlp_requested(None, None, Some("otlp"), None));
        assert!(otlp_requested(
            None,
            Some("https://collector.example"),
            None,
            None
        ));
    }

    #[test]
    fn standard_disable_controls_take_precedence() {
        assert!(!otlp_requested(
            Some("https://collector.example/v1/traces"),
            None,
            Some("otlp"),
            Some("TRUE")
        ));
        assert!(!otlp_requested(
            Some("https://collector.example/v1/traces"),
            None,
            Some("NoNe"),
            None
        ));
        assert!(!otlp_requested(None, None, None, None));
    }
}
