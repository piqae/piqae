# Spool Windows shell

The Windows V1 shell is a Win32 notification-area process, separate from the
Windows Service or user-mode agent. Its only stable dependency is the local IPC
V1 contract documented in `docs/architecture/local-agent-control.md`.

The release target is a subsystem-Windows Rust binary using `Shell_NotifyIconW`
and a named-pipe client. It provides status, dashboard, support-bundle and
controlled restart actions. It contains no queue, cloud or printing code.

The binary is enabled in the MSI only when the Windows signing and clean-login
startup gates pass. Until the Windows-specific build lane supplies the signed
binary, the agent installer remains headless.

## Native profile host

Driver configuration is delegated to the separately built
`spool-profile-host-windows` binary in `crates/executor-windows`. The tray shell
must launch it in the interactive user's session, write exactly one JSON
request to standard input, read exactly one JSON response from standard output,
and then let the process exit.

The shell and agent integration must:

- obtain a short-lived, single-use capture token from the local agent;
- pass that token in both `SPOOL_PROFILE_CAPTURE_TOKEN` and the request;
- pass the exact installed queue ID rather than a friendly alias;
- optionally pass the tray window handle so the vendor property sheet is modal
  to the tray UI;
- never log the request or response because they contain the opaque native
  `DEVMODE`;
- enforce a bounded execution deadline while still allowing the operator time
  to use a complex vendor dialog;
- treat `cancelled` as a normal operator outcome;
- return the captured envelope directly to the agent for local encrypted-at-rest
  persistence.

The host calls the genuine driver `DocumentPropertiesW` property sheet. It
captures and validates `dmSize + dmDriverExtra`, fingerprints the installed
queue and driver, and can revalidate an existing capture without displaying UI.
It does not print a document or change the queue's global defaults.

The current executor's SumatraPDF integration remains a generic preview
compatibility fallback. It cannot apply a captured native profile. Production
profile replay requires the shared executor job protocol to carry a pinned
profile blob and requires the planned PDFium-to-GDI renderer before it may be
presented as certified.
