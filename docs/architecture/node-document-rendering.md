# Node document rendering negotiation

Node-side rendering is an optional optimisation, not a different print mode.
The default and fallback remain a server-rendered PDF submitted through the
installed operating-system driver. Generic document specifications must never
be forwarded as printer RAW data. RAW output remains limited to an explicitly
authorised, printer-specific native profile.

The reusable negotiation and parity implementation lives in
`piqae_agent_core::document_render`. Installed standalone nodes advertise the
optional renderer and resource ABIs during authenticated sync. A selected job
offer carries the immutable specification and input, the server PDF digest and
byte length, the completed render's authoritative page count, tenant-scoped
resource descriptors, and the ordinary server PDF fallback. Older nodes and
offers without an authoritative page count fail closed to that PDF.

## Contract

A node may be offered a document only when it advertises all of the following:

- negotiation version `1`;
- exact renderer ABI and build;
- the exact document specification version;
- deterministic rendering;
- input, output and page limits large enough for the immutable job.

The offer must retain a normal server PDF and its trusted SHA-256 digest. The
node renders into a bounded temporary artifact and compares the result with
that digest. A capability mismatch, malformed digest, render failure, resource
limit, version change or digest disagreement selects the retained server PDF
and records a structured fallback reason. It must not silently continue with
different bytes.

This exact-build rule is intentionally conservative. A future compatibility
fixture suite may allow distinct builds to share an ABI only after byte-level
golden parity has been demonstrated on every supported platform.

Cloud rendering resolves every declared JPEG from the tenant/environment object
namespace under 4 MiB per-resource and 16 MiB aggregate in-memory bounds. The
shared renderer verifies the complete resolved set's descriptor length, SHA-256,
JPEG structure, dimensions, and decoded-pixel bound before layout, including
declared resources that are not selected by a dynamic expression.

## Support boundary and rollout gates

The additive wire contract and virtual fallback/require-node coverage are
implemented. `prefer_node` keeps byte-identical PDF fallback; `require_node`
fails before job creation when the exact destination capability is unavailable,
and fails closed on the node if acquisition or deterministic parity later fails.
This is evidence for protocol and virtual execution only, not proof of paper
delivery or physical-printer certification.

Broader production support claims still require:

1. cross-platform deterministic golden fixtures;
2. a fault test that interrupts download, render and fallback selection;
3. metrics for selection, fallback reason, parity mismatch and latency;
4. a staged feature flag with immediate server-PDF rollback;
5. confirmation that artifact cleanup retains both candidates until durable
   agent acceptance.

No public support claim should be made before these gates and physical-printer
certification are complete.
