# Node document rendering negotiation

Node-side rendering is an optional optimisation, not a different print mode.
The default and fallback remain a server-rendered PDF submitted through the
installed operating-system driver. Generic document specifications must never
be forwarded as printer RAW data. RAW output remains limited to an explicitly
authorised, printer-specific native profile.

The reusable negotiation and parity implementation lives in
`piqae_agent_core::document_render`. It is not yet advertised over the public
agent protocol, so the production path continues to transfer the server PDF.

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

## Future protocol integration gate

Before adding wire fields, define an additive optional capability on agent sync
and a document content descriptor containing only immutable spec/input object
references, digests, limits and the ordinary PDF fallback. Unknown fields must
remain ignorable by old nodes. Offers must be claim- and tenant-bound, bounded,
encrypted like other document content, and idempotent across reconnects.

Enabling the optimisation requires:

1. cross-platform deterministic golden fixtures;
2. a fault test that interrupts download, render and fallback selection;
3. metrics for selection, fallback reason, parity mismatch and latency;
4. a staged feature flag with immediate server-PDF rollback;
5. confirmation that artifact cleanup retains both candidates until durable
   agent acceptance.

No public support claim should be made before these gates and physical-printer
certification are complete.
