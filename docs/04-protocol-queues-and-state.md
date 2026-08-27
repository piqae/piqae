# Protocol, queues, and job state

## The queue answer

A local queue is necessary but not sufficient for remote printing.

- If an application talks directly to a reachable agent, the durable agent
  queue can be the only application-level queue.
- If a remote API accepts a job while an agent is offline, the control plane
  must retain that job somewhere. That is a server-side queue even when it is
  implemented as database rows rather than a dedicated broker.
- After the agent submits a job, the Windows spooler or CUPS is another queue.
- Some printers have internal buffers that behave as a fourth, usually opaque,
  queue.

The legacy service's public behavior confirms a hosted queue: it registers a job, retains
it up to `expireAfter`, sends it to the client later, and only then hands it to
the OS queue.

Our design uses three explicit durable stages:

```text
Control-plane pending
        |
        | agent fetch + durable acknowledgement
        v
Agent inbox / per-printer queue
        |
        | native spool submission
        v
Operating-system spooler
        |
        v
Printer/device (visibility varies)
```

## Delivery guarantees

### Before the OS handoff

The system can provide strong at-least-once delivery with idempotent state
transitions:

- create request is protected by an idempotency key;
- control-plane routing is represented by a durable outbox row;
- agent acceptance is persisted before acknowledgement;
- duplicate notifications refer to the same job and do not create another
  local job;
- content is content-addressed and digest-checked.

### At the OS handoff

There is no transaction shared by SQLite and a Windows/CUPS spooler. Consider:

1. The agent records “submission starting”.
2. It calls the OS.
3. The OS accepts and may print immediately.
4. The machine loses power before the agent records the spooler job ID.

Retrying may duplicate the print. Not retrying may lose it if the OS never
accepted it. No network protocol or database can remove this ambiguity without
cooperation from the spooler/device.

The default policy should be:

- safely retry failures known to occur before native submission;
- persist intent before submission;
- use a deterministic job marker in the OS document title/metadata;
- reconcile the native queue and retained job history after restart;
- if acceptance cannot be proved or disproved, transition to
  `delivery_uncertain`;
- do not automatically retry an uncertain job;
- require an explicit, audited operator resolution; authorizing another output
  creates a separate linked job and never reuses the uncertain attempt.

This is more reliable than silently claiming exactly-once.

## Physical-destination scheduling

An installed OS queue is a route, not necessarily a unique physical printer.
For routes grouped under one tenant-scoped physical destination:

- one fenced reservation crosses the native handoff boundary at a time;
- eligible control-plane work is selected in stable `(created_at, job_id)`
  order across routes;
- different physical destinations may process concurrently;
- a configurable concurrency limiter bounds total PDF rendering;
- RAW `qty` submissions are serialized and individually tracked;
- a paused application queue does not pause or mutate the OS queue unless the
  operator asks separately.

The durable node still allocates a route-local acceptance sequence. A shared
installation coordinator serializes handoff from several tenant connectors
without exposing their job data. It cannot create one global FIFO or automatic
failover ledger across independent hosted and self-hosted control planes. Jobs
submitted directly to the OS spooler are visible only as privacy-safe occupancy
and remain outside Piqae ordering and idempotency.

## Native job state model

The internal model is richer than the legacy service's stable states.

| State | Owner | Meaning |
| --- | --- | --- |
| `registered` | control plane or local API | Metadata and idempotency record are durable. |
| `content_pending` | control plane/agent | Content is not yet durably available at the next stage. |
| `waiting_for_agent` | control plane | Target agent is offline, back-pressured, or has not fetched. |
| `offered_to_agent` | control plane | Availability notification was sent; not yet a durability boundary. |
| `agent_downloading` | agent | Content transfer is in progress. |
| `agent_accepted` | agent | Metadata and content are durable locally. The control plane may apply its content-retention policy. |
| `queued_local` | agent | Waiting in tenant-scoped physical-destination order across eligible routes; the installation coordinator also serializes local handoffs that share one physical printer. |
| `preparing` | agent | Validating options, pages, and content. |
| `rendering` | renderer | PDF is being transformed for native submission. |
| `submitting_to_spooler` | agent | Native handoff has begun; crash recovery may be ambiguous. |
| `accepted_by_spooler` | OS adapter | OS returned a job identifier/positive acceptance. |
| `spooling` | OS | OS reports that bytes/pages are being spooled. |
| `printing` | OS/device | OS or device reports active printing. |
| `blocked` | OS/device | Paper, toner, offline, paused, jam, authentication, or intervention condition. |
| `completed_reported` | OS/device | Deepest available source reports completion; source is recorded. |
| `delivery_uncertain` | reconciler | Handoff outcome or physical completion cannot be determined. |
| `cancel_requested` | any | Cancellation was requested but is not yet confirmed. |
| `cancelled` | observing owner | Cancellation is confirmed at the deepest controlled stage. |
| `expired` | control plane/agent | Policy expiry elapsed before native submission. |
| `failed` | any | A terminal, classified failure occurred before successful handoff/completion. |

