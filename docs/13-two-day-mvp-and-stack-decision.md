# Superseded Go-first evaluation

Status: superseded; retained as decision history  
Decision date: 29 July 2026

This document optimized for the fastest small-team implementation and selected
Go. The objective was subsequently clarified as the best long-term native
endpoint architecture, with parallel AI implementation making initial code
speed less important. The accepted decision is now Rust and the target is a
production-operated v0.1 in 48 hours, not a disposable proof.

Do not implement from this document. See
[14-long-term-native-architecture-and-48-hour-production.md](14-long-term-native-architecture-and-48-hour-production.md).

## Executive decision

Use:

- **Go for the local print agent**;
- **SvelteKit and TypeScript for the dashboard and first HTTP control plane**;
- **Vercel for the web application and request/response API**;
- **WorkOS AuthKit for human authentication and WorkOS Organizations for SaaS
  workspace membership**;
- **Neon PostgreSQL for the durable server-side queue**;
- **private Vercel Blob initially for immutable print documents**;
- **HTTPS polling with leases for the first agent protocol**;
- **SumatraPDF as a replaceable Windows PDF backend for the prototype**;
- **CUPS commands/APIs on macOS and Linux**;
- **the native Windows spooler for RAW printing and status when that capability
  enters scope**.

Do not use Rust for the first agent, build a separate native desktop UI, create
a microservice fleet, or introduce a message broker in the first iteration.
Do not rewrite the agent in Rust later unless profiling, security isolation, or
a proven native-integration problem justifies it.

This is not a claim that Go is universally better than Rust. It is the better
decision under the actual constraint: demonstrate a real, restart-tolerant
remote print in days, while retaining code that can become the production
agent.

## What is possible in two days

A credible 48-hour result is one narrow vertical slice:

1. a person signs in;
2. a Windows machine enrolls an agent;
3. the agent reports its installed printers;
4. the person uploads a PDF and chooses a printer;
5. the server durably records the job;
6. the agent claims it, downloads and verifies the document, and invokes the
   local print path;
7. the dashboard shows timestamped states and an actionable error when the
   handoff fails;
8. restarting the agent does not silently submit a known completed job again.

It is not realistic in two days to deliver legacy-provider feature parity,
cross-platform installers, silent auto-update, physical-paper proof, complete
driver option discovery, robust offline printing, scale support, public
webhooks, or a supported self-hosted distribution. Calling the narrow slice an
MVP rather than a prototype does not make those risks disappear.

The right demonstration target is:

- Windows 10/11;
- one known physical or virtual printer;
- one known PDF fixture;
- foreground agent process;
- PDF printing only;
- copies and fit-to-page only if the chosen backend proves them;
- dashboard polling every one or two seconds;
- explicit `accepted_by_spooler` rather than a false `printed` state.

If the organisation's immediate replacement path is RAW label printing rather
than PDF, swap PDF for RAW and prove byte-exact Winspool submission first. Do
not attempt both unless the first path is already working.

## Go versus Rust

### Decision matrix

| Criterion for this product | Go | Rust |
| --- | --- | --- |
| First working agent in 48 hours | Strong | Moderate |
| Simple HTTP, JSON, hashing, process control | Strong | Strong |
| Concurrency understandable to a small team | Strong | Strong but more concepts |
| Single native binary | Yes | Yes |
| Cross-compilation and CI | Usually simple | Good, but native dependencies complicate it |
| Windows API access | Good through `x/sys/windows` | Excellent through `windows-rs` |
| C/C++ renderer FFI | Adequate | Strongest |
| Memory footprint | Low enough for this use | Usually lower and more predictable |
| Compile-time memory safety | Good, garbage collected | Strongest |
| Iteration/onboarding speed | Strongest | Slower without an experienced Rust team |
| Risk that dominates this project | Not the language | Not the language |

The dominant risks are driver behavior, PDF fidelity, spooler ambiguity,
crash-safe job handoff, signing and updates. Saving a few megabytes or
microseconds in the network loop does not solve any of them.

Go provides the useful properties of a native daemon without making ownership,
lifetimes, FFI design, and long compile cycles part of the first proof. Its
garbage collector is not a material problem for an agent that is mainly waiting
on HTTP, files, subprocesses, and blocking print APIs. Put explicit resource
budgets in CI rather than choosing a language from intuition:

