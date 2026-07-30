# Node diagnostics

**Status:** structured logs, local status, queues, printer/profile metadata, and
redacted diagnostic-request foundations are implemented. A complete one-click
support-bundle export is Planned, not a working V1 shell action.

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

A support bundle must be redacted before it leaves the node. Treat native
driver configuration as potentially sensitive even when it appears binary.
Review every archive manually, encrypt it in transit, set a retention deadline,
and record who received it.

Use [incident response](../operations/incident-response.md) for fleet impact.
