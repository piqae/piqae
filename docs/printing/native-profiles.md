# Native print profiles

**Status:** immutable profile storage and macOS/Windows capture foundations are
implemented; release support follows each platform tier.

A profile is a named, revisioned snapshot of settings for one exact native
queue and driver fingerprint. It may include:

- portable summary fields such as paper, source, colour, duplex, and DPI;
- an opaque native blob such as PrintCore state or full Windows DEVMODE;
- stock binding, dependencies, and explicitly safe per-job overrides;
- last validation/test result and publication state.

Add opens the operating system or manufacturer's real driver UI. Edit restores
the selected immutable revision and saves a new revision. Clone starts from the
same revision but creates a new profile identity. Jobs pin the exact profile
revision so later edits cannot silently change an accepted job.

Native blobs stay on the node and are not editable in the web UI. The control
plane receives safe summaries and routing state. A driver, queue, port, or
device fingerprint change can make a profile stale or mismatched.

A newly captured profile needs a physical driver test before publication.
Validation proves structure and compatibility; it does not prove media is
loaded, alignment is correct, or consumables are suitable.

The detailed model is
[`16-native-print-profiles-stock-and-routing.md`](../16-native-print-profiles-stock-and-routing.md).
