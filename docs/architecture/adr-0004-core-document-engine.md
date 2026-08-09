# ADR-0004: A core Piqae document engine with a thin Shopify profile

Status: accepted for the bounded phase-one contract and native Rust renderer;
public support remains gated by the support matrix and release audit.

## Decision

Build document generation as a completely optional, provider-neutral Piqae
capability, provisionally **Piqae Documents**, rather than implementing the
renderer inside the Shopify application.

The default authoring contract is **Piqae Document Spec v1**, a small declarative
JSON document model. It is neither Liquid, HTML nor executable raw printer
commands. Its phase-one reference implementation is a deterministic, bounded
Rust renderer that writes PDF directly without executing another template
language. Typst remains a possible future backend/profile only after its wider
language and resource surface has a separate threat model.

Offer two explicit advanced/profile inputs above the default spec:

- **Raw Typst bundle** for trusted advanced SDK/server users after a bounded
  package/import profile and threat model exist; and
- **Liquid profile** as an optional data/logic compiler into Document Spec,
  primarily for Shopify/Order Printer compatibility.

The base print API remains PDF/RAW passthrough. No document field, renderer flag,
template ID or feature negotiation is required unless a caller chooses the
optional Documents API/content type.

Piqae Documents owns:

- an open, versioned template bundle format;
- Piqae Document Spec over caller-provided typed JSON;
- a pinned deterministic Rust renderer for physical layout and PDF output;
- optional restricted Liquid and raw-Typst profile compilers;
- asset/font packaging, template compilation and immutable revisions;
- deterministic PDF rendering, preflight and QR/barcode evidence;
- content-addressed artifact caching;
- asynchronous batch rendering and progress;
- render-to-download and render-to-idempotent-print composition;
- SDK types, validators, local preview tooling and fake-render fixtures; and
- hosted and self-hosted implementations with the same contract.

The Shopify application owns only Shopify-specific responsibility:

- installation, Shopify session/staff authorization and scopes;
- expiring offline token/refresh-token lifecycle;
- Admin GraphQL queries and webhook normalization;
- Shopify billing/subscription projection;
- Shopify order, draft, B2B, tax, locale and metafield mapping;
- Admin, POS, mobile, Customer Account and Order Status extensions;
- shop-to-Piqae account/grant mapping; and
- Shopify privacy, uninstall and protected-customer-data obligations.

Publish a `piqae.shopify-order-document/v1` profile package that maps normalized
Shopify data into the generic engine. The engine must not know how to call
Shopify, store Shopify tokens or interpret a browser-supplied shop identity.

## Product value

This capability creates value beyond the Shopify app:

- fulfilment, ERP, WMS, POS, marketplace and accounting applications can design,
  render and print documents through one API;
- self-hosters receive the same document pipeline rather than a Shopify-only
  closed service;
- SDK users can send typed data instead of pre-generating PDFs when appropriate;
- template authors can share portable open bundles and fixtures;
- rendering, preflight, caching and security receive one implementation and
  release process; and
- Piqae can offer render-and-print as a composable operation while preserving the
  existing API for callers that already produce PDFs.

Keep PDF/RAW upload and direct printing first-class. The template engine is an
additional capability, never a requirement or a reason to weaken local-first or
self-hosted operation.

## The default Document Spec

Document Spec is deliberately smaller than Typst, Liquid, HTML/CSS or pdfme. It
contains only stable operations needed for common business documents:

```json
{
  "spec_version": "piqae.document/v1",
  "page": { "size": "a4", "margin_mm": 12 },
  "body": [
    { "type": "text", "value": "Receipt", "style": "title" },
    { "type": "text", "value": { "pointer": "/document/number" } },
    {
      "type": "repeat",
      "pointer": "/document/items",
      "children": [{
        "type": "row",
        "columns": [
          { "type": "text", "value": { "pointer": "./description" } },
          { "type": "text", "value": { "pointer": "./quantity" } }
        ]
      }]
    },
    { "type": "qr", "value": { "pointer": "/document/verification_url" } }
  ]
}
```

V1 primitives:

- page presets `a4`, `a5`, `letter`, `four-by-six`, `roll58mm`, and `roll80mm`;
- text, rich text subset, row, column, stack, spacer, line and page break;
- typed table with header repetition and controlled row splitting;
- image, SVG and generated 1D/2D barcode/QR;
- header/footer and first/every/last-page regions;
- JSON Pointer-style value binding, bounded `repeat`, simple comparison/boolean
  `when`, and typed formatters for money/date/number/measurement;
- named style tokens and explicitly bounded layout properties; and
- document metadata, locale, language, accessibility labels and PDF profile.

