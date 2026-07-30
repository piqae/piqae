# Native API and data model

## Purpose

The PrintNode-compatible API exists for migration. The native API should be
cleaner, stream documents efficiently, expose honest states, and remain the
contract used by the Svelte interfaces.

This document is a proposed starting contract, not a frozen 1.0 schema.

## API conventions

- Base path: `/v1`.
- JSON for metadata; `application/pdf` or `application/octet-stream` for direct
  uploads.
- ULID-style string resource IDs.
- UTC RFC 3339 timestamps.
- `Idempotency-Key` on side-effecting creates.
- Every response carries `X-Request-Id`. A caller value is preserved only when
  it is 1–128 ASCII bytes matching
  `^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$`; otherwise the server generates
  `req_<ULID>`. Native `error.request_id` and PrintNode-compatible error `uid`
  exactly equal that response header.
- Cursor pagination, with stable `nextCursor`.
- Optimistic concurrency through `ETag`/`If-Match` on mutable configuration.
- Errors use `application/problem+json` with stable machine codes.
- API version is independent of agent wire-protocol version.

## Core resources

### Workspace

Tenant/security boundary. Contains retention, URI, RAW, retry, update, and
webhook policies.

### Agent

Logical installed service. Important fields:

- `id`;
- `name`;
- `installationId`;
- enrolment/device-key status;
- OS, architecture, agent/protocol/renderer versions;
- connection state and session count;
- last seen and heartbeat health;
- queue depth, disk status, and policy revision;
- labels/tags;
- created/revoked timestamps.

### Printer

- logical ID;
- owning agent ID;
- OS queue identity and native URI/port fingerprint;
- display name/description/location;
- default flag;
- state and reasons;
- capability document plus source/revision/timestamp;
- adapter/backend selection and override;
- application queue policy;
- created, last seen, and removed timestamps.

This resource represents an installed operating-system destination. The
expanded native-profile design adds optional physical-device grouping,
immutable profiles, stock, and stable routing targets without changing the
PrintNode-compatible meaning of a printer. See
[native print profiles, stock, and routing](16-native-print-profiles-stock-and-routing.md).

### Print profile

- stable profile ID and immutable revision;
- destination and driver fingerprint;
- platform-native configuration kind and local blob digest;
- portable summary and required stock/dependencies;
- safe per-job overrides;
- readiness, validation, test, publish, and retirement state.

The native blob remains local to the agent by default.

### Stock and loaded media

Stock is a portable business definition for paper, rolls, labels, or cards.
Loaded-media state associates a stock with one physical device source/tray and
records whether it was device-reported, scanned, operator-confirmed, assumed,
or unknown.

### Target and binding

A target is a stable API destination. Each binding selects a node,
destination, profile revision, and routing priority. One delivery lease selects
one binding; no fan-out occurs.

### Job

- ID and workspace;
- destination printer and agent;
- title/source;
- content descriptor/digest/length;
- requested options;
- `qty` and child submission results;
- expiry;
- idempotency reference;
- per-printer sequence;
- current derived state;
- deepest authority/source;
- retry and ambiguous-handoff policy;
- content-retained flag;
- created/updated/final timestamps.

### Job event

Immutable ordered fact:

```json
{
  "id": "01K...",
  "jobId": "01K...",
  "sequence": 12,
  "type": "spooler.accepted",
  "state": "accepted_by_spooler",
  "observer": "windows_spooler",
  "authority": "os_queue",
  "reasonCode": null,
  "message": "Accepted as native job 41",
  "details": {
    "nativeJobId": 41,
    "driver": "Example Driver"
  },
  "occurredAt": "2026-07-29T00:00:00Z",
  "receivedAt": "2026-07-29T00:00:00.042Z",
  "agentVersion": "0.1.0",
  "traceId": "..."
}
```

## Proposed native endpoints

### Identity and workspaces

- `GET /v1/me`
- `GET /v1/workspaces`
- `GET/PATCH /v1/workspaces/{id}`
- `GET/POST/DELETE /v1/api-keys`
- `GET /v1/audit-events`

### Enrolment and agents

