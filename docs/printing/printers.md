# Printers and destinations

**Status:** local discovery and logical printer records implemented.

A Spool printer is a logical destination backed by an operating-system queue:

- macOS/Linux: CUPS destination ID;
- Windows: installed queue name used by Winspool;
- control plane: stable Spool printer ID plus node ownership.

Friendly display names are not native identifiers. Never reconstruct a queue ID
from its label. A node may expose multiple printers, and different nodes may
expose equivalent physical devices as distinct destinations.

Discovery records capabilities and native options, but capability reporting is
not proof that a driver accepts every combination. Create profiles for known
workflows and physically test them. Disable exposure before removing or
reinstalling a queue.

The same physical printer can appear through multiple OS queues when an
operator deliberately installs different driver/port configurations. Prefer
one queue plus immutable profiles when the driver reliably restores saved
state; use separate queues when vendor behavior depends on queue-global state.

See [`05-platform-printing.md`](../05-platform-printing.md) and
[native profiles](native-profiles.md).
