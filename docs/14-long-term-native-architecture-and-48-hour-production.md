# Long-term native architecture and 48-hour production plan

Status: accepted architecture and execution plan  
Decision date: 29 July 2026

## The decision

Build the endpoint data plane in **Rust**.

Build the managed control-plane application in **SvelteKit and TypeScript** on
Vercel, with WorkOS AuthKit, managed PostgreSQL, and private object storage.

Compile one shared Rust core into native Windows, macOS, and Linux agent
artifacts. Give each platform a small adapter and its own installer/service
definition. V0.1 includes a tiny native tray/menu shell for each supported
desktop platform. Do not create three independent agent implementations and do
not put the queue or connection inside Electron, Tauri, or another desktop
shell.

This decision optimizes for the next five to ten years:

- low and predictable endpoint resource use;
- memory safety around untrusted documents and native interfaces;
- explicit, type-checked job and failure states;
- native service binaries without a language runtime;
- a shared protocol and queue implementation on every operating system;
- killable process isolation around printer drivers and renderers;
- an open-source core that can run headlessly on workstations, servers, and
  small Linux devices.

The first release is a **production-operated v0.1 at hour 48**. Production means
durable, secure, observable, recoverable, deployable, and supported inside an
explicit matrix. It does not mean that every legacy-provider feature and every printer
driver has been certified by hour 48.

## Is Go worse or less performant?

No. Go would be entirely fast enough.

The hot path spends most of its time in network transfer, PDF rendering, driver
code, operating-system spoolers, and physical printers. Go's garbage collector
will not materially change print pickup or output speed. A well-built Go agent
could be excellent and would probably be easier for a larger number of
contributors to modify.

Rust wins for this particular goal for reasons other than headline throughput:

| Concern | Rust | Go |
| --- | --- | --- |
| Network and spooler throughput | More than sufficient | More than sufficient |
| Idle memory predictability | Excellent | Good |
| Runtime/GC pauses | None | Small and irrelevant to normal printing |
| Compile-time state invariants | Excellent | Good with discipline |
| Memory safety | Strong, including no GC runtime | Strong for safe Go; native calls remain unsafe |
| Native Windows API surface | Excellent through Microsoft `windows` bindings | Good through `x/sys/windows` |
| C/C++ renderer integration | Strong | Possible, usually more awkward |
| Single native executable | Yes | Yes |
| Build and contributor simplicity | Moderate | Excellent |
| Fit for a low-resource endpoint appliance | Excellent | Very good |

If the primary objective were the fewest engineering hours for a conventional
team, Go would be the recommendation. Given the clarified objective—build the
best low-resource native open-source endpoint and use parallel AI agents to
compress implementation time—Rust's long-term properties outweigh its
short-term learning and compilation costs.

Do not use benchmarks of JSON loops to decide this product. Enforce product
budgets:

- normal idle RSS below 30 MB excluding an active renderer;
- effectively zero CPU between heartbeats and queue work;
- no full-document in-memory buffering;
- bounded local storage;
- bounded threads/tasks and retry attempts;
- a responsive supervisor even if a driver call hangs;
- cold start below two seconds on supported hardware.

Measure release binaries on every supported platform. Change an implementation
only when a product budget fails.

## Why not another language?

### C or C++

They have the deepest native and rendering ecosystem, but expose the largest
memory-safety and maintenance surface for a network-connected service handling
untrusted PDFs and raw bytes. Use existing C/C++ libraries behind audited
bindings or a sandboxed helper; do not make the orchestration core C++.

### C# and .NET

C# is very attractive on Windows and could make a good Windows-only product.
Cross-platform native printing and small Linux/ARM deployment are less natural,
and a Windows C# agent plus separate macOS/Linux agents would permanently split
the state machine and reliability behavior.

### Swift

Swift is the best language for a rich macOS application, not for the shared
Windows/Linux endpoint core. A tiny Swift status-menu or permissions helper can
be added if macOS integration genuinely requires it.

### Kotlin/Java

QZ Tray proves that Java can solve cross-platform printing. Its runtime and
browser/certificate-oriented architecture conflict with the low-resource,
headless target. QZ remains a valuable compatibility reference or optional
executor.

