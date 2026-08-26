# Node host configuration contract

`node-host-configuration.schema.json` is the portable, credential-free shape
used by the Apple and .NET node SDKs. It describes the host experience; the
durable Rust runtime still owns installation identity, connector credentials,
queues, recovery, and cloud synchronization.

- A `standalone` host is the operator's general-purpose node. It normally uses
  `user_managed` connections and may retain many hosted or self-hosted
  connections.
- An `embedded` host belongs to another application. It may expose the same
  connection UI or let the integrator automate one or many invitations. The
  contract intentionally has no single-connection restriction.
- Desktop SDKs should `prefer_installed` so approved applications attach to the
  machine's standalone node and share its one durable queue. An app may select
  `isolated_application` explicitly when isolation is the intended topology.
- iOS and iPadOS applications are sandboxed. Their effective policy is an
  isolated application runtime even when the app itself is the standalone
  Piqae Node product; another app cannot attach to it as a daemon.

`display_name`, `site`, `location`, and `labels` are operator-visible metadata.
SDKs must not infer or upload the logged-in user, postal address, contacts, or
advertising/device identifiers. A host can offer a local computer/device-name
suggestion, but must show it and allow editing before using it as cloud-facing
metadata.

The schema contains no invitation, API key, device key, APNs token, printer
endpoint, or document data. Those values use the existing bounded runtime
contracts and secure stores.
