# Native print profiles, stock, and routing

## Decision

Piqae presents each installed operating-system printer destination once and
allows any number of named, immutable print profiles beneath it. Creating or
editing a profile opens the operating system's real printer-driver interface.
The web application deliberately does not recreate vendor settings.

The product contract is the same on Windows and macOS:

1. discover an installed destination;
2. create, clone, edit, validate, test, publish, and retire profiles;
3. capture the complete native settings selected by the operator;
4. pin every job to a profile revision;
5. reject or hold work if the profile, driver, device, or loaded stock is not
   ready;
6. submit using the captured native configuration without changing global
   printer defaults.

The platform implementations are intentionally different:

- Windows captures and replays a complete driver `DEVMODE`, with PrintTicket
  support where it is the authoritative driver model.
- macOS captures `NSPrintInfo` print settings and page-format state through the
  system `NSPrintPanel`, plus a canonical CUPS/IPP option mirror where possible.
- Linux uses CUPS/IPP saved options and printer instances. It has no promise of
  a vendor-native graphical preferences panel.

This gives an HP printer on macOS and an OKI label printer on Windows the same
Piqae experience without claiming that their drivers expose identical
capabilities.

## User-visible model

```text
Node: Oliver's Mac
└─ Destination: HP OfficeJet Pro
   ├─ Profile: A4 colour, Tray 1
   ├─ Profile: A4 mono draft, Tray 1
   └─ Profile: A4 photo, rear feed

Node: Labels Windows
└─ Destination: OKI Pro1050 PS
   ├─ Profile: 80 mm matte, black mark
   ├─ Profile: 100 mm gloss, gap
   └─ Profile: 125 mm continuous, heavy
```

The same destination is not added repeatedly merely to hold different
settings. Duplicate operating-system queues remain supported because some
legacy drivers only behave reliably that way, but they are a compatibility
fallback rather than the primary model.

### Terms

| Product term | Meaning | Existing implementation mapping |
| --- | --- | --- |
| Node | One enrolled Piqae installation on a computer | `Agent` |
| Physical device | Actual printer hardware, optionally shared by destinations | New resource |
| Destination | An installed OS queue on one node | Existing `Printer` |
| Profile | A versioned native driver configuration for one destination | Expand existing named profile |
| Stock | Paper, roll, labels, cards, or other loaded substrate | New resource |
| Target | Stable API address resolving to one or more profile bindings | New resource |
| Binding | A target's node/destination/profile candidate | New resource |

The compatibility API continues to call destinations or flattened targets
`printers`. The native API uses the more precise terms.

## Required V1 experience

### Tray/menu application

The native shell is the primary place for hardware setup:

- list installed destinations and live state;
- open the operating system's Add Printer interface;
- expand a destination to show profiles;
- add a profile using native driver settings;
- clone a profile and reopen native settings;
- rename, make default, validate, test, publish, or retire a profile;
- show driver/version compatibility and validation age;
- show or confirm stock loaded in each printer or tray;
- show local and native queues;
- show actionable operator holds;
- open the full web dashboard for fleet administration.

The shell remains disposable. The headless agent owns IDs, revisions, local
durability, jobs, and synchronization. Native driver UI must run in the
interactive user session, so profile capture uses a narrow native profile-host
process controlled by the shell and agent.

### Web application

The web application shows:

- node, destination, physical-device grouping, and online state;
- published profiles and immutable revision;
- human-readable settings summary;
- required and currently loaded stock;
- readiness, compatibility, last validation, and last test;
- permissions, tags, routing, and API identity;
- safe overrides;
- job and profile history.

It does not provide a generic editor for arbitrary native driver keys. The
primary edit action is:

> Edit native settings on Oliver's Mac

If the selected node is local, the dashboard calls the loopback bridge. If it
is remote, it creates a short-lived, auditable action request that appears in
that node's tray application. A web request may never cause a driver dialog to
appear without local user confirmation.

## Identity and versioning

Every layer has a separate identifier:

```text
device_id       physical hardware, when known
node_id         enrolled installation
destination_id  installed OS queue on one node
profile_id      stable named Piqae profile
profile_revision immutable captured version
stock_id        operational media definition
target_id       stable API routing address
binding_id      target-to-profile candidate
```

