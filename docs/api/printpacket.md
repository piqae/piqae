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

## Unsaved editor previews

An editor that needs the production PDF without saving or publishing a template
uses `POST /v1/printpacket/preview-renders`. The request contains a validated
`printpacket/v1` specification, a JSON object input, and an optional
`expires_in_seconds` from 60 through 1800 (900 by default). Poll only
`GET /v1/printpacket/preview-renders/{render_id}` and download only
`GET /v1/printpacket/preview-renders/{render_id}/artifact`.

This path creates no template, revision, approval, upload, or print job. Its
worker uses the same renderer and PDF artifact path as a published render, but
the render has a database-enforced `preview` purpose. Standard render,
readiness, approval, upload, job, and print paths do not accept it. The packet
and input are encrypted independently with workspace-, environment-, render-,
and purpose-bound authenticated data. They are never returned by the API.

At the expiry instant, both preview metadata and its PDF become inaccessible,
including to a worker that has not completed its lease. Bounded lifecycle work
then deletes the encrypted packet, encrypted input, and winning PDF artifact
asynchronously. Callers should treat a not-found response after expiry as
final and create a new preview; an idempotent replay never extends the original
expiry.

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

Printing a render accepts exactly one of `printer_id` or `target_id`. A target
request also requires the current `specification_revision` returned by
`GET /v1/targets/{id}/design-specification`. Target selection is media-aware
and renderer-aware: `prefer_node` and node-selected `automatic` search the
primary binding and then standbys for a printer that supports the exact packet,
resources, immutable profile, and loaded stock. If none does, their approved
fallback remains the retained PDF; `require_node` fails closed. `cloud_only`
never depends on node renderer capability.

A direct `printer_id` request is the zero-configuration path. Unless the caller
explicitly pins a saved profile through a target, the node delegates native
settings to the installed printer driver's current defaults. It does not
require a Piqae stock record or a fresh loaded-media observation. Driver
capabilities may prove that a document size is supported, but missing physical
stock evidence remains `unknown`; it is not an incompatibility claim. Exact
target/profile requests keep their stricter immutable profile, stock, and
loaded-media contract and never silently fall back to current defaults.

The chosen binding is pinned through job registration without becoming part of
the caller's idempotency payload. Before local acceptance, a waiting target job
may move to another ready binding—even on another physical destination—only
after the control plane atomically revalidates its route, immutable profile,
stock revision, trusted loaded-media observation, and exact node renderer for a
node-render job. The persisted job, route agent, binding, destination, and media
snapshot change together. A lease or accepted local responsibility fences this
automatic move; native handoff is never silently rewound.

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
Adjacent inline nodes concatenate without an implicit separator. Horizontal
whitespace collapses to one breakable space, while explicit `line_break` nodes
and LF, CRLF, or CR characters in resolved values start a new line. The
canonical PDF renderer measures its built-in Helvetica face for wrapping and
alignment, so preview and final output use the same typography decisions.
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
