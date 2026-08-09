# SDK access to professional printer, profile, and stock capabilities

## Outcome

Piqae can expose substantially more useful professional-printing information to
applications, but it should not pretend that every vendor option has a universal
meaning or allow remote apps to edit opaque driver state directly.

The recommended contract has three layers:

1. **Portable facts** for genuinely common concepts such as dimensions, copies,
   resolution, colour availability, duplex, source, media form, and imageable
   area.
2. **Normalized semantic facets** for structured professional concepts such as
   roll sensing, white/effect stations, finishing, registration, colour intent,
   and stock geometry. Every value carries provenance, confidence, units, and
   mutability.
3. **Certified workflow profiles** that pin the complete native driver state,
   driver fingerprint, stock, dependencies, validation and physical test
   evidence. Apps normally select one of these rather than assembling arbitrary
   vendor combinations.

The installed operating-system/vendor driver remains authoritative. Piqae
normalizes what it can prove, preserves unrecognized options for display, and
uses immutable profiles for accurate replay.

## What applications can use today

`GET /v1/printers` and `GET /v1/printers/{printer_id}` already expose:

- stable Piqae printer and node IDs, name, state, and update time;
- portable capabilities: bins, collate, colour, copies, DPI, duplex, extents,
  media names, N-up, paper dimensions, print rate, and custom-size support;
- a monotonic capability revision;
- driver-native option keys, display names, choices, defaults, and current
  selections where the node can discover them;
- named immutable profile revisions with status, native kind/digest, driver
  fingerprint, display-safe summary, stock binding, safe overrides, validation,
  test, and publication state.

Stocks already provide typed sheet/label/roll/continuous/envelope/card kind,
width, height/length, orientation, gap, mark interval, bleed, safe area, source,
media, and extensible attributes. Targets can return a complete design
specification containing stock, readiness, each printer/profile binding, and a
stable specification revision.

The TypeScript SDK and MCP expose those public resources now. This is enough for
an app to list permitted printers/profiles, present known paper/media/source
choices, select a tested target or immutable profile revision, retrieve stock
geometry, cache a design specification by revision, and explain readiness.

## Current limits

The current model is not yet sufficient for a rich cross-vendor professional
print editor:

- Windows discovery currently returns an empty portable capability/native-option
  snapshot even though captured DEVMODE replay exists.
- CUPS parsing exposes option names and enumerated choices but does not assign
  semantic meaning, units, constraints, conflicts, or UI grouping.
- Native options are choice lists only; they cannot represent numeric ranges,
  booleans, matrices, ordered sequences, conditional availability, or compound
  vendor settings.
- Stock attributes are partly typed but remain open-ended and do not express
  tolerances, printable area, winding, liner, calibration, colour/effect
  compatibility, or per-source loaded-media evidence in the public contract.
- Profile summaries do not distinguish authoritative, inferred,
  operator-entered, or physically certified facts.
- The API does not expose option dependencies/conflicts or a safe “is this
  combination valid?” negotiation endpoint.
- A vendor display string such as “White + CMYK” is not stable enough for app
  logic, localisation, or migration between driver versions.

An app should therefore currently choose a published target/profile for an OKI
Pro1050-class job instead of constructing white-toner, sensing, registration,
and finishing settings field by field.

## Proposed capability document

Add an additive, versioned document beside the existing `Printer` schema. Do
not replace current fields until SDK consumers have migrated.

```json
{
  "schema_version": 1,
  "printer_id": "ptr_...",
  "capability_revision": 42,
  "driver_fingerprint": {
    "platform": "windows",
    "driver_name": "OKI Pro1050(PS)",
    "driver_version": "1.2.2"
  },
  "facets": {
    "media.geometry": {
      "type": "dimensions",
      "unit": "mm",
      "range": { "width": [25.4, 130], "length": [12.7, 1320] },
      "source": "driver",
      "confidence": "authoritative"
    },
    "media.sensing": {
      "type": "enum",
      "values": ["continuous", "gap", "black_mark"],
      "source": "certified_mapping",
      "native_keys": ["OkiMediaSense"]
    },
    "color.stations": {
      "type": "set",
      "values": ["cyan", "magenta", "yellow", "black", "white"],
      "source": "certified_mapping"
    },
    "effects.white.mode": {
      "type": "enum",
      "values": ["off", "underprint", "overprint", "spot"],
      "mutability": "profile_only",
      "source": "certified_mapping"
    }
  },
  "constraints": [
    {
      "if": { "media.sensing": "black_mark" },
      "requires": ["stock.mark_interval_mm", "profile.registration"]
    }
  ],
  "unmapped_native_options": ["OkiVendorOption17"]
}
```

