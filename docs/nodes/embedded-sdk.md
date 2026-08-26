# Embedded node SDK deployment guide

This guide chooses a Piqae node topology for applications that need printing
without asking every user to install and configure a separate client. The
shared runtime and its safety rules are specified in
[RFC 0001](../rfcs/0001-embeddable-node-runtime.md).

## Choose the host before choosing the API

| Situation | Recommended mode | What remains available |
| --- | --- | --- |
| A Piqae desktop node is already installed | `automatic`, which attaches after local consent | The installed node owns one durable queue, all of its connectors, profiles, updates, and printer drivers. |
| A desktop app needs isolated local printing | `embedded_application` with an app-scoped state root | Only while the app/runtime is running and the computer is awake. It is a separate node and route. |
| A foreground iPad POS or production app prints immediately | `embedded_application` with AirPrint or a reviewed adapter | While the app has foreground execution and the selected printer is reachable. |
| An iPad must receive unattended cloud prints | A powered managed kiosk, legitimate accessory wake, direct printer integration, or an always-awake gateway | Only the verified topology may advertise the corresponding availability. An ordinary suspended app is not eligible. |
| A server or workstation must print before login | `machine_service` where the operating system package supports it | While the machine and service are awake. Hardware/network wake remains separately verified. |
| An integration must work without Piqae Cloud | `local_only`, optionally adding self-hosted connectors later | Local discovery, durable queueing, profiles, and native handoff do not require the hosted service. |

`automatic` never opens another process's database. It attaches through the
versioned local broker after the person controlling the node approves the
application and its requested capabilities. If embedded isolation is selected,
the app receives a distinct installation identity, state root, connector set,
and printer route.

## Intended application experiences

The high-level SDK follows an instance pattern:

```text
node = PiqaeNode(configuration)
await node.start()
await node.connections.connect(shortLivedInvitation)
printers = await node.printers.list(refresh: true)
receipt = await node.jobs.submit(request, idempotencyKey)
```

Applications can use optional native setup and printer-list UI or observe the
same services and build their own interface. A platform service-account secret
never belongs in an app. An integrator backend creates a short-lived invitation;
the node exchanges it into a connector credential held in Keychain, DPAPI, or
the platform's equivalent secure store.

Typical supported shapes are:

- a hospitality iPad selects a known AirPrint receipt printer and submits while
  the app is active;
- a product-labelling app bundles a licensed vendor adapter, exposes its exact
  adapter/version fingerprint, and maps only reviewed portable settings;
- a Shopify or other platform child account adds a connector to an existing
  shop-floor node without replacing the node's direct workspace connection;
- a desktop application uses the installed node's printers and profiles after
  local consent, so it does not create another competing local queue;
- a local-only application later adds hosted and self-hosted connectors while
  keeping the same installation and local history.

## Multiple apps, nodes, and authorities

One installed desktop node can hold many isolated connectors. All connectors
observe the same local printer inventory according to their grants, while their
credentials, tenant data, event cursors, and cloud outboxes remain separate.
The node's destination coordinator serializes native handoffs for routes that
strong evidence proves reach the same physical printer.

Two app-embedded runtimes are two installations, even on one device. They must
not share a SQLite database or private keys. On iPadOS an App Group may carry
non-secret handoff metadata between related apps, but it is not a shared daemon
or a license to open the same runtime state concurrently.

Within one control-plane authority, multiple nodes may expose routes to one
physical destination. The server selects and fences one fresh route. It may
fail over only before native acceptance could have occurred. After the OS,
AirPrint, or a vendor adapter may have accepted the job, an unknown outcome is
`delivery_uncertain`; another node is not tried automatically.

Hosted and independent self-hosted control planes are separate scheduling
authorities. A local installation can serialize their handoffs, but Piqae does
not claim global ordering or exactly-once failover between those authorities.
External OS/device jobs are exposed only as privacy-safe counts and can affect
queue estimates; their titles, owners, and documents are never projected.

## Sleep and background execution

A wake hint is not a lease and contains no job or document metadata. The server
waits for a fresh authenticated runtime observation before it offers work.

- macOS and Windows may hold a bounded power assertion while downloading,
  rendering, or crossing native handoff. They do not prevent idle sleep merely
  to wait for jobs. Wake-on-LAN/network wake is hardware, network, policy, and
  OS dependent.
- iPadOS grants opportunistic background execution. Silent notifications may be
  delayed, throttled, or dropped. Scheduled background work is chosen by the
  system. Force-quit, suspended, sleeping, or powered-off devices are not
  continuously available routes.
