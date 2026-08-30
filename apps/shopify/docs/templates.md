# Shopify PrintPacket

The Shopify app owns authoring and Shopify data adaptation. Piqae receives only a validated `printpacket/v1` document and normalized data; Piqae has no Shopify or Liquid behavior.

Four immutable, dynamic starters are seeded: Invoice, Packing slip, 80 mm Receipt and 100 × 50 mm Product label. All use semantic flow content backed by line-item collections. Paged documents reflow and paginate, receipts use bounded continuous-roll layout, and product labels produce fixed-size pages rather than placing fields at arbitrary coordinates. **Customize** creates a merchant draft. Publishing pins one immutable Piqae revision used by preview, PDF download and direct print.

The render input is one canonical `{shop, orders}` object. Shop identity is a bounded `{name, domain}` object, never a scalar. Money values are numeric at the PrintPacket boundary: Shopify Decimal strings are accepted only in plain notation with at most six fractional digits and a safe scaled integer, then encoded with `printpacket.canonical-data/v1`. Invalid, ambiguous, non-finite or oversized values fail before a render is registered. The same input therefore has one cross-runtime cache identity in the cloud and on every compatible node.

## Word-like editor

The visual editor is a schema-controlled ProseMirror document surface. Paragraphs and headings are edited directly. Shopify values are inserted as typed variable chips. Line-item tables, repeaters, conditions, images, QR codes and barcodes are semantic blocks. The editor stores no HTML and has no unrestricted coordinate layout.

One template has Document, Advanced Liquid and Piqae source views. Supported Liquid is compiled into the same PrintPacket tree. Visual changes regenerate normalized Liquid. A view switch with invalid source is stopped with a line/column diagnostic; content is never silently flattened.

## Media and print targets

`document.media` is authoritative for both rendering and the editor canvas. A4,
A5 and Letter use their exact paged dimensions and orientation. Continuous roll
documents use their declared width and grow with content; page breaks are
diagnosed and rejected. Fixed labels use their exact, editable width and height,
including non-preset label sizes. Receipt and label document types select the
corresponding media model, while paged invoice and fulfilment types start on A4.
The Product Label starter renders every expanded line item as one atomic
100 x 50 mm fixed-label page. Shopify does not currently select sheet or
continuous stock for that starter; sheet-cell imposition remains future work.
When a selected target declares safe-area, gap, or registration-mark facts, the
editor shows those production guides without copying them into the document.

A Shopify template may pin a Piqae Target and the exact DesignSpecification
revision observed when it was saved. The target—not Shopify—owns its stock and
immutable printer/profile bindings. Search and selection only offer targets
whose stock dimensions and kind match the document media. Printing submits
`target_id`, so Piqae re-resolves and revalidates the current binding at handoff.

Loaded stock is operational evidence, not a guess. `ready` is shown only when
the target is ready and its selected destination reports fresh, trusted,
compatible media. Missing evidence is shown as **not reported**; stale,
untrusted, and incompatible observations remain distinct blocking states. PDF
preview and download remain available when direct printing is blocked.

## Shopify Liquid profile

The bounded profile supports mixed literal text and output expressions, `money`, `date` and `number` formatting, `for` with an enforced limit, `if`/`unless`/`else`, comparisons, semantic line-item tables, QR codes, barcodes, dividers and page breaks. Includes, render, layout, arbitrary HTML/CSS, scripts, assignments, captures, plugins, network and filesystem access are rejected. Sources are limited to 64 KiB, 4,000 tokens, 12 nesting levels and 1,000 collection items.

## Assets

The editor may ingest PNG, JPEG and sanitized SVG from Shopify CDN over HTTPS. Ingestion validates a two MiB limit, media type, byte length and SHA-256 digest while blocking redirects and private addresses. Published documents reference content-addressed Piqae assets; rendering never depends on a mutable URL. Free-tier quotas remain bounded.

## Authority

PostgreSQL/Piqae is authoritative. Shopify app-owned metafields contain only a compact index of published IDs, names, kinds, media, revisions and digests. They never contain source, credentials, customer data, assets or rendered documents.
