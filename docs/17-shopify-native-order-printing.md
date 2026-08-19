# Shopify-native order printing concept

**Status:** private-development implementation. No production support claim.

The current implementation includes Shopify Admin order/detail and bulk print
actions, a preview approval gate, PDF fallback, direct Piqae destinations,
durable tenant-scoped templates, four dynamic starters, a bounded Liquid adapter
and a ProseMirror semantic business-document editor. Preview,
download and print reuse one verified immutable artifact. The Shopify app can
explicitly target fake, local or live Piqae; selecting live does not authorize a
physical-printer test.

Public release remains blocked on Shopify approvals and real development-store
evidence, including protected customer data/scopes, extension placement,
customer-account network access, POS/mobile behavior, editor browser testing,
live Piqae/node delivery, and physical printer fixtures. Document packs,
full-fidelity images/custom fonts/barcodes, full Liquid/HTML/CSS, multilingual
and jurisdiction-specific documents are not implemented parity claims.

## Decision

Build a separate public Shopify app, provisionally called **Piqae Order Print**,
that feels like a native part of Shopify Admin and sends order documents directly
to a merchant's existing printers through Piqae. PDF preview, download, email,
and archival remain available; downloading a PDF is no longer a required step
before printing.

The app has two mutually exclusive connection modes:

1. **Use my Piqae account** — free Shopify app plan. The merchant authorizes an
   existing Piqae workspace and pays Piqae directly under its existing plan.
2. **Printing included** — paid through Shopify App Pricing. The app creates and
   owns an isolated Piqae platform child account whose credential and resources
   are usable only through this Shopify integration.

Do not describe the second mode as a normal Piqae subscription or expose a child
account credential. It is an app-scoped entitlement backed by a tenant-isolated
Piqae workspace. A merchant can later migrate it to a direct Piqae account using
an explicit, audited ownership-transfer flow; that transfer API is a launch
dependency if portability is promised.

## Product promise

> Select orders in Shopify, choose a document and printer, and print. No download,
> browser print dialog, or rework when a batch is large.

The product wins on four properties:

- a one-action path from Shopify Orders to an already configured destination;
- fast, asynchronous batch rendering with progressive results instead of one
  monolithic PDF request;
- deterministic document rendering and machine-checked barcode/QR output; and
- durable, observable printing with safe retry and duplicate protection.

“Reported complete” is the strongest normal success state. It is not proof that
ink reached paper. Never label spooler acceptance as “printed successfully.”

## Competitive baseline (verified 9 August 2026)

Order Printer Pro publicly lists the following baseline:

| Area | Baseline to match |
| --- | --- |
| Documents | Invoices, receipts, gift receipts, credit notes, quotes, draft orders, delivery notes, packing slips, refunds, returns |
| Output | Print, PDF export/download, bulk download, email automation, customer PDF links |
| Workflow | Individual and bulk selection, filters, Shopify Admin, mobile, and POS |
| Templates | Logo, colour, fonts, fields, invoice numbers, tax calculation, HTML/CSS/Liquid customization, barcodes |
| Commerce | Metafields, multi-currency, multi-language, VAT/tax, B2B, sequential numbering |
| Specialist | E-invoicing/Peppol add-on |

Its published price bands are free through 50 monthly orders, USD $10 for
51–500, $20 for 501–5,000, and $40 for 5,001 or more. Feature parity is necessary
but not the initial wedge. The wedge is dependable, near-immediate physical
printing without a PDF-download detour.

## Scope

### Release 1: the narrow excellent path

- Embedded Shopify App Home using the current App Bridge and Polaris web
  components.
- Admin order and draft-order actions for one or many selected orders.
- Packing slip, invoice, receipt, return form, and gift receipt.
- Direct print, preview, and PDF download from the same immutable render.
- One-click repeat of the last destination and template, with a confirmation
  summary for batches.
