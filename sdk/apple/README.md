# PiqaeNodeKit

`PiqaeNodeKit` is the source Swift facade for embedding or attaching to a Piqae
node from iOS, iPadOS, and macOS applications. The API is instance-based, so an
application can test configurations independently without process-global
credentials or mutable singleton setup.

This package is **Preview**. The repository can build a real local XCFramework
containing the shared durable Rust runtime for macOS, iOS devices, and iOS
Simulator; it is not yet a signed or published binary release. Production cloud
enrollment, App Store distribution, and physical-printer certification remain
separate release gates. Compilation or a native print callback is not evidence
that paper was produced.

## Products

- `PiqaeNodeKit`: configuration, observable services, lifecycle admission,
  Keychain installation identity, adapter registry, and installed-node IPC
  abstraction.
- `PiqaeNodeKitAirPrint`: user-selected AirPrint inventory and native handoff
  foundation on iOS/iPadOS. The shared runtime durably owns enqueue,
  idempotency, fenced handoff, restart recovery, and exact outcome ACKs.
- `PiqaeNodeKitUI`: optional SwiftUI printer inventory for low-code adoption.
- `PiqaeNodeKitTesting`: deterministic identity, enrollment, installed-node,
  and printer fakes. It never contacts a physical printer.

The recommended API mirrors the durable runtime boundary:

```swift
let nativeRuntime = PiqaeNativeRuntime(
    configuration: .init(
        applicationID: "com.example.printing",
        availability: .backgroundOpportunistic,
        localOnly: true
    )
)
let airPrint = try PiqaeAirPrintAdapter(
    identityProvider: nativeRuntime,
    knownPrinterURLs: savedPrinterURLs
)
let node = PiqaeNode(
    .localOnly(
        startupMode: .automatic,
        embeddedRuntime: nativeRuntime,
        printerAdapters: [airPrint],
        hostLifecycleReporter: nil
    )
)

try await node.start()
let printers = try await node.printers.list(refresh: true)
let printerUpdates = await node.printers.observe()
let nodeUpdates = await node.observe()

// Optional low-code UI:
PiqaePrinterListView(node: node)
```

On macOS, `automatic` probes an authenticated installed-node implementation
first. A compatible installed node becomes an `attached_client`; an incompatible
node, denied capability, stale credential, or forged broker response fails
closed instead of silently starting a second runtime. Embedded fallback must be
enabled explicitly with `allowsEmbeddedFallback`; it is never inferred from an
attachment error. `PiqaeMacInstalledNodeBroker` uses the installed node's
private Unix-domain socket and explicit menu-bar consent. Protocol-v4 execution
uses one-time, time-bounded HMAC proofs verified inside the shared Rust client;
the bearer capability never crosses IPC. Approved applications can observe
status/printers, list profiles and retained history, and submit idempotent PDF or
raw jobs into the installed node's existing durable queue. They never open its
database or start a parallel queue. `manage_connectors` is a separate, explicit
consent grant. When granted, `node.connections.connect(...)` sends the
short-lived invitation through the installed node, which verifies the authority
and durably starts its own connector worker.

A macOS host supplies its stable application identity when it opts into the
installed node:

```swift
let installed = try PiqaeMacInstalledNodeBroker(
    application: try PiqaeBrokerApplication(
        applicationID: "com.example.pos",
        displayName: "Example POS"
    )
)
let node = PiqaeNode(.localOnly(
    startupMode: .automatic,
    installedNodeIPC: installed
))
```

If an installed node is deliberately replaced or its app grants are reset,
call `installed.resetAuthorization()` before retrying attachment. This deletes
only the current application's device-local broker credential; it never removes
the node queue, print history, cloud connectors, or connector signing keys.

On iPadOS, `automatic` and `embedded` select `embedded_application`. `attach` is
rejected because App Store applications can't install or rely on an arbitrary
persistent daemon.

## Custom UI and services

Use the same services that power the optional SwiftUI view:

```swift
try await node.connections.connect(cloudConfiguration)
let adapters = try await node.printers.adapters()
let printers = try await node.printers.list(refresh: true)
let profiles = try await node.profiles.list(for: printers[0].id)
let receipt = try await node.jobs.submit(request)
let status = try await node.jobs.status(receipt.jobID)
let history = try await node.jobs.history(offset: 0, limit: 50)
```

For an embedded host, NodeKit calls `adapter.submit` only after the shared
runtime has persisted the document, operation ID, route fence, deadline, and
idempotency key and then durably recorded that native handoff is beginning. A
restart never replays a `handoff_started` or `accepted` operation. Unverifiable
native acceptance becomes `delivery_uncertain`; a retry with the same
idempotency key returns the original durable job. Attached clients also require
an explicitly approved broker capability.

