# Contributing printer-driver support

Piqae accepts evidence-backed normalization packs for any installed printer
driver: basic office printers, label/roll printers, card printers, production
devices and driverless IPP destinations. Packs contain data, never driver
binaries or executable plugins.

## 1. Capture locally without printing

Capture the exact driver package, architecture, locale and model separately.
Capturing is read-only: it must not open a driver dialog, change queue defaults,
submit a job or claim that output reached paper.

### Windows

From a clean checkout in 64-bit PowerShell:

```powershell
& .\packaging\windows\Export-PiqaeDriverEvidence.ps1 `
  -PrinterName "EXACT LOCAL QUEUE NAME" `
  -OutputPath ".piqae-test-fixtures\windows-node\driver-evidence.json"
```

This records bounded Windows PrintCapabilities plus hashes and display-safe
metadata for the installed package. It excludes the queue name and opaque
`DEVMODE`. Private driver settings should use a local immutable native profile,
not a guessed portable mapping.

### macOS, Linux and CUPS

Python 3 and the local `lpoptions` command are required:

```console
python3 driver-support/tools/capture_cups_driver.py \
  --printer EXACT_LOCAL_QUEUE_NAME \
  --output .piqae-test-fixtures/macos-node/driver-evidence.json
```

This records native option identifiers/choices from `lpoptions -l`. When a
local PPD is present it records selected display-safe PPD metadata and hashes
the PPD, but does not copy it. Driverless IPP queues may have no package digest;
that capture is useful evidence but cannot activate an exact package-selected
pack until a stable driverless fingerprint is defined.

Neither command contacts Piqae or uploads anything. Inspect the JSON locally
before sharing it.

## 2. Redact and establish redistribution rights

Never contribute queue names, serial numbers, hostnames, usernames, paths,
network addresses, documents, credentials, tokens, native profile blobs,
`DEVMODE`, PrintCore archives, proprietary binaries, full PPDs or unreviewed raw
logs/XML. Remove site-specific strings even when a driver placed them in an
option label. Fixture data must be legally redistributable under the declared
pack licence.

If safe redaction would change a native identifier or choice, do not map it.
Open an issue describing the missing facet without attaching the sensitive
capture.

## 3. Create the candidate pack

Copy `driver-support/templates/minimal` to a vendor/family directory on your
branch. Use a stable reverse-DNS `pack_id` and fill in:

- the exact package SHA-256, normalized driver ID and driver version;
- exact platform, model/device ID and firmware predicates where available;
- reviewed capability fixtures;
- one semantic mapping only for each documented native choice;
- positive and unknown-choice rejection conformance cases;
- evidence sources, capture matrix, limitations and licence.

Do not infer settings from product marketing or a friendly model name. Do not
add native options to `profile.safe_overrides`. Unknown, ambiguous, updated or
unmatched drivers must remain unmapped.

Qualification directories under `driver-support/qualifications/` may document
future model work but are never loadable packs.

## 4. Test evidence in increasing tiers

Use the lowest truthful tier:

1. `discovered`: reviewed capture exists.
2. `mapped`: authoritative sources establish semantic meaning.
3. `replay_tested`: exact job-scoped replay was tested; this still does not
   prove physical output.
4. `physically_certified`: an explicitly authorised hardware/stock fixture
   verified the stated alignment, colour, sensing or finishing claim.

Run:

```console
python3 -m unittest driver-support/tools/test_capture_cups_driver.py
cargo test -p piqae-support-packs
cargo clippy -p piqae-support-packs --all-targets -- -D warnings
cargo fmt --all -- --check
git diff --check
```

Windows captures and scripts must also run in Windows CI or an isolated Windows
test machine. Never enable physical-test environment variables for ordinary
conformance tests.

## 5. Sign, submit and maintain

Commit with DCO sign-off and open a pull request containing the pack data,
redacted fixtures, evidence, tests and support limitations. Do not upload the
manufacturer's installer. Maintainers review privacy, licence, exact matching,
semantic accuracy, failure behavior and claimed evidence tier.

Production operators activate an accepted pack only by pinning its canonical
digest or trusting its Ed25519 publisher key, as documented in
[`README.md`](README.md). A new driver package, version, architecture, locale or
firmware combination requires new evidence and selectors; it must not silently
inherit certification from an older capture.