A driver's Favorite or macOS preset name is metadata, not identity. It can be
renamed, changed, or deleted outside Piqae.

Editing a profile appends a revision. It never changes a configuration already
referenced by an accepted job. A job records both `profile_id` and
`profile_revision`.

Destination identity uses the node ID plus the strongest available native
queue identity. Physical-device grouping uses, in descending confidence:

1. manufacturer serial number or IPP printer UUID;
2. stable device UUID reported by PrintCore/Winspool;
3. network endpoint plus make/model;
4. USB topology plus make/model;
5. explicit operator confirmation.

Piqae must not merge devices automatically using only a friendly queue name.

## Native profile contract

The cross-platform domain object stores a portable summary and an opaque
platform-owned configuration:

```json
{
  "id": "prf_01K...",
  "revision": 7,
  "destinationId": "dst_01K...",
  "name": "80 mm matte, black mark",
  "status": "ready",
  "driverFingerprint": {
    "platform": "windows",
    "driverName": "OKI Pro1050(PS)",
    "driverVersion": "1.2.2",
    "architecture": "x86_64",
    "nativeQueueId": "OKI Pro1050(PS)",
    "deviceFingerprint": "sha256:..."
  },
  "nativeConfiguration": {
    "kind": "windows_devmode",
    "schemaVersion": 1,
    "digest": "sha256:...",
    "localBlobId": "npb_01K..."
  },
  "summary": {
    "paper": "C4 80 mm Matte Black Mark",
    "dimensionsMm": [80, 102],
    "source": "Roll",
    "colour": "Colour + white",
    "copies": 1
  },
  "stockId": "stk_01K...",
  "dependencies": [
    {"kind": "custom_media", "value": "C4 80 mm Matte Black Mark"},
    {"kind": "sensor_slot", "value": "3"}
  ],
  "safeOverrides": ["copies", "pages"],
  "lastValidatedAt": "2026-07-29T08:00:00Z",
  "lastTestJobId": "job_01K..."
}
```

### Native blob rules

- Native configurations are opaque to the Rust domain and web application.
- The platform profile host creates, validates, summarizes, and migrates them.
- The agent stores the blob locally as a SQLite `BLOB`, protected by the agent
  data-directory permissions.
- Logs, diagnostics, API responses, and webhooks never contain the blob.
- The service receives the digest, kind, compatibility metadata, summary, and
  readiness, not plaintext native data.
- Optional backup is an explicitly enabled, client-encrypted opaque artifact.
- A profile cannot be copied to another node merely by changing IDs. It must be
  rebound and validated against the destination's local driver.
- Blob formats have an independent schema version and migration result.

### Status

Profiles use explicit readiness:

```text
draft
capturing
ready
needs_test
stale
driver_mismatch
destination_missing
dependency_missing
interactive_only
invalid
retired
```

No job may use `capturing`, `invalid`, or `retired`. Workspace policy decides
whether `needs_test` or `stale` blocks production work.

## Windows implementation

### Capture

Create `piqae-profile-host-windows`, a small interactive Rust/Win32 process:

1. Receive a one-time capture token, destination ID, and optional existing
   profile revision over the ACL-protected local IPC channel.
2. Resolve the current queue through `OpenPrinterW`.
3. Query the full `DEVMODEW` buffer size with `DocumentPropertiesW`.
4. Populate the initial buffer with `DM_OUT_BUFFER`.
5. For edit or clone, validate and merge the existing settings using
   `DM_IN_BUFFER | DM_OUT_BUFFER`.
6. Display the manufacturer's property sheet with
   `DM_IN_PROMPT | DM_IN_BUFFER | DM_OUT_BUFFER`.
7. On OK, ask the driver to normalize the returned buffer again.
8. Capture the complete public and private portions through
   `dmSize + dmDriverExtra`.
9. Build a public summary using `DEVMODE`, `DeviceCapabilitiesW`, Print
   Capabilities, and known portable fields.
10. Fingerprint the queue, driver package/version, architecture, port/device,
    and returned configuration.
11. Return the capture to the agent over local IPC.

The process is isolated because a vendor property sheet can block, crash, or
load third-party DLLs. It is never invoked inside the service process or web
server.