### Zig

Zig offers small binaries and excellent C interoperability, but its ecosystem,
API stability, async/network application libraries, and contributor pool make
it a higher-risk foundation here.

### TypeScript desktop applications

TypeScript is correct for the web product. A Node or Electron endpoint would be
fast to create but needlessly increases memory, update, runtime, and attack
surface. The endpoint should continue to print when nobody is logged into a
desktop UI.

## One agent, three native artifacts

The right shape is not a universal Chromium application and not three separate
products:

```text
                         shared crates
        protocol + domain + queue + storage + telemetry
                              |
             +----------------+----------------+
             |                |                |
        Windows host      macOS host       Linux host
        Winspool          CUPS/IPP         CUPS/IPP
        Windows Service   LaunchAgent      systemd
        MSI/MSIX          signed pkg       deb/rpm/tar
```

The shared core owns:

- agent identity and credential rotation;
- server protocol and version negotiation;
- local SQLite queue and event outbox;
- content hashing, storage, retention, and redaction;
- job state machine and retry policy;
- lease renewal and reconnect behavior;
- structured error taxonomy;
- tracing, metrics, diagnostics, and support bundle;
- executor supervision;
- configuration validation and safe updates.

Platform adapters own:

- printer enumeration and stable native identifiers;
- raw capability and option discovery;
- print submission;
- native spooler job identifiers and status;
- cancel/pause/resume where supported;
- secure credential storage;
- service/session lifecycle;
- OS logging integration.

### Important session model

Printer visibility can depend on the logged-in user. A Windows service running
as `LocalSystem` may not see a user's mapped network printers. A macOS
LaunchDaemon can have similar session differences.

Support two host modes with the same core:

- **user agent**: starts in a user session and sees that user's printers;
- **machine service**: runs without login for printers installed system-wide.

Do not hide this difference. Enrollment records host mode and user/session
identity. A later privileged service plus per-session helper is possible, but
it should not be invented until a target deployment needs it.

## V0.1 native shells without Chromium

The agent remains a headless service and the main dashboard is the hosted
SvelteKit application. V0.1 also ships a native companion shell on every
supported desktop platform:

| Platform | V0.1 shell |
| --- | --- |
| Windows | Small Win32/Rust notification-area process installed for user startup |
| macOS | Small native status-menu app, using SwiftUI/AppKit where that produces the cleanest OS integration |
| Linux | Small Rust status notifier/AppIndicator shell, packaged for desktops that support the chosen tray protocol |

The shell includes:

- connected/offline/error state;
- agent and workspace name;
- current queued/active job counts;
- printer warning count;
- open hosted dashboard;
- open local diagnostics;
- copy agent ID and version;
- trigger a controlled daemon restart;
- begin re-enrollment after explicit confirmation;
- export a redacted support bundle;
- exit the shell without stopping the daemon.

The CLI remains available for status, enrollment, diagnostics, printers, and
service control. The daemon exposes a narrow local IPC service over a Windows
named pipe or Unix-domain socket. Requests are authenticated using OS
permissions and a session-scoped challenge. The shell does not:

- open SQLite;
- claim, retry, cancel, or submit jobs directly;
- hold the cloud agent credential;
- render documents;
- maintain its own cloud connection;
- silently stop or pause the daemon when the user quits it.

Native shell code is intentionally disposable. Its only stable dependency is
the versioned local IPC contract. The macOS shell can be Swift and the
Windows/Linux shells Rust without splitting the actual agent implementation.
The combined shell logic should remain small enough to replace independently
when an OS changes its tray conventions.

Tauri uses the operating system webview rather than bundling Chromium and is
reasonable if a rich local application eventually becomes valuable. It is
still unnecessary for v0.1 because the required shell has no rich page layout.

## Rust workspace structure

