# Control-plane observability

`piqae-server` emits structured JSON tracing to standard output by default.
Set `RUST_LOG` to tune its filter. Every HTTP response includes a bounded
`x-request-id`; the same value is attached to the request span, completion
event, and safe API error envelope.

## Optional OTLP traces

OTLP export is compile-time optional and runtime opt-in:

```sh
cargo build --locked --release -p piqae-control-plane --features otlp
OTEL_EXPORTER_OTLP_TRACES_ENDPOINT=https://collector.example.com/v1/traces \
  target/release/piqae-server
```

Cargo's default feature set and local Compose source builds are lightweight and
exclude OpenTelemetry. Set `PIQAE_SERVER_FEATURES=otlp` for a local Compose
build. The server Dockerfile defaults to `otlp`, so official prebuilt server
images include the capability; runtime export still remains off until an
endpoint or exporter selector is configured.

The exporter uses OTLP over HTTP/protobuf. Configure it with standard
OpenTelemetry environment variables:

- `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` (preferred) or
  `OTEL_EXPORTER_OTLP_ENDPOINT`;
- `OTEL_EXPORTER_OTLP_TRACES_HEADERS` for collector authentication;
- `OTEL_EXPORTER_OTLP_TRACES_TIMEOUT` for the per-export timeout;
- `OTEL_SERVICE_NAME` and `OTEL_RESOURCE_ATTRIBUTES`;
- `OTEL_TRACES_SAMPLER` and `OTEL_TRACES_SAMPLER_ARG`;
- `OTEL_BSP_MAX_QUEUE_SIZE`, `OTEL_BSP_MAX_EXPORT_BATCH_SIZE`,
  `OTEL_BSP_SCHEDULE_DELAY`, and `OTEL_BSP_EXPORT_TIMEOUT`;
- `OTEL_SDK_DISABLED=true` or `OTEL_TRACES_EXPORTER=none` to force local-only
  tracing.

An endpoint or `OTEL_TRACES_EXPORTER=otlp` enables export. With only the
exporter selector set, the OpenTelemetry HTTP default endpoint is used.
Signal-specific settings take precedence over generic OTLP settings. Header
values can contain credentials: inject them through the deployment secret
store, never commit them, and do not enable environment dumps in support
bundles.

Trace resources always include `service.name`, `service.version`, and
`deployment.environment.name`. `OTEL_SERVICE_NAME` overrides the service-name
default, and `PIQAE_ENVIRONMENT` supplies the deployment environment. The
OpenTelemetry SDK also reads `OTEL_RESOURCE_ATTRIBUTES`.

## Correlation and data boundaries

When OTLP support is enabled, the server accepts a valid W3C `traceparent`
header as the remote parent of `http.request`. The request span records the
request ID, HTTP method, normalized route, status, duration, and stable error
type. API
errors add an event and mark server-error spans as failed. Search a trace for
the `request_id` returned to the caller to correlate support reports.

Request query strings, headers, bodies, document bytes, job content, API keys,
device credentials, webhook secrets, collector headers, and database URLs are
not added to spans. Unmatched raw paths are not recorded. Resource IDs remain
out of metric labels.

## Operational limits

Trace export is best effort and does not block request handling. The bounded
batch queue can drop spans when full or when a collector remains unavailable;
exporter diagnostics stay in the JSON log and are excluded from OTLP to avoid
recursive failure traces. On orderly process shutdown, the provider gets up to
five seconds to flush. A hard kill cannot flush buffered spans.

This slice exports traces only. It deliberately does not add an in-process
metrics or OTLP-log pipeline; use the structured JSON stream for logs and add a
collector-side metrics strategy when production service-level objectives are
defined.
