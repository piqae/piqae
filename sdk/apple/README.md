# PiqaeNodeKit

`PiqaeNodeKit` is the source Swift facade for embedding or attaching to a Piqae
node from iOS, iPadOS, and macOS applications. The API is instance-based, so an
application can test configurations independently without process-global
credentials or mutable singleton setup.

This package is **Preview**. It is a source Swift Package, not a published binary
XCFramework. The durable Rust runtime/FFI artifact, signed XCFramework release
pipeline, production cloud enrollment provider, and physical-printer
certification remain separate release gates. Compilation or an AirPrint callback
is not evidence that paper was produced.

## Products

- `PiqaeNodeKit`: configuration, observable services, lifecycle admission,
  Keychain installation identity, adapter registry, and installed-node IPC
  abstraction.
- `PiqaeNodeKitAirPrint`: user-selected AirPrint printer registry and direct
  handoff adapter on iOS/iPadOS.
- `PiqaeNodeKitUI`: optional SwiftUI printer inventory for low-code adoption.
- `PiqaeNodeKitTesting`: deterministic identity, enrollment, installed-node,
  and printer fakes. It never contacts a physical printer.

The recommended API mirrors the durable runtime boundary:

```swift
let airPrint = try PiqaeAirPrintAdapter(
    identityProvider: nativeRuntime,
    knownPrinterURLs: savedPrinterURLs
)
let node = PiqaeNode(
    .localOnly(
        startupMode: .automatic,
        printerAdapters: [airPrint]
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
node fails closed instead of silently starting a second runtime. The current
menu shell supplies `PiqaeMacInstalledNodeIPC` as a compatibility adapter over
the shipped loopback API. Applications continue to depend on the versioned
`PiqaeInstalledNodeIPC` protocol when that adapter moves to a Unix-domain socket.
The compatibility adapter is currently read-only for inventory and profiles;
job submission and cloud enrollment through an attached node remain unavailable
until the authenticated runtime ABI is published.

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
```

`PiqaeCloudEnrollmentProvider` is deliberately injected. It exchanges a
short-lived invitation and returns a connector summary. Platform service-account
keys stay on the integrator's backend; they must not be compiled into the app.
`PiqaeSensitiveString` is redacted from normal and debug descriptions.

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
`delivery_uncertain` decision.

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

## iPadOS lifecycle and sleep

`PiqaeUIKitLifecycleCoordinator` forwards foreground/background state and the
remaining execution budget. A background push is a metadata-free **wake hint**:
it reconciles inventory and connector state without leasing or accepting a job.
The admission policy refuses a new native handoff when the payload isn't durable,
the app is suspended, the route is foreground-only, or the remaining budget is
too short. Bounded background time is used only to finish work that already
started.

The coordinator can register an APNs device token through the injected provider
and can forward an opaque collapse ID from the app delegate. It begins a bounded
background task for reconciliation, reports expiration as `suspend_imminent`,
and cancels the worker. A repeated hint is safe because it only reconciles; a
lost hint performs no work and never fabricates eligibility. The API cannot run
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
swift test --package-path shells/macos
xcodebuild -scheme PiqaeNodeKit-Package \
  -destination 'generic/platform=iOS Simulator' \
  CODE_SIGNING_ALLOWED=NO build
```

The tests use only deterministic fake printers. Device, Bluetooth, MFi, AirPrint,
sleep/wake, kiosk-duration, and App Store entitlement evidence require dedicated
hardware and distribution fixtures before the related support tier can change.
