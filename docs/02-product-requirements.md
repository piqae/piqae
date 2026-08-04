# Product requirements and user experience

## Personas

### Application developer

Wants a short, stable API contract and should not need printing expertise.
Needs idempotency, test printers, SDKs, webhooks, predictable errors, and a
clear migration path from the legacy service.

### Site installer

Installs the agent at a shop, warehouse, office, or production line. Needs
silent installation, proxy support, a one-time enrolment token, printer tests,
and obvious local health.

### Site operator

Needs to see whether the machine, printer, queue, or document is at fault.
Should be able to retry safe failures, cancel jobs that have not crossed the OS
handoff, and export a diagnostic bundle.

### Self-hosted administrator

Wants a small deployment with backups, health checks, documented upgrades,
retention controls, and no mandatory third-party service.

### SaaS customer

Wants a Stripe-like onboarding flow, tenant isolation, API keys, usage data,
invoices, audit history, and no infrastructure work.

## Deployment experiences

### Local-only

1. Install the agent.
2. The agent enumerates OS-installed printers.
3. Submit to `http://127.0.0.1:{port}` or a protected LAN listener.
4. Receive a durable local job ID.
5. Query or subscribe to local events.

The local listener is loopback-only by default. LAN exposure requires explicit
configuration, TLS, and an API token.

Local-only mode must work indefinitely without a control-plane account. It can
later be enrolled without reinstalling.

### Self-hosted

1. Deploy the control plane with PostgreSQL and document storage.
2. Create a workspace and one-time enrolment token.
3. Install the agent with the control-plane URL and token.
4. The agent generates its own device key, enrols, and opens an outbound TLS
   connection.
5. Printers appear in the API and optional UI.
6. Submit jobs using either the native API or legacy compatibility API.

The customer's firewall should need outbound TCP 443 from the agent only.

### SaaS

The desired first-run flow is:

1. Create an account/workspace.
2. Copy an API key.
3. Download an installer whose short-lived enrolment token is already
   embedded, or run one command.
4. Watch the enrolled computer and printers appear live.
5. Copy a three-line API example and print a test page.
6. Add a webhook.

The first successful print should be achievable in under ten minutes by a
developer unfamiliar with print infrastructure.

## Functional requirements

### Agent lifecycle

The agent shall:

- run without an interactive desktop session;
- install as a Windows Service, macOS `launchd` daemon, and Linux `systemd`
  unit;
- support foreground mode for development and containers;
- use a stable machine/installation ID generated on first run;
- prevent two active processes from consuming the same local database;
- prevent cloned configuration from creating two agents with the same identity;
- enrol using a single-use, short-lived token;
- reconnect with exponential backoff and jitter;
- survive server restarts and network changes;
- support HTTP CONNECT proxies and custom certificate authorities;
- expose health through CLI, local API, and optional loopback UI;
- support signed, staged, rollback-capable updates;
- never auto-update while it is in the ambiguous part of an OS handoff.

### Printer discovery and capabilities

The agent shall:

- enumerate installed print queues, not just physical USB devices;
- detect additions, removals, renames, default changes, and relevant option
  changes;
- expose raw native identity plus a stable logical printer identity;
- obtain capabilities without blocking the main event loop;
- tolerate slow, offline, or broken network drivers;
- preserve native option names for exact driver selection;
- project capabilities into the legacy compatibility shape;
- maintain a richer native schema for vendor-specific or future features;
- distinguish queue state from physical-device state;
- not scan the LAN unless an administrator enables an optional discovery
  feature.

Renaming an OS printer may look like removal plus addition on some platforms.
The system should attempt identity reconciliation using driver, port, device
URI, and installation identifiers, while recording uncertainty.

### Print submission

The system shall accept:

- PDF bytes;
- PDF URIs;
- RAW bytes;
- RAW URIs;
- legacy-compatible base64 request bodies;
- native streaming uploads without base64 overhead.

It shall support:

- title and source metadata;
- expiry;
- driver copies and repeated submissions (`qty`);
- the documented legacy printing options;
- SHA-256 content integrity;
- idempotency keys;
- configurable maximum size, page count, and render time;
- optional content encryption at rest;
- cancel-before-handoff;
- explicit retry policy;
- per-printer ordering.

URI retrieval shall support compatible Basic and Digest authentication, but
must include policy controls because unrestricted agent-side URI retrieval can
be used for server-side request forgery against the customer's LAN.

### Queue inspection and control

Users shall be able to see:

- control-plane jobs waiting for an agent;
- jobs durably accepted by an agent;
- current agent worker activity;
- the OS spooler job ID and state when available;
- printer error/reason data;
- the last event and full event timeline;
- expiry, retry count, and next retry time;
- content retention state;
- whether a final status is authoritative, inferred, or unknown.