- idle resident memory target: under 40 MB for the first alpha;
- installed agent excluding optional renderer: under 30 MB;
- no unbounded goroutine creation;
- no print document held fully in memory;
- printer enumeration and print calls have deadlines or isolated helpers;
- agent heartbeat/status write remains responsive during a stuck driver call.

Rust is still useful when a small, security-sensitive component warrants it.
Candidates include a sandboxed renderer wrapper or a Windows helper that must
contain unsafe native calls. Those should be separate process boundaries with a
small protocol; the main agent need not be rewritten.

### Why not a Node or Tauri agent

A Node agent wrapping `pdf-to-printer` may produce the fastest disposable demo,
especially on Windows. It also adds a runtime, larger packaging surface, and a
likely rewrite before service-grade deployment. Use it only if the objective is
a throwaway one-day hardware experiment.

Tauri is a desktop shell, not the queue owner. The production product is a
headless service. A tray application or loopback Svelte UI can be added later
without giving it responsibility for printing, persistence, or connectivity.

## The first architecture

```text
Browser
  |
  | WorkOS session
  v
SvelteKit on Vercel
  |-- dashboard pages
  |-- enrollment and job API
  |-- short dashboard polling
  |
  +--> Neon PostgreSQL
  |      agents, printers, jobs, job_events, leases
  |
  +--> private Vercel Blob
         immutable PDF keyed by content digest

Go agent on Windows
  |-- HTTPS poll/claim/status
  |-- stream document + SHA-256 verification
  |-- local receipt store
  +-- SumatraPDF CLI -> installed Windows driver -> spooler
```

This architecture deliberately avoids a permanent server-side connection.
Vercel supports WebSockets, but a connection remains tied to a function's
maximum duration and durable coordination still needs external state. Polling
has less impressive diagrams but much better failure behavior for the first
release:

- the database, not a process connection, owns the job;
- an agent can reconnect to any function instance;
- an interrupted request loses no authoritative state;
- no connection registry, sticky routing, or Redis is required;
- latency is bounded and easily measured;
- self-hosted deployments can implement the same API.

Start with a one-second poll while an agent is online, exponential idle backoff
to at most five seconds, server-supplied `Retry-After`, jitter, and an immediate
poll after any completed operation. Long-polling can reduce empty requests once
the behavior is stable. A dedicated Go gateway is an optimization for later,
not a prerequisite for correctness.

### Durable claim protocol

The claim operation must be one database transaction:

1. find the oldest eligible job for the agent using row locking with
   `SKIP LOCKED`;
2. set `claimed_by_agent_id`, a random `lease_token`, and `lease_expires_at`;
3. append a `claimed` event;
4. return the job, document metadata, and lease token.

Every status update includes:

- job ID;
- lease token;
- monotonically increasing agent event sequence;
- agent timestamp and server receive timestamp;
- state;
- stable error code;
- safe diagnostic context.

The API accepts an already-recorded `(job_id, event_sequence)` idempotently.
Only the holder of the live lease can advance the job. A lease expiry before
spool submission allows reassignment. An expiry during or after submission
creates `handoff_uncertain`; it must not automatically print again.

This is the minimum reliable state path:

```text
queued_server
  -> claimed
  -> downloading
  -> ready_local
  -> submitting_to_spooler
  -> accepted_by_spooler
  -> completed_reported | handoff_uncertain | failed
```

The MVP can omit `completed_reported` if the backend cannot retrieve a durable
spooler job identifier. It must never rename process exit success as physical
print success.

## Authentication boundaries

### Human authentication

Use WorkOS AuthKit's hosted authentication UI through its SvelteKit package.
This gives the fastest path to password/social login and leaves a path to MFA
and Enterprise SSO. Map a WorkOS Organization to a local workspace, but retain
the application's own `workspaces` row and immutable internal ID.

WorkOS should provide:

- human identity and sessions;
- organization selection and memberships;
- invitations;
- SaaS SSO/MFA later.

The application database should provide:

- workspace ownership of printers, agents, jobs, keys, and documents;
- product roles if more detail than WorkOS membership is needed;
- audit records;
- usage and billing data.

### Agent and API authentication

Do not log a print agent in as a WorkOS user. Machine identity must remain
first-party and function in self-hosted or disconnected deployments.

Use:

1. a short-lived, single-use enrollment token created by an authorized human;
2. the agent exchanges it for an agent ID and random device secret;
3. store only a keyed hash of the secret on the server;
4. the agent stores the secret using Windows DPAPI initially and the platform
   keychain later;
5. rotate and revoke device credentials independently of human sessions.

Developer API keys are also first-party workspace credentials with a visible
prefix, one-time secret display, hashed storage, scopes, last-used timestamp,
and revocation. WorkOS sessions authorize creating them but are not sent to
agents or customer servers.

### Preserving self-hosting

A cloud build may use WorkOS without making the core server depend on WorkOS
types. Define an application-level identity boundary:

```text
HumanPrincipal {
  subject_id
  workspace_id
  roles[]
}
```

The SaaS adapter produces it from WorkOS. A self-hosted edition can produce it
from generic OIDC, a local bootstrap administrator, or API-key-only operation.
Agent enrollment and print delivery are identical in every edition.

This prevents "open source" from meaning "source available but unusable without
our auth tenant."

## Reuse instead of rebuilding

### Use immediately

| Project/service | Use | Boundary |
| --- | --- | --- |
| Go standard library | HTTP client, JSON, TLS, hashing, files, subprocesses | Keep agent framework-free |
| `golang.org/x/sys/windows` | Win32 bindings | Implement only the Winspool calls required |
| SumatraPDF | Windows PDF render/print executable | Replaceable backend; review AGPL/GPL distribution duties |
| OpenPrinting CUPS | macOS/Linux printer and job path | Begin with `lpstat`, `lp`, and documented CUPS behavior |
| SvelteKit | web UI and first API | Use `@sveltejs/adapter-vercel` |
| WorkOS AuthKit | hosted human auth | Never make it agent auth |
| PostgreSQL/Neon | durable queue and event log | Source of truth, not browser state |
| Vercel Blob | immutable private PDF objects | Keep a storage interface for S3/filesystem later |

For the prototype, invoke SumatraPDF directly with `-print-to`,
`-print-settings`, and `-silent`, capture its exit code and bounded output, and
give each invocation a private temporary directory. Sumatra's successful exit
only establishes that its submission path returned successfully. Retrieve the
spooler job when possible before claiming `accepted_by_spooler`.

SumatraPDF is under (A)GPLv3 with some BSD code. Before publishing a package
that bundles it, obtain legal review and comply with its exact license/source
obligations. For an internal prototype, require a separately installed,
pinned, checksum-verified executable. Do not silently download arbitrary latest
binaries at runtime.

### Use as references or later components

| Project | Decision |
| --- | --- |
| QZ Tray | Study printing, certificates, and platform tests; do not base the headless service on its Java/browser trust model |
| `pdf-to-printer` | Useful Node spike and Sumatra option-mapping reference; not the Go production boundary |
| `alexbrainman/printer` | Useful small Winspool reference, but too inactive to make a critical dependency |
| OpenPrinting `goipp` | Good later pure-Go IPP protocol implementation; too low-level to replace CUPS or driver behavior in the MVP |
| PDFium | Strong later candidate for controlled PDF rendering; integration and print rasterization are not a two-day task |
| SQLite | Add for the durable offline agent queue after the vertical slice; do not let schema design block the first print |

Before copying code, record the dependency's license, release cadence,
maintainers, platform matrix, vulnerabilities, and whether it is linked,
executed as a separate program, or only studied as a behavioral reference.

## Minimal repository shape

Do not begin with a complex monorepo framework.

```text
agent/
  cmd/print-agent/
  internal/api/
  internal/agent/
  internal/platform/windows/
  internal/print/sumatra/
  internal/receipts/
  go.mod

apps/web/
  src/lib/server/
    auth/
    db/
    jobs/
    storage/
  src/routes/
  drizzle/

contracts/
  openapi.yaml

fixtures/
  pdf/

docs/
```

The OpenAPI file is the cross-language boundary. During the first two days,
hand-written small Go request/response structs are faster than building a code
generation system. Add a CI check against the contract once the requests stop
changing hourly.

The web application remains one deployable unit. Keep business functions in
`src/lib/server` so Vercel route handlers are adapters, not the only place the
logic can run. That makes a later standalone control-plane extraction possible
without beginning with one.

## Minimal data model

Only create tables the vertical slice uses:

```text
workspaces
  id, name, workos_organization_id, created_at

agents
  id, workspace_id, name, platform, version, credential_hash,
  last_seen_at, revoked_at, created_at

printers
  id, workspace_id, agent_id, native_id, name, is_default,
  capabilities_json, last_seen_at

documents
  id, workspace_id, sha256, byte_length, content_type,
  storage_key, created_at, expires_at

jobs
  id, workspace_id, printer_id, document_id, state,
  idempotency_key, lease_token_hash, lease_expires_at,
  created_at, updated_at

job_events
  id, job_id, sequence, source, state, error_code,
  detail_json, source_at, received_at

enrollment_tokens
  id, workspace_id, token_hash, expires_at, used_at
```

Required database constraints:

- workspace scoping on every query;
- unique `(workspace_id, idempotency_key)` when supplied;
- unique `(job_id, sequence, source)`;
- unique `(agent_id, native_id)` for printers;
- unique live enrollment token hash;
- foreign keys and explicit deletion policy;
- index eligible jobs by agent/printer and creation time.

Capabilities and diagnostics may begin as JSON because their platform shapes
are unsettled. Jobs, states, leases, ownership, and idempotency must be typed
columns because correctness depends on them.

## Minimal API

Human/session endpoints:

```text
POST /api/enrollment-tokens
POST /api/documents
POST /api/jobs
GET  /api/jobs/:id
GET  /api/jobs/:id/events
```

Agent endpoints:

```text
POST /agent/v1/enroll
POST /agent/v1/heartbeat
PUT  /agent/v1/printers
POST /agent/v1/jobs/claim
POST /agent/v1/jobs/:id/events
```

The enroll response returns the device secret once. The claim response returns
at most one job in the MVP. The document response should be short-lived and
authorized; never expose a permanent public PDF URL.

legacy-compatible endpoints do not belong in the 48-hour slice. Once the
native API is reliable, add a compatibility adapter translating the legacy service
requests into the same internal command and state model.

## Forty-eight-hour implementation plan

### Preparation: two hours

- choose the exact Windows machine, printer, driver version, and PDF;
- record the current successful legacy-provider output for comparison;
- install a pinned SumatraPDF release manually;
- verify its command-line printing outside our code;
- create WorkOS staging, a Vercel project, Neon database, and private Blob;
- write one acceptance statement: "this PDF reaches this spooler through our
  API after agent restart without a known duplicate."

If standalone Sumatra cannot print the fixture correctly, stop and change the
backend. No web scaffold can compensate for a failed local print path.

### Day one: enrollment through durable claim

Morning:

- scaffold SvelteKit with the Vercel adapter;
- install WorkOS AuthKit's SvelteKit integration and hosted sign-in;
- create workspace mapping from WorkOS Organization;
- add Drizzle migrations for the minimal tables;
- implement one-time enrollment-token creation and exchange.

Afternoon:

- scaffold the Go command;
- enroll and persist its credential with restrictive file permissions, using
  DPAPI as soon as practical;
- enumerate printers for the proof machine;
- upload the printer snapshot;
- implement transactional job creation and claim/lease;
- expose a plain dashboard printer list and job form.

End-of-day gate:

- a signed-in user sees the enrolled machine and its real printer;
- a database job can be claimed only once during concurrent claim requests;
- credential revocation stops the agent.

### Day two: document through observable print

Morning:

- upload an immutable private PDF;
- record byte length and SHA-256;
- stream it to the claimed agent and verify both;
- invoke the pinned Sumatra backend with a hard deadline;
- record the backend command without secrets or document content;
- post ordered status events.

Afternoon:

- render the job timeline;
- add stable errors for auth, printer missing, download, digest mismatch,
  timeout, backend exit, and uncertain handoff;
- retain a small local receipt keyed by job ID before and after submission;
- kill the agent during download and prove safe resume/reclaim;
- kill it during submission and prove the job becomes uncertain instead of
  silently printing twice;
- run the same fixture at least ten times and save timings/results.

End-of-day gate:

- one real end-to-end print;
- no false physical-completion claim;
- no known automatic duplicate on restart;
- every deliberate failure appears in the timeline;
- repository contains one command for local web development and one for the
  foreground agent.

### What to cut first if time slips

Cut in this order:

1. styling;
2. live WebSocket dashboard updates;
3. copies/scaling controls;
4. DPAPI in favor of a clearly documented development credential file;
5. Blob upload in favor of a small private API download;
6. WorkOS organization switcher in favor of one fixed staging organization.

