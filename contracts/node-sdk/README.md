# Node SDK contract fixtures

These allocator-neutral fixtures pin the native SDK and local broker contract.
Bindings may add fields only through a new compatible contract version; they
must not reinterpret a checked-in fixture. Tokens in request fixtures are
synthetic and never represent an enrolled node or application capability.

Broker protocol 1 covers non-sensitive presence and existing authorized
operations. Protocol 2 adds the bounded request/status/exchange consent flow;
the claimed application and signing fields are display evidence only and never
grant access without an explicit node-side decision.

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
