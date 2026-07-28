# Platform printing and native integration

## Core adapter contract

Each platform adapter implements:

- enumerate queues quickly;
- fetch full queue details and capabilities with timeouts;
- watch printer and job changes;
- validate/map common options;
- submit RAW content and return a native job ID;
- submit rendered content and return a native job ID;
- enumerate/reconcile native jobs;
- request cancellation;
- translate native errors and states into stable domain codes;
- produce a redacted diagnostic snapshot.

Slow driver calls run in bounded blocking workers. A hung network printer or
driver must not stall heartbeats, other printers, or the local UI.

## Windows

### Discovery

Use the Windows Print Spooler API:

- fast initial names through `EnumPrinters` at a level that avoids opening
  every remote queue;
- lazy detailed data through `OpenPrinter`/`GetPrinter`;
- default queue through `GetDefaultPrinter`;
- change notifications through `FindFirstPrinterChangeNotification` and
  related calls;
- configuration and driver identity from `PRINTER_INFO_*`, DEVMODE, and Print
  Schema/PrintTicket APIs.

Microsoft documents that detailed enumeration can block because it opens
remote connections. Use timeouts at the worker/process boundary and cache
capabilities.

### Capabilities and options

Use both legacy `DeviceCapabilities`/DEVMODE and modern PrintTicket
capabilities because drivers expose different subsets. Internally retain:

- native option IDs and display names;
- paper dimensions and imageable area;
- trays/bins;
- media types;
- colour modes;
- duplex modes;
- resolutions;
- copies/collation;
- orientation;
- finishing features where reported;
- default PrintTicket/DEVMODE revision.

Validate an option set against current capabilities immediately before
submission. Compatibility mode returns an error for unsupported explicit
options rather than silently substituting defaults. The native API may support
an explicit fallback policy.

### RAW printing

Use `OpenPrinter`, `StartDocPrinter`, `WritePrinter`, and `EndDocPrinter` with
the appropriate RAW data type. Stream bytes in bounded chunks and record the
job ID returned by the spooler. Do not transform line endings or encoding.

RAW access must be scoped because device commands can cut paper, open a cash
drawer, alter persistent printer configuration, or trigger large output.

### PDF printing

Windows has no single universal API that accepts a PDF and applies every
installed Win32 driver option. The proposed first backend is:

1. Validate PDF and selected pages in a sandboxed PDFium process.
2. Build a validated DEVMODE or PrintTicket for the driver.
3. Render each selected/transformed page at an appropriate resolution.
4. Submit through a printer device context/GDI or a spool format supported by
   the driver path.
5. Record both renderer and spooler diagnostics.

Potential later backends:

- PDF-to-XPS plus XPS Print API;
- direct PDF passthrough for queues that advertise native PDF;
- a vendor-specific path;
- a compatibility backend selected for known driver classes.

Backend selection must be observable and overrideable per printer, similar in
spirit to PrintNode exposing multiple engines. There should be one safe
default, not a user-facing collection of unexplained engine numbers.

### Job and printer status

Poll or subscribe to spooler changes and query `JOB_INFO_2`/printer status.
Map paper out, offline, blocked driver queue, paused, spooling, printing,
printed, complete, user intervention, and deletion.

Windows' own documentation warns that some port monitors report “printed”
immediately after submission when they do not support true end-of-job
notification. Store the port/driver authority and never treat that value as
physical proof.

### Service identity

Use a dedicated service identity where possible. Installation diagnostics must
test:

- access to each queue;
- access to network credentials/shares;
- default settings as seen by the service;
- driver calls under the service session;
- interactive-only drivers that cannot run reliably headless.

Allow an administrator to configure the service under a managed user/domain
account when network printers require it.

## macOS

### Queue model

macOS uses CUPS/IPP for queueing but ships Apple's CUPS variant. Enumerate
destinations and query IPP attributes rather than relying on private PPD
implementation details. Observe queue/job changes through IPP where available,
with bounded polling as a fallback.

### PDF

Test two paths:

- submit PDF through CUPS and let the configured filters/driver process it;
- use native PDFKit/Core Graphics rendering when page transforms or driver
  behavior require deterministic rasterisation.

