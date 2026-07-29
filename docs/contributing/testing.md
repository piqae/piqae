# Testing

Use the narrowest command that gives meaningful evidence:

```console
cargo xtask test changed
```

This inspects committed and working-tree changes, runs Rust package tests for
changed crates, runs the TypeScript checks when JavaScript workspace files
changed, and runs Swift tests when the macOS package changed.

Before requesting review:

```console
cargo xtask test all
```

The full command runs formatting, strict Clippy, the Rust workspace tests,
TypeScript checks and tests, and macOS Swift tests when available. It does not
submit a physical print job.

## Test layers

- Unit tests cover parsing, state transitions, serialization, and failure paths.
- Repository tests cover durability, leases, idempotency, and concurrency.
- Executor tests use fake or virtual spoolers by default.
- Compatibility fixtures verify exact API shapes and stable error behavior.
- Physical certification is a separately recorded, human-authorized hardware
  activity.

Do not weaken an assertion to make a flaky test pass. Find the nondeterministic
boundary, make time or I/O controllable, and retain the original behavior claim.

## Resetting local fixtures

```console
cargo xtask fixture reset
```

This removes only the repository-local `.spool-dev` and
`.spool-test-fixtures` directories. It does not touch installed printers,
operating-system queues, user application data, or databases outside this
checkout.
