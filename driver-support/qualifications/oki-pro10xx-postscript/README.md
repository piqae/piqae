# OKI Pro1040/Pro1050 PostScript qualification

This directory defines the evidence required for Piqae support packs for the
Windows OKI Pro1040 and Pro1050 PostScript drivers. It is deliberately **not a
loadable support pack**: no vendor-native choice is invented, and a friendly
model or driver name must never activate executable job options.

The official OKI driver pages currently identify the Windows PS Printer Driver
as version `1.2.2`. Driver downloads are not redistributed here. Each distinct
installed package inventory, architecture, locale, model and firmware range
must be captured and reviewed independently.

Authoritative starting points:

- [OKI Pro1040 drivers and utilities](https://www.oki.com/uk/printing/support/drivers-and-utilities/label/46672003/)
- [OKI Pro1050 drivers and utilities](https://www.oki.com/eu/printing/support/drivers-and-utilities/label/46672103/)
- [OKI Pro1050 specifications](https://www.oki.com/eu/printing/products/label/narrow/pro1050/specifications/)

## Capture (does not print)

Use the general [printer-driver contribution workflow](../../CONTRIBUTING.md).
The commands below are the Windows application of that vendor-neutral process.

On a Windows test computer with the official PS driver installed and a queue
for the model, open 64-bit PowerShell from a clean checkout and run:

```powershell
& .\packaging\windows\Export-PiqaeDriverEvidence.ps1 `
  -PrinterName "EXACT LOCAL QUEUE NAME" `
  -OutputPath ".piqae-test-fixtures\windows-node\oki-pro1050-ps.json"
```

Repeat for Pro1040. The exporter reads documented Windows print metadata and
PrintCapabilities only. It does not open the driver property sheet, change
defaults, submit a job, export opaque `DEVMODE` bytes, or include the queue
name. Output remains test evidence until a reviewer confirms that names and
choices contain no site-specific data.

The canonical inventory digest covers the names, lengths and SHA-256 hashes of
the installed driver, configuration, data and dependent files. It is an input
to qualification, not yet an activation selector: runtime discovery must
reproduce the same algorithm before the resulting pack can be enabled.

## Required mappings

Review the captured values against OKI documentation and create mappings only
for choices actually advertised by that exact driver package. The initial
qualification targets are:

- media sensing: gap, bottom black mark and continuous/no sensor;
- roll media width, label length, thickness and feed direction where exposed;
- 600×600 and 1200×1200 dpi where exposed;
- cutter behavior where the driver exposes a job-scoped control;
- CMYK color handling for both models;
- the Pro1050 white station/white-layer mode, including ordering or knockout
  controls only when their meaning is documented and replay tested; and
- driver-managed alignment/calibration references without pretending that a
  spooler handoff proves physical alignment.

Native choices remain read-only until an exact reverse mapping exists and is
still present in the live driver choices. Private controls that exist only in
the vendor property sheet should be captured in an immutable native profile;
Piqae replays that job-scoped profile through the installed driver rather than
attempting to decode its private bytes.

## Promotion gate

An active pack requires all of the following:

1. A reviewed, redacted capture for the exact installed PS package.
2. Exact driver identity/version/package selectors and model ID.
3. Evidence for every semantic mapping and unknown-choice rejection tests.
4. Windows replay testing without default mutation.
5. A pinned pack digest or trusted Ed25519 signature.
6. Separate, explicitly authorised physical certification before claiming
   stock sensing, white registration, cutting or output accuracy.

See [`models.json`](models.json) for the model-level facts that may guide
qualification. Those facts are informational and never become native option
keys by themselves.
