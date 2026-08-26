# Remote wake and delivery reliability

Remote wake is an availability optimization, never a print-delivery authority.
The durable job, connector acceptance intent, delivery attempt, destination
reservation, and native handoff journal remain authoritative when a process,
network, device, or notification provider disappears.

## Per-job fallback sequence

Piqae applies the following order for one job and one physical destination:

1. **Connected-session nudge.** A node that is already awake observes the
   content-free hint on signed sync. The hint grants no lease and contains no
   job, printer, document, tenant, or content metadata.
2. **External provider relay.** If a separately configured relay is supported,
   the tenant webhook receives the same opaque hint ID. The relay may translate
   it to APNs, WNS, a site Wake-on-LAN service, or a vendor notification. A
   successful webhook means only that the relay accepted the event.
3. **Bounded retry.** The durable webhook/outbox retries transport failures with
   bounded backoff. The application coalesces the hint and asks its embedded
   cloud supervisor for an immediate generation-fenced sync. A mobile host
   stops retrying before its OS execution budget expires. The supervisor result
   reports whether that exact generation's loop completed, counts of connector
   successes and failures, an identity-free failure class, and whether every
   failure is retryable. A completed loop with a failed connector is never
   reported as successful.
4. **Alternate eligible route.** The control plane may select another route to
   the same confirmed physical destination only while there is no native
   handoff which might have succeeded. A single destination reservation and
   monotonically increasing fence generation prevent concurrent handoffs.
5. **Operator attention.** Expiry, no eligible route, conflicting physical
   identity, or any ambiguous native boundary remains queued or becomes
   `delivery_uncertain`. Piqae never guesses that paper did or did not emerge.

Wake hints can fan out to several candidate nodes because they carry no work
authority. Jobs cannot fan out: one scheduler owns one destination reservation.
After `handing_to_spooler`, `accepted_by_spooler`, or any ambiguous adapter
result, automatic failover is forbidden. An operator must resolve uncertainty
before authorizing a new print.

Hosted Piqae, independent self-hosted Piqae, and software outside Piqae are
separate scheduling authorities. A local installation can serialize their
native handoffs, but it cannot promise global exactly-once delivery without a
shared authenticated destination fence.

## Platform truth

### iPhone and iPad

An ordinary iOS or iPadOS application is an opportunistic route:

- APNs background notifications are low priority, may be throttled, coalesced,
  delayed, or dropped. Apple gives a delivered background notification only a
  short execution window (documented as up to 30 seconds).
- If the user force-quits the app, it receives no remote notifications until
  the user launches it again. A killed app also loses a held background
  notification. Piqae therefore marks the route unavailable rather than
  claiming a retry can wake it.
- `BGAppRefreshTask` and `BGProcessingTask` run when the system chooses. They
  are repair opportunities, not job deadlines or a persistent listener.
- `beginBackgroundTask` only finishes work which already began; it is bounded
  and must honor expiration. It is not an always-on entitlement.
- Bluetooth state restoration and External Accessory background modes apply
  only to legitimate supported accessory work. They do not authorize a
  general-purpose cloud socket. External Accessory protocols also require the
  relevant MFi relationship.

NodeKit consequently reports `foreground_only` by default. A host may opt into
`background_opportunistic` only when it forwards real lifecycle and remaining
budget. A push handler requests an immediate cloud pass, retries within that
budget, and then drains only durable runnable work. Receiving a hint alone does
not make the route eligible. Reliable unattended iPad deployments require a
powered supervised kiosk, a reviewed accessory-specific background topology, a
directly reachable certified printer, or an always-awake gateway.

NodeKit coalesces concurrent copies of the same opaque APNs collapse ID. Native
reconciliation uses request/poll rather than blocking Swift's cooperative
executor, so cancellation and expiration return promptly while a network pass
is pending. The UIKit expiration callback closes the shared execution
generation synchronously before actor cleanup. Drain, accepted-job observation,
and wake reconciliation all use that gate: no later handoff can begin after
expiration. If expiration races a native call which may already have crossed
the boundary, its accepted or ambiguous result is retained and never rewritten
as a safe retry.

Official Apple sources:

- [Choosing Background Strategies for Your App](https://developer.apple.com/documentation/backgroundtasks/choosing-background-strategies-for-your-app)
- [Pushing background updates to your App](https://developer.apple.com/documentation/usernotifications/pushing-background-updates-to-your-app)
- [Extending your app's background execution time](https://developer.apple.com/documentation/uikit/extending-your-app-s-background-execution-time)
- [Core Bluetooth background processing](https://developer.apple.com/library/archive/documentation/NetworkingInternetWeb/Conceptual/CoreBluetooth_concepts/CoreBluetoothBackgroundProcessingForIOSApps/PerformingTasksWhileYourAppIsInTheBackground.html)
- [Configuring remote notification support](https://developer.apple.com/library/archive/documentation/NetworkingInternet/Conceptual/RemoteNotificationsPG/HandlingRemoteNotifications.html)

### macOS

The installed agent resumes normal signed reconciliation after process start,
network recovery, or `NSWorkspace.didWakeNotification`. Its PrintCore replayer
wraps the one native handoff in a bounded IOKit no-idle-sleep assertion and
releases that assertion on completion or timeout. The same guard is limited to
active download/render/handoff phases; it is never held merely to wait for
future work.

“Wake for network access” and Power Nap are user settings whose availability
depends on the Mac model, power source, network, and sharing topology. They are
not an application remote-wake API and are never advertised solely because a
Mac node exists. A site relay may advertise `wake_on_lan` only after that exact
hardware/network path is configured and tested. After waking, a fresh signed
runtime and printer observation is still required before a lease.

Official Apple sources:

- [Share your Mac resources when it's in sleep](https://support.apple.com/guide/mac-help/mh27905/mac)
- [Set sleep and wake settings for your Mac](https://support.apple.com/guide/mac-help/mchle41a6ccd/mac)
- [`NSWorkspace` sleep/wake notifications](https://developer.apple.com/documentation/appkit/nsworkspace)
- [`IOPMAssertionCreateWithName`](https://developer.apple.com/documentation/iokit/1557134-iopmassertioncreatewithname)

### Windows

The Windows service or embedding host reports suspend before sleep and requests
immediate reconciliation on automatic resume and network recovery. A tray is a
disposable UI and never owns this responsibility. Piqae does not keep a machine
awake while it is idle.

The .NET SDK exposes cancellable request/poll reconciliation with the same
generation-bound aggregate result. Its synchronous compatibility call also
polls without holding the native ABI across network work. Neither API wakes
Windows: resume, WNS, a scheduled task, or a site relay must first cause the
durable service or embedding host to run.

Modern Standby does not make arbitrary desktop services continuously reachable:
Windows can place third-party services in network quiet mode. WNS and hardware
pattern matching are OS-managed paths, not proof that Piqae's process ran. On
classic S3/S4 systems, waitable timers and Wake-on-LAN depend on system power
policy, firmware, adapter/driver support, and network reachability. A wake timer
is a scheduled repair opportunity rather than an immediate response to a new
job. A magic packet needs a configured site relay on the appropriate network;
Piqae Cloud cannot infer or safely route one from a printer name or MAC-like
input.

Official Microsoft sources:

- [System power states](https://learn.microsoft.com/windows/win32/power/system-power-states)
- [System wake-up events](https://learn.microsoft.com/windows/win32/power/system-wake-up-events)
- [Automatic resume notification](https://learn.microsoft.com/windows/win32/power/pbt-apmresumeautomatic)
- [Modern Standby networking and network quiet mode](https://learn.microsoft.com/windows-hardware/design/device-experiences/networking-power-management-for-modern-standby-platforms)
- [Wake-on-LAN behavior](https://learn.microsoft.com/troubleshoot/windows-client/setup-upgrade-and-drivers/wake-on-lan-feature)

### Local-only and offline operation

A local-only runtime has no external relay and needs none. It accepts local work
only while its host execution policy allows it, persists the job before native
handoff, and drains on foreground, process restart, wake, or network/accessory
recovery as applicable. Cloud outages do not block local queue recovery.
Connector-specific cloud failures do not block other connectors or the shared
local adapter journal.

## Privacy-safe observability

Operators may see only facts needed to diagnose availability:

- hint ID, channel, requested/expiry/observed timestamps and terminal status;
- runtime lifecycle, availability class, reported execution budget and
  freshness;
- connector sync age and redacted error class;
- inventory revision/acknowledgement age;
- privacy-safe queue counts (`piqae_owned_jobs`, `external_jobs`, and
  `unknown_jobs`);
- delivery-attempt phase, route/fence generation and whether attention is
  required.

Wake payloads and metrics must never contain document titles, filenames,
content URLs, customer data, native spool-job details, credentials, APNs tokens,
printer serial numbers, or another connector's job identifiers. “Relay queued,”
“provider accepted,” “node observed,” “node eligible,” “native accepted,” and
“reported complete” are distinct observations and must not be collapsed into a
single “online” or “printed” claim.

## Release evidence

Deterministic fake-printer tests prove durable retry, idempotency, fencing,
restart recovery, expiry, and absence of duplicate native handoff. They cannot
prove OS wake or paper output. Promotion of a wake topology requires signed app
builds and a hardware/network soak matrix covering power source, locked screen,
sleep state, network loss, provider throttling, force-quit/restart, printer
offline/recovery, and ambiguous spooler outcomes. APNs entitlements/provider
credentials, managed-device policy, MFi/vendor approval, Windows service
installation, and site Wake-on-LAN configuration remain deployment-specific
prerequisites.