- Piqae computer/printer connection, driver-owned print settings, named
  destinations, readiness, queue, and recovery.
- Code-free branding editor plus an advanced sandboxed Liquid/HTML/CSS editor.
- Locale, currency, tax, discounts, refunds, duties, tips, notes, metafields,
  line-item properties, B2B company data, and partial fulfillment.
- QR and common 1D/2D barcode generation with render-time validation.
- Shopify App Pricing and bring-your-own-Piqae modes.
- Mandatory Shopify privacy, uninstall, shop-redact, and customer-redact
  lifecycle handling.

### Release 2: parity and automation

- Automated invoice email and customer-account download links.
- Credit notes, quotes, delivery notes, return labels, and configurable document
  packs such as “packing slip + return form.”
- POS and mobile extensions, location-default destinations, staff permissions,
  scheduled/fulfillment-event printing, sequential legal numbering, and a
  documented migration path for compatible Shopify Order Printer templates.
- Translation management and per-market/per-B2B template rules.

### Later or partner-led

- Country-specific fiscalization, qualified e-invoicing, and Peppol. These are
  compliance products, not template checkboxes, and require jurisdictional legal
  review, accredited delivery partners, immutable evidence, and separate support
  claims.
- Automatic printing triggered by new orders. Ship only with strict destination,
  order-status, fraud, payment, location, quantity, and rate-limit policies plus
  an obvious kill switch.

## The native Shopify experience

### Information architecture

Use Shopify's normal page patterns and keep global navigation shallow:

- **Home** — readiness, recent activity, and one primary setup/print action.
- **Templates** — document cards, code-free customization, preview, advanced
  editor.
- **Printers** — destinations and connection health.
- **Activity** — print attempts, exact status, failures, and safe replacements.
- **Settings** — defaults, billing mode, numbering, locales, email, retention.

Use tabs only for secondary navigation inside a page. Do not recreate Shopify's
sidebar, use a marketing dashboard inside App Home, or send merchants to Piqae
for routine operations.

### First run

Show a single setup checklist on Home:

1. **Brand documents** — prefill the shop logo/address and show a real order
   preview. This can be skipped because a polished default is already usable.
2. **Choose how to connect printing** — existing Piqae account or printing
   included with the Shopify plan.
3. **Connect a printer computer** — open a short-lived Piqae connection session,
   install/open the native node, select allowed printers, and wait for verified
   connection.
4. **Print a test page** — use the Test environment and a clearly identified
   virtual or user-approved fixture. Never print to physical hardware without an
   explicit printer and fixture confirmation.

Let merchants preview/download immediately. Ask them to connect a printer only
when they choose direct printing.

### Order workflow

From Shopify Orders, the merchant selects orders and invokes **Print documents**:

```text
Print 24 orders

Document        Packing slip
Destination     Packing station · Brother HL-L8360CDW · Ready
Copies          1 per order

24 documents · 38 pages
                              [Preview] [Print]
```

Remember a user's last valid choice per Shopify location, but always show the
resolved template, destination, order count, page count, and copies before a
bulk submission. Disable Print with a plain-language readiness action rather
than accepting a job that cannot run.

After submission, close the modal quickly and use a Shopify toast with a link:
“24 documents queued · View activity.” The activity page streams individual
progress. It does not block on one merged PDF and it lets successful items
continue when one order is malformed.

### Recovery instead of blind reprint

| Observed state | Merchant language | Primary action |
| --- | --- | --- |
| Registered/queued | Queued | View activity |
| Node accepted/printing | Printing | View details |
| Completed reported | Reported complete | Print another copy |
| Failed before acceptance | Not printed | Retry failed items |
| Delivery uncertain | Check the printer | Confirm outcome or print replacement |
| Printer/profile unavailable | Printer needs attention | Fix destination |

A retry repeats the same idempotent attempt only when safe. A merchant-requested
replacement creates a new linked attempt and audit record. Never turn a timeout
into an automatic duplicate physical print.

