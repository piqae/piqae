# Local agent control contract

The Spool agent is headless. Native shells and the embedded local UI are
replaceable clients of a narrow, versioned control contract; they never open
SQLite, render documents, contact the hosted control plane, or hold the device
private key.

## Transport

- Windows production transport: a named pipe restricted to the installing
  user or the local Administrators group in machine mode.
- macOS and Linux: a Unix-domain socket inside a mode `0700` directory. The
  socket is mode `0600`.
- Every request is a four-byte big-endian length followed by UTF-8 JSON.
- Messages larger than 64 KiB are rejected before their body is allocated.
- Protocol V1 accepts only protocol `1`; releases must support N and N-1 after
  a V2 exists.
- Each agent start generates a 256-bit session challenge. The shell obtains it
  through an OS-ACL-protected bootstrap file or inherited installer channel.
  The agent retains only its SHA-256 digest and compares candidates without an
  early exit.

The Rust contract and codec live in `crates/local-ipc`.

## Operations

- `status`
- `printers`
- `pause`
- `resume`
- `restart_agent`
- `export_support_bundle`
- `reenrol`

Re-enrolment includes an explicit confirmation string. Quitting a tray shell
does not stop or pause the agent.

## Loopback HTTP

The embedded local Svelte UI uses `crates/local-api` on
`127.0.0.1:39100`. It refuses non-loopback binds, caps bodies to 64 KiB, and
requires the same session challenge as a bearer token for every operational
route. `/health` is the only unauthenticated route and exposes no data.

The loopback API sends commands over a bounded Tokio channel to the agent loop.
Handlers cannot access SQLite or print drivers directly.

## Shell release status

The V1 repository contains native shell/package foundations:

- Windows Win32 notification-area process.
- macOS AppKit status item.
- Linux StatusNotifier/AppIndicator process.

A shell is shipped only after clean install, login startup, IPC ACL, upgrade,
accessibility, and signed-package gates pass. Otherwise the agent remains fully
headless and the shell is marked Preview or omitted from that installer.

