# Security, privacy, observability, and operations

## Threat model

The system handles untrusted API callers, documents, URLs, printer commands,
drivers, webhook targets, local browser traffic, and long-lived agents behind
customer firewalls.

High-priority threats:

- stolen API or device credentials;
- cross-tenant job/printer access;
- malicious PDF parser input;
- arbitrary RAW device commands;
- URI-printing SSRF into a customer's private network;
- webhook SSRF from the control plane;
- document leakage through storage, logs, crashes, or support bundles;
- replayed jobs or duplicated side effects;
- malicious update supply chain;
- compromised control plane issuing unintended print jobs;
- local unprivileged users controlling a privileged service;
- unsafe printer drivers blocking or crashing the agent.

## Trust boundaries

- application to public API;
- browser/UI to public API;
- control plane to agent;
- agent to renderer child;
- agent to OS spooler/driver;
- OS spooler to printer;
- control plane to webhook;
- agent to URI content origin;
- updater to release repository.

Each crossing has explicit authentication, validation, size/time bounds, and
trace metadata.

## Authentication and authorisation

### API keys

- show a key once;
- store Argon2id or equivalent hashes plus prefix for lookup;
- allow scope, workspace, expiry, network restriction, and rotation;
- audit create/use/revoke;
- rate-limit by key and workspace;
- never place keys in query strings or logs.

### Users

Use OIDC/OAuth for the hosted UI and allow self-hosted local accounts only when
configured. Support MFA through the identity provider. Roles should begin
small: owner, administrator, operator, developer, viewer.

### Agents

- one-time enrolment token expires quickly and is consumed atomically;
- agent generates a device key locally;
- subsequent sessions prove possession;
- revoke devices without rotating the whole workspace;
- support certificate/token rotation over an authenticated session;
- bind printer ownership to the enrolled installation;
- detect concurrent cloned identity sessions.

### Local agent API

- loopback-only by default;
- random anti-CSRF/bootstrap secret for browser UI;
- bearer token, named pipe, or Unix socket for automation;
- origin checks and strict Content Security Policy;
- no unauthenticated remote listener;
- privilege separation between read-only health and job submission/config.

## Document security

### In transit

TLS for all remote traffic. Agents validate hostname and trust roots; custom
enterprise CAs are explicit configuration. Prefer TLS 1.3 while allowing a
documented compatible floor.

### At rest

- object-level random data encryption key for hosted document content;
- wrapping key in a KMS or self-hosted configured master key;
- agent content encryption using a key protected by DPAPI, macOS Keychain, or a
  root-readable Linux secret where practical;
- encrypted database/storage volumes remain recommended;
- short default content retention;
- cryptographic deletion by dropping per-object key where supported.

### Logs

Never log:

- base64 content;
- document bytes;
- URI user-info or authentication credentials;
- API/device keys;
- webhook secrets;
- full sensitive query strings.

Log content length, SHA-256, safe origin host when permitted, and redacted
metadata.

## PDF sandbox

The renderer runs out of process with:

- a fresh or recycled bounded worker;
- no network;
- a read-only input descriptor and write-only output channel;
- memory, CPU, page-count, pixel-count, and wall-time limits;
- least OS privilege;
- Windows Job Object/restricted token or stronger sandbox where feasible;
- Linux namespaces/seccomp/AppArmor;
- a macOS sandbox profile/least-privilege service arrangement;
- crash containment and one clean-process retry.

Fuzz PDF parsing and page-range/transform code. Track PDFium security releases
and rebuild quickly.

## URI printing and SSRF

Agent-side URI printing intentionally allows access to resources the control
plane may not reach, which is useful and dangerous.

Policies:

- `disabled`;
- public HTTPS only;
- allow-listed schemes/hosts/CIDRs;
- unrestricted compatibility mode with explicit administrator warning.

Defences:

- only HTTP/HTTPS initially;
- reject redirects rather than following them;
- resolve DNS inside the agent, validate every returned address, and pin the
  approved addresses into the request client to reduce DNS rebinding;
- block loopback, unspecified, link-local, private, multicast, reserved, and
  IPv4-mapped private destinations by default;
- always block known cloud-metadata, unspecified, and multicast destinations;
- bypass ambient HTTP proxy configuration so a proxy cannot bypass destination
  checks;
- cap connect time at 5 seconds, total request time at 120 seconds, and content
  at 50 MiB while streaming;
- do not forward control-plane or agent credentials;
- validate TLS by default;
- reject credentials embedded in a URI and keep supplied authentication out of
  errors and logs.

These rules are shared by local submissions and hosted URI descriptors.
Trusted local/self-hosted installations can opt into private, loopback, and
link-local sources with `--allow-private-uri-sources` or
`SPOOL_ALLOW_PRIVATE_URI_SOURCES=true`; the default is `false`, and the
always-blocked classes remain blocked. Digest authentication is explicitly
disabled in V1 rather than being silently downgraded. Credentials are
job-scoped and deleted with content metadata.

## RAW printing controls

RAW scope is separate from PDF scope. Optional policy:

- allowed printer IDs;
- maximum bytes and `qty`;
- permitted content language;
- deny known persistent-configuration commands where reliable parsing exists;
- require extra permission for cash-drawer/cutter workflows;
- complete audit trail.

Do not claim that arbitrary RAW languages can be safely parsed. Authorisation
and size limits are the primary controls.

## Update security

- reproducible builds where practical;
- signed release manifest and per-artifact digests;
- platform code signing/notarisation;
- TUF-style metadata or equivalent rollback/freeze protection;
- stable, beta, and pinned channels;
- staged fleet rollout and health halt;
- keep previous binary until post-update health is confirmed;
- database migrations designed for rollback/forward compatibility;
- no unsigned remote plugin execution.

