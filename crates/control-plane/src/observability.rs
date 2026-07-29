use anyhow::{Context, Result};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

const DEFAULT_FILTER: &str = "info";

#[cfg(feature = "otlp")]
const SERVICE_NAME: &str = "spool-control-plane";

#[derive(Debug)]
#[must_use = "the guard flushes pending trace spans when it is shut down or dropped"]
pub struct ObservabilityGuard {
    #[cfg(feature = "otlp")]
    provider: Option<opentelemetry_sdk::trace::SdkTracerProvider>,
}

pub fn init() -> Result<ObservabilityGuard> {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER));

    #[cfg(feature = "otlp")]
    if otlp_requested_from_env() {
        return init_with_otlp(filter);
    }

    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().json())
        .try_init()
        .context("initialize structured tracing")?;

    Ok(ObservabilityGuard {
        #[cfg(feature = "otlp")]
        provider: None,
    })
}

impl ObservabilityGuard {
    // The Result is meaningful in OTLP builds, where flushing can fail. Keep
    // one call shape for both feature sets so main has no telemetry-specific
    // control flow.
    #[allow(clippy::missing_const_for_fn, clippy::unnecessary_wraps)]
    pub fn shutdown(self) -> Result<()> {
        #[cfg(feature = "otlp")]
        {
            let mut this = self;
            if let Some(provider) = this.provider.take() {
                provider
                    .shutdown()
                    .context("flush OpenTelemetry trace provider")?;
            }
        }

        #[cfg(not(feature = "otlp"))]
        let _ = self;

        Ok(())
    }
}

#[cfg(feature = "otlp")]
impl Drop for ObservabilityGuard {
    fn drop(&mut self) {
        if let Some(provider) = self.provider.take() {
            let _ = provider.shutdown();
        }
    }
}

#[cfg(feature = "otlp")]
fn init_with_otlp(filter: EnvFilter) -> Result<ObservabilityGuard> {
    use opentelemetry::{KeyValue, global, trace::TracerProvider};
    use opentelemetry_otlp::WithHttpConfig;
    use opentelemetry_sdk::{Resource, propagation::TraceContextPropagator};
    use std::env;
    use tracing_subscriber::{Layer, filter::filter_fn};

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
        nonempty_env("SPOOL_ENVIRONMENT").unwrap_or_else(|| "development".into());
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
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().json())
        .with(otlp_layer)
        .try_init()
        .context("initialize structured and OpenTelemetry tracing")?;

    tracing::info!(
        service.name = SERVICE_NAME,
        service.version = env!("CARGO_PKG_VERSION"),
        transport = "http/protobuf",
        "OTLP trace export enabled"
    );

    Ok(ObservabilityGuard {
        provider: Some(provider),
    })
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

#[cfg(feature = "otlp")]
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
