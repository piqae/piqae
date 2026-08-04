# Research sources

Reviewed 29 July 2026. Product behavior can change; compatibility tests should
record the actual date and version observed.

## Printing-platform primary sources

- [OpenPrinting CUPS](https://openprinting.github.io/cups/)
- [CUPS programming manual](https://openprinting.github.io/cups/doc/cupspm.html)
- [CUPS filter/backend programming and state reasons](https://openprinting.github.io/cups/doc/api-filter.html)
- [OpenPrinting CUPS source](https://github.com/OpenPrinting/cups)
- [Microsoft Print Spooler API structures](https://learn.microsoft.com/en-us/windows/win32/printdocs/printing-and-print-spooler-structures)
- [Microsoft `JOB_INFO_2` states and TrueEndOfJob caveat](https://learn.microsoft.com/en-us/windows/win32/printdocs/job-info-2)
- [Microsoft `PRINTER_INFO_2` states](https://learn.microsoft.com/en-us/windows/win32/printdocs/printer-info-2)
- [Microsoft `EnumPrinters` blocking behavior](https://learn.microsoft.com/en-us/windows/win32/printdocs/enumprinters)
- [Microsoft Print Ticket API](https://learn.microsoft.com/en-us/windows/win32/printdocs/print-ticket-api)
- [Microsoft `DocumentPropertiesW`](https://learn.microsoft.com/en-us/windows/win32/printdocs/documentproperties)
- [Microsoft reliable `DEVMODE` modification guidance](https://learn.microsoft.com/en-us/troubleshoot/windows/win32/modify-printer-settings-documentproperties)
- [Microsoft print provider functions](https://learn.microsoft.com/en-us/windows-hardware/drivers/print/functions-defined-by-print-providers)
- [Apple `NSPrintInfo.printSettings`](https://developer.apple.com/documentation/appkit/nsprintinfo/printsettings)
- [Apple `NSPrintPanel`](https://developer.apple.com/documentation/appkit/nsprintpanel)
- [CUPS saved options and printer instances](https://openprinting.github.io/cups/doc/options.html)
- [CUPS `lpoptions`](https://openprinting.github.io/cups/doc/man-lpoptions.html)
- [OKI Pro1050 PS Driver User's Guide](https://www.oki.com/jp/printing/download/47309602EE2_Pro1050_PSD_UG_EN_286729.pdf?id=47309602EE)
- [OKI Pro1050 drivers and utilities](https://www.oki.com/uk/printing/support/drivers-and-utilities/label/46672103/)
- [PDFium licence](https://pdfium.googlesource.com/pdfium/+/refs/heads/main/LICENSE)

## Related open-source reference implementation

- [QZ Tray documentation](https://qz.io/docs/)
- [QZ Tray source](https://github.com/qzind/tray)

QZ Tray is useful evidence that cross-platform RAW/PDF/local-device bridging is
possible, and its test cases may inform behavior. It is not the proposed base:
its Java/browser-oriented architecture and trust-dialog model do not match the
low-resource headless remote queue. Reuse must follow its licence; do not copy
implementation code merely because the repository is public.

## Developer experience, community, and scale

- [Stripe idempotent requests](https://docs.stripe.com/api/idempotent_requests)
- [Stripe API upgrades and version pinning](https://docs.stripe.com/upgrades)
- [Stripe API keys and test/live modes](https://docs.stripe.com/keys)
- [Stripe webhook endpoints](https://docs.stripe.com/api/webhook_endpoints)
- [OpenSSF Scorecard](https://github.com/ossf/scorecard)
- [OpenSSF Scorecard checks](https://github.com/ossf/scorecard/blob/main/docs/checks.md)
- [SLSA 1.2 specification](https://slsa.dev/spec/v1.2/)
- [SLSA build provenance](https://slsa.dev/spec/v1.2/provenance)
- [AWS guidance for cell-based architecture](https://docs.aws.amazon.com/solutions/cell-based-architecture-on-aws/)
- [Supabase open-source and self-hosted model](https://github.com/supabase/supabase)
- [Supabase Apache-2.0 licence](https://github.com/supabase/supabase/blob/master/LICENSE)
- [Apache License 2.0](https://www.apache.org/licenses/LICENSE-2.0)

## MVP stack and reusable components

- [Vercel SvelteKit deployment](https://vercel.com/i/what-is-sveltekit)
- [Vercel WebSocket behavior and function pinning](https://vercel.com/kb/guide/do-vercel-serverless-functions-support-websocket-connections)
- [Vercel Function duration](https://vercel.com/docs/functions/configuring-functions/duration)
- [Vercel Marketplace storage providers](https://vercel.com/docs/marketplace-storage)
- [Vercel Blob](https://vercel.com/docs/vercel-blob)
- [WorkOS AuthKit](https://workos.com/docs/authkit/overview)
- [WorkOS users and organizations](https://workos.com/docs/authkit/users-organizations)
- [WorkOS SvelteKit integration](https://workos.com/docs/authkit/cli-installer)
- [SumatraPDF command-line printing](https://www.sumatrapdfreader.org/docs/Command-line-arguments)
- [SumatraPDF source and licence](https://github.com/sumatrapdfreader/sumatrapdf)
- [`pdf-to-printer`](https://github.com/artiebits/pdf-to-printer)
- [OpenPrinting Go IPP library](https://github.com/OpenPrinting/goipp)
- [Windows Go printing reference](https://github.com/alexbrainman/printer)
- [Microsoft Rust for Windows bindings](https://microsoft.github.io/windows-docs-rs/)
- [Rust `printers` CUPS/Winspool library](https://github.com/talesluna/rust-printers)
- [Rust `printers` crate documentation](https://docs.rs/printers)

## Visual design

- [Linear's 2026 interface refresh](https://linear.app/now/behind-the-latest-design-refresh)
- [Linear's 2026 UI refresh changelog](https://linear.app/changelog/2026-03-12-ui-refresh)
- [How Linear redesigned its UI in 2024](https://linear.app/now/how-we-redesigned-the-linear-ui)
- [Linear appearance and theme preferences](https://linear.app/docs/account-preferences)

## Notes on evidence

- the legacy service's statements about server latency, memory use, document deletion,
  and broad printer compatibility are vendor claims unless independently
  measured.
- The public API documents `done` as OS-queue delivery, not physical print
  completion.
- Windows explicitly documents that some port monitors report printed status
  without true end-of-job support.
- Printer capability and status quality ultimately depends on the operating
  system, driver, port monitor, protocol, and device.
