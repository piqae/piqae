# Delivery roadmap and resourcing

## Strategy

Do not start by recreating every account and scale endpoint. First replace the
real legacy-provider workload end to end, then widen compatibility from a stable
print core.

The schedule below assumes three experienced engineers:

- one focused on Windows/native printing and renderer work;
- one on agent/distributed-state/control-plane work;
- one full-stack across API, Svelte UI, packaging, CI, and operations.

Specialist security, macOS signing, and physical QA help is still needed. With
one engineer, expect roughly 12–18 months for a hardened cross-platform product.
With a focused three-person team, a useful internal replacement may take
3–5 months and broad public printing parity 7–10 months. Scale/integrator/SaaS
polish can extend the programme to 9–15 months. These are planning ranges; the
PDF/Windows spike is intended to replace guesses with evidence.

## Priority implementation slice: native profiles and stock

The next cross-platform slice is no longer generic option-form expansion. It is
the native profile model described in
[native print profiles, stock, and routing](16-native-print-profiles-stock-and-routing.md):

- one installed destination with multiple native profiles;
- macOS `NSPrintPanel` and Windows `DocumentPropertiesW` capture;
- immutable native tickets and driver fingerprints;
- stock/tray readiness and operator holds;
- stable targets across multiple nodes;
- virtual printer IDs for legacy-compatible callers;
- a simpler web UI that delegates vendor settings to the native driver.

The current option-only named profiles migrate into this model as unverified
profiles and require validation before production use.

## Phase 0: workload and acceptance baseline (1–2 weeks)

Deliver:

- inventory of actual printers, OS versions, drivers, formats, options, volume,
  latency, offline duration, and failure history;
- redacted sample PDFs/RAW jobs;
- current legacy API calls and SDK versions;
- monthly cost and job count;
- selection of 3–5 critical physical print scenarios;
- agreed “accepted”, “printed”, and “uncertain” semantics;
- initial device lab.

Exit:

- every internal replacement requirement maps to a test;
- no unexamined dependency on scales, child accounts, or a specific SDK.

## Phase 1: native printing risk spikes (3–5 weeks)

Parallel spikes:

### Windows PDF/RAW

- enumerate real queues/capabilities under a service;
- submit RAW and capture job IDs;
- PDFium render plus GDI and/or XPS path;
- map paper, tray, copies, duplex, DPI, colour, fit, pages, and rotation;
- observe/cancel jobs;
- power-loss and spooler-restart experiment.

### CUPS/macOS/Linux

- enumerate/query through IPP/CUPS;
- PDF passthrough and RAW;
- options and status;
- headless service identity.

### Distributed queue

- SQLite intent/event outbox prototype;
- fake server/agent WebSocket;
- resumable content download;
- ambiguous-handoff state demonstration.

Exit:

- print fixtures succeed on critical hardware;
- renderer licence/size/memory is acceptable;
- native state depth is documented;
- stack decision confirmed or revised;
- more credible milestone estimate.

## Phase 2: local-first agent MVP (5–7 weeks)

Deliver:

- Rust agent daemon and CLI;
- SQLite/WAL job/event store;
- loopback native API with streaming PDF/RAW uploads;
- Windows and CUPS adapters for the required internal platforms;
- per-printer ordered workers;
- core documented options;
- renderer child isolation;
- local job/event/status API;
- Svelte local UI for health, printers, test job, queue, and diagnostics;
- Linux package and development Windows/macOS installers;
- structured logs and support bundle.

Exit:

- internal application can print locally without the legacy service;
- offline/restart/duplicate tests pass;
- resource measurements meet provisional targets.

## Phase 3: self-hosted remote control plane (5–7 weeks)

Deliver:

- workspace, API key, agent enrolment;
- PostgreSQL job/event/outbox model;
- filesystem object store;
- outbound authenticated agent WebSocket;
- resumable HTTPS content;
- live Svelte dashboard;
- native webhooks with replay;
- Docker Compose and native-server deployment docs;
- backups, health, metrics, and upgrades.

Exit:

- remote job accepted while agent is offline prints after reconnection;
- no loss across server/agent restart matrix;
- the organisation can run production shadow tests.

## Phase 4: the legacy service printing compatibility (6–10 weeks)

Deliver:

- compatibility computers/printers/jobs/states endpoints;
- Basic API-key authentication;
- the legacy service content modes and all documented options;
- pagination, filters, response headers, error shapes;
- idempotency and cancellation behavior;
- compatible webhooks;
- differential contract suite;
- SDK migration examples;
- signed unattended installers;
- physical device matrix.

Exit:

- internal integration changes base URL/key only;
- supported official SDK versions pass the matrix;
- cutover and rollback rehearsed;
- the legacy service can be removed from the internal production path.

## Phase 5: production hardening and public 1.0 (6–10 weeks)

Deliver:

- signed release/update pipeline;
- staged agent rollout and rollback;
- independent security review fixes;
- full retention/cleanup/audit;
- proxy/custom CA/enterprise identity coverage;
- macOS notarisation and platform support matrix;
- performance/soak results;
- public docs, OpenAPI, examples, contributor setup;
- licence/governance/security policy.

Exit:

- public 1.0 release gates in testing document pass;
- self-host upgrade/restore rehearsal succeeds;
- known limitations are published.

## Phase 6: optional SaaS (6–10 weeks)

Deliver the smallest Stripe-like flow:

- account/workspace signup;
- API key creation;
- pre-enrolled installer/one-command enrolment;
- live computer/printer onboarding;
- test print and copyable API request;
- usage metering and simple billing;
- tenant quotas and abuse controls;
- multi-instance control plane/object storage;
- incident/status/support operations.

Avoid initially:

- complex plan catalogues;
- enterprise sales features;
- regional promises not yet implemented;
- per-printer pricing complexity;
- bespoke branded agent builds.

Use configuration-driven branding first.

## Phase 7: scales and integrator parity (8–14 weeks)

Can be split based on demand.

### Scales

- USB HID;
- serial configuration and parser registry;
- normalised readings;
- REST and browser WebSocket compatibility;
- virtual scale;
- supported-device test matrix.

### Integrator

- child accounts;
- child selection headers;
- tags and key management;
- delegated authentication;
- client enrolment keys;
- account state/control;
- multi-tenant branding.

## First internal cutover

1. Run clone agents alongside legacy desktop clients, but never send the same
   production print job to both.
2. Compare inventory and capability reporting.
3. Print a controlled fixture matrix to both systems and compare output.
4. Route low-risk test/staging jobs to the clone.
5. Route one production printer/workflow with immediate rollback.
6. Expand by printer/workflow after an observation window.
7. Keep the legacy service credentials available but inactive for rollback.
8. Remove legacy desktop client only after queued jobs are empty and audit confirms
   no integration still uses it.

## Work breakdown by epic

| Epic | Core deliverable | Main risk |
| --- | --- | --- |
| Domain/state | durable job/event model | ambiguous side effects |
| Windows adapter | local drivers, options, status | driver inconsistency and blocking APIs |
| CUPS adapter | Linux/macOS queues and IPP | distro/Apple variation |
| PDF renderer | faithful bounded output | fonts, transforms, memory, licence |
| RAW path | byte-exact device output | dangerous commands and queue setup |
| Agent runtime | service, SQLite, updates | identity clones and power loss |
| Control plane | routing, storage, webhooks | tenant isolation and queue correctness |
| Compatibility | API/SDK contract | undocumented edge behavior |
| Svelte UI | setup and diagnostics | hiding semantic complexity |
| Packaging | signed headless installs | OS permissions and upgrades |
| Reliability lab | differential/physical/chaos | hardware matrix cost |
| SaaS operations | simple onboarding/billing | operational burden |

## Cost control

The open-source internal replacement avoids per-print service cost but creates
engineering and support cost. Keep it economically sensible by:

- shipping the exact internal workload before broad parity;
- one agent core and one modular control-plane service;
- no broker until needed;
- no UI dependency in the print path;
- using OS drivers instead of a printer database;
- limiting officially supported OS/printer combinations while leaving the
  design extensible;
- publishing community support tiers separately from tested/certified tiers;
- measuring the break-even point against current legacy-provider spend and business
  risk.

## Immediate next actions

1. Fill in the actual-workload questionnaire in the decisions document.
2. Obtain representative printers and generated/redacted job fixtures.
3. Create the Rust workspace and domain state crate.
4. Time-box Windows PDF/RAW and CUPS spikes before building the full server.
5. Turn the documented legacy-provider subset into OpenAPI/JSON fixtures and
   differential tests.
6. Decide licence and project name before publishing.