States are append-only events. A current-state projection is derived from the
event stream and may be rebuilt.

Every observed state records:

- monotonic per-job sequence;
- UTC wall time and local monotonic elapsed time;
- observer (`server`, `agent`, `renderer`, `windows_spooler`, `cups`,
  `ipp_device`, `snmp_device`);
- authority level;
- stable reason code;
- human message;
- structured details;
- agent, adapter, and renderer versions;
- trace/span identifiers.

## Compatibility state projection

| Native event | legacy-compatible state |
| --- | --- |
| `registered` | `new` |
| first job metadata delivery to a connected agent | `sent_to_client` |
| `accepted_by_spooler` | `done` |
| terminal pre-handoff `failed` | `error` |
| `expired` before client delivery | `expired` |

Do not project `completed_reported` as a second `done`; the legacy service's `done`
semantics end at OS-queue acceptance. Richer states are available only through
the native API and extension events.

## Cancellation

Cancellation is a race and its result must identify the boundary:

- `registered` through `queued_local`: system can normally cancel
  authoritatively;
- `rendering`: terminate the renderer, delete unneeded output, then cancel;
- `submitting_to_spooler`: mark requested and reconcile; outcome may be
  uncertain;
- after OS acceptance: call native cancel and observe, but the device may
  already hold the job;
- after device transmission: cancellation is best effort only.

Compatibility `DELETE /printjobs` returns only jobs cancelled before client
delivery, matching documented legacy-provider behavior.

## Retry classification

Errors are classified, not matched from human strings:

| Class | Examples | Default |
| --- | --- | --- |
| `transient_network` | timeout, connection reset, control plane restart | exponential retry |
| `transient_resource` | renderer busy, low temporary capacity | bounded retry |
| `printer_blocked` | offline, paper out, paused, jam | wait for state change; do not consume retry budget rapidly |
| `invalid_request` | bad page range, unsupported content type | terminal |
| `unsupported_option` | tray/paper/DPI no longer available | terminal unless policy allows fallback |
| `content_unavailable` | URI 404/401, digest mismatch | bounded policy retry |
| `renderer_failure` | malformed PDF, password protected, timeout | terminal or one clean-process retry |
| `native_rejected` | driver/spooler rejected job | terminal with native error |
| `ambiguous_handoff` | crash/timeout during native call | no automatic retry by default |

Retry timestamps and reasons are visible to the user.

## Wire protocol

### Envelope

All agent WebSocket messages use a small versioned envelope:

```json
{
  "protocol": 1,
  "type": "job.available",
  "messageId": "01K...",
  "sentAt": "2026-07-29T00:00:00Z",
  "sessionId": "01K...",
  "correlationId": "01K...",
  "body": {}
}
```

Requirements:

- explicit protocol and message schema versions;
- unique message IDs;
- maximum frame size;
- sequence numbers for resumable agent event upload;
- unknown optional fields ignored;
- unknown required features rejected during negotiation;
- no executable commands or arbitrary shell payloads;
- bounded decompression and parsing.

Use JSON during development for inspectability. If profiling shows it matters,
add a negotiated MessagePack or Protobuf encoding later without changing
message semantics.

### Session startup

1. TLS and device authentication.
2. `hello` with installation ID, boot ID, agent version, OS, architecture,
   supported protocol range, content modes, renderer versions, and modules.
