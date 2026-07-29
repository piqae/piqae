# Open-source, SaaS, and build strategy

> Implementation update, 29 July 2026: Rust remains the endpoint data-plane
> choice. The schedule has been replaced by a parallel 48-hour production-v0.1
> plan. See
> [14-long-term-native-architecture-and-48-hour-production.md](14-long-term-native-architecture-and-48-hour-production.md).
> The longer programme below remains the feature-parity and maturity roadmap,
> not the schedule for the first usable release.

## Recommendation in one sentence

Build one open print platform with three ways to run it—direct local,
self-hosted, and managed cloud—using a small Rust data plane, a Rust modular
control plane, and optional SvelteKit interfaces; make the API and five-minute
first print the product, and sell a dependable managed service rather than
withholding essential features from self-hosters.

## Why this is the best alignment

This model aligns the interests of:

- the project, which needs a sustainable hosted business;
- the community, which needs usable source rather than a demo edition;
- internal users, who need a no-fee deployment they control;
- developers, who need a stable API and excellent diagnostics;
- larger customers, who need signed software, security evidence, support, and
  operational accountability.

The product should not be described as a desktop application with a cloud
backend. It is an edge data plane with an optional control plane:

```text
                    one protocol and API model

  Direct local           Self-hosted             Managed SaaS
  -------------          ----------------         ----------------
  app -> agent           app -> server           app -> service
       -> spooler             -> agent                 -> agent
                              -> spooler               -> spooler
```

Every mode uses the same agent binary, job semantics, option model, state
machine, and SDKs.

## Product positioning

The concise promise:

> A dependable, open API for printing PDF and RAW jobs through the drivers
> already installed beside your printers.

Position around:

- open source and self-hostable;
- local drivers and broad printer compatibility;
- headless, low-resource agents;
- PrintNode migration compatibility;
- truthful, real-time states;
- durable offline queues;
- developer-first onboarding;
- no browser print dialog;
- hosted service available but never required.

Do not position around having the largest feature list. The advantage is that
the important path is understandable, operable, and open.

## One product, three editions

Avoid maintaining a separate “community edition” fork.

### Direct

- Agent only.
- Loopback/LAN native API.
- SQLite queue.
- No account or internet required.
- Ideal for one application and one site.

### Self-hosted

- Complete open-source control plane.
- PostgreSQL and filesystem/S3 storage.
- Docker Compose plus native/container images.
- Agents use outbound TLS.
- Same API, webhooks, dashboard, and compatibility layer as cloud.

### Cloud

- Same control-plane source operated by the project.
- Automatic upgrades, backups, high availability, monitoring, billing,
  abuse controls, support, and compliance evidence.
- Multi-tenant and later multi-region operation.

Cloud may provide operational integrations that make sense only for the hosted
environment, but self-hosters should not lose core printing, API compatibility,
webhooks, job history, diagnostics, or security controls.

## Open-source model

### Licence

The repository-wide licence decision is:

| Area | Licence | Reason |
| --- | --- | --- |
| All Spool-authored source, schemas, documentation and examples | Apache-2.0 | A single permissive licence maximises adoption and makes embedding, forking, and self-hosting straightforward. |

Do not use BSL, SSPL, Commons Clause, or “source available” while marketing the
project as open source.

Spool Cloud competes on managed availability, signing, upgrades, monitoring,
backups, and support. Core printing functionality is not withheld from
self-hosters. Spool names and branding are covered separately by
`TRADEMARKS.md`.

### Contribution model

Begin with Developer Certificate of Origin sign-off, not a broad copyright
assignment. Add:

- `LICENSES/` and per-package SPDX identifiers;
- `CONTRIBUTING.md`;
- `CODE_OF_CONDUCT.md`;
- `SECURITY.md` with private reporting and disclosure expectations;
- `GOVERNANCE.md`;
- `MAINTAINERS.md`;
- public roadmap and RFC process;
- support/version policy;
- architecture decision records;
- issue and pull-request templates.

Use a CLA only if a concrete dual-licensing requirement exists. A surprise CLA
or relicensing plan damages trust.

### Governance progression

1. **Founding stage:** named maintainers make decisions in public RFCs.
2. **Contributor stage:** documented path from contributor to reviewer and
   maintainer, based on sustained work rather than employment.
3. **Mature stage:** technical steering group with more than one organisation
   represented; trademark and security authority remain explicitly assigned.
4. **Foundation stage, if warranted:** consider a neutral home only after the
   community and commercial model are stable.