- Bluetooth restoration and External Accessory background events are valid only
  for the related supported accessory work. They are not a generic background
  cloud socket.

Relevant platform sources:

- Apple: [Choosing background strategies](https://developer.apple.com/documentation/backgroundtasks/choosing-background-strategies-for-your-app), [pushing background updates](https://developer.apple.com/documentation/usernotifications/pushing-background-updates-to-your-app), [Core Bluetooth background processing](https://developer.apple.com/library/archive/documentation/NetworkingInternetWeb/Conceptual/CoreBluetooth_concepts/CoreBluetoothBackgroundProcessingForIOSApps/PerformingTasksWhileYourAppIsInTheBackground.html), and [App Review Guidelines](https://developer.apple.com/app-store/review/guidelines/).
- Microsoft: [system power states](https://learn.microsoft.com/windows/win32/power/system-power-states), [system sleep criteria](https://learn.microsoft.com/windows/win32/power/system-sleep-criteria), and [Modern Standby networking](https://learn.microsoft.com/windows-hardware/design/device-experiences/networking-power-management-for-modern-standby-platforms).

## Printer adapters and settings

Desktop installed drivers remain authoritative. iPadOS uses AirPrint or an
application-bundled adapter. Piqae support packs are signed, declarative data:
they can normalize exact adapter/driver choices but cannot contain executable
SDKs, credentials, native profiles, or printer commands.

An embedded adapter publishes its exact reverse-DNS ID, bundled version,
transport, and optional device-family/firmware predicates. Ambiguous or missing
evidence produces no semantic mapping. Portable options such as media,
orientation, cut, density, and copies fail closed when the selected adapter
cannot prove a mapping. Supported media and currently loaded media remain
separate; an operator confirmation is freshness-bounded and a device sensor is
preferred.

Vendor SDK licensing, MFi protocol authorization, Bluetooth entitlements, local
network privacy prompts, and physical-printer certification remain the host
application's release responsibilities. No generic adapter is advertised as a
vendor-certified driver.

## Removing access without losing recovery evidence

Cleanup is deliberately split by ownership instead of being one ambiguous
"delete node" operation:

- **Revoke one connection** when a hosted workspace, managed child account, or
  self-hosted authority should stop using the installation. Other connectors,
  the installation identity, local profiles, and the local queue remain.
- **Revoke an application capability** when an attached desktop app should no
  longer use the installed node. The app loses broker access without rotating
  cloud connector or device keys.
- **Remove a node from a workspace** from the scoped dashboard only after
  confirming the node name. The server revokes that tenant's node identity and
  returns work that has not crossed native acceptance to scheduling. It does
  not erase the computer's local database, and uncertain/native-accepted work
  is never silently retried.
- **Reset an embedded installation** is a host-application uninstall/reset
  operation, not a cloud API call. Stop the runtime, prove there is no active or
  uncertain handoff, revoke its connectors, then let the app remove its exact
  sandbox-owned state and Keychain/Credential Manager entries. Never point a
  generic recursive delete at a shared state root.

Removing the app does not prove that an iPad, desktop spooler, or printer
discarded accepted work. Operators resolve any retained uncertain deliveries
before resetting the installation.

## Package and compatibility policy

The public application SDKs are the supported integration boundary. Internal
Rust crates stay `publish = false`; applications must not assemble a node from
private crates whose queue/storage contracts can change together inside the
monorepo.

- Apple applications consume `PiqaeNodeKit` through Swift Package Manager. A
  tagged binary release must reference the reviewed, signed XCFramework
  checksum produced from the same Piqae tag; a source checkout is for Preview
  development only.
- Windows applications consume the safe `Piqae.Node` .NET facade. A tagged
  NuGet package contains the matching native runtime for supported RIDs and is
  validated in a clean consumer before publication.
- Both packages expose their native ABI and local broker protocol ranges. A
  desktop client attaches only when those ranges overlap; it never guesses or
  opens the installed node's database as a fallback.
- Embedded native code updates with the host app. An attached machine service
  continues to use Piqae's signed updater, preserving the documented N/N-1
  broker window.

## Testing and support claims

Use fake printers for lifecycle, restart, connector, idempotency, and routing
tests. A native API callback or spooler acceptance proves only software handoff,
not paper output. Physical, kiosk-duration, suspend/wake, Bluetooth, MFi, stock,
and vendor-option claims require named hardware and controlled release evidence
before the support matrix can promote them.
