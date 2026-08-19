# Business documents

`piqae.business-document/v1` is Piqae's portable, bounded business-document
format. It is optional: callers can continue submitting PDF or RAW jobs without
using templates.

Use `/v1/business-document-templates` to create encrypted drafts and publish
immutable revisions. Register an asynchronous render with
`/v1/business-document-renders`, then poll its metadata. A completed artifact
can be downloaded, printed directly, or retained in an expiring preview approval
gate. Preview, download, and print all refer to the same immutable bytes.

Print and preview approval accept `render_policy`: `automatic` (the default),
`cloud_only`, `prefer_node`, or `require_node`. Automatic uses a conservative,
versioned cost model with server-measured PDF and input byte lengths. Call the
render's `render-readiness` endpoint for an authenticated destination capability
snapshot and explicit decision reason. `prefer_node` falls back only to the
exact approved server PDF; `require_node` rejects approval unless the selected
printer reports the exact renderer/resource ABIs and can acquire every
referenced resource through the active lease. A cold cache is reported as
`resources_warming`; it is compatible and remains digest-verified before use.

JPEG resources are uploaded once by lowercase SHA-256 using
`PUT /v1/business-document-resources/{digest}`. Uploads are bounded to 4 MiB,
verified before registration, tenant-scoped, retained while referenced by a
render, and downloaded by nodes only through an authenticated active job lease.
Nodes verify the same digest and length before admitting bytes to their local
content-addressed cache. The renderer itself never fetches URLs.

The TypeScript SDK exposes these operations as `client.businessDocuments`.
MCP exposes metadata-safe operations through `piqae_business_documents`; it
never returns template source or render input.

The format supports paged A4/A5/Letter media, bounded continuous media, and
simple labels. Its semantic nodes cover flow sections, rich inline text,
rows/grids, repeats, conditions, tables, headers/footers, QR and Code 128.
The initial renderer ABI supports host-resolved, content-addressed JPEG
resources after verifying their digest, length, dimensions and decoded-pixel
bounds. It rejects characters outside Windows-1252, PNG/SVG, downloadable
fonts and unsupported image fitting explicitly. Consult renderer conformance
tests and the support matrix before making support claims.

For a reproducible compute/payload probe of the bulk path, run
`cargo run --release -p piqae-document-renderer --example render_batch -- 250 20`.
This measures only deterministic local rendering; it is not a network, spooler,
fleet, or paper-delivery SLO.

Source languages and editors are outside the control plane. Integrators compile
their own Liquid, visual-editor, or programmatic source into this format. Piqae
does not expose a hosted conversion endpoint and does not execute source
languages, HTML, plugins, JavaScript, files, or remote URLs.
