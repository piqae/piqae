# Monitoring

**Status:** structured JSON logs, health/readiness, correlation fields, and
optional OTLP tracing foundations implemented. Packaged dashboards/alerts are
operator-owned.

Monitor four layers:

- API: request rate, latency, 4xx/5xx, authentication failures, readiness;
- delivery: waiting/accepted queue depth, oldest age, retries, expiry, and
  `delivery_uncertain`;
- nodes/printers: online age, heartbeat, warnings, profile mismatch, native
  queue depth;
- dependencies: PostgreSQL latency/connections/locks/replication lag, object
  errors/checksums, webhook backlog/attempts.

Page on sustained inability to accept jobs, a growing delivery backlog, loss
of a critical node group, database unavailability, or object integrity errors.
Warn on expiring jobs, stale profiles, driver changes, and rising webhook age.

`/v1/health` is process liveness. `/v1/ready` includes PostgreSQL and object
store readiness and is the traffic gate. Neither proves a physical printer can
produce correct output.

Propagate request, trace, job, node, profile revision, and native spooler IDs.
Redact credentials, content, native profile blobs, and customer label data.
Detailed fields are in
[`operations/observability.md`](observability.md).