```text
Cargo.toml
crates/
  domain/              # pure job/printer/event types and state transitions
  protocol/            # versioned wire types and signing
  queue/               # SQLite job journal and event outbox
  content/             # streaming, hashes, retention
  supervisor/          # killable executor processes
  telemetry/           # tracing/metrics/redaction
  executor-protocol/   # private child-process IPC
  executor-windows/    # Winspool and Windows PDF backend
  executor-cups/       # CUPS/IPP adapter
  agent/               # orchestration
  agent-cli/           # daemon and CLI entry points
apps/
  web/                 # SvelteKit dashboard/control-plane API
contracts/
  openapi.yaml
  schemas/
packaging/
  windows/
    tray/
  macos/
    menu/
  linux/
    tray/
fixtures/
  pdf/
  raw/
  fake-printer/
```

Dependency direction is inward:

```text
platform adapters -> application ports -> domain
HTTP/SQLite       -> application ports -> domain
```

The domain crate depends on none of Tokio, HTTP, SQLite, Win32, CUPS, WorkOS, or
Vercel. State transitions are methods returning typed errors, not string updates
issued by route handlers.

Use a small, conservative Rust dependency set:

- Tokio for asynchronous orchestration;
- rustls-based HTTP;
- Serde for boundary serialization;
- bundled SQLite through a mature binding;
- `tracing` and OpenTelemetry-compatible export;
- Microsoft's `windows` crate for Win32;
- SHA-256 and cryptographically secure random identifiers;
- explicit secret/redaction types.

Pin all dependencies, run license and vulnerability checks, produce an SBOM,
and avoid a general plugin system.

## Native printing backends

### Windows

Use a thin audited Winspool adapter for:

- `EnumPrinters` and `GetPrinter`;
- `OpenPrinter`;
- RAW `StartDocPrinter`/`WritePrinter` submission;
- `EnumJobs`/`GetJob`;
- printer change notifications;
- cancellation and state mapping.

Microsoft documents that some printer enumeration and driver calls can block.
A Tokio timeout cannot cancel a native thread blocked inside a driver. Run
enumeration, capability discovery, rendering, and submission in short-lived or
restartable executor child processes. The supervisor enforces:

- hard wall-clock deadline;
- memory limit where supported;
- bounded stdout/stderr;
- no network access for a renderer unless required;
- private working directory;
- exit/status classification;
- process-tree termination;
- crash-loop backoff.

For PDF v0.1, SumatraPDF is the fastest proven external backend and has useful
command-line options. It is (A)GPLv3. Do not casually combine or redistribute it
inside a permissive agent package. Either:

- comply with the applicable license and source obligations after legal review;
- require a separately installed, pinned executable for a private first cohort;
- or ship Windows RAW as production and label PDF preview until the renderer
  distribution is resolved.

The long-term permissive renderer candidate is PDFium in a sandboxed helper.
The helper renders bounded pages/strips and hands them to a Windows print path.
Renderer replacement must not change the cloud protocol or queue.

The MIT `printers` Rust crate is a useful accelerator: it exposes CUPS and
Winspool printing/job management. It is small and has limited maintainer depth.
Audit and wrap or vendor the exact native calls needed; do not let its public
types enter the domain.

### macOS and Linux

Use the installed CUPS system and its documented destination/job APIs. CUPS
already provides the abstraction over local drivers, IPP, USB, and network
queues. Do not reimplement a print system.

Begin with isolated `lp`/`lpstat` execution if it produces stable job IDs and
errors on the certified environment; move the adapter to the documented CUPS C
API when richer options/status or command parsing becomes a limitation.

CUPS guidance explicitly warns applications not to assume preconfigured
printers, formats, or features. Snapshot actual capabilities and reject
unsupported option combinations. Avoid deprecated PPD APIs; use current job
ticket/IPP capabilities.

### QZ Tray as an accelerator

QZ Tray can be an optional compatibility executor to unlock broad PDF/image/RAW
behavior while native backends mature. It should sit behind the same executor
protocol. The default low-resource install remains the Rust native executor.

This is a useful launch tactic, not architectural dependence. Its Java runtime,
certificate/trust workflow, status differences, and LGPL obligations must be
understood before redistribution.

## Durable queue architecture

There are three queues:

1. PostgreSQL hosted queue;
2. SQLite agent queue;
3. operating-system spooler.

