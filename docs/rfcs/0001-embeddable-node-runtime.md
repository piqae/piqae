# RFC 0001: embeddable node runtime and native SDKs

- Status: Accepted for implementation
- Accepted: 26 August 2026
- Owners: node runtime, native SDK, control-plane, and release maintainers

## Problem and users

Piqae currently ships a durable headless node plus disposable desktop shells.
Applications also need to provide printing without requiring a separate Piqae
installation. The implementation must not fork queue, identity, recovery, or
delivery semantics by operating system or by host application.

The intended users are:

- desktop applications that want to attach to an installed Piqae node;
- desktop applications that need an app-scoped embedded node;
- iPad point-of-sale, hospitality, click-and-collect, production-labelling,
  kitchen-label, and receipt applications;
- integrators enrolling devices into managed child workspaces;
- local-only and self-hosted installations that must not depend on Piqae Cloud.

## Constraints and non-goals

- The durable node remains the authority for identity, queueing, recovery, and
  cloud synchronization.
- A spooler handoff is not proof that paper was produced.
- Installed desktop drivers remain authoritative. AirPrint and certified vendor
  adapters are the available authorities on iPadOS.
- iPadOS background execution is opportunistic. The SDK must not present a
  suspended or force-quit app as an always-on node.
- No client SDK embeds a platform service-account secret.
- No runtime may open another runtime's database or copy its private keys.
- Piqae does not claim global exactly-once delivery across independent hosted
  and self-hosted scheduling authorities.
- This RFC does not certify a physical printer or a production support tier.

## Product modes

The same runtime supports four explicit host modes:

| Mode | Durable owner | Expected availability |
| --- | --- | --- |
| `machine_service` | OS service/daemon | while the machine is awake, including before login where supported |
| `user_agent` | logged-in user service | while the user session and machine are awake |
| `embedded_application` | one host app sandbox | while the host has execution time |
| `attached_client` | an existing node reached through local IPC | inherits the existing node's availability |

Every enrolled route additionally reports an availability class:

- `continuous_while_awake`;
- `foreground_only`;
- `background_opportunistic`;
- `managed_kiosk`;
- `wake_relay_capable`.

These are scheduling inputs and support truth, not marketing labels. A server
never infers a stronger class from a platform name.

## Repository and package boundaries

The monorepo will contain:

```text
crates/node-runtime/       durable, OS-independent orchestration
crates/node-host-api/      host capabilities and lifecycle traits
crates/node-client/        typed client of the local broker contract
crates/node-ffi/           narrow stable C ABI around node-runtime
crates/piqae-agent/        composition binary only
sdk/apple/                 PiqaeNodeKit Swift package/XCFramework
sdk/dotnet/                Piqae.Node NuGet package and safe bindings
sdk/typescript/            server-side platform SDK
contracts/openapi/         public control-plane contract
contracts/node-sdk/        versioned local SDK/IPC model fixtures
```

`node-runtime` owns the state machine. SDK UI modules and native shells consume
snapshots and commands; they never implement retries or manipulate storage.
Bindings are generated from the versioned contract and conformance fixtures.

The Apple SDK is distributed through Swift Package Manager. A source-only
facade may live in the repository, while tagged releases reference checksummed
XCFramework artifacts produced by the native release workflow. The Windows SDK
is distributed as a signed NuGet package. Both share the Piqae release version
and publish a tested compatibility table. The TypeScript package remains the
server-side integration SDK.

Embedded SDK releases cannot download executable feature code. The host app
updates its embedded runtime through its normal application release. Desktop
companion services use the existing signed updater and can update separately
within the documented N/N-1 protocol window.

## High-level SDK contract

The recommended API uses an instance rather than process-global configuration:

```text
client = PiqaeNode(configuration)
await client.start()
await client.connections.connect(invitation)
client.printers.observe()
await client.jobs.submit(request, idempotencyKey)
await client.profiles.capture(printer, intent)
```

The package provides both:

