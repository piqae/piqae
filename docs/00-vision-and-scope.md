# Vision, principles, and scope

## Executive summary

The opportunity is to build a smaller and more transparent print relay with
three deployment modes:

1. **Local-only:** an application submits directly to an agent on the same
   machine or LAN. The durable queue lives on the agent. No hosted service is
   required.
2. **Self-hosted remote:** a lightweight control plane accepts jobs, retains
   them while agents are offline, routes them over outbound connections, and
   receives live state. The customer owns all infrastructure and data.
3. **Hosted SaaS:** the same control plane is operated as a multi-tenant
   service. The experience should be as straightforward as Stripe: create a
   workspace, create an API key, install and enrol an agent, choose a printer,
   and submit a job.

This is not primarily a “cloud printing” problem. It is a distributed systems
problem joined to inconsistent native printing APIs and printer drivers. The
networking portion is relatively conventional; correct PDF rendering, option
mapping, spooler reconciliation, installer quality, and recovery after crashes
are the differentiators.

## Product promise

> Print PDF or RAW jobs through the drivers already installed on a remote
> computer, without browser dialogs, with durable queues, explicit states, and
> enough diagnostics to explain every failure.

The project should offer:

- legacy-compatible REST resources for the common integration path;
- first-class modern APIs that improve state detail and authentication;
- local printer discovery through the operating system;
- PDF and RAW printing;
- local-driver options such as paper, tray, copies, duplex, colour, DPI,
  collation, page selection, scaling, media, N-up where available, and
  rotation;
- Windows, macOS, Linux, and ARM Linux support;
- a background service and a genuinely headless installation;
- live agent, printer, queue, print-job, and optional scale data;
- durable offline operation;
- end-to-end request and trace identifiers;
- a clean self-hosting story and an optional hosted service.

## Principles

### Local first, remotely manageable

The agent must remain useful without the internet. Remote control is layered on
top of local printing rather than being a prerequisite for it.

### Use the operating system and installed drivers

Do not build a printer-driver catalogue. Enumerate the printers and options
known to Windows or CUPS and submit through those systems. This is the basis of
The legacy service's broad hardware compatibility and is the only practical route to the
same coverage.

### Durable before fast, then make durable fast

An acknowledged job must survive process restarts and power loss according to a
documented durability level. Sub-100 ms routing is useful; returning quickly
and then losing the job is not.

### Honest state

“Accepted by the OS queue” and “physically printed” are not equivalent.
Different devices expose different levels of truth. Each state must say which
component observed it and what it proves.

### No silent duplicate printing

Printing is an external side effect and cannot be made mathematically
exactly-once across every crash boundary. The system must prevent duplicates
where it can and surface an explicit ambiguous state where it cannot. Retry
policy is a product decision, not an invisible implementation detail.

### One agent core, thin platform adapters

Maintain one protocol, queue, configuration, update, logging, and security
implementation. Isolate Windows and CUPS behavior behind native adapter
interfaces. A separate full application per platform would multiply
reliability work and allow semantics to drift.

### Optional UI

The service, CLI, configuration file, and API are the product. A Svelte UI is a
convenience layer. Printing must not depend on a desktop session or embedded
browser.

### Operational simplicity

The initial self-hosted deployment should be one control-plane process,
PostgreSQL, and optional S3-compatible storage. Kafka, Redis, NATS, and a
microservice fleet are not initial requirements.

## What “drop-in” means

The compatibility goal is:

- preserve legacy resource shapes, endpoint paths, important response
  headers, authentication style, pagination, option names, idempotency
  behavior, and stable job states;
- allow an existing integration or official SDK to use a different base URL;
- keep modern extensions namespaced so they do not change compatibility
  responses;
- publish a tested compatibility matrix rather than make an unbounded claim.

Some official legacy-provider SDKs allow the API base URL to be configured. Others
hard-code a hard-coded third-party API origin or expose host changes awkwardly. We should
provide maintained replacement SDKs and a migration guide; DNS or TLS
impersonation of the legacy service is not a supported migration technique.

## What “feature parity” does not mean

It does not mean:

- incorporating a third party's proprietary client, protocol, branding, or rendering
  engines;
- guaranteeing identical raster output for every driver combination;
- declaring a page printed when the operating system cannot prove that;
- supporting mobile operating systems in the first release;
- scanning the LAN for printers by default;
- building billing and a marketing site before the print path is reliable.

Public API behavior can be matched closely. Output fidelity must be established
through differential and physical testing.

## Scope layers

### Layer A: replacement for our own usage

- RAW and PDF jobs;
- URI and uploaded content;
- required print options;
- installed-printer discovery;
- Windows plus whichever other operating systems are actually in use;
- job history, live state, and diagnostic bundles;
- local and self-hosted remote deployment.

### Layer B: public legacy-compatibility parity

- all documented print options and content modes;
- compatible computers, printers, jobs, states, cancellation, webhooks,
  pagination, errors, idempotency, and API-key authentication;
- signed installers and unattended/headless deployments for all supported
  platforms;
- SDKs and examples.

### Layer C: broader platform parity

- scale support over HTTP and WebSocket;
- child accounts, tags, API-key management, delegated authentication, and
  branding;
- multi-tenant controls and SaaS billing;
- integration marketplace connectors.

### Layer D: deliberate improvements

- richer job and printer states;
- pause, resume, retry, and policy-controlled cancellation;
- queue inspection at both agent and OS levels;
- resumable content transfer;
- stronger webhook signatures and retry policies;
- OpenTelemetry traces and metrics;
- fleet policy, staged updates, and remote diagnostics;
- optional IPP/SNMP device detail.

## Success measures

The initial product is successful when:

- it replaces the organisation's legacy-provider bill without reducing reliability;
- an existing integration changes only base URL and API credentials for the
  supported compatibility subset;
- an agent can be offline, reconnect, and safely process retained jobs;
- every job has a complete event trail from API acceptance to the deepest
  status the printer stack can report;
- a support engineer can diagnose most failures from a redacted bundle without
  remote desktop access;
- agent idle resource consumption is materially below the current legacy-provider
  client;
- installs and upgrades do not cause duplicate agents or duplicate jobs.