Publish meeting notes and decisions. Keep security incidents private until
coordinated disclosure is safe.

### Community support channels

- GitHub Issues: reproducible bugs and accepted feature requests.
- GitHub Discussions or forum: design, help, and integration questions.
- Chat: community conversation, never the only durable support record.
- Security channel: private.
- Paid support: response-time commitments and production help.

Do not promise free maintainer support for every printer. Publish three support
levels:

- **certified:** continuously tested by the project;
- **community verified:** successful reports with environment details;
- **expected:** should work through installed drivers but is untested.

### Community flywheel

- excellent contributor setup in one command;
- fake spoolers and virtual agents so contributors need no printer;
- small adapter contract;
- generated test PDFs/RAW fixtures;
- labelled `good first issue` and `help wanted` work;
- monthly compatibility/device report;
- plugin/adapter examples only after the core is stable;
- public funding/sponsorship and hardware-donation policy;
- credit contributors in releases.

## Supply-chain trust is a product feature

The agent is installed beside business-critical printers and automatically
accepts remote work. Open-source trust needs more than visible code.

From the first public release:

- protected branches and required independent review;
- least-privilege CI tokens;
- pinned CI actions by immutable digest/commit;
- automated dependency updates and vulnerability scanning;
- SBOM for every installer/container;
- signed commits/tags according to project policy;
- signed and notarised platform packages;
- checksums and Sigstore signatures;
- SLSA provenance generated by the build platform;
- reproducible builds where platform signing permits;
- OpenSSF Scorecard in advisory mode;
- prompt security backports for supported branches.

Target SLSA Build L2 for 1.0 artifacts. Pursue L3 after the release process and
team justify its operational cost.

## The Stripe-like developer experience

Stripe's lasting lesson is not visual design. It is removal of uncertainty at
every integration step.

### Five-minute first print

The cloud quickstart should be:

1. Create a workspace.
2. Copy a test API key.
3. Run one installer command containing a short-lived enrolment token.
4. Wait for the named printer to appear.
5. Run one copyable `curl`.
6. Open the returned job URL and watch its timeline.

Example:

```bash
curl https://api.example.com/v1/printers \
  -H "Authorization: Bearer opr_test_..."

curl https://api.example.com/v1/printers/prn_123/jobs:upload \
  -H "Authorization: Bearer opr_test_..." \
  -H "Idempotency-Key: order-481-label" \
  -H "Content-Type: application/pdf" \
  --data-binary @label.pdf
```

The response includes a stable job ID, request ID, current state, and links to
events and documentation.

### Test mode without wasted paper

Test mode must be a real product boundary:

- `opr_test_...` and `opr_live_...` keys;
- resources visibly scoped to test/live;
- virtual printers and deterministic test agent;
- a rendered-output artifact for test PDF jobs;
- simulated offline, paper-out, rejection, delay, and ambiguous states;
- webhook events identical in shape to live events;
- no physical output unless a printer is explicitly enabled for test mode.

This is more useful than a simple API sandbox because printing has physical side
effects.

### Documentation as part of the API

- one conceptual “how printing works” page;
- one 5-minute quickstart;
- complete API reference generated from reviewed OpenAPI;
- copyable examples in TypeScript, Python, PHP, C#, Java, Go, and cURL as demand
  warrants;
- request/response examples beside every endpoint;
- errors link directly to a stable explanation;
- webhook tester and replay UI;
- API explorer using test mode;
- migration guide for each tested PrintNode SDK;
- changelog with before/after examples;
- status and incident history.

Write the documentation before implementing an endpoint. Contract review is
part of design review.

### Compatibility guarantees

- every create accepts idempotency;
- API keys are scoped and separately test/live;
- stable request IDs;
- predictable pagination;
- webhook events have stable IDs and replay;
- account-level API version pinning;
- request-level version override for testing upgrades;
- no breaking field removal or semantic change within a version;
- minimum 12-month migration window for a retired version;
- SDKs are thin wrappers, not alternate sources of truth.

PrintNode compatibility is a named compatibility version, separate from the
native API. It can improve internally without silently changing observed
legacy behavior.

### Dashboard philosophy

The dashboard should explain the API, not replace it. Every UI operation maps
to a documented API call and can reveal a copyable equivalent. The most
important screen is the job timeline, showing:

- API registration;
- content transfer;
- agent acceptance;
- local queue;
- rendering;
- OS spooler ID/state;
- device state when known;
- webhook delivery;
- exact error and retry decision.

