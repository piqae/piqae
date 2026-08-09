# ADR-0003: Shopify document rendering architecture

Status: proposed performance plan. ADR-0004 supersedes its initial choice of a
TypeScript/pdfme production renderer with a smaller provider-neutral Document
Spec compiled by a Rust/Typst engine. Retain the workload, scheduling, caching,
isolation and release gates here; re-run them against both candidates before the
renderer decision is accepted.

## Context

Piqae Order Print must feel faster and more dependable than established Shopify
document applications from its first public release. The critical experience is
not merely total batch completion time. A merchant must see the app immediately,
confirm a print quickly, receive the first useful result quickly, and understand
the progress or failure of every document without causing a duplicate print.

The selected editor foundation is pdfme. Templates use a restricted
Shopify-style Liquid profile for data and logic plus pdfme-derived schemas for
physical pages and presentation. Piqae's existing Rust control plane and durable
native agent remain authoritative for printer delivery.

No language or library can substantiate “fastest.” Only a versioned workload,
competitor measurements obtained lawfully, production percentiles, and sustained
load evidence can support a comparative claim.

## Superseded decision record

The following hybrid decision is retained as historical design input only. It
is not the current production renderer choice. ADR-0004 selects the bounded
`piqae.document/v1` contract and deterministic Rust renderer; pdfme is an editor
and explicit conversion input, or a client-side renderer for callers that send
finished PDF bytes. Hosted Piqae conversion never runs pdfme or Liquid.

Use a deliberately hybrid architecture:

- **TypeScript and React Router** for the public embedded Shopify application,
  current App Bridge, Polaris web components, Admin API integration, editor and
  Shopify extensions.
- **Historical proposal:** TypeScript on pinned Node.js 22 for Liquid and pdfme
  generation. This was superseded before acceptance and must not be interpreted
  as the deployed native render path.
- **Rust** for the existing Piqae tenant, upload, durable print job, webhook and
  native delivery state machines. The Shopify application calls the supported
  account-scoped Piqae API rather than importing control-plane internals.
- **PostgreSQL** for durable state and transactional outboxes. The thin Shopify
  backend owns installations, token rotation, billing and normalization cursors;
  the provider-neutral Piqae Documents plane owns templates, immutable revisions,
  render intents/attempts and artifact metadata as refined by ADR-0004.
- **S3-compatible object storage** for content-addressed templates/assets and
  rendered artifacts. Do not put PDF bytes or large images in PostgreSQL.

Start as two TypeScript deployables sharing versioned internal packages:

1. a latency-sensitive thin Shopify web/BFF process; and
2. an independently scalable, non-public Piqae Documents render-worker process.

This is a modular monolith split at a resource and failure boundary, not a fleet
of small network services. The worker has no public HTTP API. It leases durable
render tasks from PostgreSQL, writes immutable objects, commits results and emits
an outbox event. The BFF remains responsive when rendering is saturated or a
malformed document kills a worker.

## Why TypeScript is the render language

pdfme's generator and schema/plugin model are TypeScript and run in Node and the
browser. LiquidJS is also TypeScript/JavaScript-native. Keeping the primary
renderer in Node provides:

- one plugin implementation across designer preview, tests and server output;
- no serialization/FFI cost between Liquid, layout, fonts and PDF generation;
- faster delivery of pdfme fixes and upstream releases;
- a smaller parity risk than reimplementing pdfme semantics in Rust; and
- one canonical code path for QR/barcode schemas and font measurement.

Rust may later replace an isolated, measured hotspot behind a stable interface,
such as image normalization, barcode decoding or PDF concatenation. Do not start
with a Rust rewrite based on an assumption. Require a profiler showing that the
candidate operation materially affects p95 cost and prove byte/render
compatibility before changing it.

Node worker threads are suitable for CPU-intensive JavaScript, but starting one
per task costs more than it saves. Use warm pools when profiling proves useful.
Because merchant templates, fonts, images and order data are untrusted resource
inputs, the primary isolation boundary is a replaceable worker process/container,
not a thread inside the public BFF. Node thread heap limits do not bound all
external allocation and cannot make the whole host immune to OOM.

## Request and render path