They cannot form one atomic transaction. Exactly-once physical printing is
therefore impossible to promise.

The server transactionally leases a job to one agent. The agent durably writes
job metadata, content digest, and lease before acknowledging receipt. Before
calling the executor it writes `spool_intent`. If the executor returns a native
job ID it stores that before sending the cloud event.

Core states:

```text
queued_server
  -> leased
  -> downloading
  -> queued_local
  -> spool_intent
  -> accepted_by_spooler
  -> spooler_completed_reported

Any state may enter:
  failed_retryable
  failed_terminal
  cancelled
  handoff_uncertain
```

A crash before `spool_intent` is safely retryable. A crash after intent but
before a reconciled native job ID is uncertain and is not automatically
resubmitted. An operator or policy can resolve it with the evidence shown.

Never use `printed` as a terminal claim unless a printer/device protocol
provides physical evidence. The default terminal description is
`spooler_completed_reported`, with observer and native status attached.

## Managed control plane

Use:

- SvelteKit/TypeScript on Vercel for dashboard, public REST, enrollment,
  ordinary agent polling, and documentation;
- WorkOS hosted AuthKit and Organizations for human identity and SaaS
  membership;
- first-party agent credentials and developer API keys;
- managed PostgreSQL/Neon as the authoritative queue and event log;
- private immutable object storage for documents;
- polling/long-polling over outbound HTTPS for initial agent transport.

Vercel now supports WebSockets, but an established connection is pinned to a
function for its maximum duration and durable state still belongs outside the
connection. Polling with transactional leases is simpler and reliable. Extract
an always-on Rust gateway only when measured connection volume, cost, or latency
requires it. It speaks the same protocol; the agent does not care where it is
hosted.

WorkOS authenticates humans, not machines. An enrolled node receives a
first-party rotatable credential. The database owns internal workspace IDs and
resource authorization. A self-host build can replace the WorkOS adapter with
generic OIDC/local bootstrap auth without changing printing.

## Production v0.1 support envelope

The launch envelope is fixed during hour zero:

- one region;
- invite-only workspaces;
- Windows 10/11 x64 Tier 1 if hardware and signing are available;
- Linux x64/ARM and macOS as Tier 1 only when their real-printer, service,
  installer, signing, and restart gates pass; otherwise preview;
- PDF and RAW are independently tiered;
- copies, page range, color, duplex, media, orientation, tray, and scaling are
  advertised per adapter only when discovered and physically verified;
- maximum document size, pages, copies, concurrent jobs, agents, and workspaces
  are published;
- `accepted_by_spooler` semantics, not physical completion;
- manual controlled agent updates for the first cohort;
- native tray/menu shell installed by default on supported desktop editions,
  with a headless opt-out;
- no arbitrary document URI fetching;
- RAW disabled by default and explicitly enabled per workspace.

This is a real production service for supported workloads. Feature-parity work
expands the envelope after release.

## Parallel AI implementation topology

Use one integration lead and independent worktrees/branches. The integration
lead alone merges shared contracts and state transitions.

| Lane | Deliverable |
| --- | --- |
| 1. Domain/contracts | State machine, errors, wire schemas, OpenAPI, compatibility fixtures |
| 2. Cloud queue/API | PostgreSQL schema, lease transactions, documents, API keys, webhooks |
| 3. Auth/workspaces | WorkOS, tenant scoping, enrollment, audit |
| 4. Agent core | Protocol, SQLite, content, reconnect, outbox, supervisor |
| 5. Windows executor | Enumeration, RAW, status, cancellation, PDF backend |
| 6. CUPS executor | Discovery, PDF/RAW, options, status, cancellation |
| 7. Web product | Onboarding, agents, printers, submit job, event timeline |
| 8. Visual system | Linear-aligned tokens, primitives, themes, motion, visual regression |
| 9. Native shells | Windows tray, macOS menu, Linux notifier, local IPC |
| 10. Packaging/release | Windows/macOS/Linux artifacts, signing, SBOM, update metadata |
| 11. Reliability/test | Fake executor, contract, restart, ambiguity, load, tenant tests |
| 12. Operations/security | Telemetry, alerts, backups, restore, runbooks, threat review |

