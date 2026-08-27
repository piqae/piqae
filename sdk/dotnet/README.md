# Piqae.Node for .NET

`Piqae.Node` provides two explicit modes:

- `PiqaeNode` embeds an application-scoped runtime and exposes lifecycle
  delivery. State is resolved below the current user's private Piqae SDK data
  root; the relative directory never depends on the process working directory.
- `PiqaeBrokerClient` attaches to an installed Windows node. Applications
  request bounded capabilities, wait for explicit approval in the node UI, and
  exchange the approval once. The capability is stored in Windows Credential
  Manager, not a configuration file.

Installed-node consent never accepts an application name or signing claim from
the SDK. The broker derives the package family or verified Authenticode signer
and canonical executable identity from the accepted named-pipe client. Stored
credentials are slotted by that executable and are rejected after the verified
principal changes.

Upgrading from a caller-claimed broker credential requires one fresh consent.
After the verified credential is stored successfully, the SDK removes only the
matching legacy application-ID credential entry.

Cloud-capable embedded hosts must install a connector-key provider before the
runtime starts:

```csharp
var host = new HostConfiguration(
    NodeHostProduct.Embedded,
    "com.example.shipping",
    new NodeIdentityConfiguration("Shipping workstation", site: "Main warehouse"),
    InstalledHostPolicy.IsolatedApplication,
    new ConnectionPolicy(ConnectionManagement.UserManaged));
var options = new PiqaeNodeOptions(
    HostMode.EmbeddedApplication,
    AvailabilityClass.ContinuousWhileAwake,
    LocalOnly: false,
    ApplicationId: "com.example.shipping",
    DataDirectory: "node-runtime",
    HostConfiguration: host);
var keys = new WindowsCredentialConnectorKeyProvider(options.ApplicationId);
using var node = new PiqaeNode(options, keys);
node.Start();

// Revision-fenced display metadata is durable locally and reconciles to every
// connector independently. It never rotates credentials, routes, or queues.
var renamed = node.UpdateNodeIdentity(
    expectedRevision: 1,
    identity: new NodeIdentityConfiguration("Dispatch PC", site: "Main warehouse"));

// The embedding host forwards real Windows resume/network facts. This asks
// every configured connector for one immediate bounded sync; it does not wake
// Windows and grants no print authority.
node.ApplyLifecycle(LifecycleEvent.Woke);
node.ApplyLifecycle(LifecycleEvent.NetworkAvailable);
var reconciliation = await node.ReconcileCloudAsync(
    TimeSpan.FromSeconds(5),
    CancellationToken.None);
// LoopCompleted is not sufficient by itself: inspect FailedCount, Retryable,
// and FailureClass. Counts/classes contain no connector or tenant identity.

var prepared = node.PrepareConnectorInvitation();
// Send prepared.PublicKeyBase64 to the trusted UI that issued the invitation,
// then redeem only the authority-issued token and the prepared opaque handle.
string invitationToken = "<authority-issued, single-use invitation token>";
var connector = node.Connect(new PiqaeConnectorInvitation(
    new Uri("https://api.piqae.com"),
    invitationToken,
    prepared.KeyHandle,
    PiqaePrinterGrant.AllLocalPrinters,
    Array.Empty<string>(),
    Environment.MachineName,
    Environment.MachineName));
```

If the user abandons the flow, call
`CancelPreparedConnectorInvitation(prepared.KeyHandle)`. Pending-key expiry,
cancel cleanup, and deletion retry are durable native-runtime operations. The
SDK does not maintain a second enrollment state machine. `Connect` accepts no
connector record: ownership, workspace identity, agent identity, and management
URLs come only from the verified response at the exact HTTPS invitation origin.

The provider returns only opaque handles, public keys, and signatures to the
native runtime. Connector records and application configuration never contain
private key bytes. A stable installation key is isolated from invitation keys;
connector revocation cannot delete the installation identity. Connector-key
deletion is idempotent so durable cleanup can safely retry after a crash.

The supported Windows baseline does not provide a documented persistent,
non-exporting CNG Ed25519 signing contract. `Piqae.Node` therefore uses the
explicit fallback: a 32-byte Ed25519 seed held as a current-user generic secret
in Windows Credential Manager and copied only for bounded signing calls. The
managed and unmanaged copies are zeroed after use. This is protected at rest,
not a hardware-backed/non-exporting-key claim. See Microsoft guidance for
[Credential Manager and DPAPI](https://learn.microsoft.com/windows/win32/secbp/threat-mitigation-techniques)
and the [`CredWrite` lifecycle](https://learn.microsoft.com/windows/win32/api/wincred/nf-wincred-credwritew).

No API treats a claimed application ID or signing digest as authorization.
Installed-node SDK operations use the native Rust protocol-v4 client. The .NET
process passes the credential only into that in-process ABI; the bearer token is
never sent through the named pipe. Rust owns canonicalization, request and
response proofs, replay rejection, and downgrade rejection, and returns data
only after authentication succeeds.
The SDK does not claim background execution or physical-print support merely
because the native library loads. Modern Standby, wake timers, and Wake-on-LAN
remain hardware, driver, power-policy, network, and service-topology dependent.
Embedding applications must forward suspend/resume and network changes from the
actual Windows host; the tray is not a durable lifecycle authority.

The product release candidate is `Piqae.Node.<version>.nupkg`. Its only native
RID is currently `win-x64`, and the package pins
`BouncyCastle.Cryptography` 2.6.2 exactly. The release gate restores that exact
NuGet and dependency from an isolated local feed into a new consumer, publishes
for `win-x64`, executes the packaged native ABI, and checks the managed facade,
dependency, and runtime DLL in the output. The accompanying SPDX document lists
all three components and their staged checksums. These unsigned candidates
remain engineering evidence rather than a public NuGet publication promise.
