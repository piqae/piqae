# Node diagnostics

**Status:** remote structured diagnostic collection is implemented. Requests
are durable, uploaded through authenticated node sync, acknowledged only after
server persistence, retained for 14 days, and available through the node API.
Raw-log and archive export remains intentionally unavailable.

Collect the minimum evidence needed:

- Piqae version, OS version, architecture, and node ID;
- timestamps with timezone and Piqae job IDs;
- logical printer/profile ID and revision;
- local state transitions and native spooler job ID;
- redacted agent/executor logs around the failure;
- printer state and driver name/version;
- whether a local OS test page succeeds.

Never include API keys, enrolment tokens, `local.token`, device private keys,
database URLs, signed object URLs, document contents, native profile blobs,
full DEVMODE/PrintCore state, or customer labels.

Remote reports contain only version/platform identifiers, aggregate queue
counts, SQLite integrity, executor crash count, and bounded machine-readable
error codes. They never contain log messages, filesystem paths, job or document
identifiers, content, credentials, URLs, printer/profile blobs, or driver state.
The node retains at most eight unacknowledged reports; the server accepts at
most 16 KiB per report, returns at most fifty reports, and hides expired data.

Use `POST /v1/nodes/{node_id}/diagnostics` to request collection, `GET` on the
same path to list retained status, and
`GET /v1/nodes/{node_id}/diagnostics/{request_id}` for one result. A `failed`
report includes only a stable collection error code.

Native helper failures expose bounded structured evidence: the exit/timeout
class, a coarse stderr classification, byte counts, and whether the diagnostic
pipe closed cleanly. Raw native stderr and stable hashes of that potentially
sensitive text are deliberately not sent to the control plane. Detailed raw
driver output remains outside automatic diagnostics.

A support bundle must be redacted before it leaves the node. Treat native
driver configuration as potentially sensitive even when it appears binary.
Review every archive manually, encrypt it in transit, set a retention deadline,
and record who received it.

Use [incident response](../operations/incident-response.md) for fleet impact.