Rules for parallel work:

- freeze contract v0.1 by hour two;
- no lane invents a state or error locally;
- generated fixtures test every implementation;
- platform adapters implement one executor protocol;
- each branch includes tests and a short integration note;
- merge in dependency order every two to four hours, not once at the end;
- continuously deploy staging and continuously run virtual-printer canaries;
- stop feature work at hour 24; only integration, reliability, security, and
  release fixes follow.

AI can compress implementation and review. It cannot manufacture Apple
notarization access, code-signing certificates, printer hardware, or time for a
physical printer to execute tests. Missing external prerequisites downgrade a
platform or feature to preview; they do not turn an untested claim into support.

## Forty-eight-hour execution

### Hours 0–2: freeze

- appoint integration, release, and incident owners;
- lock platform/feature/capacity support envelope;
- lock state machine, errors, executor protocol, and API;
- inventory credentials, signing, runners, hardware, drivers, fixtures;
- decide Sumatra/QZ/PDF licensing and distribution for the first cohort;
- define SLOs and no-go release gates.

### Hours 2–6: skeletons and first integration

- create Rust workspace, SvelteKit app, migrations, OpenAPI, generated fixtures;
- create staging/production Vercel, WorkOS, database, object-store separation;
- create CI for Rust and TypeScript across target platforms;
- create fake printer/executor and virtual production canary;
- create the Linear-aligned visual token sheet and component gallery route;
- establish structured logs and correlation identifiers from the first commit.

### Hours 6–14: vertical production path in parallel

- WorkOS sign-in, workspace mapping, API key and agent enrollment;
- durable document/job transaction, idempotency, leasing, and event log;
- Rust credential storage, polling, SQLite queue/outbox, content verification;
- Windows and CUPS discovery/executor paths;
- native shell processes connected to the fake/local IPC service;
- dashboard agents/printers/job creation/timeline;
- production screens built only from the approved visual primitives;
- installer/service scaffolding.

Gate at hour 14: a job travels through hosted queue, real agent, native spooler,
and observable timeline. This is an integration gate, not the final product.

### Hours 14–24: complete supported features

- native spooler IDs, statuses, cancellation, capability snapshots;
- PDF/RAW and supported options;
- local restart recovery and uncertain handoff;
- signed webhooks with retry outbox;
- rate/size/copy limits and retention;
- service/session host modes;
- production shell status/actions, autostart, accessibility labels, and
  daemon-independent exit/crash behavior;
- dark and light visual themes, responsive density, keyboard/focus treatment,
  and snapshot coverage;
- build installers and upgrade/rollback artifacts;
- API examples and operator diagnostics.

Gate at hour 24: supported fixtures run repeatedly on real printers, concurrent
claims never double-lease, and all known faults produce stable visible events.
No new feature scope after this gate.

### Hours 24–32: security and destructive testing

- cross-tenant API and object-access tests;
- credential revoke/rotate/replay tests;
- agent kill during download, before submit, during submit, after native ID;
- driver hang and executor crash;
- network partition, disk full, corrupt download, checksum mismatch;
- database/storage/WorkOS outage behavior;
- dependency, license, secret, SBOM, and artifact review;
- least-privilege runtime and migration database roles.

### Hours 32–40: operations and scale

- load-test twice the published launch ceiling;
- prove bounded memory/tasks/database connections;
- enable Sentry/OpenTelemetry-compatible traces, metrics, and alerts;
- enable PITR and restore an isolated database;
- exercise Vercel application rollback and compatible migration path;
- complete incident, compromise, queue, uncertain-handoff, provider, and
  retention runbooks;
- exercise job admission and agent-claim kill switches.

### Hours 40–44: install and canary

- fresh install/service/restart/uninstall on every Tier 1 platform;
- verify shell autostart, daemon-unavailable behavior, shell-only restart, and
  that killing the shell does not interrupt an active print;
- install the previous/current artifact and test controlled upgrade/rollback;
- run virtual printer continuously;
- enroll one internal production workspace and real printer;
- verify dashboards, paging, status page, backups, and audit log.

