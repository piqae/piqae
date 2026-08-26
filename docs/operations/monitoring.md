# Monitoring

**Status:** structured JSON logs, health/readiness, correlation fields, and
optional OTLP tracing foundations implemented. Packaged dashboards/alerts are
operator-owned.

Monitor five layers:

- API: request rate, latency, 4xx/5xx, authentication failures, readiness;
- delivery: waiting/accepted queue depth, oldest age, retries, expiry, and
  `delivery_uncertain`;
- nodes/printers: online age, heartbeat, warnings, profile mismatch, native
  queue depth, route projection health, observation age and freshness;
- destinations: routable-route count, reservation age, delivery attempts,
  destinations needing review, and fencing conflicts;
- dependencies: PostgreSQL latency/connections/locks/replication lag, object
  errors/checksums, webhook backlog/attempts.

Page on sustained inability to accept jobs, a growing delivery backlog, loss
of a critical node group, database unavailability, or object integrity errors.
Warn on expiring jobs, stale profiles, driver changes, route projections that
are pending or failed, observations older than their advertised freshness
window, rising webhook age, and uncertainty resolutions waiting for node
acknowledgement.

Route telemetry is an observation, not a physical guarantee. A route is
`live` only through its server-projected `fresh_until` time; after that it is
recent or stale according to the API response. Current nodes normally publish
a 90-second freshness window, and observations older than five minutes are
stale. Schedulers may use only fresh, accepting, idle/processing routes. A
spooler state still does not prove paper output, stock, ink, or the absence of
an out-of-band job submitted directly to the operating system.

Independent hosted and self-hosted control planes do not share a reservation
ledger. Monitor each authority separately and do not aggregate their queue
positions into a claim of global FIFO, automatic cross-server failover, or
exactly-once physical delivery. A local installation can serialize its own
Piqae handoffs, but it cannot expose another tenant's job data or control work
submitted outside Piqae.

`/v1/health` is process liveness. `/v1/ready` includes PostgreSQL and object
store readiness and is the traffic gate. Neither proves a physical printer can
produce correct output.

Propagate request, trace, job, node, profile revision, and native spooler IDs.
Redact credentials, content, native profile blobs, and customer label data.
Detailed fields are in
[`operations/observability.md`](observability.md).
