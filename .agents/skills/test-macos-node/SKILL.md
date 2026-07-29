---
name: test-macos-node
description: Build and validate the Spool macOS menu app, node IPC, native profile capture, packaging, and non-physical integration. Use for shells/macos, CUPS executor, LaunchAgent, or Sparkle changes.
---

# Test the macOS node

1. Read `AGENTS.md`, `shells/macos/AGENTS.md`, and the native safety rules.
2. Use `.spool-test-fixtures/macos-node` for all disposable state.
3. Run `swift test --package-path shells/macos`.
4. Run focused Rust tests for `spool-agent`, `spool-local-api`, and
   `spool-executor-cups`.
5. Build an unsigned local package only; verify launch, IPC, tray responsiveness,
   profile-dialog cancellation, and clean shutdown.
6. Record architecture, macOS version, commands, and test results.

Do not open a real driver dialog or print unless the user explicitly names the
printer and fixture. Never log Keychain material, node private keys, API keys,
device codes, native profile payloads, or document content. Remove only the
skill state and unsigned local artifact.