```text
Shopify Admin action
  -> Shopify BFF authenticates session + staff/shop authorization
  -> resolve immutable order GIDs, template revision and destination
  -> register one render/print intent transactionally
  -> return intent and live progress immediately
  -> PostgreSQL task lease
  -> warm Node render worker
       1. load normalized order snapshot
       2. load compiled template plan and pinned assets/fonts
       3. evaluate bounded Liquid expressions
       4. expand pdfme flow/page schemas
       5. generate one immutable PDF per order/document
       6. structural preflight + expected barcode/QR decode
       7. hash and PUT artifact only if absent
       8. commit result + outbox event
  -> direct print path creates idempotent Piqae upload/job
  -> preview/download/email reuse the exact same artifact
```

Do not wait for an entire batch before registering ready documents. A 500-order
batch is 500 independently observable document renders grouped by one intent.
Schedule interactive single-order/direct-print tasks ahead of background merged
exports while retaining per-shop fairness and bounded starvation.

## Monorepo structure

Extend `pnpm-workspace.yaml` with `packages/*` and keep deployables distinct from
libraries:

```text
apps/
  shopify/                   public React Router app and server BFF
  render-worker/             private Node task worker; no public ingress

packages/
  shopify-domain/            installation, roles, billing and intent types
  shopify-admin/             pinned GraphQL queries, webhooks and normalization
  order-document-model/      versioned normalized Shopify document data
  liquid-profile/            parser, strict profile, objects, tags and filters
  pdf-template/              Piqae template format and pdfme adapter
  pdfme-schemas/              flow, table, condition, page, rich text, QR plugins
  document-renderer/         deterministic render and artifact manifest API
  document-preflight/        PDF structure, fonts, geometry and code decoding
  order-printer-import/      legacy HTML/CSS/Liquid analysis and conversion
  render-test-corpus/        fixtures, expected semantics and visual goldens

crates/
  ...                        existing Piqae control-plane and agent crates

contracts/
  internal/
    render-task-v1.schema.json
    render-result-v1.schema.json
```

Do not place Shopify routes, billing or Liquid in `apps/web`; Piqae's dashboard
and the public Shopify app have different authentication, navigation, release and
failure boundaries. Do not let `apps/shopify` import Rust control-plane database
tables or reach into its storage. Use the public Piqae SDK/API so Shopify-paid
and bring-your-own modes exercise the same supported contract.

Keep package dependencies acyclic:

```text
order-document-model <- liquid-profile <- pdf-template <- document-renderer
                              ^                 ^                |
shopify-admin ---------------+           pdfme-schemas          v
                                                     document-preflight
```

UI/editor code may depend on the format/parser packages. Server packages must not
depend on React, Polaris, App Bridge or browser globals.

## Data model and durable responsibility

Use immutable identifiers and revisions:

- `print_intent_id`: one merchant confirmation;
- `render_item_id`: one document for one order and template revision;
- `order_snapshot_revision`: normalized data content hash/version;
- `template_revision_id`: immutable published template;
- `render_profile_id`: renderer/pdfme/Liquid/font/plugin version set;
- `artifact_sha256`: exact output bytes;
- `piqae_job_id`: one physical print attempt;
- `replacement_of`: explicit new attempt after operator decision.

The render idempotency key is derived from the semantic input, not a timestamp:

```text
SHA-256(
  shop_id || order_snapshot_revision || template_revision_id ||
  locale || currency || render_profile_id || document_variant
)
```

Copies and physical destinations generally do not change the PDF artifact. They
belong to the print intent/job. Never include a random retry ID in the artifact
key. A cached artifact is usable only after its manifest, length, digest,
preflight version and tenant authorization have been verified.

Register render intent and task/outbox rows in one PostgreSQL transaction. Lease
tasks with bounded duration and heartbeat; a crashed worker makes the task
eligible again. Result commit is compare-and-set/idempotent. Creating the Piqae
print job is a separate durable step with its own stable idempotency key. No
worker crash may silently create a second physical job.

## Performance strategy

### Remove work before optimizing code

1. Maintain an incremental, webhook-driven normalized projection of recently
   relevant Shopify order data. Reconcile freshness before issuing legal output,
   but do not build every document from dozens of serial Admin API calls.
2. Fetch Shopify data in bounded GraphQL batches and request only normalized
   fields required by the published template capabilities.