3. `welcome` with selected version, server time, policy revision, heartbeat
   interval, upload cursor, and pending work hint.
4. Agent resumes event outbox from the acknowledged cursor.
5. Agent sends inventory digest; server requests a full or delta inventory.
6. Server notifies pending jobs.

### Heartbeats and connection state

WebSocket ping/pong alone does not prove the agent event loop and database are
healthy. Application heartbeats include:

- monotonic uptime;
- queue depth by state;
- disk free/quota;
- current jobs;
- printer watcher health;
- renderer health;
- last successful control-plane and spooler operations;
- clock offset estimate.

The control plane marks a session stale after missed intervals, then
disconnected. Presence is a set of sessions, allowing controlled overlap during
upgrades without treating both sessions as independent consumers.

### Job offer and claim

Notifications are hints, not ownership:

1. Server sends `job.available` with job ID and metadata revision.
2. Agent calls/requests `job.claim` with its local capacity and current
   revision.
3. Server returns a renewable claim lease plus content descriptor.
4. Agent fetches content and persists the job.
5. Agent sends `job.accepted` with digest and local sequence.
6. Server records agent acceptance and releases/downgrades its delivery claim.

If any message is duplicated, stable IDs make processing idempotent. If a lease
expires before `agent_accepted`, another valid session for the same installation
may claim it. Only one installation identity may own a printer at a time.

The V1 polling transport implements this as a durable two-phase local handoff.
During content materialisation and acceptance work, the agent renews the lease
at most every ten seconds with a five-second renewal-request timeout. It first
stores a `cloud_accept_pending` job and the exact lease/digest/local-sequence
accept intent in `SQLite`; this state is not runnable and emits no queue event.
An ambiguous response or restart retries that exact intent against the server's
idempotent acceptance record. Only a confirmed response atomically changes the
job to `queued_local`, emits its first outbox event, and deletes the persisted
lease capability. A server cancellation or policy expiry terminalises a
prepared job and deletes the capability without inventing a transient-failure
cancellation.

The control plane expires pre-acceptance work in bounded oldest-first batches.
Expiry commits the `expired` lifecycle event, current-state payload, tenant
outbox item, lease removal, capability-recovery removal, and any still
`route_leased` attempt/reservation in one transaction. A durable node
acceptance or a delivery attempt beyond `route_leased` is an ownership fence:
the server leaves that job unchanged because the node may still print it.
Unsafe rows are excluded before applying the batch limit so they cannot starve
later safe expiries. Embedded hosts apply the same local expiry transition to
each isolated connector queue on restart and before exposing adapter work.

### Agent event outbox

The agent writes events and local state changes in the same SQLite transaction.
A sender batches unacknowledged events in order. The control plane stores them
idempotently using `(agent_id, boot_id, sequence)` and acknowledges the highest
contiguous sequence. Events survive process and network failure.

The server uses the same transactional-outbox pattern for webhooks and
connected dashboard updates.

## Content lifecycle

Suggested default:

1. Control plane stores uploaded content before acknowledging registration.
2. Agent downloads and commits it.
3. Agent acknowledges digest.
4. Control plane may delete bytes after `accepted_by_spooler`, while retaining
   metadata/events for the configured history period.
5. Agent deletes bytes after the deepest final state plus a short diagnostic
   grace period.

Policies may retain content longer for reprint or delete it immediately after
handoff. Metadata must clearly show whether content still exists. URI content
may expire independently, so the agent should fetch promptly after claim.

## Backpressure

The agent advertises:

- maximum concurrent downloads;
- maximum pending bytes;
- maximum renderers;
- per-printer queue limit;
- low-disk state.

The control plane does not offer more than the agent can durably accept. Local
submissions reject with a useful capacity error or block only when the caller
explicitly requests a streaming backpressure mode.

## Local-only protocol

The loopback native API reuses the same domain commands and state events:

- streamed multipart/binary upload;
- optional legacy compatibility routes;
- SSE/WebSocket event feed;
- bearer token or OS-user access controls;
- Unix domain socket/named pipe option for local privileged integrations.

When an agent is later enrolled, existing local job history remains local by
default. Uploading historical metadata requires explicit policy.