Each facet should include a stable semantic key/version, data type, canonical
units/tolerance, allowed/current/default values, mutability, provenance,
confidence, native derivation keys, optional UI metadata, and dependency or
conflict references. Unknown facet keys and enum values must be ignored safely
by older SDKs.

## Professional semantic vocabulary

Start small and evidence-driven:

| Namespace   | Examples                                                                                                              |
| ----------- | --------------------------------------------------------------------------------------------------------------------- |
| `media.*`   | form, dimensions, thickness, weight, coating, adhesive, liner, gap, mark interval, sensing, feed orientation, winding |
| `layout.*`  | printable area, bleed, safe area, rotation, scaling, imposition, registration offsets                                 |
| `color.*`   | mode, stations, ICC intent/profile identifier, density/quality                                                        |
| `effects.*` | white, clear, foil or varnish mode and layer order when certified                                                     |
| `feed.*`    | source, roll, tray, manual feed, tension, speed                                                                       |
| `finish.*`  | cutter, cut interval, rewind, peel, stack, staple                                                                     |
| `quality.*` | DPI, screening, pass count, speed/quality intent                                                                      |
| `device.*`  | model, firmware, consumable readiness, calibration state                                                              |

Do not standardize a setting merely because two vendors use similar English.
Add a mapping only when capture/replay and physical evidence demonstrate
equivalent behavior. Otherwise retain the native option as display-only and
require a tested profile.

## Intended printer-family coverage

The core must continue to work for ordinary office and receipt/label printers
without a vendor support pack. Deeper packs should prioritize commercially
accessible business equipment whose important behavior lives in driver options:

- **General business printers:** IPP Everywhere/AirPrint-class printers and
  installed CUPS or Windows queues, with paper, trays, media, colour, duplex,
  resolution, printable area and finishing where reported.
- **OKI business and Pro label printers:** office/graphic models plus Pro1040 and
  Pro1050-class roll-label workflows. The Pro1050 specifically combines CMYK
  with white, gap/black-mark/continuous detection, adjustable sensors, roll
  winding, media thickness, custom dimensions and cutting.
- **Brother professional label/mobile printers:** TD, TJ, RJ and compatible
  families using continuous or die-cut media, black-mark/gap sensing, cutter or
  peel behavior and model-specific raster/driver choices. QL-family support can
  share the basic stock model while retaining its device-specific roll rules.
- **Epson ColorWorks:** C4000, C6000/C6500 and C8000-class colour-label devices,
  including matte/gloss ink/media compatibility, gap/black-mark detection,
  custom roll/die-cut geometry, cutter/peeler variants, print-quality modes and
  OS/driver ICC colour-management paths.
- **Business card printers:** commonly deployed Zebra, Evolis, HID FARGO,
  Magicard and Entrust families. These need card thickness/size, input/output,
  single/dual side, ribbon panel sequence, resin black, overlay/varnish,
  laminate, magnetic/smart-card exclusions, orientation and encoding/finishing
  readiness. Encoding must remain a separately permissioned operation rather
  than an incidental print option.
- **Other contributed commercial families:** Primera, Afinia/Memjet, TSC,
  SATO, Honeywell, cab, Rollo and comparable business devices may be added when
  maintainers provide legal fixtures, exact driver fingerprints and evidence.

This list sets priorities, not support claims. A family is `discovered` when
Piqae can report its driver data, `mapped` when semantics are verified,
`replay-tested` when native profiles reproduce deterministically, and
`physically-certified` only after the checked-in media/output matrix passes.
Support stays scoped to exact operating system, architecture, driver package,
firmware range, connection path and tested workflows.

## Colour, effects, and document preparation

Treat colour management as a linked set of resources rather than a profile name
string:

- input document colour space and declared spot/separation names;
- process stations/colorants available in the device and installed supplies;
- rendering intent and black-generation/black-preservation choices when the
  driver exposes them;
- ICC profile identifier, digest, source, version and applicable printer/media/
  quality combination;
- whether conversion happens in the application, operating system, driver,
  RIP, or device;
- white/clear/overlay/varnish layer role, order and knockout/overprint policy;
- calibration date/state and evidence for the exact stock/profile; and
- a soft-proof/display hint clearly separated from execution authority.

Do not distribute third-party ICC files unless their licence permits it. A
support pack can map identifiers and expected digests while requiring the
operator to install vendor-supplied profiles locally. Native blobs and licensed
vendor resources remain node-local.

The workflow contract should declare required named separations, accepted PDF
version/features, transparency-flattening expectations, page boxes, scaling,
bleed and raster/vector constraints. It should return a preparation error before
upload or print when a file lacks a required white/effect separation. “Overprint”
must identify whether it refers to PDF graphics semantics, driver white-layer
behavior, ribbon overlay, or a second physical pass; those are not interchangeable.

## Open-source driver support packs

Make mappings data-driven and reviewable rather than accumulating vendor/model
conditionals in executor code. A support pack should be a versioned directory:

```text
driver-support/epson/colorworks-c6000/
├─ manifest.yaml
├─ mappings/
│  ├─ windows-printticket.yaml
│  └─ cups-ipp.yaml
├─ schemas/
│  └─ stock-extension.schema.json
├─ fixtures/
│  ├─ capabilities.redacted.json
│  └─ profile-summary.redacted.json
├─ tests/
│  └─ conformance.yaml
├─ evidence/
│  └─ README.md
└─ LICENSES.md
```

The manifest records vendor/family, maintainer, pack/schema version, exact
hardware/firmware and driver fingerprint selectors, platforms, connection
paths, supported semantic facets, required local assets, evidence tier and
licences. Mapping rules may translate only display-safe driver output into
semantic facets; they never contain executable vendor code, private keys,
licensed binaries, opaque native profile blobs, customer serial numbers or
print documents.

Pack matching must be deterministic and fail closed:

1. exact signed/hashed driver-package fingerprint and platform;
2. exact normalized driver identifier/version range explicitly listed by the
   pack; and
3. optional device/firmware constraints confirmed from authoritative discovery.

Friendly model-name substring matching may suggest a pack to an operator but
cannot enable execution mappings automatically. If no pack matches, Piqae still
provides portable discovery, display-only native options, and locally captured
opaque profiles.

Contribution acceptance requires:

1. provenance and redistribution rights for every fixture and mapping;
2. redaction validation and bounded fixture sizes;
3. schema validation plus positive, negative and unknown-option tests;
4. proof that mappings never broaden `safe_overrides` or bypass native driver
   normalization;
5. capture/replay evidence on the exact declared matrix;
6. physical evidence for any alignment, sensing, colour/effect or finishing
   claim; and
7. a maintainer/expiry policy so stale packs become informational rather than
   silently authoritative.

Third-party packs can live outside the main repository, but production loading
should require an operator-configured trust root and a signed pack digest.
In-tree packs use normal DCO review, CI and release evidence. Packs extend
normalization; they do not replace the installed driver or Piqae's safety rules.

## Rich stock contract

Evolve stock from a loose attribute bag into a versioned specification while
retaining `attributes` for forward compatibility:

- nominal width/length plus manufacturing and accepted tolerances;
- sheet, continuous roll, die-cut label, card, envelope, or other form;
- gap, black-mark geometry/location, pitch, corner radius, and repeat interval;
- liner, adhesive, face material, coating/finish, colour and opacity;
- thickness, basis weight, core/outer diameter, winding and feed direction;
- bleed, safe area, printable area and registration tolerance;
- compatible sensing modes, sources, profiles, colour/effect processes, cutter
  modes and finishing;
- barcode/SKU and operator loading/calibration instructions; and
- provenance and revision for every externally sourced definition.

Separate stock definition from loaded-media state. Loaded state should include
device/source, stock revision, remaining estimate when trustworthy, confidence,
who or what confirmed it, and timestamp.