- `POST /v1/enrolment-tokens`
- `POST /v1/agents/enrol` used once by an agent;
- `GET /v1/agents`
- `GET/PATCH/DELETE /v1/agents/{id}`
- `GET /v1/agents/{id}/health`
- `GET /v1/agents/{id}/events`
- `POST /v1/agents/{id}/diagnostics`
- `POST /v1/agents/{id}/update`

### Printers

- `GET /v1/printers`
- `GET/PATCH /v1/printers/{id}`
- `GET /v1/printers/{id}/capabilities`
- `GET /v1/printers/{id}/native-queue`
- `POST /v1/printers/{id}/validate-job`
- `POST /v1/printers/{id}/test-jobs`
- `POST /v1/printers/{id}/pause`
- `POST /v1/printers/{id}/resume`

### Profiles, stocks, and targets

- `GET /v1/destinations/{id}/profiles`
- `POST /v1/destinations/{id}/profile-capture-sessions`
- `POST /v1/profiles/{id}/profile-capture-sessions` for edit/clone capture
- `GET /v1/profiles/{id}`
- `POST /v1/profiles/{id}/validate`
- `POST /v1/profiles/{id}/test-jobs`
- `POST /v1/profiles/{id}/publish`
- `POST /v1/profiles/{id}/retire`
- stock CRUD and loaded-media confirmation;
- target CRUD, bindings, and readiness.

### Jobs

- `POST /v1/jobs` for URI or pre-uploaded content;
- `POST /v1/printers/{id}/jobs:upload` for streaming one-call upload;
- `GET /v1/jobs`
- `GET /v1/jobs/{id}`
- `GET /v1/jobs/{id}/events`
- `POST /v1/jobs/{id}/cancel`
- `POST /v1/jobs/{id}/retry` only when safe;
- `POST /v1/jobs/{id}/resolve-uncertain`
- `POST /v1/jobs/{id}/reprint` creates a linked new job;
- `GET /v1/jobs/{id}/content` only with explicit scope and while retained.

### Events and webhooks

- `GET /v1/events/stream` using SSE for simple dashboards;
- `GET /v1/ws` for multiplexed live subscriptions;
- webhook CRUD;
- delivery attempt listing;
- test and replay operations;
- dead-letter listing/resolution.

### Local-agent additions

The same resource routes exist on loopback where meaningful. Agent-only
endpoints include:

- `/v1/local/status`;
- `/v1/local/config`;
- `/v1/local/logs`;
- `/v1/local/diagnostics`;
- `/v1/local/enrol`;
- `/v1/local/control-plane`.

## Job creation examples

### URI

```http
POST /v1/jobs
Authorization: Bearer opr_live_...
Idempotency-Key: order-481-label
Content-Type: application/json

{
  "printerId": "01KPRINTER...",
  "title": "Order 481",
  "source": "warehouse",
  "content": {
    "kind": "uri",
    "format": "pdf",
    "uri": "https://example.internal/labels/481",
    "authentication": {
      "kind": "basic",
      "username": "job-reader",
      "password": "write-only-secret"
    }
  },
  "options": {
    "paper": {"nativeName": "4x6"},
    "fit": "contain",
    "rotation": 0,
    "copies": 1
  },
  "expiresInSeconds": 86400,
  "uncertainPolicy": "manual"
}
```

### Streamed upload

```http
POST /v1/printers/01KPRINTER.../jobs:upload
Authorization: Bearer opr_live_...
Idempotency-Key: order-481-label
Content-Type: application/pdf
X-Job-Title: Order 481
X-Job-Options: <base64url JSON or use multipart metadata>
Digest: sha-256=:...:

<PDF bytes>
```

For large or browser uploads, provide a create-upload/complete-upload flow with
short-lived object URLs.

## Response

Return after configured durability:

```json
{
  "id": "01KJOB...",
  "state": "registered",
  "printerId": "01KPRINTER...",
  "createdAt": "2026-07-29T00:00:00Z",
  "links": {
    "self": "/v1/jobs/01KJOB...",
    "events": "/v1/jobs/01KJOB.../events"
  }
}
```

