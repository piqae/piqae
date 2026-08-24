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
build, or `otlp,sentry` to also include the error reporting described below.
The server Dockerfile defaults to `otlp,sentry`, so official prebuilt server
images include both capabilities; runtime export still remains off until an
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

### Choosing a destination

OTLP is the vendor-neutral pipe. `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` can name a
collector you run yourself, a hosted vendor, or Sentry, which ingests OTLP
natively:

```sh
OTEL_EXPORTER_OTLP_TRACES_ENDPOINT=https://o000000.ingest.sentry.io/api/0000000/integration/otlp/v1/traces
OTEL_EXPORTER_OTLP_TRACES_HEADERS=x-sentry-auth=sentry%20sentry_key%3D<public-key>
```

Nothing in the server changes between those destinations; only the endpoint and
the authentication header differ. Pointing OTLP at your own collector is a
first-class configuration, not a degraded one.

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

## Optional Sentry error reporting

Sentry error reporting is a separate compile-time optional, runtime opt-in
capability. It complements OTLP rather than replacing it: OTLP carries traces,
Sentry groups failures and panics. Run neither, either, or both.

```sh
cargo build --locked --release -p piqae-control-plane --features sentry
SENTRY_DSN=https://<public-key>@o000000.ingest.sentry.io/0000000 \
  target/release/piqae-server
```

Without `SENTRY_DSN` no client is constructed, no layer is installed, no
transport thread starts, and the process makes no outbound connection. A DSN
that is present but malformed fails startup rather than running blind; the
rejected value is never echoed into the error.

Configuration:

- `SENTRY_DSN` enables reporting. A blank value is treated as unset.
- `SENTRY_SAMPLE_RATE` (default `1.0`) samples error events.
- `SENTRY_TRACES_SAMPLE_RATE` (default `0`) is deliberately off. Performance
  tracing belongs to the OTLP path; Sentry spans are not produced.
- `PIQAE_ENVIRONMENT` labels the deployment, matching the OTLP resource.

Reporting is wired through `tracing` rather than a parallel API. `error!`
events become Sentry issues, `warn!` and `info!` become breadcrumbs on the next
issue, and `debug!`/`trace!` are ignored. Panics are captured. The release is
reported as `piqae-control-plane@<version>`.

### Redaction

Every event and breadcrumb passes an in-process scrubber before the transport
serializes it, holding the same line as the dashboard's
`apps/web/src/lib/observability/sentry.ts`. Dropped outright: user identity, IP
addresses, `server_name`, request bodies, query strings, cookies, headers,
environment maps, stack-frame locals, source context lines, absolute build
paths, and breadcrumbs from user interaction. Stack frames keep `function`,
`package`, `filename`, and `lineno`, which locate a failure without exposing
the build machine's account name. Pattern scrubbed from whatever remains: Piqae API keys,
platform keys, device codes and enrollment tokens (`piq_*`/`spl_*`),
`Authorization` material, secret-bearing `key=value` pairs, email addresses,
database and broker connection URLs, and credentials embedded in any URL. URLs
are reduced to scheme, host, and a de-identified path. Free text is truncated
and structured payloads are bounded in depth and width.

Redaction is enforced in the client's `before_send` and `before_breadcrumb`
hooks, so it cannot be bypassed by a future call site. See the unit tests in
`crates/control-plane/src/observability/redaction.rs` for the asserted output.

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

The OTLP slice exports traces only. It deliberately does not add an in-process
metrics or OTLP-log pipeline; use the structured JSON stream for logs and add a
collector-side metrics strategy when production service-level objectives are
defined.

Sentry error reporting is best effort on the same terms. Events are queued and
drained on orderly shutdown within the same five-second budget; a hard kill
cannot flush them. A failure to reach the ingest endpoint never blocks request
handling and never fails a print job.