## Modern application architecture

### Start deliberately boring

Initial production components:

- Rust agent;
- Rust control-plane modular monolith;
- SvelteKit static/dashboard app;
- PostgreSQL;
- S3-compatible object storage for cloud, filesystem option for self-hosting;
- OpenTelemetry;
- CDN/load balancer for cloud.

Do not initially add:

- Kubernetes as a self-host requirement;
- Kafka, Redis, NATS, or RabbitMQ;
- GraphQL;
- service mesh;
- separate auth, webhook, billing, and job microservices;
- Electron;
- a plugin runtime that can execute arbitrary code;
- multiple server implementation languages.

The modular monolith has internal module boundaries and outbox tables. It can
be profiled, deployed, and debugged as one system.

### Code structure

Keep four stable layers:

```text
domain
  pure entities, states, policies, errors

application
  commands, queries, transactions, ports

adapters
  PostgreSQL, SQLite, object storage, Winspool, CUPS, HTTP, WebSocket

delivery
  REST, agent protocol, CLI, Svelte static assets
```

Rules:

- domain crates do not depend on web/database/platform crates;
- unsafe Rust exists only in reviewed native adapter modules;
- no generic “framework” until two real implementations need the abstraction;
- no repository interface for every table; model transactions around use cases;
- one canonical error taxonomy;
- one canonical state machine;
- generated code stays at boundaries;
- prefer deletion to deprecation of unused internal code;
- performance budgets and dependency budgets are reviewed like functionality.

### Desktop approach

The primary deliverable is a daemon, not a GUI application.

- Windows Service;
- macOS `launchd` daemon;
- Linux `systemd` service;
- CLI and loopback Svelte UI;
- v0.1 native Windows tray, macOS status menu, and supported Linux notifier
  shells;
- Tauri only if a self-contained desktop shell becomes valuable;
- shell/Tauri process never owns queue state or remote connectivity.

This gives one cross-platform product while retaining native printing code where
it matters.

## Scaling toward a large SaaS

Do not build the final scale architecture on day one. Preserve a path to it.

### Stage 1: one region, modular monolith

- multiple stateless API/connection processes;
- one highly available PostgreSQL cluster;
- object storage;
- database outbox;
- load balancer with agent session affinity or session directory;
- background workers from the same release artifact.

This can support a substantial service if queries, indexes, connection memory,
and payload transfer are designed well.

### Stage 2: separate deployable roles, same codebase

Run the same binary/image with roles:

- public API;
- agent connection gateway;
- routing/outbox worker;
- webhook worker;
- retention worker.

Separate only because scaling or failure characteristics differ. Keep one
repository, domain model, and release train.

### Stage 3: regional cells

When tenant count and blast-radius evidence justify it:

- assign each workspace to a regional cell;
- a thin global directory routes API and agent enrolment;
- each cell owns connection gateways, workers, database partition/cluster, and
  object prefix/bucket;
- failures and overload remain within a bounded set of workspaces;
- agent configuration has primary and recovery endpoints;
- data residency follows explicit cell placement.

Cell architecture is an evolution boundary, not a reason to begin with
microservices.

### Scale triggers

Add complexity only after a measured trigger:

| Change | Trigger |
| --- | --- |
| Redis/session directory | In-memory/PostgreSQL session lookup measurably limits gateways. |
| NATS/broker | PostgreSQL outbox/notify cannot meet proven dispatch throughput or isolation. |
| Dedicated WebSocket gateway | Connection memory/release cadence conflicts with API workloads. |
| Database partitioning | Table/index size or maintenance breaches SLO. |
| Regional cells | Blast radius, residency, or independent capacity requires it. |
| Kubernetes | Team operates enough replicated services that orchestration reduces total complexity. |

Architecture decision records must include the measurement that crossed the
trigger.

## Reliability model used by serious SaaS companies

### SLOs before scale

Define:

- durable job registration availability;
- connected-agent notification latency;
- agent acceptance latency;
- event propagation latency;
- webhook delivery latency;
- control-plane durability;
- ambiguous-handoff rate.

Do not define “prints succeeded” as one global SLO because physical device
failures are outside service control. Publish the boundary.

### Operational discipline

- error budgets;
- on-call ownership;
- dashboards linked to runbooks;
- canary server and agent releases;
- staged agent update cohorts;
- automated rollback on health regression;
- database migration compatibility across one previous release;
- quarterly restore and regional-loss exercises;
- incident review focused on system improvement;
- capacity tests and dependency failure drills;
- status page scoped to actual subsystem impact.