### Hours 44–48: controlled production release

- run a real operational canary;
- fix only release blockers;
- tag immutable agent and web releases;
- publish checksums, SBOM, licenses, support matrix, limits, semantics, and
  known limitations;
- invite the first production cohort only if every relevant gate passes.

If a single platform adapter fails, disable that adapter and launch the
remaining supported envelope. If core tenant isolation, durable queue,
ambiguous-handoff safety, rollback, or credential security fails, do not accept
production jobs.

## Release gates

Every advertised Tier 1 combination must pass:

1. fifty repeated known-fixture prints per certified printer/driver profile;
2. no lost job across safe server/agent restarts;
3. same idempotency key always returns the same logical job;
4. concurrent claims cannot double-lease;
5. crash after possible handoff becomes visible `handoff_uncertain`, never blind
   duplicate retry;
6. missing printer, bad option, corrupt content, backend timeout, storage
   outage, and credential revocation have stable actionable errors;
7. cross-workspace resource and document requests fail;
8. short-lived document authorization is limited to the current lease holder;
9. service install/restart/uninstall and controlled rollback pass;
10. each native shell passes autostart, accessibility, local-IPC authorization,
    crash independence, and daemon-upgrade compatibility tests;
11. the component gallery and all production screens pass approved dark/light
    visual snapshots, responsive checks, keyboard navigation, visible focus,
    and WCAG AA contrast for functional content;
12. twice launch load stays within published latency/resource budgets;
13. database restore and application rollback are exercised;
14. no known critical exploitable vulnerability or undisclosed license blocker;
15. alerts, kill switches, status, incident owner, and runbooks are live.

The detailed visual specification and implementation sequence are in
[15-linear-aligned-visual-system.md](15-linear-aligned-visual-system.md).

Suggested initial SLOs:

- durable job registration: 99.9% monthly;
- online healthy agent pickup: p95 under three seconds;
- accepted agent event visible: p95 under three seconds;
- no known silent acknowledged-job loss;
- database RPO at most five minutes and RTO at most one hour, only after the
  managed plan and restore drill prove them.

Physical printer success is not a cloud service SLO unless hardware telemetry
can actually prove it.

## What follows hour 48

The architecture does not change. The supported envelope expands:

- differential legacy API compatibility;
- more driver options and printer certification;
- PDFium renderer helper;
- more OS/architecture installers and signing;
- agent staged auto-updates;
- webhook replay and SDK generation;
- test/live environments, metering, Stripe billing;
- generic OIDC and complete self-host Docker Compose;
- direct IPP/device status enrichment;
- scales and other local peripherals;
- regional cells and an always-on connection gateway if measurements demand it.

The system launched at hour 48 is not thrown away. Every later feature enters
through the same domain model, executor boundary, queue, and protocol.

## Source evidence

- [Microsoft Rust for Windows bindings](https://microsoft.github.io/windows-docs-rs/)
- [Rust Winspool `GetPrinterW` binding](https://microsoft.github.io/windows-docs-rs/doc/windows/Win32/Graphics/Printing/fn.GetPrinterW.html)
- [OpenPrinting CUPS programming manual](https://openprinting.github.io/cups/doc/cupspm.html)
- [Current CUPS job APIs and deprecated PPD warning](https://openprinting.github.io/cups/doc/api-ppd.html)
- [Rust `printers` CUPS/Winspool library](https://github.com/talesluna/rust-printers)
- [SumatraPDF command-line printing](https://www.sumatrapdfreader.org/docs/Command-line-arguments)
- [SumatraPDF source and license](https://github.com/sumatrapdfreader/sumatrapdf)
- [PDFium license](https://pdfium.googlesource.com/pdfium/+/refs/heads/main/LICENSE)
- [QZ Tray source](https://github.com/qzind/tray)
- [Vercel WebSocket behavior](https://vercel.com/kb/guide/do-vercel-serverless-functions-support-websocket-connections)
- [WorkOS AuthKit](https://workos.com/docs/authkit/overview)