- optional native setup/printer-selection components for low-code adoption;
- the lower-level observable services used by those components for completely
  custom UI.

Configuration chooses `automatic`, `attach`, or `embedded`. `automatic` first
attempts an authenticated broker attachment on desktop, then uses an
app-scoped embedded runtime if allowed. iPadOS always uses the embedded host.

`localOnly` is a first-class configuration. Cloud enrollment is an additive
connector, not a different runtime. The application can later add hosted or
self-hosted connectors without replacing the installation identity or local
queue.

## Local broker and coexistence

The installed desktop node exposes a versioned named-pipe or Unix-domain-socket
broker. Presence discovery reveals no tenant or printer information. Attaching
an app requires an explicit local consent transaction and issues an app-scoped,
revocable capability. The app never reads the node's bootstrap token.

One installation lock protects each state root. A second runtime must attach,
choose a different app-scoped root, or fail with a typed `nodeAlreadyRunning`
error. It may never recover by opening the existing SQLite files directly.

Multiple unrelated iPad apps cannot share a process or durable state. Each app
is a separate installation and route. Related apps may use an Apple App Group
for non-secret handoff metadata, but still must not concurrently open one node
database. The server and local destination coordinators prevent duplicate
handoffs where they share an authority.

## Identity and physical destinations

Identity is layered:

- installation: a runtime in an OS/app security boundary;
- connector: one workspace/environment authorization;
- route: one native path to a printer;
- physical destination: the actual output device;
- scheduling authority: the control plane coordinating jobs.

Strong automatic destination evidence includes IPP printer UUID, authenticated
device certificate, vendor/USB/accessory serial, or another stable hardware
identifier. Endpoint, queue name, make/model, and driver fingerprint are
supporting evidence only. Weak evidence never silently merges destinations.

Within one authority, an internal coordination domain may span tenant routes
without exposing cross-tenant identity or job data. Tenant APIs remain scoped;
the scheduler sees only the minimum owner and ordering envelope. Independent
authorities coordinate only on the same local installation. Cross-authority
automatic failover remains disabled unless a future authenticated federation
protocol is separately accepted.

## Queue and failover contract

Wake is separate from delivery:

1. The control plane durably queues the job.
2. It may send a bounded wake hint without leasing or exposing content.
3. A route publishes a fresh authenticated availability observation.
4. The scheduler acquires the physical-destination reservation and route fence.
5. The selected node durably accepts the job and content.
6. The local destination coordinator serializes the native handoff.

If a node disappears before durable native acceptance, the lease and fence may
expire and another eligible route may be selected. Once the OS or device may
have accepted the job, automatic rerouting stops. Unknown outcomes enter
`delivery_uncertain` and require the existing explicit resolution workflow.

Native queue telemetry distinguishes Piqae-owned jobs from privacy-safe
external/unknown counts. Piqae cannot promise ordering relative to jobs sent
directly to an OS spooler or printer by software outside the scheduling
authority.

## Printer capabilities, profiles, and media

Each route publishes monotonically sequenced observations containing:

- state and freshness deadline;
- Piqae-owned, external, and unknown queue counts;
- native job progress where available;
- portable capabilities and opaque native capability revision;
- profile summaries and compatibility revision;
- driver/device alerts;
- supported media;
- loaded-media value, source, confidence, and confirmation time.

Supported media is not loaded media. Loaded media is authoritative only when a
device sensor reports it or an operator recently confirms it. Native profile
payloads remain opaque, immutable, route-bound, and node-local.

On iPadOS, the system AirPrint contract or a certified adapter replaces a
desktop installed driver. Portable intents such as media, orientation, cut,
density, and copies map through an adapter that returns validation warnings or
fails closed. Unsupported options never silently fall back.

## iPadOS lifecycle

The iPad runtime uses:

- foreground execution for discovery, setup, profile selection, and normal
  submission;
- bounded background time to finish an in-flight handoff or begin durable work
  only when the host supplies enough explicit remaining execution budget;
