# Decisions, risks, and open questions

## Recommended decisions

| Topic | Recommendation | Status |
| --- | --- | --- |
| Product shape | Local agent plus optional self-hosted/hosted control plane | Proposed |
| Agent implementation | One Rust core compiled natively for each OS, with thin Windows/CUPS adapters | Accepted |
| UI | Optional SvelteKit UI; system browser; CLI always available | Proposed |
| First control plane | SvelteKit server routes on Vercel; extract a Go agent gateway only when required | Accepted for MVP |
| Agent queue | SQLite WAL plus content files | Proposed |
| Server queue | PostgreSQL job/outbox tables, not Kafka/Redis initially | Proposed |
| Content transport | Resumable HTTPS; WebSocket for control/status only | Proposed |
| PDF engine | Sandboxed PDFium first candidate | Validate in spike |
| Windows printing | Winspool/PrintTicket plus GDI/XPS evaluated in spike | Open |
| CUPS printing | Documented CUPS/IPP APIs, native PDF when possible | Proposed |
| Completion semantics | Honest observer-attributed states | Proposed |
| Ambiguous handoff | No automatic retry by default | Proposed |
| Compatibility | Public REST behavior, not proprietary wire protocol | Proposed |
| Licence | Apache-2.0 agent/protocol/SDK; AGPL-3.0 control plane/UI | Recommended, pending legal review |
| Mobile | Not initial scope | Proposed |
| Scales | Later module unless internal workload requires it | Proposed |

The Go-first 48-hour evaluation was rejected after the objective was clarified
as the best long-term endpoint architecture built rapidly with parallel AI
implementation. See
[14-long-term-native-architecture-and-48-hour-production.md](14-long-term-native-architecture-and-48-hour-production.md).

## Top risks

### 1. Windows PDF fidelity

Severity: critical.

Local drivers accept different spool formats and expose options inconsistently.
PDFium rendering plus GDI may differ from PrintNode's proprietary engines in
rotation, scaling, fonts, colour, and label dimensions.

Mitigation:

- phase-one physical spike;
- multiple selectable but understandable backends;
- driver/renderer compatibility database based on test evidence;
- golden and barcode tests;
- do not claim universal fidelity before certification.

### 2. Exactly-once expectation

Severity: critical.

OS spool submission cannot be atomically committed with agent state. A crash
can make acceptance ambiguous.

Mitigation:

- persist intent;
- deterministic job marker;
- native queue reconciliation;
- no automatic uncertain retry;
- printer-specific retry policy;
- explicit API semantics and UI.

### 3. Misleading physical status

Severity: high.

Some spoolers/port monitors report completion before physical output.

Mitigation:

- observer/authority on every event;
- distinguish spooler acceptance and reported completion;
- optional IPP/SNMP enrichment;
- never promise proof unavailable from hardware.

### 4. Native driver instability

Severity: high.

Enumeration/capability/printing calls may hang or crash because of network or
vendor driver behavior.

Mitigation:

- bounded blocking pools or helper processes;
- timeouts and circuit breakers per queue;
- fast shallow enumeration then lazy detail;
- agent core isolation from renderer/driver failures;
- device lab.

### 5. Cross-platform scope

Severity: high.

“Windows, macOS, and Linux” is many OS versions, architectures, CUPS variants,
driver models, service identities, signing systems, and installer paths.

Mitigation:

- publish a narrow supported matrix;
- architecture-specific CI and real-device tests;
- common adapter contract;
- community-supported versus certified distinction.

### 6. Untrusted document and URL input

Severity: high.

PDF parsing and URI fetching create RCE/DoS/SSRF exposure near customer
printers and private networks.

Mitigation:

- renderer process sandbox;
- resource limits/fuzzing/security updates;
- URI policy and redirect/IP validation;
- no network in renderer;
- independent security review.

### 7. “Drop-in” overclaim

Severity: medium/high.

Public documentation does not describe every error or SDK assumption.

Mitigation:

- endpoint-by-endpoint differential tests;
- explicit versioned matrix;
- migration SDKs;
- separate native extensions;
- intentional differences documented.

