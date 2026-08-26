# Node SDK contract fixtures

These allocator-neutral fixtures pin the native SDK and local broker contract.
Bindings may add fields only through a new compatible contract version; they
must not reinterpret a checked-in fixture. Tokens in request fixtures are
synthetic and never represent an enrolled node or application capability.
