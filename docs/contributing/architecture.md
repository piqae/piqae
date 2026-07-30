# Contributing to the architecture

Piqae keeps durable edges explicit:

```text
integration → control plane/PostgreSQL/S3 → agent SQLite → executor → OS spooler
```

The control plane is portable Rust. Nodes own driver access and native blobs.
Tray/menu apps are disposable clients of the authenticated local API. PostgreSQL
coordinates leases, idempotency, routing, outboxes, and tenant state; object
storage holds content; SQLite protects accepted local work.

Before changing a boundary:

1. Identify which process owns durability and retries.
2. Define crash behavior before and after every external side effect.
3. Preserve tenant scoping and exact profile revision pins.
4. Keep native driver state opaque and node-local.
5. Add protocol versioning and N/N-1 compatibility where applicable.
6. Test duplicate, timeout, stale lease, restart, and partial failure cases.

Avoid hidden queues, in-memory-only acceptance, silent option fallback,
browser-held infrastructure credentials, and tray-owned business state.

Start with [`03-architecture-and-stack.md`](../03-architecture-and-stack.md),
[`04-protocol-queues-and-state.md`](../04-protocol-queues-and-state.md), and the
[architecture decision records](../architecture/adr-0001-rust-postgres-durable-edges.md).
Material reversals require an RFC/ADR.
