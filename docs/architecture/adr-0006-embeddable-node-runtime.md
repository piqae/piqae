# ADR 0006: one embeddable node runtime with capability-aware hosts

- Status: Accepted
- Date: 26 August 2026
- Related: [RFC 0001](../rfcs/0001-embeddable-node-runtime.md)

## Decision

Piqae will expose its durable node as one OS-independent Rust runtime with
capability-aware host adapters. The shipped agent binary, desktop companion
service, native SDKs, and app-embedded nodes all compose that runtime rather
than owning parallel queues or cloud clients.

Desktop SDKs attach to an installed node by default. Embedded mode uses a
separate app-scoped installation only when no broker is available or the caller
explicitly requests isolation. iPadOS uses embedded mode and advertises
foreground or opportunistic availability; it is never treated as continuously
reachable solely because the app is installed.

Wake requests never allocate a job lease. Physical-destination fencing begins
only after a fresh authenticated availability observation. Failover is allowed
only before the native handoff can have succeeded.

Local-only operation and self-hosting remain equal runtime configurations.
Cloud support is added through tenant-scoped connectors and never through a
secret embedded in an application.

## Consequences

- `piqae-agent` becomes a composition root and existing shells move to the
  versioned node-client contract.
- Platform adapters report their actual capabilities and lifecycle limits.
- Swift and Windows bindings share the state machine and conformance fixtures.
- App-specific UI may be fully custom without reimplementing durable behavior.
- iPad unattended printing requires an explicitly supported kiosk, accessory,
  direct-printer, or always-awake gateway topology.
- Independent control planes cannot silently share a global delivery fence.
