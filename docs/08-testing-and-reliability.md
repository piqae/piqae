# Testing and reliability strategy

## Quality model

Reliability is not “the request returned 201”. The tested path is:

1. request validation and durable registration;
2. content integrity and retention;
3. routing across disconnects/restarts;
4. durable agent acceptance;
5. option mapping and rendering;
6. native spooler handoff;
7. spooler/device observation;
8. event/webhook delivery;
9. cleanup and recovery.

Every stage needs deterministic tests plus failure injection.

## Test layers

### Domain tests

Pure tests for:

- job-state transition legality;
- event ordering and projection;
- PrintNode-compatible state projection;
- retry classification;
- cancellation races;
- expiry;
- idempotency;
- page-range parsing;
- option validation;
- `copies` versus `qty`;
- per-printer order;
- content reference/retention;
- webhook scheduling;
- clock-skew handling.

Use property-based tests for state sequences, ID sets, pagination, page ranges,
and duplicate/reordered messages.

### Storage tests

Run every migration and repository operation against real SQLite/PostgreSQL:

- transaction rollback;
- WAL/crash recovery;
- outbox atomicity;
- duplicate constraints;
- lease expiry;
- concurrent claims;
- cleanup;
- disk-full/write-error behavior;
- old-version upgrade.

Avoid mocked SQL for correctness properties.

### Protocol tests

- golden fixtures for each message version;
- unknown optional and required fields;
- reconnect/resume from every event cursor;
- duplicates, gaps, reordering, and delayed acknowledgements;
- malformed/oversized frames;
- slow consumer and backpressure;
- key/certificate rotation;
- incompatible agent/server versions;
- proxy and TLS edge cases.

Build a deterministic fake control plane and fake agent for CI.

### Adapter contract tests

Every platform adapter runs the same semantic suite against a fake/native test
queue:

- inventory and stable identity;
- capability refresh;
- RAW byte preservation;
- option validation;
- native job ID capture;
- cancellation;
- job-state mapping;
- slow/hung printer calls;
- queue removal during submission;
- spooler restart;
- service account permissions.

### Renderer tests

Curate generated, redistributable PDFs for:

- embedded and missing fonts;
- barcodes and fine label lines;
- vector, image, transparency, clipping, and overprint;
- portrait/landscape/mixed size;
- crop/media/trim boxes;
- Unicode and right-to-left text;
- forms and annotations;
- encrypted/password-protected input;
- malformed and fuzzed input;
- very large pages and page counts;
- page ranges, rotation, fit, and DPI;
- CMYK/ICC and grayscale.

Compare rendered rasters with perceptual tolerances and inspect barcode
readability. Golden changes require review, renderer-version annotation, and
physical smoke tests.

## PrintNode differential compatibility suite

Use a dedicated PrintNode test account and non-physical/file-controlled queues.
For each public contract under implementation:

- generate requests from a fixture table;
- capture status, selected headers, JSON, ordering, and job states;
- run the clone with equivalent fixture resources;
- normalise IDs/timestamps;
- compare;
- store the observed reference version/date;
- record intentional differences.

Focus on edge cases:

- missing/null/empty options;
- invalid enums and types;
- duplicate IDs in sets;
- default pagination;
- cancellation timing;
- repeated idempotency keys;
- base64 whitespace and malformed data;
- URI authentication failures;
- offline client and expiry;
- webhook success/failure acknowledgement.

Do not automate high-volume physical printing against PrintNode. Use explicit
quotas and fixture printers.

## Virtual and file printer environments

### Windows

Maintain Windows VMs with:

- a controllable test print provider/port monitor or file-backed queue;
- Microsoft inbox virtual queues where suitable;
- representative vendor drivers installed without physical devices;
- service and interactive-user modes.

A purpose-built test adapter is useful but cannot replace real spooler tests.
Record raw spool data or raster output where licensing permits.

### Linux

- isolated CUPS instance;
- file/dummy backend;
- IPP test server/printer;
- driverless and legacy queues;
- forced job/printer state reasons;
- CUPS restart and database corruption scenarios.

### macOS

- current supported macOS versions on real or hosted Apple hardware;
- file/IPP test destinations;
- `launchd` lifecycle and notarised package tests;
- Intel/Apple Silicon coverage while both builds are supported.

## Physical device lab

Minimum useful matrix:

- Zebra-class ZPL label printer;
- DYMO/Brother label printer using a local driver;
- ESC/POS receipt printer with cutter/cash drawer test disabled by default;
- monochrome office laser with duplex/trays;
- colour inkjet/laser;
- network IPP Everywhere printer;
- USB printer on Windows and Linux;
- optional USB HID and serial scales.

