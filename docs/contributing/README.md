# Contributing

Spool accepts Apache-2.0 contributions with DCO sign-off. Start a deterministic
virtual node and fake printers from a fresh checkout:

```console
git clone https://github.com/C4CoffeeCo/spool.git
cd spool
cargo xtask doctor
cargo xtask dev
```

Before editing, read the root `AGENTS.md` and the nearest scoped instructions.
Normal tests must never reach physical hardware. Update OpenAPI before changing
a public route, use append-only PostgreSQL migrations with cross-tenant tests,
and preserve the distinction between spooler acceptance and physical delivery.

Run focused checks before a small DCO-signed commit:

```console
cargo xtask test changed
git commit -s
```

Open an issue or RFC before making a compatibility, native-profile, protocol,
or deployment decision that other operators must preserve. Pull requests
should include failure-path and restart evidence where behavior crosses a
queue, process, tenant, or network boundary. Never use customer documents as
fixtures.

Detailed contributor references:

- [Development](development.md)
- [Architecture](architecture.md)
- [Testing](testing.md)
- [Releases](releases.md)
- [RFC process](rfc-process.md)
- [Security policy](../../SECURITY.md)
- [Code of conduct](../../CODE_OF_CONDUCT.md)
