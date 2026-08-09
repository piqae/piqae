# Shopify document templates

PostgreSQL/Piqae is authoritative. Shopify receives only an app-installation-owned `piqae.template_index` JSON metafield containing published IDs, names, kinds, page sizes, revision numbers and SHA-256 digests. It contains no source, customer data, credential, font, image or rendered document. Failure to update this cache fails the mutation visibly; it never changes the authority boundary.

Eight immutable system documents are seeded on first use. **Customize** clones one into a merchant draft. Publishing first converts the editor envelope to `piqae.document/v1`, creates an immutable revision in the linked Piqae account and records its revision ID and canonical digest. Preview, download and direct print must reference that exact revision/artifact.

## Editors and adapters

`piqae.shopify-template/v1` is an editor envelope around one canonical `piqae.document/v1` document. Its `visual`, `liquid` and `native` modes are views, not three rendering engines. Visual data is a deliberately bounded PDFme-compatible model. Liquid uses the app's bounded allowlist. Neither arbitrary PDFme plugins nor arbitrary Liquid/HTML are supported. A switch is allowed only with an explicit `lossless`, `lossy` or `unsupported` result and warnings; the canonical document remains authoritative.

### Bounded Liquid compatibility

Liquid code is parsed into canonical nodes; it is not rendered as HTML and is never executed by LiquidJS. The lossless v1 subset is deliberately line-oriented:

- A plain-text line becomes a text node. A whole-line output such as `{{ order.name }}` becomes a JSON-pointer binding.
- `{% for item in order.line_items %}` / `{% endfor %}` becomes a repeat node. Within it, `{{ item.title }}` maps to the current-item pointer `./title`.
- `{% if order.note %}` / `{% endif %}` becomes a root-pointer condition.
- `{% piqae_line %}`, `{% piqae_page_break %}`, `{% piqae_spacer 4 %}` and `{% piqae_qr order.status_url size_mm: 24 %}` map to their matching canonical nodes.

Mixed text/interpolation, filters, `else`, assignments, captures, includes/renders, HTML, scripts, styles and unknown tags are rejected with a stable code and line number. Sources are limited to 32 KiB, 500 lines/nodes and eight nested blocks. Rows, stacks, canvases and literal QR values remain canonical/visual-only; the editor reports that limitation instead of flattening them. A supported Liquid save replaces the canonical body and stores normalized Liquid, making later Liquid ↔ canonical switching deterministic. Publishing is allowed only after this exact conversion succeeds.

## External assets

Free-tier installations do not permanently upload fonts or images to Piqae. Published revisions store HTTPS CDN references plus expected byte length, media type and SHA-256 digest. Fetching must use an operator allowlist (`TEMPLATE_ASSET_CDN_HOSTS`), a 2 MiB per-asset and 20-asset limit, redirects disabled, private-network addresses blocked, response streaming limits, exact media-type/length/digest checks and a one-hour bounded cache. A missing or changed published asset fails closed rather than silently changing output. Paid persistent asset storage can be introduced later as a separately metered feature.