### Multi-tenant safety

- tenant ID in every storage key and domain command;
- database constraints and optional row-level security;
- separate test/live namespaces;
- resource quotas and admission control;
- no unbounded user-controlled metric labels;
- content/URI/RAW policies;
- audit log;
- operator access is temporary, approved, and recorded.

## Self-hosting that people will actually use

Support it from the first remote-control release, but do not pretend every
topology is supported.

### Supported profiles

1. **Development:** one command, disposable containers, seeded virtual agent.
2. **Small production:** Docker Compose or native process, PostgreSQL, local
   durable content volume, reverse proxy.
3. **Scaled production:** external PostgreSQL and S3, multiple stateless
   processes, documented load-balancer requirements.

Kubernetes manifests/Helm arrive only when there is user demand and project
capacity to test upgrades.

### Self-host contract

- no licence phone-home;
- no mandatory project cloud account;
- offline-capable release artifacts and docs;
- telemetry and update checks opt-in;
- all core APIs/UI available;
- config export and backup/restore;
- upgrade paths for supported versions;
- explicit matrix of supported topologies;
- community support best-effort, paid support optional.

The easiest self-host deployment should take less than 15 minutes, excluding
DNS/TLS, and survive `docker compose pull && docker compose up -d` with a
documented backup step.

## SaaS business model

Sell outcomes the cloud uniquely provides:

- zero infrastructure;
- signed pre-enrolled agents;
- automatic safe upgrades;
- multi-zone durability;
- monitoring and status;
- managed backups and retention;
- support and SLA;
- audit/compliance evidence;
- fleet policy and enterprise identity;
- regional placement when available.

Suggested metering:

- generous free developer/test tier;
- bill on jobs accepted by the OS spooler, not pages;
- do not charge simulated test-mode jobs;
- include agents/printers generously;
- simple included volume plus transparent overage;
- enterprise annual plans for support/SLA/data residency, not artificial API
  feature locks.

Before choosing prices, calculate support and connection/storage cost. Printing
requests are cheap; fleet support and platform signing are not.

## Delivery plan

This plan complements the engineering phases in [09-roadmap.md](09-roadmap.md).

### Programme 0: principles and evidence (weeks 1–2)

Decide:

- project name and promise;
- actual internal workload;
- certified OS/printer baseline;
- licence with legal review;
- test/live semantics;
- billable event;
- completion/uncertain semantics;
- target SLOs and resource budgets.

Create:

- public product principles;
- RFC/ADR templates;
- governance/security/contribution drafts;
- five-minute quickstart written against the proposed API;
- generated fixtures and device lab inventory.

Gate: no application scaffolding until the Windows PDF/RAW acceptance fixtures
and API quickstart are agreed.

### Programme 1: prove the hard parts (weeks 3–7)

Build disposable spikes:

- Windows service printer enumeration/options;
- PDFium rendering through candidate Windows print paths;
- RAW byte-exact printing;
- CUPS PDF/RAW and status;
- SQLite crash-safe job intent/outbox;
- OS handoff crash injection;
- fake agent/server protocol;
- Svelte job timeline prototype using fixture events.

Measure fidelity, RSS, package size, latency, and ambiguity.

Gate: choose the Windows backend and confirm the one-agent architecture. Stop
or rescope if critical printers cannot be supported reliably.

### Programme 2: local open-source alpha (weeks 8–14)

Build:

- repository and four-layer crates;
- daemon/service and CLI;
- SQLite queue/content cache;
- Windows and CUPS adapters;
- sandboxed renderer;
- local streaming API;
- printer/job/event models;
- Svelte loopback UI;
- virtual/fake print adapter;
- cross-platform development packages;
- documentation and contributor workflow.

Release publicly once installation is safe enough for technical testers.

Gate:

- internal local workflow succeeds;
- restart/duplicate tests pass;
- no critical security finding;
- resource budgets are met;
- external contributor can run tests without a printer.

### Programme 3: self-hosted beta (weeks 15–22)

Build:

- Rust modular control plane;
- PostgreSQL migrations/outboxes;
- content storage abstraction;
- workspace/API key/agent enrolment;
- outbound WebSocket and resumable HTTP;
- native webhooks/replay;
- Svelte hosted dashboard;
- Docker Compose production profile;
- backup/restore and upgrade test;
- signed CI artifacts with SBOM/provenance.

