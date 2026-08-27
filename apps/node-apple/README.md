# Piqae Node for iPhone and iPad

This directory contains the thin standalone SwiftUI host for the shared
`PiqaeNodeKit` runtime. It is a **Preview source target**, not an App Store or
TestFlight release. The app does not implement a second queue, connector
worker, scheduler, or printer identity system.

The build includes:

- first-run Node name, Site, Location, and Labels setup with a privacy-safe
  generic iOS suggestion;
- a user-managed, multi-connection list for hosted and self-hosted Piqae
  authorities;
- user-selected AirPrint inventory and the extension point for reviewed vendor
  or accessory adapters;
- explicit profile, loaded-media/stock, native-queue, freshness, and missing
  telemetry truth;
- durable queue/history, runtime/background status, and local diagnostics;
- Dynamic Type, VoiceOver labels, and system light/dark appearance.

The node name is visible and editable. The app does not read or upload the
logged-in user, contacts, postal address, advertising identifier, device serial
number, or Apple user-assigned device name. Apple requires a special entitlement
for the latter on iOS 16 and later; Piqae intentionally does not request it.

## Generate and build

XcodeGen 2.46 or newer is required to regenerate the checked-in project:

```console
apps/node-apple/scripts/generate-project.sh
xcodebuild -project apps/node-apple/PiqaeNodeApple.xcodeproj \
  -scheme PiqaeNode -destination 'generic/platform=iOS Simulator' \
  CODE_SIGNING_ALLOWED=NO build
```

Build the unsigned linked Preview candidate with:

```console
apps/node-apple/scripts/build-preview.sh
```

The script assembles the repository's local XCFramework if necessary and
produces a checksum-bearing unsigned `.app.zip` below the ignored
`apps/node-apple/.artifacts` directory. This proves compilation and linkage only.
It is not installable through TestFlight and is not physical-print evidence.

## Distribution gates still required

Before uploading a build, all of the following remain external release work:

1. Apple Developer team membership, a registered `com.piqae.node` App ID,
   development/distribution certificates, and matching provisioning profiles.
2. A signed, provenance-bound native XCFramework from the same source revision.
3. An App Store Connect record, privacy disclosures, export-compliance answers,
   support/privacy URLs, screenshots, review notes, and age/category metadata.
4. APNs entitlement/provisioning plus a production backend which stores tokens
   tenant-safely and sends metadata-free, collapseable wake hints. Provider keys
   never belong in the app or NodeKit. This Preview target does not register an
   APNs token and reports remote wake as not configured; it still retries while
   foregrounded, after network/foreground recovery, and during best-effort
   scheduled maintenance granted by iOS.
5. TestFlight internal/external testing and App Review. A buildable archive does
   not imply review approval.
6. Named-device AirPrint, Bluetooth, MFi/vendor, supervised-kiosk, sleep/wake,
   force-quit, network-loss, and long-duration duplicate-prevention evidence
   before any relevant support tier changes.

Apple controls background scheduling. Background notifications may be delayed,
throttled, coalesced, or dropped, and a force-quit app is unavailable. Piqae
uses them only as hints; the durable runtime must reconcile and publish a fresh
eligible route before cloud work can be offered. The app delegate holds at most
32 opaque in-memory hint IDs during cold launch,
coalesces duplicates, and completes every fetch handler within a 20-second
deadline; it never persists a hint, job identifier, or document metadata.
See Apple's
[background strategy](https://developer.apple.com/documentation/backgroundtasks/choosing-background-strategies-for-your-app),
[background notification](https://developer.apple.com/documentation/usernotifications/pushing-background-updates-to-your-app),
[device-name entitlement](https://developer.apple.com/documentation/BundleResources/Entitlements/com.apple.developer.device-information.user-assigned-device-name),
and [TestFlight](https://developer.apple.com/help/app-store-connect/test-a-beta-version/testflight-overview)
documentation.