For v4 drivers where PrintTicket is authoritative, capture both normalized
PrintTicket XML and a driver-produced `DEVMODE`. Mark one as the execution
backend and the other as a diagnostic/translation artifact.

### OKI Pro1050 specifics

The profile workflow must preserve, or explicitly depend on:

- media form: continuous, black mark, gap/die-cut, or continuous label;
- custom media name and dimensions;
- media type and weight/thickness;
- label length, gap, or black-mark interval;
- printer sensor-registration number;
- print-position and cut-position correction;
- colour/white and quality options exposed by the installed driver;
- any installed ICC/profile identifiers.

An OKI driver Favorite name and exported `.xmn` file can be attached as source
and recovery metadata. Runtime does not depend only on that mutable name.

The current official Pro1050 PS/PCL driver listings are Windows-only.
Pro1050 certification therefore uses a Windows node and the exact installed
OKI driver, not the macOS adapter.

### Replay

Replace the Preview SumatraPDF path for certified Windows PDF printing:

1. Load the pinned profile blob.
2. Recompute and compare the driver fingerprint.
3. Revalidate the `DEVMODE` through `DocumentPropertiesW`.
4. Apply only profile-allowed job overrides, asking the driver to merge them.
5. Create a printer device context with `CreateDCW` and the resulting
   `DEVMODE`.
6. Render the selected PDF pages through a sandboxed PDFium renderer into the
   device context.
7. Use `StartDocW`, `StartPage`, `EndPage`, and `EndDoc` and capture the native
   spooler job correlation/ID.
8. Reconcile through Winspool notifications and `GetJobW`.

Do not alter the queue's default `DEVMODE` per job. That creates a race between
concurrent applications and cannot preserve immutable profile semantics.

RAW jobs either:

- ignore the rendered profile and use a separately declared RAW target; or
- use a vendor-specific RAW preamble/backend explicitly certified for that
  device.

They must never imply that Windows driver settings transform arbitrary RAW
bytes.

## macOS implementation

### Capture

Create `PiqaeProfileHost` in the native Swift shell:

1. Ask the agent for a one-time capture session.
2. Resolve the destination to `NSPrinter`/`PMPrinter`.
3. Build `NSPrintInfo` using the destination's current defaults, or restore the
   selected profile when editing/cloning.
4. Open `NSPrintPanel` directly with that `NSPrintInfo`.
5. Set the default button title to **Save Profile**. No customer document is
   selected and nothing prints.
6. Add a Piqae accessory controller for name, stock, safe overrides, and a
   summarized confirmation. Vendor panes continue to come from macOS/the
   driver.
7. On Save, capture:
   - the complete property-list-safe `NSPrintInfo.printSettings` dictionary;
   - a PrintCore data representation of `PMPrintSettings`;
   - a data representation of `PMPageFormat`;
   - selected printer ID/UUID and page/paper/imageable-area summary;
   - canonical IPP/CUPS options where the mapping can be proven.
8. Submit the opaque capture to the agent.
9. Validate without changing global printer defaults.

Apple documents that `NSPrintInfo.printSettings` includes values supplied by
printer-driver print-dialog extensions. This is why the native panel is the
source of truth instead of a web-generated option form.

No PDF is necessary to configure the profile. A generated calibration or A4
test PDF is used only when the operator explicitly chooses **Test profile**.

### Replay

Implement two observable macOS backends:

#### `macos_printcore`

- Restore `PMPrintSettings` and `PMPageFormat`.
- Bind them to the exact `PMPrinter`.
- Apply allowed overrides through documented PrintCore/AppKit settings.
- Submit PDF using PrintCore/Core Graphics or a bounded native helper process.
- Record the CUPS/native job ID and observe it through CUPS/IPP.

This is preferred when it reproduces values from a driver's print-dialog
extension.

#### `cups_options`

- Submit the PDF with the captured canonical CUPS/IPP options.
- Use a CUPS printer instance only when a driver requires saved destination
  options or when it materially improves replay reliability.
- Continue to address the base destination as one Piqae destination.

This is preferred for driverless AirPrint/IPP Everywhere queues and drivers
whose complete settings are represented as CUPS/IPP attributes.

