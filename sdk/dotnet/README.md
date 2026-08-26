# Piqae.Node for .NET

`Piqae.Node` provides two explicit modes:

- `PiqaeNode` embeds an application-scoped runtime and exposes lifecycle
  delivery. State is resolved below the current user's private Piqae SDK data
  root; the relative directory never depends on the process working directory.
- `PiqaeBrokerClient` attaches to an installed Windows node. Applications
  request bounded capabilities, wait for explicit approval in the node UI, and
  exchange the approval once. The capability is stored in Windows Credential
  Manager, not a configuration file.

No API treats a claimed application ID or signing digest as authorization.
The SDK does not claim background execution or physical-print support merely
because the native library loads.