### 8. Project economics

Severity: medium/high.

Engineering, code signing, hardware QA, and on-call cost may exceed the current
PrintNode bill if the goal remains internal savings only.

Mitigation:

- calculate current total spend and operational dependency;
- deliver the internal subset first;
- decide whether open-source/SaaS is strategic or incidental;
- stop at a reliable internal tool if public-market economics are weak.

## Workload questionnaire

Complete before Phase 1:

### Current use

- Monthly PrintNode bill:
- Jobs per day/month and peak per minute:
- Average/p95/max document size:
- PDF versus RAW percentage:
- URI versus base64 percentage:
- Required `expireAfter`/offline duration:
- Required ordering:
- Current SDK/language/version:
- Webhooks currently consumed:
- Scale usage:
- Integrator/child-account usage:

### Platforms

- Windows versions/architectures:
- Service versus logged-in application:
- macOS versions/architectures:
- Linux distributions/architectures:
- Raspberry Pi or other low-power devices:
- Proxies/custom CAs:
- Air-gapped/offline sites:

### Printers and drivers

- Manufacturer/model:
- USB/network/shared/virtual:
- Driver name/version:
- Print language for RAW:
- Required trays/papers/media:
- Required DPI/duplex/colour/copies/collation:
- Label/receipt cutters or cash drawers:
- Known current PrintNode engine setting/workaround:

### Reliability

- Cost of a missed print:
- Cost of a duplicate print:
- Preferred uncertain-handoff policy:
- Maximum acceptable status latency:
- Is OS spooler acceptance enough:
- Is device-reported completion required:
- Retention/compliance requirements:

## Product questions

1. Is the primary goal internal cost removal, a community project, or a
   commercial SaaS? This changes licence, polish, and roadmap.
2. Is public API compatibility required at 1.0, or only the subset used
   internally?
3. Are scales part of current usage?
4. Which OS must be first? Windows is likely, but should be confirmed.
5. Is a hosted server ever allowed to hold document bytes?
6. Should URI printing be unrestricted for compatibility or allow-listed by
   default for security?
7. Is the default uncertain-handoff policy at-most-once (avoid duplicate) or
   at-least-once (avoid missing)?
8. How much job/content history should exist?
9. Does a reprint require retaining content or resubmission by the caller?
10. Are custom-branded installers actually required, or is configuration-driven
    branding enough?

## Technical questions for spikes

### Windows

- Which PDF submission path best preserves the critical drivers and options?
- Can deterministic metadata be recovered after a rapid completed job?
- Which printer change notifications are reliable for remote queues?
- What service identity is required for existing network printers?
- Can the renderer be sandboxed without breaking driver access?

### CUPS/macOS/Linux

- Which queues accept PDF directly and which need raster fallback?
- What status/subscription depth is available on the actual fleet?
- Are RAW queues already configured?
- Does Apple's CUPS behavior require a distinct macOS adapter?

### Distributed system

- At what point may server content be deleted?
- How long can an agent claim remain valid?
- Can two sessions overlap during zero-downtime agent upgrade safely?
- Which event history is authoritative after control-plane restore?
- Is PostgreSQL alone sufficient at expected peak connection/job volume?

## Legal and project questions

- Select a non-infringing name and branding.
- Obtain counsel on API compatibility in intended jurisdictions.
- Do not use PrintNode binaries, private protocol captures, trademarks, or
  copied documentation/code.
- Review PDFium and every bundled native dependency licence.
- Decide contributor licence agreement or Developer Certificate of Origin.
- Publish security disclosure and supported-version policies.
- Decide Apache-2.0 versus AGPL strategy before accepting outside
  contributions.

## Definition of “almost simpler”

Complexity still exists, but users should see:

- one daemon;
- one config file or enrol command;
- one base URL;
- one API key;
- printers appear automatically;
- one create-job call;
- one understandable job timeline;
- one diagnostic bundle.

Implementation complexity belongs behind those boundaries, not in the
installer or integration guide.