Profile validation selects and records the backend. If neither can replay a
driver's captured configuration without a logged-in interactive application,
the profile is `interactive_only` and cannot be exposed for unattended API
printing. Product parity means exposing this limitation honestly, not silently
dropping the unsupported settings.

### macOS HP acceptance profiles

The currently installed HP printer is the first physical macOS fixture:

- A4 colour using default source;
- A4 monochrome/draft;
- another driver-visible paper or source choice when available;
- two profiles for the same destination with visibly different output;
- driver-panel capture, agent restart, replay, edit-to-new-revision, and
  hosted-to-local delivery;
- queue/status observation and cancellation.

The test must prove that alternating profiles does not mutate the default
settings seen by another macOS application.

## Linux implementation

Linux uses the same domain and APIs, with a narrower setup experience:

- discover CUPS destinations;
- expose all standard and vendor CUPS/IPP options;
- create a named CUPS instance or immutable Piqae option set;
- validate against current printer attributes;
- submit with the captured option set;
- observe and cancel through CUPS/IPP.

The web/local Svelte editor may display CUPS options on a headless Linux node
because there may be no vendor UI. It must be explicitly labelled **CUPS
options**, not presented as equivalent to a manufacturer application.

## Stock and loaded-media model

### Stock

```json
{
  "id": "stk_01K...",
  "name": "C4 80 mm Matte Black Mark",
  "sku": "LABEL-080-MAT-BM",
  "kind": "roll_label",
  "dimensionsMm": {"width": 80, "length": 102},
  "mediaForm": "die_cut_black_mark",
  "thicknessMm": 0.12,
  "gapMm": null,
  "markIntervalMm": 102,
  "loadingInstructions": "Black mark facing sensor",
  "barcode": "..."
}
```

Stocks are portable business resources. Driver-specific media definitions and
sensor slots remain profile dependencies.

### Loaded media

Loaded media belongs to a physical device and source/tray:

```json
{
  "deviceId": "dev_01K...",
  "source": "roll",
  "stockId": "stk_01K...",
  "confidence": "operator_confirmed",
  "confirmedAt": "2026-07-29T08:00:00Z",
  "confirmedBy": "usr_01K..."
}
```

Confidence values:

```text
device_reported
driver_reported
barcode_scanned
operator_confirmed
assumed
unknown
```

For multi-tray printers, one record exists per tray. A production profile
chooses an explicit tray unless its routing policy permits automatic selection.

### Mismatch workflow

If the selected binding requires another stock:

1. the job becomes `awaiting_stock`;
2. no native handoff occurs;
3. the node shows loading instructions and waiting-job count;
4. the operator loads/scans/confirms the stock;
5. optional calibration/test runs;
6. compatible waiting jobs resume in sequence.

Workspace policy controls batching and whether jobs may be reordered to reduce
stock changes. Explicit order constraints always win.

## Targets and multiple nodes

A caller should normally choose a target, not raw destination settings:

```json
{
  "targetId": "tgt_oki_80mm_matte_blackmark",
  "title": "Order 481",
  "content": {
    "kind": "uri",
    "format": "pdf",
    "uri": "https://example.internal/labels/481.pdf"
  },
  "options": {"copies": 2},
  "onStockMismatch": "hold"
}
```

A target can have one or more bindings:

```text
Target: 80 mm matte, black mark
├─ primary:   Node A / OKI #1 / Profile rev 7
└─ standby:   Node B / OKI #2 / Profile rev 3
```

V1 supports one active binding plus an explicit standby. Pool and
least-changeover routing follow after the single-binding path is proven.

The service grants one delivery lease to one node. It may fail over before
native handoff. It must not automatically fail over after
`accepted_by_spooler` or an ambiguous crash; that becomes
`delivery_uncertain` to avoid duplicate labels.

Readiness is derived per binding:

```text
ready
node_offline
destination_offline
stock_not_loaded
needs_operator
profile_stale
driver_mismatch
dependency_missing
busy
delivery_uncertain
```

The target is ready when routing policy finds an eligible binding.

## API design

### Native endpoints

