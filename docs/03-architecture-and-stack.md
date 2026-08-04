# Proposed architecture and technology choices

## Recommendation

Use a Rust workspace for the agent, shared protocol/model crates, the first
control-plane binary, and CLI tools. Use SvelteKit/TypeScript for local and
hosted web interfaces.

This is the best fit for a low-memory always-on agent, strong state-machine
types, native Windows/CUPS integration, static or self-contained distribution,
and shared code across platforms.

Go is a credible alternative for the control plane and would probably shorten
initial server development. It is less compelling as the only agent language
because the most difficult work is native printing, PDF rendering FFI, and
precise lifecycle/resource control. Maintaining Rust for the agent and Go for
the server would be reasonable for a larger team, but it adds two build,
dependency, observability, and security ecosystems before product-market need
justifies them.

## Why not separate full native applications?

Windows and CUPS require different print adapters, but they do not require
different queue, network, security, configuration, or observability products.
Three separate clients would duplicate the most reliability-sensitive code.

Build:

- one headless `relay-agent` core;
- `platform-windows` and `platform-cups` adapter crates;
- an optional macOS adapter for framework-specific rendering while still using
  CUPS for queue observation;
- small installer/service definitions per OS;
- v0.1 native tray/menu shells that show health and open diagnostics/dashboard
  controls, while remaining independent of the daemon and queue.

## High-level component view

```text
Application / SDK
        |
        | REST (legacy-compatible or native)
        v
+--------------------------- optional ----------------------------+
| Control plane                                                   |
| API + auth + job/event store + routing + webhooks + Svelte UI  |
| PostgreSQL                 filesystem or S3 object storage      |
+------------------------------+----------------------------------+
                               |
                 metadata/status: outbound WSS
                 content: resumable HTTPS
                               |
                         +-----v------+
                         | Rust agent |
                         | SQLite/WAL |
                         +-----+------+
                               |
                     platform print adapter
                               |
                +--------------+--------------+
                | Windows spooler or CUPS/IPP |
                +--------------+--------------+
                               |
                  installed local driver/device
```

In local-only mode, the application calls the agent directly and the entire
control-plane box is absent.

## Suggested repository layout

```text
/
  Cargo.toml
  crates/
    agent-core/
    agent-daemon/
    agent-local-api/
    control-plane/
    protocol/
    domain/
    storage-sqlite/
    storage-postgres/
    object-store/
    platform-windows/
    platform-cups/
    renderer-pdfium/
    webhook-worker/
    test-support/
  apps/
    web/
    docs/
  installers/
    windows/
    macos/
    linux/
  compatibility/
    openapi/
    fixtures/
    differential-tests/
  docs/
```

Keep domain state transitions independent of Axum, SQLx, Windows, CUPS, or the
UI so they can be exhaustively tested.

## Agent stack

Proposed foundations:

- Tokio for bounded asynchronous tasks;
- Rustls for TLS;
- Axum for the local HTTP API and embedded static UI;
- SQLite in WAL mode for durable job metadata, receipts, and the event outbox;
- SQLx or a narrow internal storage layer with explicit migrations;
- `tracing` plus OpenTelemetry export;
- Serde with versioned wire envelopes;
- platform FFI through tightly contained unsafe modules;
- PDFium as the initial cross-platform PDF renderer where native submission is
  insufficient.

PDFium's BSD-style licence is compatible with an open-source agent. It should
run in a separate, resource-constrained child process because PDF is untrusted
input and rendering can consume substantial memory. Load it only while needed,
then terminate an idle renderer to preserve agent memory.

Do not invoke general shell commands for normal printing if stable native APIs
are available. Command-line CUPS tools are useful for diagnostics and early
spikes, but native IPP/CUPS calls provide better error and job-ID handling.

## Control-plane stack

Start as a modular monolith:

- Axum REST and WebSocket server;
- PostgreSQL for tenants, identities, jobs, events, idempotency, routing
  outbox, and webhook outbox;
- local filesystem object storage for the smallest single-node deployment;
- S3-compatible storage for SaaS or horizontally scaled deployment;
- SvelteKit static dashboard served by the control-plane binary or a CDN;
- OpenTelemetry logs, metrics, and traces;
- an OpenAPI description for native and compatibility APIs.

Avoid an external broker initially. PostgreSQL transactional outbox rows make
job registration, routing intent, and event publication atomic. Connected
agents are notified through an in-process session registry. In a multi-instance
control plane, PostgreSQL `LISTEN/NOTIFY` can wake the node holding the session;
the durable outbox remains the source of truth. Add NATS or another broker only
after measured connection or fan-out load demonstrates a need.

