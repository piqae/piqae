# Complex and vendor drivers

**Status:** native opaque capture is implemented in source for macOS PrintCore
and Windows DEVMODE; vendor-specific certification is not complete.

Spool should not recreate every OKI, Zebra, Epson, Fiery, or PostScript option
in a web form. Install the manufacturer's driver on the node, open its genuine
advanced settings through Add/Edit Profile, and save the resulting native
state.

For an OKI Pro1050-class workflow, separate profiles might represent:

- a named roll/label stock and dimensions;
- gap, black-mark, or continuous sensing;
- feed direction and registration offsets;
- colour/white-toner behavior and quality;
- cutter, finishing, tray, or manual-feed choices.

The driver may prompt the operator to load matching stock. Spool's stock and
loaded-media records route work and explain readiness; they do not bypass
device safety prompts. Avoid per-job overrides for vendor-private settings.
Copies or page ranges may remain safe when explicitly allowed by the profile.

Recommended qualification:

1. Capture a profile on the final node, queue, port, driver, and firmware.
2. Record stock SKU, dimensions, sensing mode, and loading instructions.
3. Print alignment, barcode, colour, multi-page, restart, and low-stock cases.
4. Re-test after driver, firmware, port, or queue changes.
5. Keep the prior revision available for rollback.

See [native profiles](native-profiles.md) and the
[stock/routing specification](../16-native-print-profiles-stock-and-routing.md).
