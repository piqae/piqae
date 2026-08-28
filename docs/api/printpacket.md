# PrintPacket

`printpacket/v1` is the sole vendor-neutral, portable, bounded packet format.
Pre-release identifiers are rejected rather than normalized or migrated. The
format is optional: callers can continue submitting PDF or RAW jobs without
templates.

Use `/v1/printpacket/templates` to create encrypted drafts and publish
immutable revisions. Register an asynchronous render with
`/v1/printpacket/renders`, then poll its metadata. A completed artifact
can be downloaded, printed directly, or retained in an expiring preview approval
gate. Preview, download, and print all refer to the same immutable bytes.

Print and preview approval accept `render_policy`: `automatic` (the default),
`cloud_only`, `prefer_node`, or `require_node`. Automatic uses a conservative,
versioned cost model with server-measured PDF and input byte lengths and the
completed render's authoritative page count. Call the
render's `render-readiness` endpoint for an authenticated destination capability
snapshot and explicit decision reason. `prefer_node` falls back only to the
exact approved server PDF; `require_node` rejects approval unless the selected
printer reports the exact renderer/resource ABIs and can acquire every
referenced resource through the active lease. A cold cache is reported as
`resources_warming`; it is compatible and remains digest-verified before use.
Node offers without the authoritative page count are incompatible and cannot
select node rendering. Policy may select the retained server PDF only when the
caller explicitly permits cloud fallback.

JPEG resources are uploaded once by lowercase SHA-256 using
`PUT /v1/printpacket/resources/{digest}`. Uploads are bounded to 4 MiB,
verified before registration, tenant-scoped, retained while referenced by a
render, and downloaded by nodes only through an authenticated active job lease.
Nodes verify the same digest and length before admitting bytes to their local
content-addressed cache. Cloud rendering resolves them from the same
tenant/environment namespace with a 16 MiB aggregate bound. The shared renderer
verifies the complete declared set, including JPEG structure and pixel bounds;
the renderer itself never fetches URLs.

The TypeScript SDK exposes these operations as `client.printPackets`.
MCP exposes metadata-safe operations through `piqae_print_packets`; it
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
`cargo run --release -p printpacket-renderer --example render_batch -- 250 20`.
This measures only deterministic local rendering; it is not a network, spooler,
fleet, or paper-delivery SLO.

Source languages and editors are outside the control plane. Integrators compile
their own Liquid, visual-editor, or programmatic source into this format. Piqae
does not expose a hosted conversion endpoint and does not execute source
languages, HTML, plugins, JavaScript, files, or remote URLs.
