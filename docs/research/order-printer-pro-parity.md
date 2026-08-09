# Order Printer Pro parity and editor assessment

**Research date:** 9 August 2026. **Status:** product research, not a support
claim. Public and help-center behavior is verified; implementation details that
are not public are explicitly marked as inference.

## Recommendation

Build a **Liquid-first Piqae document editor on pdfme**. pdfme is the preferred
visual and PDF-template foundation because its canvas, measurements, pages,
schemas, viewer and generator model the artifact merchants are designing: a PDF,
not a responsive web page. It is MIT licensed, TypeScript-based, embeddable, and
already provides a WYSIWYG designer plus text, image, SVG, table and barcode
extension points.

Do not replace pdfme's JSON template format with unstructured Liquid strings.
Extend its schema/plugin system with Liquid-aware fields, conditions, repeaters
and flow regions. The canonical template combines a versioned pdfme-derived
layout tree with exact Liquid expressions. A synchronized advanced Liquid editor
supports experts and imported templates, while ordinary merchants edit paper
visually through Polaris controls.

The principal engineering risk is variable-length commerce documents. pdfme's
paper-coordinate model is ideal for invoices as a visual artifact, but existing
capability must not be assumed to cover every flowing table, repeated header,
conditional section and unknown page-count case. Build those as first-class
Piqae flow-layout plugins and prove them with large fixtures before committing to
full compatibility claims.

Do not base the product on Unlayer. Its React wrapper is MIT licensed, but the
actual editor is a remotely delivered email-builder service with a commercial
product boundary. Email layout semantics also do not solve print pagination.

Atlas PDF is useful prior art for HTML/CSS/JSON preview and template promotion,
not the recommended embedded visual-editor foundation. WeasyPrint is a credible
HTML/CSS-to-PDF renderer to benchmark, not an editor. Any renderer choice needs
golden comparison for Shopify Liquid, CSS paged media, fonts, barcodes and the
existing Rust/Node operating model.

Primary project sources:

- [pdfme repository](https://github.com/pdfme/pdfme) — MIT, JSON PDF templates,
  React-based UI and browser/Node generation.
- [GrapesJS repository](https://github.com/GrapesJS/grapesjs) — evaluated but
  not selected because its primary abstraction is an HTML/CSS web template.
- [Atlas PDF](https://www.atlas-pdf.com/) — open-source HTML/CSS/JSON template
  and PDF system.
- [WeasyPrint](https://weasyprint.org/) — open-source HTML/CSS document renderer.

Pin and review every dependency and transitive license before adoption. A
repository license does not automatically license hosted services, fonts,
sample templates, logos, or optional commercial SDKs on the same vendor site.

## Why Liquid-first does not mean Liquid-only

Liquid is Shopify's open-source template language for data, conditions,
iteration and filters. It deliberately does not define document layout. A useful
template therefore has three explicit layers:

| Layer | Authority | Example |
| --- | --- | --- |
| Data and logic | Restricted Liquid | `{{ order.name }}`, `{% if order.tax_lines.size > 0 %}`, `{% for line_item in line_items %}` |
| PDF structure | pdfme-derived page, region and schema tree | page, header region, flow table, footer region, QR block |
| Print presentation | pdfme measurements and restricted visual style tokens | page size, margins, type, colour, column widths, repeating regions |

Markdown can be offered inside rich-text blocks for headings, paragraphs,
lists, emphasis and links. It should compile to a safe pdfme rich-text schema
before generation and must not become the whole page-layout format: Markdown
cannot faithfully express invoice tables, conditional sections, repeating
headers, page regions or print geometry without inventing a second templating
language.

Shopify documents several distinct Liquid variations for themes,
notifications, order-printer templates and packing slips. “Uses Liquid” does not
guarantee identical objects, tags, filters or behavior. Define and version a
**Piqae Order Document Liquid profile**:

- portable open-source Liquid tags and filters by default;
- an explicit Shopify-order object schema owned by this app;
- documented compatibility aliases for Shopify Order Printer constructs;
- Piqae-specific filters only where necessary, namespaced when practical;
- strict parsing, strict variables and strict filters when publishing;
- bounded loop iterations, render operations, output bytes, include depth,
  collection sizes, asset sizes and render time;
- no arbitrary network access, filesystem access, dynamic code evaluation or
  unrestricted includes;
- conformance fixtures against Shopify's open-source Liquid specification plus
  application fixtures for every supported Shopify object/filter.

[Shopify's Liquid reference](https://shopify.dev/docs/api/liquid) explicitly
notes that themes, notifications, Order Printer and packing slips use different
variations. [Shopify Liquid](https://github.com/Shopify/liquid) is MIT licensed
and designed as a non-evaluating, customer-editable template engine. For a Node
implementation, [LiquidJS](https://github.com/harttle/liquidjs) is MIT licensed
and Shopify-compatible, but compatibility must be proven against the selected
profile rather than assumed from its description.

## Canonical template format

Store both the exact authored source and a parsed, versioned AST:

```text
template revision
  template.json            versioned pdfme-derived pages, regions and schemas
  expressions.liquid       exact advanced Liquid source and named expressions
  ast.json                 parsed Liquid plus layout relationship graph
  editor.json              non-semantic selection/panel state only
  fixtures/                redacted data cases and expected render hashes
  manifest.json            schema version, locale, paper and capabilities
```

`template.json` and `expressions.liquid` together are the portable render and
audit artifact. The AST is the safe relationship model. `editor.json` must never
be required to render a document. On every save, parse Liquid, normalize the
layout relationship graph and compare semantics. If a code edit contains a
construct the visual model cannot represent losslessly, keep it as an
**advanced Liquid region** instead of rewriting it.

A visual line-item block can retain understandable Liquid expressions:

```liquid
{% for line_item in order.line_items %}
  {% row %}
    {% cell key: "item" %}
      {{ line_item.title | escape }}
      {% if line_item.sku != blank %}
        {{ line_item.sku | escape }}
      {% endif %}
    {% endcell %}
    {% cell key: "quantity" %}{{ line_item.quantity }}{% endcell %}
    {% cell key: "price" %}
      {{ line_item.final_line_price | money }}
    {% endcell %}
  {% endrow %}
{% endfor %}
```

The pdfme-derived table schema owns column geometry, fonts, borders, padding,
header repetition and overflow. Liquid owns the rows, values, conditions and
formatting. The visual properties panel changes both through typed controls. An
expert can edit the Liquid directly; visual mode resumes for known nodes and
preserves unknown valid nodes as advanced regions.

## Build versus fork decision

Prefer a maintained pdfme dependency plus Piqae plugins over an immediate hard
fork. Fork only if required extension points cannot support flow layout,
Liquid-aware schemas or the Polaris shell without fragile patching. Recommended
sequence:

1. Pin pdfme and build a thin adapter so stored templates are not coupled to
   unstable internal types.
2. Define the Liquid profile, parser/formatter, Shopify object schema and strict
   deterministic renderer.
3. Implement Piqae schema plugins for Liquid text, conditional region, repeated
   flow group, commerce table, page region, barcode/QR and Markdown-rich text.
4. Replace/wrap the stock designer chrome with a Polaris editor while continuing
   to use pdfme selection, guides, movement, measurement, pages and preview.
5. Contribute generally useful fixes upstream where feasible; keep Piqae's
   Shopify-specific Liquid/profile layer separate.
6. Fork only after documenting the missing upstream hooks, maintenance owner,
   merge strategy, security update process and exit path.

The first proof should cover the hard document behavior, not a decorative
invoice: 200 flowing line items, automatic page creation, repeating header,
partial fulfillment, a conditional tax block, a user-edited Liquid expression,
CJK text, an explicit page break, and a QR code decoded after PDF rendering.

## Required pdfme extensions

| Extension | Responsibility |
| --- | --- |
| Liquid value schema | Bind a text/image/barcode property to a parsed Liquid output expression with type validation |
| Conditional region | Include/collapse a group based on a bounded Liquid condition |
| Repeated flow group | Iterate a Liquid collection, measure each generated child, wrap across pages and preserve stable order |
| Commerce table | Typed headers/cells, variable row height, split policy, repeated header, subtotals and fulfillment grouping |
| Page regions | First/every/last-page headers and footers plus content bounds |
| Rich text | Markdown-like editing compiled to safe styled runs, with inline Liquid variables |
| Page break | Explicit conditional or unconditional page transition |
| Barcode/QR | Typed payload expression, vector output, quiet-zone rules and post-render decoding |
| Overflow diagnostics | Identify the exact block/data item that clipped, overlapped or created an unwanted blank page |

Avoid embedding Liquid independently in every low-level pdfme string property.
That would make templates impossible for general users to reason about. Bind
Liquid at typed content/condition/repeater boundaries and expose ordinary visual
properties for presentation.

## The editor merchants should see

The goal is not a miniature design program. Most merchants should make a good
document in minutes without learning layout, Liquid, CSS, or PDF terminology.

### Three editing levels

1. **Quick style** — logo, accent, typography, density, paper size, address/tax
   identity, visible columns and footer. All controls update a real order preview.
2. **Layout** — reorder safe sections and configure document-aware blocks:
   header, addresses, order summary, line items, payment/tax totals, notes,
   barcode/QR, policies and footer.
3. **Liquid** — syntax-aware Liquid expressions and advanced regions, with
   schema completion, variable documentation, linting and a safe diff before
   publishing. Imported legacy HTML/CSS/Liquid remains available in a separate
   compatibility view when it cannot be converted losslessly.

Default to Quick style. Remember the selected level per user, not per shop, so
an expert's choice does not expose code to every staff member.

### Document-aware blocks

Generic text boxes are insufficient. Each block understands Shopify data and
pagination:

| Block | Friendly controls | Advanced capability |
| --- | --- | --- |
| Shop identity | Logo, name, address, contact, tax IDs | Market/location identity rules |
| Document title | Type, number, date, status | Legal sequence and locale formatting |
| Customer addresses | Billing/shipping/both, labels | Company, VAT and address fallback |
| Order facts | Order, PO, payment, shipping, tags | Conditional fields and metafields |
| Line items | Columns, images, SKU, quantity, price | Sort/group/filter, properties, bundles, fulfillments, metafields |
| Totals | Discount, shipping, tax, duties, refunds, balance | Tax-inclusive/exclusive and multi-currency rules |
| Notes | Order/customer/internal/policies | Conditions and explicit page breaks |
| Barcode/QR | Source picker, symbology, size, caption | Liquid expression and validated payload |
| Pay action | Label and draft-order URL | Safe signed/Shopify-native destination |
| Footer | Page number, contact, legal text | First/last/every-page rules |

The editor should show field names in merchant language—“Purchase order number”
rather than `order.po_number`—and offer search. Clicking a preview value reveals
its Shopify source, current value, fallback and whether it may be absent.

### Layout safety

- Flow layout by default; absolute positioning only in explicit label/form mode.
- Printable-area guides from paper size and margins.
- Page-break preview, orphan/widow rules and repeating table/header indicators.
- Responsive editor canvas, but one authoritative print geometry per revision.
- Warn about clipped content, missing glyphs/assets, illegible font sizes,
  inaccessible contrast, blank pages and oversized images.
- Preview fixtures for 1, 20 and 200 line items, long addresses, discounts,
  partial refunds, multiple fulfillments, CJK, RTL, B2B and missing fields.
- Undo/redo, autosaved draft, named versions, compare, publish and one-click
  rollback. Production jobs always retain the immutable published revision.

### Compatibility contract

Support three template origins:

- **Native component template:** fully visual and safest.
- **Compatible HTML/Liquid import:** parsed into known blocks where lossless;
  unsupported fragments remain locked advanced-code blocks with an explanation.
- **Code template:** remains code-first; never claim visual round-tripping if
  conversion to pdfme-derived schemas would alter meaningful HTML, CSS or Liquid
  structure.

Never silently rewrite or discard imported Liquid. Before saving a visual edit,
show any lossy conversion and require an explicit “convert to native template”
choice. Store the original import for rollback and provenance.

## Pricing: match exactly, win on product

| Plan | Price and order band | Our additional value at the same price |
| --- | --- | --- |
| Free | $0, up to 50 monthly orders | Bring an existing Piqae account; visual editor; preview/download; direct printing according to the merchant's Piqae entitlement |
| Starter | $10/month, 51–500 | Printing included, durable queue, verified QR/barcodes, destination defaults, failed-item retry |
| Growth | $20/month, 501–5,000 | Same feature set and higher operational capacity; document packs and automation rules as released |
| Scale | $40/month, 5,001+ | Same public band; fair-use/capacity policy must be explicit if there is a technical limit |

Use the same 14-day trial on paid plans if Shopify App Pricing supports the
desired presentation at submission time. Avoid artificial feature gating across
paid tiers if cost is driven mostly by volume. The free bring-your-own mode must
make it clear that Shopify app usage is free while the merchant's separate Piqae
plan and limits still apply.

Match the definition of “monthly orders” only after verifying it experimentally:
the public listing states bands but does not fully specify whether draft orders,
reprints, automated links, multiple documents per order, cancelled orders or
historical exports count. Publish our own precise and merchant-favorable meter.
Recommended: count each distinct Shopify order/draft-order GID first successfully
rendered in a Shopify billing period once, regardless of previews, templates,
copies, retries, downloads or print destinations.

## Complete verified product surface inventory

This inventory comes from the current Shopify App Store listing, product site,
public update history and its 88-article help-center collection. It is a complete
inventory of publicly documented surfaces found in that research, not a claim
to know private flags, experiments or backend behavior.

### Installation, navigation and account lifecycle

| Their behavior | How it works publicly | Match | Improve |
| --- | --- | --- | --- |
| Shopify App Store install | Public embedded app; Built for Shopify listing | OAuth install, embedded App Home | Minimal scopes, clear value before charge, resumable setup |
| Free/trial/subscription | Free through 50 orders; paid bands and 14-day trial | Shopify-hosted pricing and lifecycle | Exact usage explanation, in-app live meter, BYO-Piqae free mode |
| Uninstall | Help article documents uninstall | Clean uninstall and required privacy webhooks | Show export/revocation/retention consequences before uninstall |
| Localized app UI | Public update says UI in 21 languages based on Shopify admin settings | At least their 21 UI locales | Translate field/schema help and errors too; locale fallback tests |
| Support | 24/7, typical response publicly stated under 30 minutes | In-app help and searchable documentation | Contextual diagnostic bundle, job/template IDs, public status and honest SLA |

### Entry points and order selection

| Their behavior | How it works publicly | Match | Improve |
| --- | --- | --- | --- |
| Shopify Admin order action | Select one or multiple orders, invoke app print/export | Admin action for selected order GIDs | Inline default destination and immediate progressive queue |
| In-app generation | Generate documents for any order from the app | Search/filter/order picker | Saved views, keyboard workflow, recent and exception-first lists |
| Draft orders | Print/export draft documents and quotes; Pay Online links | Draft-order action and search | Explicit quote/invoice state, expiration and safe pay-link validation |
| Shopify POS | Print from POS | POS UI extension/action | Location/printer defaults, device-aware status, direct printing |
| Shopify mobile | Print through Shopify mobile app | Mobile-compatible action/UI | Direct queue avoids fragile mobile PDF/browser handoff |
| Zapiet | Up to 50 selected Zapiet orders passed into OPP | Compatible integration entry point | General signed bulk-intent contract; no arbitrary tenant/order IDs |

### Printing and export

| Their behavior | How it works publicly | Match | Improve |
| --- | --- | --- | --- |
| Browser printing | Generates document then invokes normal print behavior | PDF/browser print fallback | Piqae direct print without download/dialog |
| PDF export | Single or bulk PDF export from Admin/app | Same immutable artifact for download | Stream first results; per-order artifacts plus optional merged pack |
| Large batch queue | Sequential per shop, blocks duplicate requests, retries failures, live status | Durable bounded batch orchestration | Parallel but bounded per-order rendering, fairness, partial success, exact retry semantics |
| Export history | Dedicated export-history menu | Searchable retained export attempts | One activity model for renders/downloads/emails/prints with redacted timeline |
| Small paper | A5 and 4×6 documented | Page presets and custom geometry | Destination validates paper/profile; no silent scale-to-fit |
| File naming | Automated PDF filenames can be changed | Safe naming tokens | Preview collisions/invalid characters and stable archive naming |
| Return to Orders | Help path to go back to Shopify orders | Native Shopify navigation | Preserve filters/selection where Shopify permits |
| Print troubleshooting | Missing colors/images, browser headers/footers, fixed footer limitations | Documented browser fallback guidance | Server render removes browser variability; preflight assets and page geometry |

### Automated delivery and customer surfaces

| Their behavior | How it works publicly | Match | Improve |
| --- | --- | --- | --- |
| Automated PDF links | One-time setup produces customer document links | Signed, revocable scoped document links | Short-lived exchange, configurable expiry, audit and regenerate without leaking order data |
| Shopify notifications | Add link to email notifications and SMS | Supported Liquid/snippet or native extension path | Setup validator and test delivery; avoid manual copy/paste when extensions allow |
| Thank-you/Order Status | PDF link can be added/auto-published | Current Shopify extension-compatible surface | Theme/checkout-safe extension, clean uninstall, localization preview |
| Customer Account page | Automated PDF setup | Customer account UI extension | Native list by order/document with authorization at request time |
| Customer Accounts Hub | Third-party integration displays download links | Match documented partner contract if demanded | Prefer first-party customer account extension; generic partner SDK |
| Supplier/shipper | Automatically send packing slip | Event-driven email destination | Rules, redaction, least-data document, delivery audit and allowlist |
| Direct customer invoices | Can send invoice rather than only a link | Email attachment/link options | Deliverability, consent, locale, attachment limit and bounce visibility |
| Merchant copy | Send merchant a copy of customer invoices | BCC/copy setting | Role-based recipients, digest mode, prevent PII leakage |
| Custom PDF-link domain | Documented custom-domain support | Custom domain if justified | Managed verification, TLS/DNS state and safe fallback |
| Link failures | Blank page, inactive account, invalid link and link-text errors documented | Clear stable errors | Health checks before publishing, status-specific recovery, no dead customer links |
| Other files | FAQ considers sharing non-generated files | Explicitly scope supported artifacts | Separate secure file product; do not overload invoice authorization |

### Template library, editor and migration

| Their behavior | How it works publicly | Match | Improve |
| --- | --- | --- | --- |
| Default professional templates | Invoice/receipt/packing slip/returns etc. | High-quality localized defaults | Task-specific presets with accessibility and golden fixtures |
| Code-free tweaks | Logo and common document details | Quick style controls | Real preview, searchable fields, explain data source, undo/versioning |
| Full customization | HTML, CSS and Shopify Liquid | Sandboxed advanced editor | Completion, schema docs, lint, security limits, visual/code provenance |
| Import from Shopify Order Printer | Dedicated import flow | Compatibility analyzer/importer | Loss report, fixture comparison, retain original and rollback |
| Import from Order Printer Templates | Dedicated import flow from companion paid template app | Import only with legal/technical permission | Native recreations and documented unsupported constructs; never copy proprietary templates |
| Restore default | Reset a template | Restore/duplicate default | Version history and selective section reset instead of destructive reset |
| Store details refresh | Refreshes store identity used by documents | Explicit projection refresh | Field-level live/snapshot status and location/market identity rules |
| Images and logo | Upload logo/custom image, resize/remove product images | Managed assets | Validate content/type/size, strip metadata, immutable hash, no remote runtime fetch |
| Google Fonts | Template can reference Google fonts | Curated licensed font set | Self-host pinned subsets, glyph coverage checks, no runtime Google dependency |
| Page layout | Save space, page break for policies, repeat headers, footer guidance | Paged-media controls | Live page boundaries, orphan control, first/last/every-page regions |

### Shopify data and document logic

| Their behavior | How it works publicly | Match | Improve |
| --- | --- | --- | --- |
| Metafields | Orders, draft orders, customers, products, variants | Typed selector plus Liquid access | Namespace search, type-aware formatting, permissions and missing-value preview |
| Store/customer identity | Store name/email/phone/address/tax ID and customer email/address | Friendly identity fields | Location/market/company-aware resolution with provenance |
| Invoice number | Customizable invoice numbering | Legal sequence configuration | Atomic jurisdiction-aware sequences, no duplicate on retry, audit/void handling |
| Prices | Decimal visibility controls | Currency-aware formatting | Never use float arithmetic; locale/currency and rounding evidence |
| Taxes | Tax invoice support and troubleshooting; retroactive-tax FAQ | Shopify tax data faithfully represented | Explain source, inclusive/exclusive status, tax lines and immutable historical snapshot |
| Multi-currency | Receipts/invoices supported | Presentment and shop currency | Both currencies/exchange context where Shopify supplies it; rounding tests |
| Multi-language | Translate templates, multi-language setup, translated month names | Per-locale template/output | One schema with translation resources, market/customer locale resolution and fallback |
| B2B | PO number, company/VAT collection/display and automatic translation of B2B blocks | Shopify company and PO fields | No duplicate customer-data collection when Shopify owns field; validation/provenance |
| Store credit | Customer/store credit documented | Reflect supported Shopify credit transactions | Explicit balance/effect date; separate tender from discount/refund |
| HS/origin | Tariff code and country of origin | Product customs fields | Completeness warnings and per-line provenance |
| Address updates | Customer shipping/billing address workflow documented | Render current or order snapshot by policy | Never silently mutate an issued legal invoice; correction/credit-note workflow |
| Product sorting | Sort line items by keywords | Visual sort configuration and Liquid escape hatch | Multi-key stable sort, grouping, preview and preserve fulfillment semantics |
| Fulfilled only | Template option/snippet | Fulfillment filter | Clear partial-fulfillment document identity and remaining quantities |
| Split by fulfillment | Packing slip splitting documented | Per-fulfillment packs | Route each fulfillment/location to its correct destination |
| Bundle apps | Compatibility documented | Normalize component/bundle lines | Show bundle hierarchy and validate quantities rather than vendor-specific hacks |
| Variable hierarchy | Help reference for Liquid variables | Searchable schema explorer | Examples using actual redacted order data and API-version compatibility report |
| Useful Liquid snippets | Copy/paste recipes | Recipe library | Tested, versioned snippets inserted structurally with explanation |

### Barcode, QR and machine-readable output

| Their behavior | How it works publicly | Match | Improve |
| --- | --- | --- | --- |
| Product barcodes | Add product barcodes to template | 1D symbologies and source picker | Validate length/check digit and decode rendered result |
| QR codes | Added in 2022; generic QR template support | Vector QR block | Enforce quiet zone/contrast/size; rasterize at target DPI and decode before publish/print |
| ZATCA/Fatoora QR | Dedicated help article | Only after compliance review | Typed TLV builder, authoritative fields, conformance fixture and jurisdiction support claim |
| Known QR fixes | 2025 update states QR template issues were fixed | Regression fixtures | Every expected code must decode to exact source payload in CI and runtime preflight |

### Document types and specialist capability

| Publicly listed type/capability | Match behavior | Improve beyond parity |
| --- | --- | --- |
| Invoice | Branded tax/payment/order document | Issue/revision state, legal numbering evidence, direct print |
| Receipt and gift receipt | Payment/order receipt; gift-safe version | Privacy-safe field defaults and POS/location routing |
| Packing slip and delivery note | Fulfillment contents and destination | Per-fulfillment/location generation, pick sequence and direct dispatch print |
| Quote/draft order | Draft-order document with Pay Online action | Expiry/version, signed action and conversion lineage |
| Credit note/refund | Refund/credit document | Link to original invoice, immutable accounting event and numbering policy |
| Return form | Customer return paperwork | QR return intent, eligibility/status and warehouse routing |
| E-invoicing/Peppol add-on | Paid specialist add-on publicly advertised | Partner-led validated structured invoice, delivery receipt and country-specific release evidence |

## Beyond parity: defensible features

Parity removes reasons not to switch. These features create reasons to choose us:

1. Direct-to-printer Shopify actions with no PDF download or browser dialog.
2. Durable offline queue and precise accepted/printing/reported-complete/uncertain
   states inside Shopify.
3. Exact duplicate prevention across UI retries, webhook retries and ambiguous
   network failures.
4. Runtime rendered QR/barcode decoding, not merely checking that template markup
   contains a code.
5. Per-document progressive batches and failed-item recovery rather than one
   all-or-nothing merged export.
6. Logical destinations binding real driver profiles and paper geometry; no
   silent scaling or generic-driver fallback.
7. Visual editor that exposes Shopify data in merchant language while retaining
   lossless advanced Liquid for experts.
8. Draft/publish/version/compare/rollback with exact revision attached to every
   output.
9. Document packs and explainable routing by Shopify location, fulfillment,
   shipping method, market, tags and B2B company.
10. Bring-your-own-Piqae for $0 through Shopify and app-scoped child accounts for
    merchants who want one Shopify bill.
11. One activity timeline for render, delivery, download and physical-print
    attempts, with safe replacement decisions.
12. Template/data compatibility diagnostics before Shopify API upgrades or a
    merchant publishes a schema/metafield change.

## Validation still required

- Install the competitor on an authorized Shopify development store and record
  every visible page, action, modal, empty/error state, template control, billing
  transition, mobile and POS flow. Public documentation cannot establish exact UI
  behavior or private feature flags.
- Run controlled fixtures to determine their precise order meter, batch caps,
  generated-file behavior, template syntax compatibility and performance.
- Obtain and review export/import samples the test store is authorized to use;
  do not copy proprietary templates, source, branding or non-public behavior.
- Interview merchants about real workflows and failure history; reviews and one
  user's QR/latency reports identify hypotheses, not statistical defect rates.
- Re-check pricing, help center, App Store fields and Shopify requirements before
  each roadmap/release decision.
