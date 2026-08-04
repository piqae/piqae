# macOS node

**Release tier:** Preview on macOS 13+ Apple silicon and macOS 15 Intel.
Local development artifacts are unsigned. Credentialed tagged Preview
artifacts are Developer ID signed, Apple notarised, and stapled; this does not
promote the platform to Supported without the remaining installation, update,
rollback, and printer evidence.

**Implemented in source:** CUPS discovery/submission, durable local queue,
authenticated loopback API, menu-bar status/printers/profiles, native PrintCore
profile capture, immutable edit/clone revisions, and headless PrintCore replay.

**Tested:** automated Rust and Swift suites run in development. A real HP A4
print has been exercised during development, but that is not the formal
clean-install, packaging, driver-matrix, sleep/wake, and physical release gate.

Build the native menu app:

```sh
shells/macos/build-app.sh
open shells/macos/build/Piqae.app
```

The menu app connects only to the local loopback agent. Add Profile opens the
real macOS print panel and saves driver settings without printing. Edit restores
the exact prior PrintCore revision; Clone creates a new profile.

Complex vendor panes are captured as opaque native state. Piqae does not
recreate them in the web UI. Test every saved profile on its actual printer and
stock before publishing it.

Service installation remains Preview. Follow
[`operations/agent-service-installation.md`](../operations/agent-service-installation.md)
and [native profiles](../printing/native-profiles.md).
