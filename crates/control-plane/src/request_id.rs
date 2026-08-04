use axum::{
    extract::{MatchedPath, Request},
    http::{HeaderName, HeaderValue},
    middleware::Next,
    response::Response,
};
use std::time::Instant;
use tracing::Instrument;
use ulid::Ulid;

#[cfg(feature = "otlp")]
use opentelemetry::propagation::TextMapPropagator;
#[cfg(feature = "otlp")]
use tracing_opentelemetry::OpenTelemetrySpanExt;

pub const HEADER_NAME: HeaderName = HeaderName::from_static("x-request-id");
/// Authoritative server clock, in Unix milliseconds, echoed on every response.
///
/// Nodes sign requests with a timestamp the control plane checks against a
/// bounded window. Publishing the server clock on every response—including the
/// rejection a skewed node receives—lets a node with a drifting clock correct
/// its own offset and recover without operator involvement.
pub const SERVER_TIME_HEADER: HeaderName = HeaderName::from_static("x-piqae-server-time");
const MAX_REQUEST_ID_BYTES: usize = 128;

tokio::task_local! {
    static CURRENT_REQUEST_ID: String;
}

#[must_use]
pub fn current() -> String {
    CURRENT_REQUEST_ID
        .try_with(Clone::clone)
        .unwrap_or_else(|_| generated())
}

pub async fn middleware(mut request: Request, next: Next) -> Response {
    let request_id = request
        .headers()
        .get(&HEADER_NAME)
        .and_then(|value| value.to_str().ok())
        .filter(|value| valid(value))
        .map_or_else(generated, str::to_owned);
    let header_value = HeaderValue::from_str(&request_id)
        .unwrap_or_else(|_| HeaderValue::from_static("req_invalid"));
    request
        .headers_mut()
        .insert(HEADER_NAME, header_value.clone());
    let method = request.method().clone();
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map_or("unmatched", MatchedPath::as_str)
        .to_owned();
    let completion_request_id = request_id.clone();
    let completion_method = method.clone();
    let completion_route = route.clone();
    let started_at = Instant::now();
    let span = tracing::info_span!(
        "http.request",
        request_id = %request_id,
        method = %method,
        route = %route,
        http.request.method = %method,
        http.route = %route,
        http.response.status_code = tracing::field::Empty,
        otel.kind = "server",
        otel.status_code = tracing::field::Empty,
        error.type = tracing::field::Empty,
        status_code = tracing::field::Empty,
    );
    #[cfg(feature = "otlp")]
    let _ = span.set_parent(remote_parent(request.headers()));
    async move {
        let mut response = CURRENT_REQUEST_ID
            .scope(request_id, next.run(request))
            .await;
        let status_code = response.status().as_u16();
        let elapsed_ms = u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX);
        tracing::Span::current().record("status_code", status_code);
        tracing::Span::current().record("http.response.status_code", status_code);
        if response.status().is_server_error() {
            tracing::Span::current().record("otel.status_code", "ERROR");
        }
        tracing::info!(
            request_id = %completion_request_id,
            method = %completion_method,
            route = %completion_route,
            status_code,
            elapsed_ms,
            "http request completed"
        );
        response.headers_mut().insert(HEADER_NAME, header_value);
        if let Ok(server_time) =
            HeaderValue::from_str(&chrono::Utc::now().timestamp_millis().to_string())
        {
            response
                .headers_mut()
                .insert(SERVER_TIME_HEADER, server_time);
        }
        response
    }
    .instrument(span)
    .await
}

#[cfg(feature = "otlp")]
fn remote_parent(headers: &axum::http::HeaderMap) -> opentelemetry::Context {
    opentelemetry_sdk::propagation::TraceContextPropagator::new()
        .extract(&opentelemetry_http::HeaderExtractor(headers))
}

fn generated() -> String {
    format!("req_{}", Ulid::new())
}

fn valid(value: &str) -> bool {
    let bytes = value.as_bytes();
    (1..=MAX_REQUEST_ID_BYTES).contains(&bytes.len())
        && bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':'))
}

#[cfg(test)]
mod tests {
    use super::valid;

    #[test]
    fn caller_request_id_format_is_strict_and_bounded() {
        assert!(valid("trace-01HV.example:4"));
        assert!(!valid(""));
        assert!(!valid(" bad"));
        assert!(!valid("bad id"));
        assert!(!valid(&"a".repeat(129)));
    }

    #[cfg(feature = "otlp")]
    #[test]
    fn extracts_a_valid_w3c_remote_parent() {
        use axum::http::{HeaderMap, HeaderValue};
        use opentelemetry::trace::TraceContextExt;

        let mut headers = HeaderMap::new();
        headers.insert(
            "traceparent",
            HeaderValue::from_static("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"),
        );

        let parent = super::remote_parent(&headers);
        let span = parent.span();
        let span_context = span.span_context();
        assert!(span_context.is_valid());
        assert!(span_context.is_remote());
        assert_eq!(
            span_context.trace_id().to_string(),
            "4bf92f3577b34da6a3ce929d0e0e4736"
        );
    }
}