```text
GET  /v1/nodes
GET  /v1/devices
GET  /v1/destinations
GET  /v1/destinations/{id}

GET  /v1/destinations/{id}/profiles
POST /v1/destinations/{id}/profile-capture-sessions
GET  /v1/profiles/{id}
POST /v1/profiles/{id}/profile-capture-sessions
POST /v1/profiles/{id}/validate
POST /v1/profiles/{id}/test-jobs
POST /v1/profiles/{id}/publish
POST /v1/profiles/{id}/retire

GET/POST/PATCH /v1/stocks
GET/PUT         /v1/devices/{id}/loaded-media/{source}

GET/POST/PATCH /v1/targets
GET/POST/DELETE /v1/targets/{id}/bindings
GET             /v1/targets/{id}/readiness

POST /v1/jobs
```

Local-only capture routes mirror the resources under `/v1/local`. Capture
sessions expire quickly, are single-use, and are bound to the local OS user,
destination, requested operation, and expected profile revision.

### Safe overrides

Each profile declares an allowlist. The job service rejects an override not in
the profile:

```json
{
  "code": "profile_override_not_allowed",
  "detail": "The profile does not allow paper to be changed per job.",
  "allowed": ["copies", "pages"]
}
```

Overrides are merged locally, after driver compatibility validation, into a
temporary job ticket. The stored profile revision remains unchanged.

### PrintNode compatibility

The compatibility API flattens each published target/profile into a virtual
printer with a stable integer ID:

```text
4101 OKI Pro1050 — 80 mm matte, black mark
4102 OKI Pro1050 — 100 mm gloss, gap
4201 HP OfficeJet Pro — A4 colour
4202 HP OfficeJet Pro — A4 mono draft
```

Its capability response is constrained to the selected profile and safe
overrides. Generic PrintNode `paper`, `bin`, or `media` options must not be used
to bypass an immutable complex profile. The base destination can optionally be
published as a generic printer for callers that need the original behavior.

Compatibility IDs never change when a target binding moves between nodes.

## Storage migration

### Agent SQLite

Retain the existing `printers` table as the installed destination and expand
the current immutable `printer_profiles` model.

Add:

```text
physical_devices
printer_device_bindings
profile_native_blobs
profile_dependencies
stocks
loaded_media
targets
target_bindings
profile_validation_events
profile_capture_sessions
```

Extend each `printer_profiles` revision with:

```text
status
native_kind
native_blob_id
native_digest
driver_fingerprint_json
summary_json
stock_id
safe_overrides_json
last_validated_unix_ms
last_test_job_id
published
```

Extend jobs with:

```text
target_id
binding_id
profile_id
profile_revision
stock_id
loaded_media_snapshot_json
```

Existing option-only named profiles migrate as `cups_options` where the
destination is CUPS and as `portable_options` elsewhere. They become
`needs_test`; they are not falsely upgraded into native captures.

### Control-plane PostgreSQL

Mirror all metadata resources and revisions, excluding plaintext native blobs.
The local agent remains authoritative for the native configuration. Target
binding readiness is a projection of synchronized agent facts plus
control-plane routing policy.

## Local IPC changes

The profile host never writes SQLite and the web application never receives a
native blob.

Add versioned messages:

```text
BeginProfileCapture
ProfileCaptureAuthorized
CommitProfileCapture
CancelProfileCapture
ValidateProfile
ProfileValidationResult
ConfirmLoadedMedia
```

Security requirements:

- Windows named-pipe ACL or macOS mode-`0600` Unix socket;
- peer-user/session verification;
- one-time 256-bit capture token;
- destination and operation binding;
- five-minute maximum expiry;
- native blob size ceiling;
- digest verification;
- optimistic `expected_revision`;
- audit event on start, commit, cancel, and failure;
- no capture initiated silently by a remote browser.

## Web and native UI changes

### macOS menu

```text
Piqae
├─ Agent online
├─ Printers
│  └─ HP OfficeJet Pro
│     ├─ Ready · 0 queued
│     ├─ Exposed to Piqae ✓
│     ├─ Profiles
│     │  ├─ A4 colour — Default
│     │  │  ├─ Test…
│     │  │  ├─ Edit native settings…
│     │  │  └─ Retire…
│     │  ├─ A4 mono draft
│     │  └─ Add profile…
│     ├─ View queue…
│     └─ Open Printer Settings…
├─ Operator actions
├─ Recent jobs
└─ Open Dashboard
```