Do not add general functions, recursion, arbitrary expressions, network calls,
filesystem access, plugins or embedded scripts to Document Spec v1. Advanced
logic belongs in a profile compiler or in the caller that prepares typed JSON.
This keeps validation fast, portable and safe enough for local nodes.

## Open template bundle

The bundle must be vendor-neutral at its core:

```text
piqae-document-bundle/
  manifest.json
  template.json              Piqae Document Spec
  profile/                   optional Liquid or raw Typst source/manifest
  data.schema.json           typed caller input contract
  assets/<sha256>            content-addressed images/fonts/base PDFs
  fixtures/*.json            non-secret example input
  expectations/*.json       page/code/semantic assertions
  LICENSES/
```

`manifest.json` identifies:

- bundle and schema versions;
- required compiler/profile versions;
- input JSON Schema digest;
- paper/page capabilities;
- asset length, type, digest and license metadata;
- optional supported/required Liquid or Typst profile features;
- expected resource bounds; and
- optional profile identity such as `piqae.shopify-order-document/v1`.

The format should have a public JSON Schema, canonical serialization, validation
CLI, test corpus and compatibility policy. “Open standard” should initially mean
an openly specified, Apache-2.0 reference format and implementation with public
versioning—not an unsupported claim of industry standardization.

## Typst is a deferred backend/profile, not the current renderer

Typst is a strong implementation foundation because it is Rust-native,
Apache-2.0, produces paged PDFs directly, supports tables/images/markup, exposes
its compiler as crates and is designed for fast incremental compilation. Typst
also contains a powerful scripting language, so accepting arbitrary source is a
materially larger security and compatibility surface than accepting Document
Spec.

The current reference renderer does not compile or execute Typst. If a future
backend is accepted, it must compile validated Document Spec in a closed world:

- no network;
- no host filesystem;
- no unpinned community packages;
- only bundle assets/fonts identified by digest;
- bounded source, data, elements, pages, time, memory and output;
- fixed compiler, standard-library and PDF profile in `render_profile_id`; and
- killable helper-process isolation even though the compiler is Rust.

Raw Typst is a later opt-in content profile for trusted server-side callers. It
must declare exact imports/packages/assets, compile in the same closed world and
receive a different capability/support tier from declarative Document Spec.

## Optional Liquid boundary

Liquid is not the generic default. When the Liquid profile is selected, it sees
only typed caller data and emits validated Document Spec nodes/values:

```liquid
{{ document.number }}

{% for item in document.items %}
  {{ item.description | escape }}
  {{ item.quantity }}
  {{ item.total | money: document.currency }}
{% endfor %}
```

The optional profile supplies safe portable tags/filters and typed values such as
decimal money, date/time, measurement and image references. It does not expose
Shopify theme objects or make network calls.

The Shopify profile adds documented aliases, schemas and filters required for
Order Printer compatibility. Its pdfme-based visual editor authors a richer
layout model that is exported/compiled into Document Spec plus optional Liquid
regions. The authoritative preview is always the core renderer output; pdfme's
browser canvas is an editing surface, not a second production PDF engine.

If a feasibility spike proves that Document Spec cannot represent required
Order Printer parity without becoming a clone of pdfme or Typst, keep an explicit
`piqae.shopify-rich/v1` profile rather than contaminating the small generic v1.
The base engine remains useful for receipts and basic documents.

## Fonts, images, HTML and remote content

Runtime CDN dependencies are incompatible with fast, repeatable printing.

- Bundle a small Apache/OFL-licensed core font set for common Latin documents.
- Additional fonts are uploaded or fetched once by a trusted publish service,
  validated, hashed, license-described and frozen into the template bundle.
- The hosted or local renderer reads only bundle fonts. It never calls Google
  Fonts or another CDN while rendering/printing.
- Images and SVG follow the same content-addressed bundle rule, with strict byte,
  pixel, decompression, media-type and SVG sanitization limits.
- Callers may use a one-time HTTPS import URL at template publication only; the
  URL is not part of the immutable render contract.
- Data URLs may be accepted by SDK tooling within small bounds and converted to
  content-addressed assets before publication.

Do not support arbitrary inline HTML, CSS, JavaScript, iframes or remote images
in the core Document Spec. They create a browser engine, SSRF/privacy risk and
non-deterministic layout. A Markdown-like rich-text subset maps to semantic
Document Spec nodes. Legacy HTML/CSS/Liquid import belongs to the separately
isolated Shopify compatibility profile.

## Proposed public capability shape

The exact API requires the repository's OpenAPI process. Conceptually:

```text
templates
  create draft -> validate -> publish immutable revision -> archive

renders
  register one or a batch with template revision + typed input snapshot
  retrieve progress/result/preflight evidence
  request an optional combined artifact

render-and-print
  register render intent and, once preflight passes, create one idempotent
  Piqae job for the exact artifact and target/profile
```