Named profile create, update, delete, and list operations are stored by the same
runtime. An adapter may capture native settings, but the durable profile record
is authoritative. Attached clients currently list the installed node's profiles;
driver UI capture and mutation remain node-owned. Invitation exchange is
available to an attached client only with `manage_connectors`; listing and
revocation remain runtime-owned. Embedded cloud enrollment
hands a short-lived invitation to the shared Rust worker. Its signing key is a
device-only Keychain Ed25519 key reached through synchronous non-exporting
callbacks; Swift never fabricates a connector record or receives private key
bytes. Origin, expiry, exchange response, and returned tenant identifiers are
verified before the connector is durably committed.

Create `PiqaeCloudConfiguration` with only an HTTPS authority and the
short-lived invitation. `PiqaeSensitiveString` is redacted from normal and
debug descriptions. The preview `PiqaeCloudEnrollmentProvider` initializer is
retained only for source compatibility and is ignored; it cannot bypass the
shared worker or install a caller-fabricated connector.

Remote notification registration is also opt-in and backend-injected:

```swift
try await node.remoteNotifications.register(
    deviceToken: deviceToken,
    environment: .production,
    bundleIdentifier: "com.example.printing"
)
```

`PiqaeRemoteNotificationRegistrationProvider` receives a redacted device token
and installation ID. The host application's backend registers that route; APNs
signing keys and platform credentials never belong in NodeKit or the app binary.
No registration occurs implicitly.

Printer adapters publish an exact ID/version descriptor and, for mobile support
pack matching, a display-safe `PiqaeAdapterFingerprint`: platform, exact adapter
version, and optional device family/firmware. Never put serial numbers,
Bluetooth addresses, credentials, or opaque driver payloads in that fingerprint.
Portable print intents fail closed when an adapter can't map them. Loaded media
is modelled separately from supported media and includes source, confidence, and
freshness.

`PiqaeJobReceipt.acceptedBySpooler` means the native printing subsystem accepted
the handoff. It never means that ink reached paper. If a route may have crossed
that boundary, automatic rerouting must stop and the durable runtime owns the
`delivery_uncertain` decision. `queuedLocally` means the content is durable but
the host has not begun native handoff, for example while iPadOS has not granted
enough background time.

## AirPrint

UIKit doesn't provide silent enumeration of all nearby printers. Present
`PiqaeAirPrintPicker.selectPrinter()`, retain the returned `ipp`/`ipps` URL in the
host app, and pass registered URLs back to `PiqaeAirPrintAdapter` on launch.
Discovery contacts only those user-selected printers.
The adapter canonicalizes a user-selected route by removing credentials, query,
and fragment before retaining it. Published inventory identity is gated on the
shared runtime's installation-keyed, domain-separated HMAC; neither the
canonical endpoint nor the raw installation key may be persisted or projected.

The AirPrint adapter currently accepts PDF or image data, one copy, and optional
orientation. Media pinning, density, raw printer languages, route-bound profiles,
and cutter instructions fail closed and require a certified network, Bluetooth,
External Accessory, or vendor adapter.

UIKit does not return an authoritative native spooler job identifier for this
API. After the system print controller reports completion, NodeKit therefore
records the durable attempt as `delivery_uncertain` instead of inventing an ID
or claiming spooler acceptance. A certified adapter that returns a stable native
job ID can report `accepted_by_spooler` and later reconcile a terminal status.

## Native artifact

Build the local artifact from repository root:

```console
sdk/apple/scripts/build-xcframework.sh
```

The script builds universal macOS, iOS arm64, and arm64/x86_64 iOS Simulator
static-library slices, assembles `sdk/apple/.artifacts/PiqaeNode.xcframework`,
archives it, and writes a local JSON manifest with SHA-256 and SwiftPM checksum.
Pass `--replace` only to replace those generated outputs. When the XCFramework
is present, this package consumes it as a binary target; otherwise it remains a
source facade and `PiqaeNativeRuntime.start()` fails closed.

Validate clean consumers with:

```console
swift build --package-path sdk/apple/Examples/ConsumerFixture
xcodebuild -scheme PiqaeNodeKitConsumerFixture \
  -destination 'generic/platform=iOS Simulator' \
  -sdk iphonesimulator CODE_SIGNING_ALLOWED=NO build
```

The artifact is unsigned. Signing, provenance, notarization where applicable,
publication, and App Store review evidence belong to the native release gate.

A product `v<version>` candidate additionally contains two versioned assets:
`PiqaeNodeKit-<version>.zip` (the Swift package sources) and
`PiqaeNode.xcframework-<version>.zip` (the binary target). The package manifest
references the immutable GitHub release URL and the exact SwiftPM checksum in
`PiqaeNode.artifact.json`. Download both assets from the same tag, verify their
SHA-256 sidecars and repository-bound provenance, then extract the source
package and use its directory as a SwiftPM package dependency. The release gate
does this from a clean temporary consumer and executes the packaged macOS ABI;
repository-local `.artifacts` output is not accepted as release evidence.