## Quality-of-life improvements

- **Instant first content:** show cached order rows and the shell immediately;
  render previews progressively and cancel stale requests.
- **Document packs:** one action can produce documents routed to different
  destinations, such as A4 invoice to office and 4×6 return label to dispatch.
- **Saved destination rules:** shop location, order tags, shipping method,
  market, product type, and B2B company can choose defaults; show the resolved
  rule before printing.
- **Exception-first batches:** complete valid orders, isolate failures, and offer
  “retry 2 failed” rather than regenerating all 500.
- **Preflight:** page size, missing assets/fonts, clipped content, blank pages,
  unsupported glyphs, barcode quiet zones, and target/profile compatibility.
- **Golden previews:** templates keep fixtures for tax, discounts, refunds,
  long addresses, CJK/RTL, many line items, QR codes, and missing optional data.
- **Explainable data:** selecting any preview field reveals its Shopify source
  and fallback without requiring Liquid knowledge.
- **Safe template publishing:** draft, compare, test, publish, and roll back an
  immutable revision; print attempts retain the exact revision and data snapshot.
- **Fast find:** searchable templates/destinations and sensible defaults rather
  than multi-step wizards for repeat work.
- **No surprise coupling:** PDF download works while a node is offline; direct
  printing queues durably according to explicit policy.

## Rendering and QR correctness

Render exactly once per `(shop, order data revision, template revision, locale,
currency, output profile)` and use the same immutable artifact for preview,
download, email, and print. Never use a browser screenshot as the production
document.

Pipeline:

```text
Shopify IDs from authenticated action
  -> bulk GraphQL fetch with bounded pagination/cost
  -> normalized immutable document model
  -> Shopify Liquid/visual source compiled into typed business-document nodes
  -> deterministic Piqae flow layout with content-addressed assets
  -> business-document PDF generation per order (parallel, bounded)
  -> PDF structural and visual preflight
  -> immutable object + SHA-256
  -> preview/download/email and/or idempotent Piqae print attempt
```

Generate QR/barcodes as vector SVG from validated payloads, preserve quiet zones,
and forbid arbitrary remote images or scripts in templates. Preflight rasterizes
at the destination's effective DPI, decodes every expected symbol from the
rendered page, and compares the decoded payload with the source. A missing or
incorrect symbol fails that document before printing. Maintain golden visual
tests across business-document format, PDF-library, font and renderer-profile
upgrades.

## Performance contract

Measure with warm and cold workers, realistic Shopify API latency, long orders,
custom fonts, and batches of 1, 50, 500, and 5,000 orders.

| Measure | Launch objective |
| --- | ---: |
| Embedded shell usable, p75 | < 1.0 s after iframe navigation |
| Single-order native render execution, warm p95 | < 300 ms after normalized data is available |
| Direct-print intent durably registered, p95 | < 250 ms after confirmation |
| First batch item ready, p95 | < 750 ms |
| 50 simple documents rendered, warm p95 | < 5 s |
| 500 simple documents rendered, warm p95 | < 30 s, progressive |
| Duplicate physical jobs caused by app retry | 0 |
| Expected QR/barcodes decoded in preflight | 100% |

These are objectives until load evidence exists, not marketing claims. Keep
Shopify API calls out of the critical print-confirmation path when a fresh,
webhook-maintained projection is available; reconcile by ID and revision before
rendering legal documents. Use bounded queues, per-shop fairness, admission
control, and backpressure so one 5,000-order batch cannot starve interactive jobs.

## Architecture and ownership

```text
Shopify Admin / POS / mobile extension
  -> Shopify session token
  -> Shopify app BFF
       - shop/staff authorization
       - Admin GraphQL + webhook projection
       - template/render/preflight service
       - immutable artifacts and audit records
       - Shopify billing entitlement
       - shop -> Piqae workspace mapping
  -> account-scoped Piqae API
       -> durable node -> installed OS driver -> printer
```