For each, record:

- OS and architecture;
- connection type;
- driver name/version;
- firmware;
- supported/unsupported options;
- expected status depth;
- known quirks;
- approved fixture outputs.

Automated physical runs need paper/label budgets and safety limits on `qty`.

## Chaos and recovery matrix

Kill or disconnect at every durability boundary:

| Injection | Expected outcome |
| --- | --- |
| API dies before DB commit | Caller receives failure/timeout; retry with same key creates at most one job. |
| API dies after commit before response | Same key reveals conflict/recorded result per API contract; no second job. |
| Agent disconnects before claim | Job remains control-plane pending. |
| Disconnect during content download | Partial file resumes or is discarded; no local acceptance yet. |
| Power loss after local commit before ack | Agent re-sends acceptance; one local job. |
| Renderer crashes | Agent survives; bounded retry/classified failure. |
| Spooler unavailable before call | Safe retry/wait. |
| Power loss during native submission | Reconcile; use `delivery_uncertain` if acceptance cannot be proved. |
| Disconnect after spooler acceptance | Agent outbox later uploads state; no reprint. |
| OS spooler restarts | Reconcile by native ID/marker; status authority may degrade. |
| Control-plane DB failover | Existing agent jobs continue locally; new durable creates fail rather than falsely acknowledge. |
| Object storage unavailable | Uploaded creates do not acknowledge until content durability contract is met. |
| Disk full on agent | Stop claims, report health, preserve current state. |
| Clock jumps | Monotonic durations remain valid; wall timestamp correction is visible. |

Use fault-injection hooks compiled into test builds, not timing-only tests.

## Duplicate prevention tests

- same API key/idempotency key concurrently from many callers;
- duplicated server notifications;
- two overlapping sessions during upgrade;
- cloned agent database on another machine;
- agent restart at every submission line;
- spooler job ID reuse;
- `qty > 1` partial handoff;
- reprint command versus retry command;
- webhook replay.

The release gate is no silent automatic duplicate in ambiguous-handoff tests.
An explicit operator reprint is allowed and linked to the original.

## Performance and soak tests

Measure:

- idle agent RSS/CPU over 24 hours;
- reconnect storms;
- 1, 10, 100, and 1,000 installed queues;
- queue depth with small RAW jobs and large PDFs;
- concurrent printers with bounded renderers;
- 500-page PDF;
- slow URI origin and range resume;
- server p50/p95/p99 registration and dispatch;
- event propagation latency;
- PostgreSQL/outbox backlog;
- WebSocket memory per connection;
- log, event, and content cleanup over a 30-day accelerated soak.

Resource targets from the product requirements become CI or release-dashboard
budgets. Track regressions by build.

## Security tests

- API/agent authorisation matrix;
- cross-tenant object and integer-ID probing;
- SSRF with redirects, DNS rebinding, IPv4/IPv6 variants, metadata and private
  ranges;
- webhook target SSRF;
- malicious PDF corpus and continuous fuzzing;
- oversized base64/form bodies;
- decompression and JSON nesting limits;
- RAW scope/quantity limits;
- local UI CSRF/CORS/CSP;
- path traversal and symlink attacks in content cache;
- update signature/rollback/freeze;
- secret and document-log scanning;
- diagnostic bundle redaction.

Commission an independent review before exposing a public SaaS.

## Installer and upgrade matrix

Test clean install, silent install, repair, upgrade, downgrade policy, and
uninstall on supported OS versions.

Validate:

- service starts before login;
- one service instance;
- queue/database preserved across upgrade;
- old and new server protocol overlap;
- rollback retains compatible schema/state;
- printers accessible under actual service identity;
- proxy/custom CA configuration preserved;
- uninstall clearly offers to retain or remove local data;
- no orphan credentials or services.

## Release gates

### Alpha

- vertical print path on one Windows and one CUPS environment;
- durable local queue;
- no known duplicate on tested recovery paths;
- traceable states;
- signed development artifacts not yet required.

### Beta

- supported OS/architecture package matrix;
- API compatibility subset green;
- physical printer matrix green;
- security threat checklist;
- migration/rollback tests;
- resource targets close to budget;
- documentation and support bundles.

### 1.0

- self-use production soak;
- zero unresolved data-loss/duplicate critical defects;
- independent security review;
- signed/notarised installers and update chain;
- backup/restore rehearsal;
- public compatibility matrix;
- incident and release procedures;
- clear known limitations for physical completion.