Safe controls:

- cancel before OS handoff;
- request OS cancellation after handoff, without claiming guaranteed success;
- pause/resume an application-level printer queue;
- retry only when the state is known to be pre-handoff;
- require a conscious choice for `delivery_uncertain`;
- reprint as a new job linked to the original.

### Events and webhooks

The native event system shall publish:

- agent connected/disconnected/health changed;
- printer added/removed/capabilities changed/state changed;
- job state and progress changes;
- OS spooler state changes;
- security and enrolment events;
- scale device and measurement events when that module is enabled.

Consumers may use:

- WebSocket or Server-Sent Events for dashboard/browser live updates;
- durable webhooks for application integration;
- REST history queries.

Webhooks need HMAC signatures, unique event IDs, ordered sequence numbers within
a job or aggregate, exponential retry, dead-letter visibility, and a replay
operation. Compatibility mode also emits the legacy service's documented event shapes.

### Logs and diagnostic bundles

Every job shall carry:

- external request ID;
- tenant/workspace ID;
- job ID;
- idempotency key hash, never the raw secret if sensitive;
- agent ID;
- printer ID;
- connection/session ID;
- content digest;
- renderer attempt;
- OS spooler job ID;
- trace ID.

A local support bundle shall include configuration with secrets removed,
versions, recent structured logs, platform information, printer and capability
snapshots, spooler state, crash reports, and event timelines. Document content
is excluded unless the operator explicitly opts in.

### Scale parity

When implemented, the scale module shall:

- support USB HID scales through a cross-platform HID layer;
- support configurable serial devices and model parsers;
- normalise mass and resolution without losing raw readings;
- identify devices by stable hardware path where the OS permits it;
- stream readings over the same agent channel;
- expose legacy-compatible HTTP and WebSocket projections;
- include a virtual test scale;
- allow the whole module to be disabled.

## UI requirements

### Local UI

The default agent has no embedded desktop window. It serves a static Svelte
application on a random or configured loopback port and can open it in the
system browser. The UI includes:

- connection and enrolment status;
- printers and capabilities;
- local queue and recent job events;
- test printing;
- logs and diagnostic bundle export;
- proxy, update channel, retention, and printer policy configuration.

The local UI remains optional at build/install time. A CLI must cover all
administrative operations.

### Control-plane UI

A SvelteKit application, compiled as static assets where practical, provides:

- onboarding and enrolment;
- computer, printer, and queue views;
- job creation/test printing;
- job/event search;
- API keys and webhook management;
- user and workspace management;
- retention/security policy;
- update/fleet health;
- SaaS usage and billing when hosted.

The UI uses only the public native API. It does not receive privileged direct
database access or special undocumented endpoints.

## Non-functional targets

These are proposed release gates, not current measurements.

### Agent resource targets

- Idle RSS: target <= 25 MB on Windows/macOS and <= 15 MB on headless Linux,
  excluding an actively loaded PDF renderer.
- Idle CPU: effectively zero, with event-driven watchers and a bounded health
  interval.
- Core installed size: target <= 30 MB; full package with PDF renderer target
  <= 70 MB.
- No Java, Electron, or bundled Chromium runtime.
- Content and history disk usage bounded by configurable quotas.

### Latency targets

- API registration p95 <= 50 ms on a healthy self-hosted control plane,
  excluding content upload.
- Connected-agent command notification p95 <= 100 ms in-region.
- Agent acknowledgement p95 <= 250 ms after metadata arrival, excluding
  content transfer and fsync variance.
- Live state visible in the UI p95 <= 500 ms after local observation.

### Durability targets

- No loss after a create response at the configured durable acknowledgement
  level.
- Idempotent resubmission during client timeouts.
- Per-printer FIFO for sequentially acknowledged submissions.
- Explicit state after power loss at every state-machine transition.
- No automatic duplicate output in ambiguous handoff tests.

### Availability targets

- Local printing works while the control plane is unavailable.
- Already delivered agent jobs continue through the local queue.
- Control-plane maintenance does not require agent re-enrolment.
- SaaS target may begin at 99.9%; stronger claims require measured operations,
  multi-zone storage, and rehearsed recovery.

## Out of scope for the first production release

- mobile agents;
- a general-purpose virtual printer driver;
- arbitrary HTML, Office document, or image rendering;
- print accounting and secure-release badge workflows;
- full printer fleet consumables management;
- automatic internet exposure of a local agent;
- direct replacement of every third-party legacy-provider integration;
- physical proof of paper output where hardware does not report it.
