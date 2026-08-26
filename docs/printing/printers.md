# Printers and destinations

**Status:** local discovery and tenant-scoped printer records are implemented.
Physical-destination grouping, route telemetry, and fenced multi-route handoff
are Preview and have no physical redundancy or fleet-soak support evidence.

A Piqae `printer` is the compatibility view of one installed operating-system
queue. The native topology keeps three identities separate:

- **physical destination:** the tenant's inferred real printer;
- **route:** one installed queue on one node that can reach it; and
- **printer:** the stable compatibility resource projected for that route.

Native route identity comes from:

- macOS/Linux: CUPS destination ID;
- Windows: installed queue name used by Winspool;
- control plane: stable Piqae printer ID plus node ownership.

Friendly display names are not native identifiers. Never reconstruct a queue ID
from its label. A node may expose multiple routes. Different nodes may expose
the same physical device, but matching labels, models, drivers, or capabilities
alone never prove that they do.

The node hashes normalized hardware evidence before sending it. The control
plane immediately converts that value to a tenant-keyed HMAC and never exposes
the digest through operator APIs. An unambiguous same-kind strong identifier can
attach a newly seen route to an existing tenant destination; conflicting or
weak evidence stays separate and requires a reversible operator decision.

Discovery records capabilities and native options, but capability reporting is
not proof that a driver accepts every combination. Create profiles for known
workflows and physically test them. Disable exposure before removing or
reinstalling a queue.

The same physical destination can have multiple routes when an
operator deliberately installs different driver/port configurations. Prefer
one queue plus immutable profiles when the driver reliably restores saved
state; use separate queues when vendor behavior depends on queue-global state.

Route health is an authenticated observation, not a paper sensor. Every
observation includes `observed_at` and `fresh_until`; an expired observation must
not be treated as live or as proof that stock and consumables are present.

See [`05-platform-printing.md`](../05-platform-printing.md) and
[native profiles](native-profiles.md), plus
[multi-integrator node connectors](../api/multi-integrator-node-connectors.md).
