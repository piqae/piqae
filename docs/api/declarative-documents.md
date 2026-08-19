# Business documents

`piqae.business-document/v1` is Piqae's portable, bounded business-document
format. It is optional: callers can continue submitting PDF or RAW jobs without
using templates.

Use `/v1/business-document-templates` to create encrypted drafts and publish
immutable revisions. Register an asynchronous render with
`/v1/business-document-renders`, then poll its metadata. A completed artifact
can be downloaded, printed directly, or retained in an expiring preview approval
gate. Preview, download, and print all refer to the same immutable bytes.

The TypeScript SDK exposes these operations as `client.businessDocuments`.
MCP exposes metadata-safe operations through `piqae_business_documents`; it
never returns template source or render input.

The format supports paged A4/A5/Letter media, bounded continuous media, and
simple labels. Its semantic nodes cover flow sections, rich inline text,
rows/grids, repeats, conditions, tables, headers/footers, QR and Code 128.
The initial renderer ABI supports host-resolved, content-addressed JPEG
resources after verifying their digest, length, dimensions and decoded-pixel
bounds. It rejects characters outside Windows-1252, PNG/SVG, styled table-cell
runs and unsupported image fitting explicitly. Consult renderer conformance
tests and the support matrix before making support claims.

Source languages and editors are outside the control plane. Integrators compile
their own Liquid, visual-editor, or programmatic source into this format. Piqae
does not expose a hosted conversion endpoint and does not execute source
languages, HTML, plugins, JavaScript, files, or remote URLs.
