# Piqae.Node for .NET

`Piqae.Node` provides two explicit modes:

- `PiqaeNode` embeds an application-scoped runtime and exposes lifecycle
  delivery. State is resolved below the current user's private Piqae SDK data
  root; the relative directory never depends on the process working directory.
- `PiqaeBrokerClient` attaches to an installed Windows node. Applications
  request bounded capabilities, wait for explicit approval in the node UI, and
  exchange the approval once. The capability is stored in Windows Credential
  Manager, not a configuration file.

Cloud-capable embedded hosts must install a connector-key provider before the
runtime starts:

```csharp
var options = new PiqaeNodeOptions(
    HostMode.EmbeddedApplication,
    AvailabilityClass.ContinuousWhileAwake,
    LocalOnly: false,
    ApplicationId: "com.example.shipping",
    DataDirectory: "node-runtime");
var keys = new WindowsCredentialConnectorKeyProvider(options.ApplicationId);
using var node = new PiqaeNode(options, keys);
node.Start();
```

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
The SDK does not claim background execution or physical-print support merely
because the native library loads.