- silent push as an unreliable wake hint, never as acceptance;
- scheduled background processing for reconciliation and maintenance, not
  immediate print delivery;
- Bluetooth or External Accessory wake only for the related supported device;
- a managed-kiosk capability only after runtime readiness checks.

The runtime checks remaining execution budget before accepting cloud work. If
it cannot safely persist, prepare, and begin the handoff, the job stays on the
control plane. Suspension flushes storage, records route unavailability, and
releases any lease that has not crossed the native boundary. Resume always
reconciles local storage, the system/vendor queue, inventory, and cloud state.

Reliable unattended iPad deployments require one of:

- an always-awake desktop/Linux gateway;
- a directly reachable certified printer adapter;
- a powered supervised kiosk that keeps the host app active;
- a supported accessory whose legitimate background mode supplies wake events.

Force-quit or powered-off iPads are unavailable routes.

## Desktop power lifecycle and wake relay

Desktop hosts hold a bounded system power assertion only while downloading,
rendering, or crossing the native handoff. They never prevent sleep merely to
wait for cloud work. Suspend handlers flush durable state and stop new leases;
wake handlers rotate sessions and reconcile before advertising readiness.

Wake-on-LAN or network wake is optional and verified, not inferred. A wake relay
contains no document or tenant data, sends only an authenticated/ratelimited
local wake request, and never receives a job lease. The scheduler waits for a
new signed heartbeat before normal routing.

## Security and privacy

- Platform service-account keys are server-side only.
- Invitations are short-lived, single-purpose, and exchange into a node-held
  connector credential.
- Installation and connector keys use platform secure storage.
- App clients receive least-privilege local capabilities that can be revoked
  without revoking the connector.
- Printer identity evidence is HMAC-protected before leaving the node and is
  not returned through tenant APIs.
- Background pushes contain no job metadata or content URLs.
- Support bundles and SDK errors remain redacted by construction.

## Migration and cleanup

All server schema changes are append-only and N/N-1 compatible. Existing node
installations retain their installation, connector, agent, route, profile, and
queue identities. New host-capability fields default conservatively.

Compatibility adapters are time-bounded:

1. Existing shells move to `node-client`.
2. The legacy loopback control API delegates to the same command bus.
3. After one documented compatibility window and usage evidence, operational
   loopback routes are removed; health and one-time browser handoff may remain.
4. Duplicate runtime structs, shell-owned mappings, and orchestration in
   `piqae-agent/main.rs` are deleted as each consumer moves.

There is one state machine, one binding contract, and one conformance suite at
the end of the migration.

## Testing and release gates

Automated gates cover local-only/cloud modes, attach/embedded collision,
connector isolation, suspend at every job transition, wake-hint loss/replay,
route failover before handoff, uncertainty after handoff, multiple app routes,
cross-tenant privacy, hosted/self-hosted separation, migration N-1, SDK ABI,
and generated contract compatibility.

Physical gates separately cover AirPrint, network thermal, Bluetooth LE, MFi or
vendor accessories, CUPS, Winspool, sleep/wake, offline media, spooler restart,
duplicate observation, and prolonged kiosk operation. No platform moves above
Preview from compilation or virtual acceptance alone.

## Alternatives rejected

- Separate Swift and Windows queue implementations: they would drift at the
  delivery boundary.
- A permanent loopback HTTP SDK: it exposes the wrong authentication and local
  discovery model.
- An iPad companion daemon: App Store applications cannot install an arbitrary
  persistent process, and background execution is system-controlled.
- Treating a printer name or IP as physical identity: it creates unsafe merges.
- Cross-server automatic failover without federation: it cannot provide a
  shared fence and can duplicate physical output.

## Rollback

New fields and routes are additive. Older nodes ignore wake hints and publish no
stronger availability than `continuous_while_awake`. A server rollback retains
new observations without leasing them to an older scheduler. SDK applications
can disable cloud connectors while retaining their local queue. Schema columns
are not removed during rollback.
