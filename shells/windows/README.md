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