Do not expose a synchronous request that holds an HTTP connection while 5,000
documents render. Registration is fast and durable; results progress
asynchronously. A convenience SDK may await a bounded single render while using
the same registered operation underneath.

Potential SDK experience:

```ts
const revision = await piqae.documents.templates.publish(bundle);

const render = await piqae.documents.render({
  templateRevisionId: revision.id,
  input: normalizedDocument,
  idempotencyKey: `invoice:${invoiceRevision}`,
});

const print = await piqae.documents.renderAndPrint({
  templateRevisionId: revision.id,
  input: normalizedDocument,
  targetId,
  copies: 1,
  idempotencyKey: `invoice:${invoiceRevision}:target:${targetId}:attempt:1`,
});
```

The SDK validates input and bundles locally but never embeds a platform key in a
browser. Account/tenant selection remains server-authoritative. Keep Documents
in an optional package/subpath so applications that only upload PDF/RAW do not
ship Typst, template schemas or browser preview code:

```ts
import { PiqaeClient } from "@piqae/sdk"; // existing PDF/RAW API, unchanged
import { defineDocument } from "@piqae/documents"; // optional authoring tools
```

The native agent receives renderer support as an optional installed/bundled
capability. An agent without it continues accepting supported PDF/RAW jobs
exactly as before.

## Receipts and raw printer languages

Document Spec should make receipts a first-class page mode:

- 58 mm, 80 mm and bounded custom roll width;
- automatic/continuous page height with maximum output bounds;
- left/center/right rows, columns, rules, logo, totals and QR/barcode;
- monospaced and proportional layouts;
- cut/drawer intent represented as separate typed device operations only when a
  target explicitly advertises and authorizes them.

The default output remains PDF because installed drivers and Piqae already know
how to deliver PDFs portably. Existing RAW printing remains available for callers
that already possess printer-language bytes.

Do not define a supposedly universal “raw receipt language.” ESC/POS variants,
code pages, raster commands, cutters, drawers and vendor extensions differ by
device. A later optional `receipt-escpos/v1` backend may compile the same receipt
Document Spec only for an explicitly compatible target/profile and must be
covered by virtual and named physical certification. It must never be silently
selected in place of PDF or sent to an unknown queue.

## Rendering locations

Expose rendering as an explicit capability with identical bundle semantics:

| Location | Initial status | Use |
| --- | --- | --- |
| Hosted Piqae render workers | First production implementation | Lowest integration effort, centralized caching/scaling, render then print |
| Self-hosted control-plane workers | Same contract after parity evidence | Private/self-hosted document generation |
| Browser preview | Non-authoritative | Immediate editor feedback with fixture/redacted data |
| Native/local node helper | Later opt-in capability | Confidential/on-network render, offline operation and reduced PDF transfer |

Do not put the Typst compiler directly inside the long-lived Rust agent process.
A local renderer must be a signed, versioned, killable helper with CPU,
memory, time, file, asset and output limits. The agent advertises exact engine,
profile and plugin capabilities; the control plane assigns local rendering only
when the immutable template bundle is compatible.

The native node continues to persist durable print responsibility separately:

```text
render bundle/input accepted locally
  -> helper produces artifact + digest + preflight evidence
  -> agent persists content and job metadata
  -> agent reports accepted
  -> installed OS driver/spooler handoff
```

Failure to render is not a spool attempt. A helper crash cannot produce an
automatic replacement print. Retain `delivery_uncertain` once responsibility may
have crossed into the native spooler.

## Does local rendering make it faster?

Sometimes, but not universally.

It can reduce latency when normalized data/template assets are small but the PDF
is large, the printer network is remote from the hosted region, or documents must
remain on the local network. It can also allow an already-synchronized template
to render while the cloud connection is impaired.

It can be slower on underpowered printer PCs, cold helpers, missing font/asset
caches or large image workloads. It expands native package size, update surface,
security exposure and the test matrix. It does not remove the Shopify API/token
backend because the node cannot safely impersonate the installed Shopify app.

Choose hosted versus local using measured capability and policy, not an implicit
fallback. Never silently switch renderer/profile versions for a pinned legal
document. Record the exact render location and profile in artifact evidence.

## Thin Shopify deployment

The Shopify app can become thin, but it cannot be a public frontend-only app.

Current Shopify constraints require a trusted backend for this product:

- a public App Store app uses iframe-based App Home rather than the custom-only
  hosted App Home UI-extension model;
- background jobs/webhooks require offline access and, for new public apps,
  expiring offline access tokens plus refresh-token rotation;