## iPadOS lifecycle and sleep

`PiqaeUIKitLifecycleCoordinator` forwards foreground/background state and the
remaining execution budget. A background push is a metadata-free **wake hint**:
it reconciles inventory and may advance a job that was already durably accepted
by the embedded runtime when the host reports enough remaining budget. It never
carries a document or grants eligibility by itself. The admission policy refuses
a new native handoff when the payload is not durable, the app is suspended, the
route is foreground-only, or the remaining budget is too short.

The coordinator can register an APNs device token through the injected provider
and can forward an opaque collapse ID from the app delegate. It begins a bounded
background task for reconciliation, reports expiration as `suspend_imminent`,
and cancels the worker. A repeated hint is safe because Rust idempotency and
handoff fences prevent native replay; a lost hint leaves durable work queued and
never fabricates eligibility. The API cannot run
after user force-quit and reports only opportunistic availability while the app
is installed.

Apple controls background scheduling and doesn't guarantee silent push delivery.
A force-quit, powered-off, or suspended iPad is an unavailable route. Do not
advertise an ordinary App Store app as an always-on cloud print node.

Reliable unattended deployments use one of:

- an always-awake macOS/Windows/Linux gateway;
- a directly reachable and physically certified printer adapter;
- a powered, supervised kiosk whose app remains active;
- a supported Bluetooth or MFi accessory whose legitimate background event can
  wake the app for accessory-related work.

Bluetooth state restoration can relaunch an app for relevant Bluetooth events;
it is not permission to maintain a generic cloud listener. External Accessory
support requires an MFi protocol authorized by the accessory manufacturer.
Background work must be limited to the declared purpose.

Host applications need the appropriate usage descriptions and capabilities for
the adapters they actually ship. Don't enable unrelated background modes to try
to keep the process alive.

## Apple source decisions

- [Choosing background strategies](https://developer.apple.com/documentation/backgroundtasks/choosing-background-strategies-for-your-app): background continuation is bounded and scheduled processing is system-controlled.
- [Pushing background updates](https://developer.apple.com/documentation/usernotifications/pushing-background-updates-to-your-app): background notifications are low priority, may be throttled, and aren't guaranteed.
- [Core Bluetooth background processing](https://developer.apple.com/library/archive/documentation/NetworkingInternetWeb/Conceptual/CoreBluetooth_concepts/CoreBluetoothBackgroundProcessingForIOSApps/PerformingTasksWhileYourAppIsInTheBackground.html): state restoration is scoped to Bluetooth work.
- [External Accessory](https://developer.apple.com/documentation/externalaccessory): MFi manufacturers control the protocols third-party apps may use.
- [`UIPrintInteractionController`](https://developer.apple.com/documentation/uikit/uiprintinteractioncontroller) and [`UIPrinter.contactPrinter`](https://developer.apple.com/documentation/uikit/uiprinter/contactprinter(_:)): direct printing and known-printer capability contact.
- [Keychain Services](https://developer.apple.com/documentation/security/using-the-keychain-to-manage-user-secrets): installation identity is stored as a device-only Keychain item.
- [`SMAppService`](https://developer.apple.com/documentation/servicemanagement/smappservice): persistent macOS helpers remain explicit, bundled, and user/admin approved.
- [`NSWorkspace` notifications](https://developer.apple.com/documentation/appkit/nsworkspace): macOS sleep/wake facts come from the workspace notification center.
- [`NWPathMonitor`](https://developer.apple.com/documentation/network/nwpathmonitor): network availability and Low Data Mode constraints are observations, not wake guarantees.
- [`IOPMAssertionCreateWithName`](https://developer.apple.com/documentation/iokit/1557134-iopmassertioncreatewithname): the menu/replay host uses a bounded no-idle-sleep assertion only during an active native handoff and always releases it.
- [App Review Guidelines 2.5.2 and 2.5.4](https://developer.apple.com/app-store/review/guidelines/): apps can't download executable feature code and background modes must serve their intended purpose.

## Validation

```console
swift test --package-path sdk/apple
release/tools/test_apple_node_sdk_linked.sh
swift test --package-path shells/macos
xcodebuild -scheme PiqaeNodeKit \
  -destination 'generic/platform=iOS Simulator' \
  CODE_SIGNING_ALLOWED=NO build
```

The tests use only deterministic fake printers. Device, Bluetooth, MFi, AirPrint,
sleep/wake, kiosk-duration, and App Store entitlement evidence require dedicated
hardware and distribution fixtures before the related support tier can change.