## Storage modes

### Agent

Always SQLite plus content files:

- metadata and state in SQLite;
- content in content-addressed files;
- atomic download to a temporary name, `fsync`, digest validation, then rename;
- reference counts and retention cleanup;
- filesystem quota and minimum-free-space guard;
- optional platform-backed encryption key.

Storing large PDFs as SQLite blobs complicates streaming and vacuum behavior.

### Single-node self-host

Recommended production default: PostgreSQL plus local content directory on
durable storage. This remains operationally understandable and supports
backups.

An all-in-one SQLite control plane could be offered later as a development or
home deployment, but supporting two server database semantics from day one
would enlarge the correctness matrix. Do not call an SQLite control plane HA.

### SaaS

PostgreSQL plus S3-compatible object storage. Documents have per-object
envelope encryption metadata, short retention, tenant ownership, and lifecycle
cleanup. The database never stores raw API keys or document bytes.

## Connection and data flow

### Agent session

The agent initiates one TLS WebSocket connection on port 443. The session:

- authenticates the enrolled device;
- negotiates protocol versions and capabilities;
- announces boot/session ID and health;
- publishes printer inventory deltas;
- receives lightweight job availability notifications;
- acknowledges durable local acceptance;
- streams status and event batches;
- exchanges heartbeats.

The WebSocket does not carry large PDFs. Avoiding large frames prevents
head-of-line blocking of status and cancellation messages.

### Content transfer

For uploaded content:

1. The control plane returns a short-lived, job-bound download URL.
2. The agent performs a streaming HTTPS GET with range/resume support.
3. The agent writes to a temporary file while calculating SHA-256.
4. It verifies length and digest, then atomically commits local content.
5. It sends `agent_accepted` only after metadata and content are durable.

For URI mode, policy decides whether:

- the agent downloads directly, preserving legacy-style data locality; or
- the control plane fetches and scans content, then serves it to the agent.

Direct fetch is the compatibility default but should have an administrator
allow-list or explicit unrestricted mode.

## Identity model

Use separate identifiers for:

- installation: generated once and stored locally;
- device public key;
- boot/session: changes on restart/reconnect;
- computer: API-visible logical agent;
- OS printer queue: adapter identity;
- logical printer: survives some queue renames/reinstalls;
- job: globally unique sortable ID;
- compatibility ID: positive integer mapped to the native ID;
- OS spooler job: platform-specific and reusable over time, so it is scoped to
  printer and observation window.

The agent generates an Ed25519 keypair on first enrolment. A one-time token
authorises certificate issuance or device registration. Subsequent sessions
use mTLS or proof-of-possession tokens. Copying the database to another machine
must not silently activate the same identity; bind identity to protected
platform storage where possible and detect concurrent sessions.

## UI architecture

Use SvelteKit with TypeScript:

- a shared component and API-client package;
- an `adapter-static` build for the loopback agent UI;
- the same core views in the control-plane dashboard;
- server rendering only where the SaaS marketing/auth experience benefits;
- SSE or WebSocket for live state;
- no Electron;
- no dependency on the UI for service operation.

The local agent embeds hashed static assets at build time. Opening the UI uses
the system browser. A v0.1 tray/menu shell shows health, job/printer warning
counts, and opens the page, but never owns the agent or queue.

## Packaging

### Windows

- signed MSI with per-machine install;
- Windows Service under a dedicated least-privilege virtual service account by
  default, not `LocalSystem` unless a driver requires it;
- documented option to run under a domain/user account for network printers;
- recovery actions and delayed automatic start;
- Event Log integration plus local structured files;
- native notification-area shell installed by default with a headless opt-out;
- x64 first, ARM64 after printing validation.

### macOS

- signed and notarised `.pkg`;
- universal binary where feasible;
- `launchd` daemon;
- local UI/CLI rather than a mandatory menu-bar application;
- explicit entitlements and permissions;
- rollback-aware updater.

### Linux

- `.deb`, `.rpm`, and tarball;
- x86_64 and aarch64 first, armv7 if still required;
- `systemd` service and hardened unit options;
- a dedicated service user with CUPS access;
- optional Docker image for server/headless scenarios, with clear CUPS socket
  and device-mapping documentation.

## Licence

All Piqae-authored source, schemas, documentation, and examples use
Apache-2.0. The managed service competes on operations and support rather than
withholding core functionality. Trademark rights remain separate. See
[12-open-source-saas-and-build-plan.md](12-open-source-saas-and-build-plan.md)
for the full operating model.
