---
name: test-windows-node
description: Build and validate the Spool Windows tray, DPAPI identity, PDFium replay, native driver profiles, installer, and WinSparkle integration. Use for Windows node or packaging changes.
---

# Test the Windows node

1. Read `AGENTS.md`, `crates/executor-windows/AGENTS.md`, and Windows packaging
   guidance.
2. Use `.spool-test-fixtures/windows-node` or an equivalent isolated Windows
   path owned by the checkout.
3. Run Windows-target Rust checks and installer validation without installing
   into another user's profile.
4. Validate tray responsiveness, DPAPI round-trip, unsigned package structure,
   profile capture cancellation, PDFium digest, and uninstall cleanup.
5. Record Windows build, architecture, commands, hashes, and results.

Physical HP/OKI replay requires explicit approval naming the printer, stock,
profile, and safe fixture. Never log DPAPI plaintext, device keys, enrolment or
device codes, lease tokens, native DEVMODE payloads, or documents. Remove only
isolated test state and unsigned artifacts.