Prefer native PDF passthrough when it preserves fidelity and options. Record
which path was chosen.

### RAW

Submit as `application/octet-stream`/raw through the configured CUPS queue only
when the queue supports that workflow. Document raw-queue setup for label and
receipt printers.

### Packaging

The daemon must not rely on a logged-in user's menu bar or keychain session.
Notarisation, signing, permissions, universal builds, and `launchd` lifecycle
need real-device CI and upgrade tests.

## Linux

### CUPS/IPP

Use CUPS/IPP to:

- enumerate destinations;
- query `printer-attributes`;
- submit jobs and options;
- read `job-state`, `job-state-reasons`, and printer-state reasons;
- cancel jobs;
- subscribe to events where supported, with polling fallback.

OpenPrinting recommends using documented APIs and not assuming a configured
printer, format, or PPD. Driverless IPP Everywhere is preferred, while legacy
installed queues remain supported because local drivers are part of the value
proposition.

### PDF and filters

Submit PDF directly when the queue accepts it. Let CUPS filters convert to the
printer format. A PDFium rendering fallback can rasterise problematic PDFs or
implement transforms consistently, but it should not replace a working native
pipeline without evidence.

### RAW

Support raw queues and explicit raw job submission. Detect/filter setups that
would interpret RAW bytes as text and fail before causing unintended output.

### Distribution

Test:

- Ubuntu/Debian and a representative RPM distribution;
- CUPS 2.x variants;
- x86_64 and aarch64;
- Raspberry Pi-class storage/power-loss behavior;
- SELinux/AppArmor profiles;
- container access to `/run/cups/cups.sock`, USB devices, and network queues.

## Common option normalization

The internal schema must distinguish:

- option omitted: use current driver default;
- explicit value: require or validate it;
- fallback allowed: choose a documented substitute and record it.

Do not normalise away native strings prematurely. Paper/tray names can be
localised or vendor-specific. Store a stable native key where available plus
display name and dimensions.

### Page selection

Parse PrintNode-compatible page expressions into a canonical ordered page set.
Reject invalid or out-of-range input before native handoff. Decide and document
whether repeated pages are allowed in the native API; compatibility behavior
must match the reference service.

### Rotation and fit

Define transformations mathematically in document space:

- rotation is absolute;
- fit-to-page preserves aspect ratio unless a separate stretch option is added;
- selected paper imageable area, not only physical dimensions, determines fit;
- no implicit auto-rotation in compatibility mode unless reference testing
  confirms it.

Golden render tests should cover portrait, landscape, crop boxes, mixed page
sizes, labels, transparency, embedded fonts, and malformed PDFs.

### Copies, quantity, and collation

- `copies` maps to a driver/spooler copy count when supported;
- `qty` produces independent native jobs;
- collation applies to copies within a job;
- each `qty` child receives its own native job ID but belongs to one API job;
- partial success is represented, not collapsed to a misleading Boolean.

## Printer identity and renames

Expose two IDs:

- immutable API logical printer ID;
- current OS queue identity.

Reconciliation can use:

- installation/agent ID;
- queue name;
- port/device URI;
- driver name/version;
- hardware or IPP UUID;
- serial number when reported.

Never merge automatically when evidence is ambiguous. A new queue may receive a
new logical ID and the UI can offer an administrative merge.

## Deeper device status

OS spoolers provide the baseline. Optional enrichers:

- IPP `printer-state-reasons`, supplies, and job states;
- SNMP Printer-MIB for network devices;
- bidirectional driver/port monitor data;
- vendor plugins.

These are best-effort modules. They must not be required for printing, and
network probing is opt-in. Attribute each state to its source and timestamp so
stale SNMP data cannot overwrite a fresh spooler error.

## PDF renderer strategy and risk

PDF rendering is the most important spike before committing to a schedule.
Evaluate:

- PDFium output fidelity and binary size;
- text/font, transparency, colour, barcode, label, and large-document behavior;
- GDI versus XPS submission on Windows;
- memory and timeout containment;
- licences of all transitive native components;
- reproducible signed builds;
- per-page versus full-document rendering cost.

Do not select MuPDF or Poppler without deliberate licence review. Do not use
Adobe Reader automation or a desktop application as the core service backend.