HTTP 201 means the system durably registered the job and any uploaded content;
it does not mean agent or spooler acceptance.

## Relational data model

Suggested PostgreSQL tables:

### Identity and tenancy

- `workspaces`
- `users`
- `workspace_members`
- `service_accounts`
- `api_keys`
- `enrolment_tokens`
- `agents`
- `agent_credentials`
- `agent_sessions`

### Printing

- `printers`
- `printer_native_identities`
- `printer_capability_revisions`
- `printer_state_events`
- `physical_devices`
- `printer_device_bindings`
- `printer_profiles`
- `profile_native_metadata`
- `profile_dependencies`
- `stocks`
- `loaded_media`
- `print_targets`
- `target_bindings`
- `jobs`
- `job_submissions` for each `qty` child/native job;
- `job_events`
- `job_idempotency`
- `contents`
- `content_references`

### Delivery

- `routing_outbox`
- `agent_event_receipts`
- `webhooks`
- `webhook_events`
- `webhook_deliveries`

### Administration

- `policies`
- `audit_events`
- `compatibility_ids`
- `usage_ledger`

Important constraints:

- tenant/workspace included in unique and foreign-key paths;
- unique `(workspace_id, api_key_id, idempotency_key_hash)` or documented
  alternative scope;
- unique `(agent_id, boot_id, sequence)` for agent events;
- unique `(job_id, sequence)` for job events;
- unique per-printer sequence;
- content digest/ownership does not permit cross-tenant deduplication that leaks
  whether another tenant owns a document;
- compatibility integer ID is unique by resource type/deployment and never
  reused.

## Agent SQLite model

- `settings` with schema/policy revision;
- `identity` and protected credential references;
- `printers` and `capability_cache`;
- `jobs`;
- `job_submissions`;
- `job_events`;
- `content_files`;
- `server_inbox_receipts`;
- `event_outbox`;
- `migration_history`.

State mutation and its outbox event commit in one transaction. The content file
is atomically committed before the transaction marks it available.

## State projection

Do not overwrite a single state column as the only record. Append the event,
then update a current projection transactionally for efficient list queries.

If projection and event disagree after a crash or bug:

- events are authoritative;
- stop consumption for the affected job if safety is unclear;
- rebuild projection;
- audit the repair;
- never infer that an ambiguous native side effect should be repeated.

## Policy shape

Workspace defaults can be overridden at printer/job scope when authorised:

```json
{
  "uriFetch": {
    "mode": "allowlist",
    "allowedHosts": ["labels.example.internal"],
    "allowPrivateNetworks": true
  },
  "raw": {
    "enabled": true,
    "maxBytes": 1048576,
    "maxQty": 10
  },
  "handoff": {
    "uncertainPolicy": "manual"
  },
  "retention": {
    "contentAfterSpoolerAcceptance": "PT1H",
    "jobEvents": "P90D"
  },
  "updates": {
    "channel": "stable",
    "automatic": true
  }
}
```

The agent receives a signed/versioned effective policy subset. It rejects jobs
that violate local policy even if a compromised or stale control plane offers
them.

## SaaS usage ledger

Piqae Cloud has one published billable event:

- `print_job_accepted`, recorded when a Live job first reaches
  `accepted_by_spooler`.

The event is append-only, linked to the tenant and job, and unique per job.
Test jobs, registration retries, lease retries, and later spooler states do not
add usage. Invoices and quota decisions are derived from this ledger, never
from the mutable current job table. Spooler acceptance remains distinct from
proof that ink reached paper.

## Configuration

Precedence:

1. command-line arguments for explicit process overrides;
2. environment variables for container/server secrets and addresses;
3. TOML configuration file;
4. safe defaults.

Secrets may reference files or platform secret stores. The effective
configuration command redacts secrets and shows each value's source.

The agent configuration should remain short:

```toml
[agent]
name = "Packing station 3"
data_dir = "/var/lib/open-print-relay"

[control_plane]
url = "https://print.example.com"

[local_api]
listen = "127.0.0.1:0"

[logging]
level = "info"
```

Enrolment writes credentials separately; they do not appear in this file.