Profile creation uses a small native management window rather than expanding a
menu into a complex form. The window hosts the profile list and launches the
system print panel.

### Windows tray

The hierarchy and terminology match macOS. The profile editor launches the
manufacturer's `DocumentPropertiesW` property sheet. Windows-native controls,
focus, accessibility, and elevation behavior are preserved.

### Web

Remove the generic native-option form from the normal printer page. Retain it
only in a labelled diagnostics view. The normal page emphasizes readiness,
profiles, stocks, routing, and history.

## Error and trace model

Stable profile errors:

```text
profile_capture_cancelled
profile_capture_timed_out
profile_host_crashed
profile_blob_invalid
profile_driver_mismatch
profile_destination_missing
profile_dependency_missing
profile_override_not_allowed
profile_requires_interactive_session
stock_not_loaded
stock_confirmation_required
target_has_no_ready_binding
```

Every capture, validation, selection, and execution contributes OpenTelemetry
spans:

```text
profile.capture.authorize
profile.capture.native_dialog
profile.capture.commit
profile.validate.driver
profile.validate.dependencies
target.resolve
stock.check
profile.ticket.merge
renderer.render
spooler.submit
```

Spans include IDs, revisions, digests, backend, duration, and redacted
capability summaries. They never include document bytes or native blobs.

## Test and release matrix

### Domain and storage

- immutable revision and optimistic-concurrency properties;
- a job always resolves one exact profile revision;
- retired/deleted profiles remain available for job history;
- override allowlist property tests;
- target leasing never selects two bindings;
- stock state and source/tray matching;
- migrations from current named profiles;
- native blobs excluded from logs/API serialization.

### Native capture contract

For every supported platform:

- create from defaults;
- cancel without side effects;
- clone and edit;
- edit appends a revision;
- shell/host crash;
- driver dialog hang and timeout;
- queue removed during capture;
- driver upgraded after capture;
- blob truncation/corruption;
- non-ASCII queue/profile/option names;
- capture under a standard, non-administrator account.

### Replay

- alternating two profiles on one destination;
- concurrent jobs do not change global defaults;
- restart between acceptance and rendering;
- renderer/helper crash;
- spooler restart;
- profile driver mismatch before handoff;
- unsupported override fails closed;
- native job ID and state reconciliation;
- test PDF includes colour, grayscale, fine lines, exact dimensions, text,
  QR/barcode, page boxes, and a unique job marker.

### Physical certification

Initial fixtures:

1. the current HP printer on macOS, with at least two visibly distinct native
   profiles;
2. an office printer with multiple trays;
3. OKI Pro1050 PS on Windows with two stocks and sensor modes;
4. one driverless AirPrint/IPP Everywhere printer;
5. one RAW label or receipt printer kept separate from rendered profiles.

Certification records:

- OS build and architecture;
- driver package/version and source;
- firmware;
- connection/port;
- capture backend;
- replay backend;
- profile summary and native digest;
- printed fixture measurement/photos;
- barcode scan result;
- observed status authority;
- known limitations.

### Release gates

A platform profile backend moves from Preview only when:

- capture and replay pass on certified hardware;
- two profiles alternate for 100 jobs without settings leakage;
- restart and ambiguous-handoff tests pass;
- driver-upgrade mismatch is detected;
- native shell is signed/notarized as applicable;
- no plaintext native blob appears outside the agent;
- uninstall/upgrade retains or intentionally exports profile metadata;
- accessibility and standard-user operation pass.

## Implementation workstreams

The work can proceed in parallel after the domain contract lands.

### A. Domain, storage, and protocol

- introduce destination/profile/stock/target types;
- add SQLite/PostgreSQL migrations;
- extend local IPC and executor protocol;
- pin jobs to profile revisions;
- add readiness and mismatch states;
- migrate existing option-only profiles;
- add golden JSON/protocol fixtures.

### B. macOS

- implement native profile-management window;
- implement `NSPrintPanel` capture with **Save Profile**;
- serialize/restore print settings and page format;
- implement PrintCore and CUPS replay backends;
- connect profile and loaded-stock actions to the agent;
- certify using the current HP.

