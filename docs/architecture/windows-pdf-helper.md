# Windows PDF helper backend

The built-in Windows executor supports native Winspool discovery, RAW
submission and cancellation. Its initial PDF backend can invoke a separately
installed SumatraPDF executable:

```text
SPOOL_WINDOWS_PDF_HELPER=C:\Program Files\SumatraPDF\SumatraPDF.exe
```

The executor starts the configured executable directly, without `cmd.exe`,
PowerShell or string-built shell commands. The parent executor supervisor
enforces the hard deadline and terminates the complete executor process on
timeout.

The backend uses SumatraPDF's documented `-print-to` and `-print-settings`
arguments for page ranges, copies, colour, collation, paper and input bin:

- <https://www.sumatrapdfreader.org/docs/Command-line-arguments>
- <https://www.sumatrapdfreader.org/docs/FAQ>

SumatraPDF is GPLv3 software and is **not bundled** with Apache-licensed Spool
artifacts. Operators must install and approve their own copy and comply with
its licence. An unset or invalid helper path fails closed before handoff.

The helper does not return the native Winspool job ID. Spool records a
backend-scoped correlation marker after a zero exit status. The Windows
executor recognizes that marker and reports the job as unobservable instead
of passing it to `GetJobW` or `SetJobW`. If no authoritative native outcome
becomes available before the bounded reconciliation deadline, the agent emits
`delivery_uncertain`; it never converts the helper's successful process exit
into a claim of printing or physical delivery.

This backend remains Preview until physical printer, driver-option, process
tree termination, cancellation and signed-installer gates pass. A sandboxed
PDFium/GDI backend remains the preferred long-term default.