3. Compile and validate each Liquid/template revision once. Cache a safe compiled
   render plan by revision and render-profile version.
4. Load, validate, subset and cache fonts/assets when a revision is published,
   not on every job. Never fetch a Google font or merchant URL in a render task.
5. Render once and reuse the exact content-addressed artifact for preview,
   download, email and print.
6. Generate one PDF per order/document. Merge only when the merchant explicitly
   requests a combined download, and do it after individual outputs are useful.
7. Decode an expected QR/barcode once per unique artifact and store the preflight
   evidence; do not repeat it for identical downloads or copies.

### Warm worker design

- Keep a minimum warm worker count in each production region/AZ.
- Use one render process per allocated CPU core as the initial safe default.
- Each process handles one CPU-heavy render at a time until measurement proves a
  higher concurrency is faster without tail-latency or memory collapse.
- Preload pinned generator/plugin code and common fonts at process start.
- Use bounded LRU caches for compiled plans, decoded fonts, base PDFs and assets;
  key every entry by content hash and renderer version.
- Transfer `ArrayBuffer`s rather than copying them when an internal thread pool
  is used.
- Recycle a process after a bounded job count, heap threshold, timeout, malformed
  input or invariant failure.
- Cap input count, pages, elements, loop iterations, images, font bytes, output
  bytes, wall time and memory. Fail the specific document with an actionable
  reason; do not stall the shop queue.

### Scheduling

Maintain separate logical priorities:

1. direct-print single/interactively selected documents;
2. interactive previews and downloads;
3. remaining items in interactive batches;
4. automated email/customer-link generation;
5. historical bulk exports and merged archives.

Apply weighted fair scheduling by shop, per-shop concurrency limits, global
admission control and backpressure. A Scale merchant can receive greater weight,
but no one shop can occupy every renderer. Rate limiting must distinguish
idempotent retries from new work.

### Preview path

The editor may show an immediate local pdfme preview using redacted fixture/order
data, but label draft/local state if the server has not validated it. Debounce
and cancel stale server preview generations. Publishing always runs the exact
server renderer and the full fixture/preflight suite. Production output never
uses a browser-created PDF.

## Performance objectives and budgets

Record queue delay separately from execution. Publish p50, p75, p95 and p99 by
fixture class, worker version, region and cache state.

| Measure | Proposed release gate |
| --- | ---: |
| Embedded shell usable, p75 | < 1.0 s |
| Print intent durable response, p95 | < 250 ms excluding Shopify navigation |
| Warm simple one-page render execution, p95 | < 300 ms |
| Warm 10-page/200-line render execution, p95 | < 1.5 s |
| First item of a 50-order batch ready, p95 | < 750 ms |
| Entire 50-order simple batch ready, warm p95 | < 5 s |
| First item of a 500-order batch ready, p95 | < 1 s |
| Entire 500-order simple batch ready, warm p95 | < 30 s |
| Cached artifact lookup to authorized URL, p95 | < 100 ms |
| Expected QR/barcode decode success | 100% or artifact fails preflight |
| Duplicate Piqae jobs caused by retry | 0 |

These are proposed gates, not public claims. Tighten or revise them only from
repeatable measurements. A fast average does not compensate for a bad p99,
crashed worker, missing glyph, invalid QR code or duplicate print.

## Benchmark and regression system

Create `packages/render-test-corpus` before implementing the full editor. It must
include versioned, non-personal fixtures:

- 1, 20 and 200 line items;
- one and 500-order batches;
- product images at bounded realistic sizes;
- no images, cached images and cold asset decode;
- Latin, CJK, Arabic/RTL and mixed glyph runs;
- discounts, duties, tips, inclusive/exclusive taxes and multiple currencies;
- refunds, partial fulfillment, bundles and B2B/PO/VAT data;
- long addresses, notes, metafields and missing optional values;
- QR, Code 128 and other supported symbologies;
- A4, Letter, A5, 4×6 and custom geometry;
- explicit and automatic page breaks; and
- adversarial bounded Liquid/templates and malformed assets.

For every commit affecting Liquid, schemas, fonts or rendering:

- run semantic assertions and PDF structural checks;
- decode all expected machine-readable codes;
- render pages to pinned images and compare perceptual plus targeted regions;
- benchmark cold and warm p50/p95 with CPU time, wall time, peak RSS, bytes and
  page count;
