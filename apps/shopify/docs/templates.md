# Shopify PrintPacket

The Shopify app owns authoring and Shopify data adaptation. Piqae receives only a validated `printpacket/v1` document and normalized data; Piqae has no Shopify or Liquid behavior.

Four immutable, dynamic starters are seeded: Invoice, Packing slip, 80 mm Receipt and Credit note. All use semantic flow tables backed by line-item collections. They reflow and paginate rather than placing fields at fixed coordinates. **Customize** creates a merchant draft. Publishing pins one immutable Piqae revision used by preview, PDF download and direct print.

## Word-like editor

The visual editor is a schema-controlled ProseMirror document surface. Paragraphs and headings are edited directly. Shopify values are inserted as typed variable chips. Line-item tables, repeaters, conditions, images, QR codes and barcodes are semantic blocks. The editor stores no HTML and has no unrestricted coordinate layout.

One template has Document, Advanced Liquid and Piqae source views. Supported Liquid is compiled into the same PrintPacket tree. Visual changes regenerate normalized Liquid. A view switch with invalid source is stopped with a line/column diagnostic; content is never silently flattened.

## Shopify Liquid profile

The bounded profile supports mixed literal text and output expressions, `money`, `date` and `number` formatting, `for` with an enforced limit, `if`/`unless`/`else`, comparisons, semantic line-item tables, QR codes, barcodes, dividers and page breaks. Includes, render, layout, arbitrary HTML/CSS, scripts, assignments, captures, plugins, network and filesystem access are rejected. Sources are limited to 64 KiB, 4,000 tokens, 12 nesting levels and 1,000 collection items.

## Assets

The editor may ingest PNG, JPEG and sanitized SVG from Shopify CDN over HTTPS. Ingestion validates a two MiB limit, media type, byte length and SHA-256 digest while blocking redirects and private addresses. Published documents reference content-addressed Piqae assets; rendering never depends on a mutable URL. Free-tier quotas remain bounded.

## Authority

PostgreSQL/Piqae is authoritative. Shopify app-owned metafields contain only a compact index of published IDs, names, kinds, media, revisions and digests. They never contain source, credentials, customer data, assets or rendered documents.