Open-source users can disable automatic updates and install distribution
packages manually.

## Error taxonomy

Stable machine codes are hierarchical:

```text
API.AUTH.INVALID_KEY
API.REQUEST.INVALID_PAGE_RANGE
CONTENT.URI.BLOCKED_BY_POLICY
CONTENT.DIGEST_MISMATCH
AGENT.DISK.LOW
RENDERER.PDF.MALFORMED
RENDERER.RESOURCE_LIMIT
PRINTER.CAPABILITY.OPTION_UNSUPPORTED
SPOOLER.NATIVE_REJECTED
SPOOLER.HANDOFF_UNCERTAIN
DEVICE.PAPER_OUT
DEVICE.OFFLINE
```

Each error has:

- safe user message;
- technical detail;
- observer/stage;
- retryability;
- native code/message redacted as necessary;
- documentation link;
- correlation and trace IDs.

## Observability

### Traces

Trace:

```text
create request
  -> content persist/fetch
  -> route/notify
  -> agent claim/download
  -> validate/render
  -> native spool submission
  -> spooler/device observations
  -> webhook/live-event delivery
```

Propagate W3C Trace Context across HTTP and encode trace context in agent
messages. Do not rely on a single process log to reconstruct a job.

### Metrics

At minimum:

- API request latency/error/rate;
- job count and age by state;
- content upload/download bytes and duration;
- connected/stale agents;
- agent event-outbox lag;
- printer queue depth and blocked reasons;
- render duration, peak memory, crashes, and timeout;
- native submission duration/errors;
- state propagation latency;
- webhook success, age, retry, and dead-letter count;
- disk capacity and cleanup failures;
- ambiguous handoff count;
- duplicate prevention/idempotency conflicts.

High-cardinality job/printer/tenant IDs belong in traces/logs, not metric
labels.

### Logs

Structured JSON in services; human-readable CLI view. The agent keeps bounded
rotating logs. Windows also emits important lifecycle failures to Event Log;
Linux integrates with journald; macOS integrates with unified logging as
appropriate.

### Audit

Append-only audit events for:

- user, role, and key changes;
- agent enrol/revoke;
- printer policy changes;
- job create/cancel/reprint;
- ambiguous-state resolution;
- webhook changes and replay;
- content retention access;
- updates and remote diagnostics.

## Health model

Separate:

- process alive;
- database writable;
- object store usable;
- agent connected;
- local queue writable;
- printer watcher healthy;
- renderer healthy;
- spooler reachable;
- individual printer ready.

A single red/green status is insufficient. Readiness endpoints must avoid
claiming the service is ready when it cannot durably register jobs.

## Webhook delivery semantics

Webhook delivery is durable and at least once. Spool claims no more than 100
due deliveries for five minutes and processes at most eight requests
concurrently. Each request has a five-second DNS budget and a ten-second HTTP
budget, so the claim remains valid beyond the bounded worst-case duration of a
full batch. One slow or failing destination does not block unrelated
destinations. Failed deliveries retain the existing retry and dead-letter
policy.

The event ID, `created_at`, event type, data, and serialized JSON body remain
stable across retries. The `spool-attempt`, `spool-timestamp`, and
`spool-signature` headers are generated for each attempt. A process failure or
an ambiguous network response can still produce a duplicate delivery.
Consumers must therefore deduplicate using `spool-event-id` (the same value as
the payload `id`) and make handlers idempotent before returning a successful
2xx response.

## Backups and disaster recovery

### Self-hosted

Back up:

- PostgreSQL with point-in-time recovery appropriate to risk;
- object/document storage for the configured retention window;
- server signing/encryption configuration;
- release and migration version;
- no agent private keys.

Document restore into an isolated environment and how agents trust/reconnect to
the restored control plane.

### Agent

The agent database is operational state, not the authoritative long-term
history in remote mode. Do not clone it to deploy another machine. Backups are
usually unnecessary; if restored, identity-clone detection and job
reconciliation must run before consumption.

## Retention defaults

Proposed defaults:

- control-plane document: delete after spooler acceptance plus short grace, or
  earlier after durable agent acceptance for strict privacy;
- agent document: delete after final/uncertain state plus 24 hours;
- job metadata/events: 90 days self-host configurable;
- diagnostic logs: 14 days;
- audit: one year for hosted service;
- webhook attempts: 30 days.

Make every value configurable and show actual deletion state. A cleanup job is
observable, idempotent, and tested under failure.

## SaaS isolation and operations

- tenant ID required in every domain/storage access;
- PostgreSQL row-level security as defence in depth where practical;
- per-tenant object prefixes and encryption context;
- quotas for jobs, bytes, agents, connections, webhooks, and event rate;
- abuse detection for RAW commands and URI fetches;
- regional data-residency plan before promising it;
- incident response and content-access audit;
- status page and public incident history;
- metering is derived from immutable job events, not mutable counters.

Start billing only after usage semantics are clear: charge on registered job,
agent acceptance, or spooler acceptance. Spooler acceptance is the fairest
default but creates delayed/reversed usage records for expired jobs.

## Self-host operations

Provide:

- Docker Compose for evaluation;
- Helm only when Kubernetes demand exists;
- native binary/system package option;
- `/health/live`, `/health/ready`, and metrics endpoints;
- migration command with preflight and backup warning;
- configuration reference with environment-variable and file forms;
- version compatibility table for server/agent;
- offline upgrade procedure;
- capacity guidance based on concurrent agents, job rate, and retained bytes.

The control plane should remain usable without external telemetry. Remote crash
reporting and update checks are opt-in for self-hosters.