- fail CI on correctness regressions and flag statistically material performance
  regressions against a controlled runner baseline.

Nightly and release runs exercise sustained mixed load, worker termination,
lease expiry, Postgres/object-store latency, cold deployment, autoscaling lag and
one noisy shop. Capture flamegraphs/CPU profiles and heap snapshots on the exact
slow fixtures before optimizing.

Competitor comparisons must use an authorized test store, identical legal input
and output requirements, disclosed network conditions and multiple repetitions.
Compare time-to-first-useful-document, total batch, failure recovery and output
correctness—not just a cherry-picked stopwatch result.

## Reliability and deterministic output

- Pin Node, pdfme, Liquid implementation, plugins, PDF library, fonts and Unicode
  data in `render_profile_id`.
- Disallow runtime network and dynamic module access inside the render process.
- Normalize time zone, locale, numeric rounding, sort stability and metadata.
- Where the PDF library permits, remove volatile creation IDs/timestamps or keep
  them outside semantic/visual comparison.
- Use decimal/minor-unit money types; never JavaScript floating point for
  accounting calculations.
- Store the normalized order snapshot and all source/provenance needed to explain
  an issued document under the declared retention policy.
- Scan/normalize uploads before use; bound decompression, pixels, fonts and PDFs.
- Treat a worker timeout/OOM as a render failure with a lease retry policy. It is
  never evidence that physical printing failed or should be duplicated.

## Alternatives considered

### Rewrite the renderer in Rust now

Rejected initially. Rust can offer excellent resource control and native speed,
but would require recreating pdfme layout, schemas, plugin behavior, fonts and
output compatibility. That is high schedule and parity risk without benchmark
evidence that JavaScript generation is the bottleneck. Rust remains an option
for isolated profiled hotspots.

### Run rendering in the Shopify BFF

Rejected for production. CPU/memory saturation and malformed template failures
would damage OAuth, Admin UI and webhook latency. It also prevents independent
autoscaling and rollout/rollback of the render plane.

### Use browser/Chromium HTML-to-PDF as the native renderer

Rejected for native templates. Browser startup, CSS/font/network variability,
larger memory and HTML compatibility complexity work against deterministic
low-latency output. A separately isolated Chromium compatibility worker may be
required for legacy Order Printer HTML/CSS/Liquid templates; it must not become
the fast native path.

### Introduce Kafka/NATS/Redis immediately

Rejected until measured PostgreSQL task/outbox contention requires it. A durable
PostgreSQL lease/outbox keeps registration and scheduling understandable and
atomic. A broker can later carry wakeups while PostgreSQL remains authoritative.

## Archived acceptance gates for the superseded proposal

These gates explain why TypeScript/pdfme was not accepted as the native renderer
and remain useful when evaluating client-side editor fidelity:

1. implements Liquid value, condition, repeated flow, commerce table, page
   regions and QR schemas without an unmaintainable pdfme fork;
2. renders the full corpus deterministically on pinned Node 22;
3. meets the proposed warm single and 50-order objectives on the intended
   production CPU class;
4. survives malformed inputs, worker kill and lease recovery without duplicate
   artifacts or Piqae jobs;
5. proves 200-line pagination, repeating headers, CJK/RTL and barcode decoding;
6. records profiles showing the actual dominant costs; and
7. documents any required pdfme patches and the upstream/fork decision.

If the spike misses performance gates, optimize measured costs in this order:
data fetching/projection, duplicate work/caching, font/image decode, Liquid plan,
pdfme schema/layout, PDF serialization, barcode validation, scheduling. Consider
a Rust replacement only for a remaining isolated hotspot with a stable contract.

## Current external references

- Shopify recommends its React Router template for most public embedded apps:
  [Scaffold an app](https://shopify.dev/docs/apps/build/scaffold-app).
- pdfme documents separate generator/UI packages and Node/browser generation:
  [pdfme getting started](https://pdfme.com/docs/getting-started).
- Node documents worker threads for CPU-intensive JavaScript and recommends a
  pool rather than a new thread per task:
  [Node.js worker threads](https://nodejs.org/download/release/v22.11.0/docs/api/worker_threads.html).

Re-check current Shopify, Node and pdfme requirements before implementation.
