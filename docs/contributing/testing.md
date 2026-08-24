# Testing

**Status:** implemented automated test workflow; physical certification remains
human-authorized and platform-specific.

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

To reproduce what CI will actually run for this change, including the jobs
`cargo xtask test all` does not cover:

```console
cargo xtask preflight
```

See [Continuous integration](ci.md) for the scope classifier, prerequisites,
and post-deploy verification.

The full command runs formatting, strict Clippy, the Rust workspace tests,
TypeScript checks and tests, and macOS Swift tests when available. It does not
submit a physical print job.

## Test layers

- Unit tests cover parsing, state transitions, serialization, and failure paths.
- Repository tests cover durability, leases, idempotency, and concurrency.
- Executor tests use fake or virtual spoolers by default.
- Compatibility fixtures verify exact API shapes and stable error behavior.
- Configuration-matrix tests exercise the API under every identity provider it
  ships with, because a passing default configuration is not evidence for the
  one production runs.
- Physical certification is a separately recorded, human-authorized hardware
  activity.

### PostgreSQL routing recovery

The cross-node reroute fence has a database-backed integration test. Point it
only at a disposable PostgreSQL database; the test creates and drops its own
random schema and never touches printer executors:

```console
PIQAE_TEST_DATABASE_URL=postgres://postgres:password@127.0.0.1:5432/piqae_test \
  cargo test -p piqae-storage-postgres --test routing_recovery -- --nocapture
```

Without `PIQAE_TEST_DATABASE_URL`, the test reports that database evidence was
skipped so a normal unit-test run does not unexpectedly connect to developer or
production infrastructure. Release evidence must include a non-skipped run.

The test uses two independent connection pools and proves that concurrent
pre-acceptance attempts create one reassignment and one durable
`job_routing_attempts` row. It separately proves that an active lease and a
durable node acceptance each prevent reassignment.

Platform service-account release evidence uses the same disposable database:

```console
PIQAE_TEST_DATABASE_URL=postgres://postgres:password@127.0.0.1:5432/piqae_test \
  cargo test -p piqae-storage-postgres --test platform_service_accounts -- --nocapture

PIQAE_TEST_DATABASE_URL=postgres://postgres:password@127.0.0.1:5432/piqae_test \
  cargo test -p piqae-control-plane --test platform_service_accounts_postgres -- --nocapture
```

Normal contributor checks may omit both PostgreSQL suites. The release-only
wrapper requires the exact routing, grant-lifecycle, and HTTP-auth tests. It
fails closed if any target is
missing, skipped, filtered to zero tests, or unsuccessful:

```console
PIQAE_TEST_DATABASE_URL=postgres://postgres:password@127.0.0.1:5432/piqae_test \
  python3 release/tools/check_postgres_release_tests.py
```

Do not weaken an assertion to make a flaky test pass. Find the nondeterministic
boundary, make time or I/O controllable, and retain the original behavior claim.

## Resetting local fixtures

```console
cargo xtask fixture reset
```

This removes only the repository-local `.piqae-dev` and
`.piqae-test-fixtures` directories. It does not touch installed printers,
operating-system queues, user application data, or databases outside this
checkout.