Dogfood for all internal non-critical prints, then one production workflow.

Gate:

- offline agent jobs recover;
- server/agent chaos matrix passes;
- self-host fresh install under 15 minutes;
- restore rehearsal succeeds;
- job timeline explains every tested failure.

### Programme 4: PrintNode replacement release (weeks 23–32)

Build:

- compatibility API subset required internally;
- differential tests against PrintNode;
- all documented print options used by target integrations;
- compatibility webhooks;
- signed Windows/macOS/Linux installers;
- official TypeScript plus internal-language SDK;
- migration/cutover/rollback tooling;
- physical certification matrix;
- 30-day production soak.

Gate:

- existing application changes only base URL/key in the supported path;
- no lost or silent duplicate job during soak;
- PrintNode can be disabled with rehearsed rollback.

### Programme 5: cloud developer preview (weeks 28–36, partly parallel)

Build:

- signup/workspace;
- test/live mode;
- virtual printer;
- pre-enrolled install command;
- hosted API/docs explorer;
- usage ledger and quotas;
- billing;
- status/incident/support systems;
- tenant isolation review;
- managed update cohorts.

Invite a small number of design partners, not an unrestricted public launch.

Gate:

- median first test print under five minutes;
- cloud unit economics measured;
- tenant isolation and independent security review pass;
- on-call/runbooks/restore exist;
- compatibility promise is explicit.

### Programme 6: public 1.0 and scale (months 10–15)

- close security review;
- SLSA L2 release provenance;
- OpenSSF/security baseline review;
- public API version policy;
- supported self-host matrix;
- paid support terms;
- wider printer certification;
- server multi-instance tests;
- agent staged update system;
- contributor-to-maintainer path;
- launch cloud only at an SLO the team can operate.

Scale architecture changes require real thresholds, not launch-day projection.

## Workstreams and ownership

Minimum serious team:

| Workstream | Primary responsibility |
| --- | --- |
| Native/Windows | Winspool, PrintTicket, PDF backends, service/installer |
| Agent/distributed state | SQLite, protocol, queue/state, CUPS |
| Platform/API | PostgreSQL, API compatibility, auth, webhooks, SaaS |
| Product/web/docs | Svelte, onboarding, docs, SDK experience |
| Reliability/security | CI, signing, device lab, chaos, supply chain, operations |

Three strong engineers can cover the first four with shared reliability work,
but a public cloud and cross-platform signed 1.0 need dedicated operational and
security capacity. Do not hide this work in “DevOps later”.

## Product scorecard

Review monthly:

### Developer

- median time to first simulated and physical print;
- API integration success without support;
- documentation search failures;
- webhook replay success;
- compatibility-test pass rate.

### Reliability

- durable-registration SLO;
- p95 notification and event latency;
- lost jobs;
- prevented duplicates;
- ambiguous handoffs per 100,000;
- agent crash-free sessions;
- update rollback rate.

### Efficiency

- agent idle RSS/CPU;
- installed size;
- server cost per connected agent and 1,000 jobs;
- database/object growth;
- support minutes per 1,000 jobs.

### Community

- active external contributors and maintainers;
- median first response/review time;
- issue close reason quality;
- certified/community-verified devices;
- security disclosure response;
- percentage of roadmap decisions made through public RFCs.

### Business

- self-host to cloud conversion without lock-in;
- trial to first physical print;
- retained active workspaces;
- gross margin including support;
- churn reasons;
- SLA performance.

## Things to refuse

- a second independent agent implementation for UI convenience;
- microservices before module boundaries are proven;
- features that silently weaken print semantics;
- “exactly once” marketing;
- a self-host build that lacks core reliability features;
- a proprietary cloud protocol;
- breaking APIs to move faster;
- unbounded support claims for all printers;
- unsigned opaque auto-updates;
- premature enterprise checklists that delay the five-minute print;
- copying PrintNode code, private protocols, branding, or documentation.

## Definition of success

The project is forward-looking when:

- a developer can understand and complete the integration in minutes;
- a small site can run locally forever;
- a company can self-host without the project cloud;
- the hosted version is the easiest and best-operated option, not the only
  complete option;
- an API request remains compatible for years;
- failures are explained rather than hidden;
- the agent is smaller and calmer than typical desktop middleware;
- the architecture can become cellular without being born as a distributed
  maze;
- community contributors can reproduce, test, inspect, and trust every release.