### C. Windows

- implement isolated `piqae-profile-host-windows`;
- capture/normalize full `DEVMODEW`;
- fingerprint drivers and validate saved blobs;
- add PrintTicket conversion/capture where required;
- implement PDFium-to-GDI replay;
- capture native job correlation and states;
- implement the equivalent Windows profile-management/tray experience;
- certify the OKI Pro1050.

### D. Service, API, compatibility, and Svelte UI

- synchronize metadata/readiness;
- add stock, target, binding, and profile APIs;
- add target resolution and single-node delivery lease;
- flatten targets into PrintNode-compatible virtual printers;
- simplify the printer page;
- add stock/operator and profile-history views;
- update SDK, OpenAPI, migration guide, and examples.

### E. Reliability and release

- add driver-host watchdogs and crash isolation;
- add fixture generators and physical certification runner;
- add profile/target observability;
- run security review for local capture authorization;
- package/sign/notarize native components;
- publish support matrix by OS/driver/backend.

## 48-hour code-complete sequence

This is an execution ordering for parallel agents, not a claim that every
third-party driver can receive physical certification without the hardware.

### Hours 0–4

- freeze domain/API/protocol names;
- create migrations and fixtures;
- add profile-capture IPC envelope;
- create macOS and Windows host targets;
- create target/stock Svelte route skeletons.

### Hours 4–12

- complete SQLite/domain job pinning;
- implement macOS panel capture;
- implement Windows `DEVMODE` capture;
- implement service metadata resources;
- build profile-management screens.

### Hours 12–24

- implement macOS PrintCore/CUPS replay;
- implement Windows PDFium/GDI replay;
- implement stock holds and loaded-media confirmation;
- implement target routing and compatibility flattening;
- complete unit, migration, IPC, and UI tests.

### Hours 24–36

- integrate hosted-to-node target jobs;
- run HP macOS alternating-profile tests;
- run Windows virtual/file queue tests;
- exercise restarts, mismatches, cancellation, and trace correlation;
- fix cross-workstream contract differences.

### Hours 36–48

- run the full repository suite;
- run physical HP end-to-end tests;
- package local macOS build;
- package Windows build and certify virtual/available hardware;
- update support matrix and operator documentation;
- cut a versioned V1 candidate.

The OKI-specific release remains Preview until the actual Windows node, OKI
driver, Pro1050, and required stocks complete the physical matrix.

## Definition of done

The slice is complete when:

1. the HP appears once on the Mac with at least two native profiles;
2. **Add profile** opens the real macOS panel and saves without printing;
3. a hosted PDF job targeting either profile arrives locally and uses that
   exact revision;
4. alternating profiles does not mutate global HP defaults;
5. Windows provides the same workflow through the manufacturer's property
   sheet and complete private `DEVMODE`;
6. jobs stop on driver mismatch or missing stock;
7. one target can move between node-specific bindings without changing its API
   ID;
8. PrintNode-compatible callers can address published profiles as virtual
   printers;
9. the web UI contains no imitation of vendor-specific settings;
10. every handoff and readiness decision is visible in job events and traces.

## Primary platform evidence

- [Apple `NSPrintInfo.printSettings`](https://developer.apple.com/documentation/appkit/nsprintinfo/printsettings)
- [Apple `NSPrintPanel`](https://developer.apple.com/documentation/appkit/nsprintpanel)
- [Microsoft `DocumentPropertiesW`](https://learn.microsoft.com/en-us/windows/win32/printdocs/documentproperties)
- [Microsoft guidance for reliable `DEVMODE` modification](https://learn.microsoft.com/en-us/troubleshoot/windows/win32/modify-printer-settings-documentproperties)
- [CUPS saved options and printer instances](https://openprinting.github.io/cups/doc/options.html)
- [CUPS `lpoptions`](https://openprinting.github.io/cups/doc/man-lpoptions.html)
- [OKI Pro1050 PS Driver User's Guide](https://www.oki.com/jp/printing/download/47309602EE2_Pro1050_PSD_UG_EN_286729.pdf?id=47309602EE)
- [OKI Pro1050 drivers and utilities](https://www.oki.com/uk/printing/support/drivers-and-utilities/label/46672103/)
