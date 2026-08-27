# Node SDK contract fixtures

These allocator-neutral fixtures pin the native SDK and local broker contract.
Bindings may add fields only through a new compatible contract version; they
must not reinterpret a checked-in fixture. Tokens in request fixtures are
synthetic and never represent an enrolled node or application capability.

Broker protocol 1 covers non-sensitive presence and existing authorized
operations. Protocol 2 added the bounded request/status/exchange consent flow.
Protocol 4 authorization requests omit application identity entirely: the node
derives the process identity from the accepted operating-system transport and
shows only that verified principal in consent UI. Legacy claimed identities are
accepted only when they exactly match that evidence; they never grant access.

Protocol 3 adds the durable embedded-adapter SDK operations. A host must pull a
persisted operation, acknowledge `begin_adapter_handoff` before invoking the
native print API, and then report either authoritative acceptance followed by a
terminal result, a proven pre-handoff rejection, or an ambiguous handoff. The
same unresolved operation and fence are replayed after restart; an operation in
`handoff_started` or `accepted` must never be submitted to the native API again.

Protocol 4 removes bearer capability tokens from secret-bearing IPC requests.
Rust derives a proof key from the one-time credential, authenticates each
request and response with domain-separated HMAC-SHA256, and durably rejects
fresh-nonce replay. Platform bindings call the Rust broker client and must not
reimplement JSON canonicalization. Protocol 1 presence remains intentionally
non-secret; protocol 3 bearer execution is rejected by current brokers.

The `v1/adapter-*.json` fixtures are allocator-neutral `piqae_node_command`
payloads. Paths, identifiers, and document bytes are synthetic.

`reconcile-cloud-request.json` and `reconcile-cloud-poll.json` pin the
nonblocking generation-fenced cloud-reconciliation ABI used by Swift and .NET.
The poll result contains aggregate counts, success scope, retryability, and a
privacy-safe failure class only; it never contains connector or tenant identity.

`printpacket-validate.json` and `printpacket-enqueue.json` pin the vendor-neutral
`printpacket/v1` direct/offline path. Rust validates and renders before entering
the existing durable embedded queue; bindings never create another renderer
queue. The v1 reference target is deterministic PDF. A `printer_native` target
fails closed unless a later registered language/profile capability matches it
exactly.
