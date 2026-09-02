# Order Printer Pro parity direction

Status: pre-release implementation direction, August 2026.

Piqae Order Printing uses a structured, Word-like Shopify authoring experience backed by the open `printpacket/v1` format. It is deliberately not a general graphic-design or arbitrary-position PDF tool. Its priorities are dynamic commerce documents, deterministic pagination, fast rendering, exact preview-to-print artifacts, and eventual node-local rendering.

## Product boundary

The initial format targets invoices, packing slips, receipts, credit notes, returns forms, purchase orders, and simple barcode/batch labels. Semantic paragraphs, headings, styled inline values, sections, rows, grids, tables, repeaters, conditions, headers, footers, images, QR codes and Code 128 barcodes describe the intended document. The renderer owns measurement, reflow, page breaks and continuous-media length.

Shopify owns its editor, Liquid profile, variable catalogue and normalized order snapshot. Piqae owns only the language-neutral document standard, validation, rendering, artifact approval and print delivery. Other integrators can provide different editors or source languages and compile into the same standard.

## Editor

The Shopify editor is a schema-controlled ProseMirror surface with direct text editing, variable chips and semantic PrintPacket blocks. Design and Code are two lossless views of the same canonical `printpacket/v1` document; valid Code edits update Design live, while invalid JSON is kept as a draft and blocks the mode switch. The bounded Liquid mapper remains an explicit compatibility/import boundary: supported Liquid compiles into the typed tree, while unsupported executable or presentation constructs fail with stable diagnostics. HTML, CSS, includes, plugins, scripts, filesystem and network access are not execution options.

Four dynamic starters ship initially: Invoice, Packing slip, 80 mm Receipt and Credit note. Each uses collection-backed tables and is tested against empty, small and large line-item collections.

## Rendering and printing

Preview, PDF download and direct print must reference one immutable published revision and artifact. A print approval releases that exact preview. Node-local rendering may be selected only when renderer ABI, template digest, assets, fonts and target capabilities match; otherwise the cloud-created artifact is delivered without a visually different fallback.

Continuous receipt documents use natural measured length. Native receipt commands require certified complete feature mappings; otherwise Piqae selects an explicit raster/PDF backend before submission.

## Parity gates

- Dynamic tables, repeated headers and bounded pagination
- Discounts, tax, refunds, fulfillments and multi-currency snapshots
- Bulk progress, partial retry and stable idempotency
- QR/barcode decoding from final output
- Shopify Admin, POS and customer-account acceptance testing
- Cloud/node determinism fixtures
- Accessibility and localization
- Explicit Shopify approval for protected scopes and surfaces

Passing virtual-spooler tests proves software delivery behavior, not that ink reached paper. Physical support claims remain limited to explicitly certified printers and workflows.
