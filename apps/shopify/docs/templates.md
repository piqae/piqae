# Shopify document templates

PostgreSQL/Piqae is authoritative. Shopify receives only an app-installation-owned `piqae.template_index` JSON metafield containing published IDs, names, kinds, page sizes, revision numbers and SHA-256 digests. It contains no source, customer data, credential, font, image or rendered document. Failure to update this cache fails the mutation visibly; it never changes the authority boundary.

Three immutable system documents are seeded on first use: Invoice, Packing slip and 80 mm Receipt. **Customize** clones one into a merchant draft. Retired system starters are hidden without deleting merchant drafts or immutable revision history. Publishing first converts the editor envelope to `piqae.document/v1`, creates an immutable revision in the linked Piqae account and records its revision ID and canonical digest. Preview, download and direct print must reference that exact revision/artifact.

The invoice starter adapts the proportions and hierarchy of pdfme's official MIT-licensed [Invoice example](https://github.com/pdfme/pdfme/tree/main/playground/public/template-assets/invoice). SVG, expression, multi-variable and table schemas that Piqae cannot yet map exactly are not copied. The checked-in starter uses only the exact text, line and binding subset and requires no remote assets.

## Editors and adapters

`piqae.shopify-template/v1` is an editor envelope around one canonical `piqae.document/v1` document. Its `visual`, `liquid` and `native` modes are views, not three rendering engines. Both PDFme data and normalized Liquid are retained when they can represent the canonical document exactly. Visual edits regenerate Liquid; Liquid edits regenerate PDFme data. A failed conversion leaves the other representation intact and reports the incompatibility instead of deleting or flattening it. Visual text/QR content written as `{{ orders.0.name }}` is stored as a Piqae JSON-pointer binding; ordinary text remains literal.

### Bounded Liquid compatibility

Liquid code is parsed into canonical nodes; it is not rendered as HTML and is never executed by LiquidJS. The lossless v1 subset is deliberately line-oriented:

- A plain-text line becomes a text node. A whole-line output such as `{{ order.name }}` becomes a JSON-pointer binding.
- `{% for item in order.line_items %}` / `{% endfor %}` becomes a repeat node. Within it, `{{ item.title }}` maps to the current-item pointer `./title`.
- `{% if order.note %}` / `{% endif %}` becomes a root-pointer condition.
- `{% piqae_line %}`, `{% piqae_page_break %}`, `{% piqae_spacer 4 %}` and `{% piqae_qr order.status_url size_mm: 24 %}` map to their matching canonical nodes.
- `{% piqae_canvas %}` blocks contain absolute `piqae_canvas_text`, `piqae_canvas_qr` and `piqae_canvas_line` tags with bounded millimetre coordinates. This is the lossless bridge used by the PDFme visual editor.

Mixed text/interpolation, filters, `else`, assignments, captures, includes/renders, HTML, scripts, styles and unknown tags are rejected with a stable code and line number. Sources are limited to 32 KiB, 500 lines/nodes and eight nested blocks. Rows and stacks remain canonical-only; the editor reports that limitation instead of flattening them. Publishing is allowed only after exact conversion succeeds.

## External assets

Free-tier installations do not permanently upload fonts or images to Piqae. Published revisions store HTTPS CDN references plus expected byte length, media type and SHA-256 digest. Fetching must use an operator allowlist (`TEMPLATE_ASSET_CDN_HOSTS`), a 2 MiB per-asset and 20-asset limit, redirects disabled, private-network addresses blocked, response streaming limits, exact media-type/length/digest checks and a one-hour bounded cache. A missing or changed published asset fails closed rather than silently changing output. Paid persistent asset storage can be introduced later as a separately metered feature.