- the app must receive, verify, deduplicate and process uninstall and privacy
  webhooks;
- billing state and asynchronous plan changes require reconciliation;
- automated generation/email/customer links require work without an interactive
  staff session; and
- Shopify session tokens must be verified before mapping a shop to its Piqae
  account/grant.

Vercel can host the React Router app and server functions. That is still an app
backend. It needs durable session/install state, encrypted refresh tokens,
webhook receipts/idempotency and shop-to-Piqae mappings in a production database
or service, plus a secret manager for the Shopify client secret and any Piqae
integration credential.

Use Shopify app-data metafields for small, non-sensitive, per-installation
configuration such as default template IDs, onboarding flags or display choices
when their consistency semantics are sufficient. Shopify explicitly advises
that sensitive credentials belong in a secure application database, environment
variables or a dedicated secret manager, not app-data metafields.

Interactive direct Admin API access can make the UI thinner and avoid proxying
some foreground queries. It does not replace offline tokens for webhook-driven
projection, automation, large durable batches or token refresh.

## Minimal Shopify-owned persistent data

Keep the installation database narrow:

```text
shop_installation
  immutable shop GID/domain history/status/scopes
  encrypted expiring offline access + refresh token metadata
  Piqae connection mode and opaque account/grant mapping
  Shopify subscription/plan projection
  installed/uninstalled/privacy lifecycle timestamps

webhook_receipt
  Shopify delivery/topic/shop/digest/processed result

shopify_projection_cursor
  webhook/bulk reconciliation version and status
```

Templates, render tasks, artifacts, preflight, print jobs and operational history
belong to Piqae Documents/Piqae accounts. The Shopify database may retain opaque
IDs and a product-facing activity projection, but must not duplicate the complete
Piqae state.

## Latency impact

Moving rendering into core improves reuse and can improve speed only if the API
boundary avoids extra serial hops:

- deploy the Shopify BFF and hosted Piqae API/render plane in low-latency regions;
- let one durable Piqae operation compose render -> preflight -> upload -> print;
- send typed normalized input once, not through multiple JSON transformations;
- content-address and cache templates/assets/artifacts in the render plane;
- return intent/progress immediately and stream events to the Shopify UI;
- keep interactive priorities ahead of bulk/automated work; and
- use the same normalized schema in the Shopify adapter and core SDK.

A generic service can become slower if it requires chatty calls such as “create
template, upload each asset, render, download, upload PDF, create print job” for
every document. The SDK/API must offer an atomic-looking registered
`renderAndPrint` workflow backed by durable internal steps, without pretending
they form a distributed transaction.

## Scope and release sequence

1. Specify Document Spec, bundle, optional profiles, deterministic renderer
   manifest and threat/resource model.
2. Implement hosted Rust document render workers and optional TypeScript SDK
   authoring tools behind a Preview capability without changing existing PDF/RAW
   jobs.
3. Build the Shopify profile/app on the same public capability and fake printers.
4. Prove self-hosted parity and document the operational worker.
5. Add local native-helper rendering only after hosted correctness, capability
   negotiation, package/update security and performance evidence.
6. Consider broader “open standard” governance after at least two independent
   non-Shopify integrations use the bundle without private extensions.

Do not delay the Shopify product indefinitely to generalize every future
document use case. The generic core should contain only behavior exercised by the
Shopify profile plus one small neutral SDK example; extract new abstractions from
working profiles.

## Consequences

Positive:

- thinner Shopify product code and one renderer/preflight implementation;
- broader SDK and self-hosted value;
- a credible open document-to-print workflow;
- shared caching, performance work and security evidence; and
- future confidential/local rendering without changing template semantics.

Costs and risks:

- public API/product scope grows materially;
- core control-plane storage, quotas, retention and abuse controls expand;
- multi-profile compatibility and deterministic engine upgrades become support
  obligations;
- local rendering increases native package/security/test complexity; and
- an over-generalized v1 could slow Shopify delivery.

## References checked 9 August 2026

- [Shopify App Home](https://shopify.dev/docs/api/app-home) documents direct
  Admin API access and online/offline modes.
- [Shopify offline access tokens](https://shopify.dev/docs/apps/build/authentication-authorization/access-tokens/offline-access-tokens)
  documents background use and expiring-token/refresh requirements for public
  apps.
- [Shopify metafields](https://shopify.dev/docs/apps/build/metafields) documents
  app-data metafields and cautions against storing sensitive credentials there.
- [Shopify App Home UI extensions](https://shopify.dev/docs/api/app-home-ui-extension/latest)
  documents that hosted App Home extensions are custom-distribution only and
  that webhooks/background work still require a backend.

Re-verify these requirements before implementation or App Store submission.
