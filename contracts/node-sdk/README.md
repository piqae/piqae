# Node SDK contract fixtures

These allocator-neutral fixtures pin the native SDK and local broker contract.
Bindings may add fields only through a new compatible contract version; they
must not reinterpret a checked-in fixture. Tokens in request fixtures are
synthetic and never represent an enrolled node or application capability.

Broker protocol 1 covers non-sensitive presence and existing authorized
operations. Protocol 2 adds the bounded request/status/exchange consent flow;
the claimed application and signing fields are display evidence only and never
grant access without an explicit node-side decision.