Never cut durable job creation, idempotency, digest verification, state/error
recording, or the uncertain-handoff behavior. Those are the product.

## After the two-day proof

### Days three to seven: internal alpha

- package the Go agent as a Windows Service;
- replace the receipt file with SQLite WAL storage;
- reconcile receipts with the Windows spooler;
- add RAW printing if required internally;
- add CUPS printer discovery/printing on one Linux or macOS machine;
- add API keys and one legacy-compatible job endpoint;
- add webhook outbox/retry;
- create an MSI and basic update channel only after signing decisions;
- add OpenTelemetry traces correlated by job ID;
- run restart, network-loss, duplicate, and stuck-driver tests.

### Weeks two to four: dependable self-use beta

- complete local offline queue semantics;
- map the legacy printing options actually used by internal integrations;
- build differential and physical print fixtures;
- add printer capability snapshots and changes;
- isolate native calls that can hang;
- add document retention/deletion jobs;
- implement generic OIDC and filesystem/S3 storage adapters;
- publish Docker Compose for the server;
- document backup/restore and upgrades;
- start public source releases only after dependency licensing and secret
  handling are reviewed.

### Later: managed SaaS

Keep Vercel for the dashboard, public REST API, docs, and ordinary control-plane
requests as long as it remains operationally simple. Add a small always-on Go
agent gateway only after polling cost or measured latency demands it. That
gateway can run on any container platform; it should remain optional and speak
the same versioned agent protocol.

SaaS additions are:

- test/live environments and keys;
- workspace quotas and usage ledger;
- Stripe billing;
- webhook delivery and replay;
- SSO/MFA/SCIM through WorkOS as customer demand warrants;
- regional data placement;
- staged agent update cohorts;
- tenant isolation, audit, support, status, and incident operations.

They are not changes to the local printing core.

## Self-hosting decision

Offer self-hosting, but do not make it a day-two deliverable.

The open-source product should eventually run as:

- one server container;
- PostgreSQL;
- S3-compatible storage or local filesystem for small deployments;
- optional generic OIDC;
- no WorkOS, Vercel, Redis, Kafka, or Kubernetes requirement;
- the same Go agent and documented API.

The managed SaaS is the easiest installation and funds signing, compatibility
testing, hosted operations, and support. Self-hosting builds trust and protects
the project from becoming a hosted lock-in clone of the service it replaces.

Vercel/WorkOS/Neon are accelerators for the managed edition, not domain
dependencies. Enforce that with narrow interfaces and a CI self-host profile,
not with premature abstraction for every imaginable provider.

## Go/no-go measurements

At the end of the proof, record:

| Measure | Initial gate |
| --- | --- |
| Successful known-fixture submissions | 10/10 |
| API-to-agent claim latency while active | p95 under 3 seconds |
| Agent idle memory | under 40 MB |
| Agent idle CPU | effectively zero between polls |
| Duplicate on safe pre-submit restart | 0 |
| Ambiguous mid-submit restart | visible as uncertain |
| Unexplained terminal failures | 0 |
| Human-readable failure timeline | all injected cases |

Proceed to the internal alpha only if the local print output is acceptable and
the server/agent state explains every run. If not, investigate the print backend
before building more SaaS.

## Sources supporting this decision

- [Vercel SvelteKit deployment](https://vercel.com/i/what-is-sveltekit)
- [Vercel WebSocket behavior](https://vercel.com/kb/guide/do-vercel-serverless-functions-support-websocket-connections)
- [Vercel Function duration](https://vercel.com/docs/functions/configuring-functions/duration)
- [PostgreSQL providers on the Vercel Marketplace](https://vercel.com/docs/marketplace-storage)
- [Vercel Blob private storage](https://vercel.com/docs/vercel-blob)
- [WorkOS AuthKit](https://workos.com/docs/authkit/overview)
- [WorkOS users and organizations](https://workos.com/docs/authkit/users-organizations)
- [SumatraPDF command-line printing](https://www.sumatrapdfreader.org/docs/Command-line-arguments)
- [SumatraPDF source and license](https://github.com/sumatrapdfreader/sumatrapdf)
- [OpenPrinting CUPS](https://github.com/OpenPrinting/cups)
- [OpenPrinting Go IPP library](https://github.com/OpenPrinting/goipp)
- [QZ Tray print server reference](https://github.com/qzind/tray/wiki/print-server)