## SDK experience

The SDK should provide both raw and opinionated paths:

```ts
const document = await account.printers.capabilityDocument(printerId);
const workflows = await account.printWorkflows.list({ printerId });
const validation = await account.printWorkflows.validate(workflowId, {
  stockId,
  document: { widthMm: 80, heightMm: 102, colorants: ["cmyk", "white"] },
});
```

`capabilityDocument` is for advanced UI discovery. `printWorkflows` should be the
default abstraction: a published, tested target/profile/stock contract with
layout requirements, document kinds, permitted overrides, readiness and
evidence status.

Validation should be pure and non-printing. It returns errors, warnings,
required local actions, and the exact specification revision. It must not open a
driver dialog, mutate queue defaults, capture a profile, or submit a test job.

## OKI Pro1050 qualification slice

Use the OKI Pro1050 as the first deep professional-driver fixture, followed by
one Brother professional label printer, one Epson ColorWorks model and one card
printer. Keep the schema generic. The OKI Windows qualification matrix should
cover:

- PS versus PCL driver package/version and firmware;
- continuous, gap/die-cut, and black-mark media;
- label width, length/pitch, gap/mark interval and sensor-registration slot;
- feed direction, print/cut-position correction, cutter and rewind behavior;
- CMYK versus CMYK+white modes, underprint/overprint/spot behavior, layer/order
  assumptions and required document preparation;
- ICC/profile identifiers and quality/resolution choices;
- restart, queue/port change, driver upgrade, rollback and stale-state behavior;
  and
- physical alignment, colour, barcode, repeat and low-stock evidence.

File formatting remains explicit. A driver profile cannot make an arbitrary PDF
contain a correct white separation. The workflow specification should state
document kind, page/label geometry, colour-space/separation convention, bleed,
orientation, scaling and preprocessing requirements. RAW data uses a separately
certified target and never inherits rendered-driver semantics.

## Delivery sequence

1. Capture redacted Windows PrintCapabilities/PrintTicket, DEVMODE summary,
   CUPS/IPP and PPD fixtures for simple and professional printers.
2. Define additive capability-document, stock-specification and
   workflow-validation schemas in OpenAPI; regenerate the SDK and prove unknown
   field/enum compatibility.
3. Populate Windows portable facts from documented DEVMODE,
   `DeviceCapabilitiesW`, Print Capabilities and normalized PrintTicket.
4. Ship versioned semantic mappings keyed by exact driver fingerprint, with
   provenance and fail-closed tests; never use fuzzy model names for execution.
5. Expose raw capability documents plus workflow-oriented selectors and pure
   validation in SDKs and UI.
6. Add revisioned stock specifications, a public loaded-media projection and
   detailed readiness reasons.
7. Physically qualify OKI Pro1050 workflows, then add professional families
   based on fixtures and demand.

Steps 2 and 6 change public and persistent contracts and therefore require an
OpenAPI compatibility review and append-only PostgreSQL migrations. No support
tier should advance from simulated or spooler-only evidence.

## Vendor references for fixture planning

These sources establish example hardware capabilities only; they are not Piqae
certification evidence:

- [OKI Pro1050 specifications](https://www.oki.com/eu/printing/products/label/narrow/pro1050/specifications/)
  document CMYK+white, roll dimensions/thickness, gap/black-mark/continuous
  detection, winding, resolution and cutter limits.
- [Epson ColorWorks C6000/C6500 technical reference](https://files.support.epson.com/pdf/pos/bulk/cw-c6000_c6500_trg_en_rev_h.pdf)
  documents its media, quality and OS ICC colour-correction paths.
- [Brother TD-2130N user guide](https://www.brother-usa.com/-/media/brother/product-catalog-media/documents/2020/05/12/18/32/cv_td2130n_usaeng_usr_c.pdf)
  provides an example of die-cut and black-mark media handling.
- [Zebra ZC300 specifications](https://www.zebra.com/us/en/products/spec-sheets/printers/card/zc300-series.html)
  and [ZXP Series 7 guide](https://cpws.zebra.com/cpws/docs/crawl/UG_Card/ZXP7_UG.pdf)
  illustrate ribbon, overlay/varnish and card-driver requirements.
