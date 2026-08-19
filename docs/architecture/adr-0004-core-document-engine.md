# ADR-0004: Portable business-document format

Status: accepted

Piqae exposes one editor- and provider-neutral wire format:
`piqae.business-document/v1`. It is a bounded semantic document tree for
invoices, receipts, purchase orders, packing documents, and simple labels.

Applications own authoring. A Shopify application may offer a Word-like editor
and Liquid source; another integrator may use its own editor or language. Those
applications compile locally into the public Piqae format. The control plane
does not host adapters and does not execute Liquid, HTML, JavaScript, plugins,
files, or remote template code.

The format provides paged, continuous, and fixed-size label media; flow blocks;
headers and footers; paragraphs and typed inline expressions; rows, grids,
repeats, conditions and tables; images as content-addressed resources; QR codes;
and Code 128 barcodes. It deliberately excludes arbitrary canvases and general
graphic-design/prepress semantics.

Published revisions and render input are encrypted before persistence. Render
workers claim durable registrations, create an immutable artifact, and retain
the exact artifact through preview approval, download, and zero-copy print-job
registration. A spooler acceptance remains distinct from physical delivery.

The initial renderer ABI uses bounded Windows-1252 typography and verified JPEG
resources. It explicitly rejects characters outside that profile, PNG/SVG,
styled table-cell runs, and unsupported image fitting rather than producing
degraded output. The checked-in renderer tests and support matrix, not the
breadth of the schema, determine what can be claimed as supported.

The pre-release legacy document format, unrestricted canvas, hosted conversion
API, and editor-specific adapter package were removed in migration 0038. That
migration deletes only pre-release document subsystem rows and detaches retained
uploads/jobs; it does not delete accounts, nodes, printers, queues, or unrelated
jobs.
