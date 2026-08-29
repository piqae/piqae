# Local agent control contract

The Piqae agent is headless. Native shells are optional clients; they do not
open SQLite, render documents, contact the hosted control plane, or hold the
device private key.

## V1 implementation

The current agent exposes `crates/local-api` on loopback HTTP, defaulting to
`127.0.0.1:39100`. The root remains an API surface. An intentionally small
node-owned queue view is available only through an authenticated browser
handoff; navigating to the loopback root does not expose it.

The server refuses non-loopback binds, caps request bodies at 72 MiB, and
requires the startup-generated bearer token for every operational route.
`/health` is the only unauthenticated route and exposes no private data. The
token is stored as `local.token` under the configured agent data directory.
Handlers send commands over a bounded Tokio channel and cannot access SQLite or
print drivers directly.

Implemented V1 operations include status, printer/profile/queue reads, exposure,
pause/resume, local job/test submission, native profile capture
authorization/commit/cancel/validation, loaded-media confirmation, and explicit
resolution of a local native-handoff uncertainty. See `crates/local-api` for
the exact route contract.

`GET /v1/local/jobs/{job_id}/resolve-ambiguous-handoff` returns the opaque
`ambiguity_id` for the current exact local route fence. `POST` to the same path
requires that ID and accepts exactly one of `release_for_retry` or
`confirm_accepted`; both operations require the normal loopback bearer token.
It applies only to a local-only `delivery_uncertain` job. Cloud-managed jobs
must be resolved by their authenticated remote authority. A release is an
explicit operator authorization for a bounded local retry; confirmation keeps
the truthful `delivery_uncertain` queue state and makes the attempt
non-runnable. The decision is durably bound to that ambiguity ID, so a delayed
replay cannot release a newer uncertain attempt for the same job. Both
decisions are idempotent across restart. The endpoint never exposes the route
fencing token or treats native acceptance as proof that paper was produced.

Native shells open the node queue by authenticating `POST
/v1/local/dashboard-sessions` with `local.token` and opening the returned URL.
The URL capability expires after 30 seconds, is single-use, and is exchanged
for a 15-minute `HttpOnly`, `SameSite=Strict` loopback cookie before redirecting
to a clean URL. The page shows paged retained local history and provider-neutral
connector state. It never exposes connector credentials. Connector management
or reauthorization links are shown only when enrolment records an explicit safe
destination; the agent does not guess provider-specific paths.

Reprint is an explicit, confirmed new attempt using content still retained by
the node. It is limited to terminal attempts and a currently present printer.
The original job plus caller idempotency key deterministically identifies the
new attempt, so replaying a browser request cannot create another job. A
reported completion remains an observation of the node or OS queue, not proof
that ink reached paper.

Shell capabilities are intentionally described narrowly:

- Linux Preview shell reads status from loopback HTTP using `local.token`.
- macOS Preview shell reads authenticated status, printers, queues, and
  profiles; it can pause/resume, expose printers, run tests, and open native
  create/edit/clone profile capture.
- Windows development shell reads authenticated loopback status, lists queues
  and profiles, and opens native create/edit/clone profile capture. It remains
  Disabled for production release.
- Linux and macOS show **Open Piqae** only when `PIQAE_DASHBOARD_URL` contains
  an explicit HTTP(S) dashboard URL. They never open the loopback API root as
  though it were a UI.

## Target transport, not implemented in V1

The desired production transport is a named pipe with an installer-scoped ACL
on Windows and a mode `0600` Unix-domain socket in a mode `0700` directory on
macOS and Linux. The target protocol uses length-prefixed JSON, a 64 KiB
message limit, protocol versioning, and an OS-ACL-protected session challenge.
The Rust contract and codec foundations live in `crates/local-ipc`.

Target operations also include restart, support-bundle export, and local
re-enrolment. These remain roadmap shell actions even though connected
one-time enrolment is implemented as an agent bootstrap command. Quitting a
tray shell never stops or pauses the agent.

## Release gates

A shell moves out of Preview/Disabled only after clean install, login startup,
transport ACL, upgrade, accessibility, and signed-package gates pass. Until
then, the source-built agent remains fully headless and no shell or installer
is described as production or signed.
