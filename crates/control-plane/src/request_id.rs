use axum::{
    extract::Request,
    http::{HeaderName, HeaderValue},
    middleware::Next,
    response::Response,
};
use std::time::Instant;
use tracing::Instrument;
use ulid::Ulid;

pub const HEADER_NAME: HeaderName = HeaderName::from_static("x-request-id");
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
    let path = request.uri().path().to_owned();
    let completion_request_id = request_id.clone();
    let completion_method = method.clone();
    let completion_path = path.clone();
    let started_at = Instant::now();
    let span = tracing::info_span!(
        "http.request",
        request_id = %request_id,
        method = %method,
        path = %path,
        status_code = tracing::field::Empty,
    );
    async move {
        let mut response = CURRENT_REQUEST_ID
            .scope(request_id, next.run(request))
            .await;
        let status_code = response.status().as_u16();
        let elapsed_ms = u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX);
        tracing::Span::current().record("status_code", status_code);
        tracing::info!(
            request_id = %completion_request_id,
            method = %completion_method,
            path = %completion_path,
            status_code,
            elapsed_ms,
            "http request completed"
        );
        response.headers_mut().insert(HEADER_NAME, header_value);
        response
    }
    .instrument(span)
    .await
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
}
