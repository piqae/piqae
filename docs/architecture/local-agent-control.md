# Local agent control contract

The Spool agent is headless. Native shells are optional clients; they do not
open SQLite, render documents, contact the hosted control plane, or hold the
device private key.

## V1 implementation

The current agent exposes `crates/local-api` on loopback HTTP, defaulting to
`127.0.0.1:39100`. This surface is an API only: there is no embedded local web
UI at the loopback root.

The server refuses non-loopback binds, caps request bodies at 72 MiB, and
requires the startup-generated bearer token for every operational route.
`/health` is the only unauthenticated route and exposes no private data. The
token is stored as `local.token` under the configured agent data directory.
Handlers send commands over a bounded Tokio channel and cannot access SQLite or
print drivers directly.

Implemented V1 operations are status, printers, pause, resume, and local job
submission. See `crates/local-api` for the exact route contract.

Shell capabilities are intentionally described narrowly:

- Linux Preview shell reads status from loopback HTTP using `local.token`.
- macOS Preview shell currently displays `Agent status unavailable`; it is not
  connected to the control API.
- Windows development shell reads authenticated loopback status, lists queues
  and profiles, and opens native create/edit/clone profile capture. It remains
  Disabled for production release.
- Linux and macOS show **Open Spool** only when `SPOOL_DASHBOARD_URL` contains
  an explicit HTTP(S) dashboard URL. They never open the loopback API root as
  though it were a UI.

## Target transport, not implemented in V1

The desired production transport is a named pipe with an installer-scoped ACL
on Windows and a mode `0600` Unix-domain socket in a mode `0700` directory on
macOS and Linux. The target protocol uses length-prefixed JSON, a 64 KiB
message limit, protocol versioning, and an OS-ACL-protected session challenge.
The Rust contract and codec foundations live in `crates/local-ipc`.

Target operations also include restart, support-bundle export, and re-enrolment.
These are roadmap contract entries, not working V1 shell actions. Re-enrolment
will require an explicit confirmation string. Quitting a tray shell never stops
or pauses the agent.

## Release gates

A shell moves out of Preview/Disabled only after clean install, login startup,
transport ACL, upgrade, accessibility, and signed-package gates pass. Until
then, the source-built agent remains fully headless and no shell or installer
is described as production or signed.