The browser never receives a Shopify offline token, Piqae platform key, Piqae
tenant-selection headers, webhook secret, enrollment token, document object
credential, or native profile payload.

Use immutable Shopify shop GID as the external Piqae account key, namespaced by
the app identity. Do not key isolation by shop domain because domains can change.
The BFF resolves the shop from a verified Shopify session and chooses Test or
Live; request data cannot choose a Piqae workspace/environment.

### Bring-your-own-Piqae

Use OAuth-style authorization or a short-lived server-created handoff that grants
this Shopify app only the chosen workspace/environment and required capabilities.
Do not ask merchants to paste a platform key. Record the granting Piqae actor,
Shopify shop, scopes, expiry/revocation state, and audit evidence. Uninstall
revokes the connector without deleting the merchant-owned Piqae workspace.

### Shopify-paid child account

The app backend holds one dedicated Piqae platform identity and calls platform
account get-or-create for the immutable shop external ID. Each shop receives
separate Test and Live environments, printers, nodes, documents, jobs, quotas,
webhooks, and audit history. Shopify staff roles remain app authorization;
Piqae tenant isolation does not replace them.

On uninstall: immediately revoke Shopify sessions and connector grants, stop new
jobs, let already durable jobs follow documented policy, archive the platform
account, unregister webhooks, and schedule Shopify/customer data deletion under
the declared retention policy. Archive is not a GDPR deletion primitive.

## Data minimization and Shopify access

Request the smallest Admin API scope set that implements the released features.
Do not copy the competitor's broad permissions. Prefer read-only order, draft
order, product/metafield, location, shop, and company data where possible; add
write scopes only for an explicit released feature such as attaching a document
reference. Declare protected customer data and minimize retention.

Use current stable GraphQL APIs, bulk operations for historical/backfill work,
and webhooks for incremental projection. Verify webhook HMAC against raw bytes,
deduplicate delivery IDs, handle out-of-order events, and implement mandatory
privacy webhooks. Pin API versions and run upgrade contract tests before each
version sunset.

## Pricing recommendation

Lead with simple Shopify-hosted plans and make bring-your-own-Piqae genuinely
free, not a crippled tier:

| Plan | Shopify price | Included monthly orders | Intended merchant |
| --- | ---: | ---: | --- |
| Connect | $0 | App features; printing billed by existing Piqae plan | Existing Piqae customer |
| Starter | $10/month | 500 | Small shop |
| Growth | $20/month | 5,000 | Growing operation |
| Scale | $40/month | 25,000, then transparent usage | High volume |

This matches the competitor's public $10/$20/$40 bands. Compete on value,
reliability, direct printing, and usability rather than signaling a cheaper
product. Scale gives a defined included allowance instead of an ambiguous
“5,001+” band; confirm whether matching their unlimited-order presentation is
commercially preferable after load and unit-economics evidence. Validate Shopify
revenue share, render/storage/email, Piqae reported-complete usage, support,
failed renders, and data egress before submission. Prefer Shopify App Pricing
for public plans and subscription lifecycle. Entitlement must be derived
server-side from Shopify's current subscription state, not a redirect parameter
alone.

Avoid charging by print attempt: it penalizes failures and creates distrust.
Use Shopify orders processed as the understandable plan meter, with direct-print
jobs metered internally for capacity. Publish exactly what counts when one order
produces multiple documents or is reprocessed.

## Built for Shopify strategy

Built for Shopify is an outcome after launch evidence, not a launch badge to
self-assign. The app should be designed for it from day one:

- embedded App Home with the latest App Bridge and Polaris web components;
- Shopify page patterns, title bar, contextual save bar, toasts, modals, and
  secondary tabs instead of a custom admin shell;
- admin actions/extensions where the task begins, with minimal context switching;
- fast loading, responsive/mobile layouts, accessible keyboard/screen-reader
  operation, localized UI, and no storefront/checkout performance impact;
