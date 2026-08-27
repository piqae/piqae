# PrintPacket

PrintPacket is an open, vendor-neutral format for small, data-driven print
layouts. One bounded language covers:

- paged PrintPacket such as invoices, packing slips, purchase orders,
  CRM/ERP forms, and pick lists;
- continuous receipts and kitchen/customer tickets;
- fixed production, shipping, price, shelf, and barcode labels.

It is intentionally simpler than HTML/CSS, SVG, a desktop-publishing format, or
an arbitrary-position design canvas. Templates can still produce clean,
professional layouts with text, tables, images, icons, QR codes, Code 128,
repeated data, conditions, and basic arithmetic/formatting.

The sole v1 identifier is `printpacket/v1`. Preview identifiers are not
accepted, normalized, migrated, or included in canonical template digests.

PrintPacket is independent of Piqae. Piqae is one transport, queue, node, and
reference-renderer implementation. A POS, ERP, browser service, desktop app, or
mobile app can validate and render a packet locally without a Piqae account or
network connection.

## Deliberate boundaries

PrintPacket defines template semantics, bounded input data, content-addressed
resources, render requirements, and deterministic output profiles. It does not
define printer discovery, identity, job routing, leases, retries, driver
options, spooler status, or proof that ink reached paper.

Portable job intent should use standard printer/job-ticket concepts such as
media, orientation, copies, and finishing. Installed drivers remain the
authority for vendor-specific controls.

The general output baseline is deterministic PDF through a native driver.
Printer-native output is permitted only when a renderer advertises an explicit,
reviewed language and device profile (for example a particular ESC/POS, ZPL, or
TSPL raster profile). Hosts must never infer raw support from a printer name,
generic `application/octet-stream`, or a failed PDF attempt.

## Files

- [SPECIFICATION.md](SPECIFICATION.md) defines the normative compatibility,
  caching, data, and rendering rules.
- [schema/printpacket-v1.schema.json](schema/printpacket-v1.schema.json) is the
  portable JSON Schema.
- [conformance](conformance) contains deterministic receipt, label, and paged
  document fixtures. The Rust `printpacket` crate is the reference validator.

The specification and reference implementation are licensed under Apache-2.0.