- minimal scopes, protected-customer-data compliance, mandatory privacy webhooks,
  clean uninstall, clear pricing, accurate listing, and reliable support;
- automated monitoring of the live Partner Dashboard BFS criteria, because
  thresholds and requirements change and Shopify evaluates usefulness/reviews.

Do not claim that using Polaris alone makes the app Built for Shopify. Shopify
evaluates safety, performance, integration, ease of use, proven usefulness, and
listing quality, and the app must apply after prerequisite criteria are met.

## Release gates

### Gate 0 — feasibility

- Confirm app name/trademark and App Store category.
- Validate Admin API access for every normalized field and document fixture.
- Validate the direct-print connection UX with five merchants and at least two
  fulfillment locations.
- Load-test deterministic rendering and barcode decoding against the objectives.
- Complete pricing unit economics and data-retention/privacy review.

### Gate 1 — internal alpha

- Shopify development-store install/auth/uninstall and privacy lifecycle tests.
- Fake-printer end-to-end tests only: selection -> render -> preflight -> upload
  -> durable job -> webhook -> UI state -> idempotent retry.
- Tenant tests prove two shops cannot address each other's orders, artifacts,
  templates, printers, jobs, webhooks, usage, or connect sessions.
- Template sandbox escape, SSRF, oversized asset, Liquid resource exhaustion,
  malformed QR, webhook replay, session forgery, and billing downgrade tests.

### Gate 2 — private beta

- Piqae platform service accounts pass the repository support-matrix release
  evidence for tenant isolation, revocation, audit, redaction, native packaging,
  and fleet soak. Until then, the Shopify app is not production-ready.
- Ten design partners, real but explicitly authorized printers/fixtures, and
  documented rollback/support playbooks.
- At least 30 days of SLO, render-correctness, queue, billing, deletion, and
  restore evidence; no unexplained duplicate physical jobs.

### Gate 3 — App Store release

- Shopify app review, protected customer data approval, privacy policy, terms,
  support route, status page, incident process, subprocessors, and data deletion.
- Feature/listing/pricing claims match tested production behavior.
- Progressive rollout with per-shop kill switches for direct and automatic
  printing; PDF fallback remains available.

### Gate 4 — Built for Shopify application

- Partner Dashboard reports every current prerequisite met.
- Merchant usefulness/review thresholds are met organically.
- Run the current BFS checklist again immediately before applying; do not rely on
  this dated concept as the authority.

## Decisions still required

1. Separate repository/deployable or an `apps/shopify` workspace in this monorepo.
   Recommendation: monorepo initially for shared contracts and atomic tests, but
   separate service, database role, secrets, domain, and deployment.
2. App name and whether Piqae is merchant-facing at routine boundaries.
3. Exact bring-your-own authorization/transfer contract; neither exists as a
   supported public flow merely because platform accounts exist.
4. Template engine compatibility target and whether importing third-party
   templates is legally and technically supportable.
5. Legal invoice/numbering retention by launch country.
6. Email and customer-account surfaces in release 1 or release 2.

The detailed editor evaluation and verified surface-by-surface competitor matrix
are in [Order Printer Pro parity and editor assessment](research/order-printer-pro-parity.md).
The accepted provider-neutral format, rendering boundary, and thin Shopify
profile are in
[ADR-0004: Portable business-document format](architecture/adr-0004-core-document-engine.md).

## Sources checked for this concept

- Order Printer Pro product site and Shopify App Store listing, checked
  9 August 2026.
- Shopify developer documentation for App Home, App Bridge, Polaris web
  components, Shopify App Pricing, and Built for Shopify requirements, checked
  9 August 2026.
- Piqae platform accounts, integrator UX, web design integration, billing,
  printing state, and support-matrix documentation in this repository.

External requirements and competitor pricing are volatile. Re-verify them before
planning a release or making a public comparison.
