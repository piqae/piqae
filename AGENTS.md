# Piqae agent guide

This file applies to the entire repository. More specific `AGENTS.md` files may
narrow these rules but must not weaken safety or support-claim requirements.

## Product invariants

- Piqae is Apache-2.0 open-source printing infrastructure.
- The durable agent owns identity, queueing, recovery, and cloud synchronization.
  Tray/menu shells are thin and disposable.
- Installed operating-system drivers remain the authority for vendor options.
- A spooler handoff is not proof that ink reached paper. Preserve the distinction
  between accepted, printing, reported complete, and uncertain delivery.
- Local-first and self-hosted operation must remain first-class.

## Before editing

1. Read the nearest documentation and manifests for the area.
2. Inspect `git status`; preserve unrelated user and agent work.
3. Prefer a small reversible change with an observable test.
4. For external API, protocol, database, or printer-profile changes, identify
   compatibility and migration implications before implementation.

## Commands

```console
cargo xtask doctor
cargo xtask test changed
cargo xtask test all
cargo xtask preflight
cargo xtask release check
```

`cargo xtask preflight` reproduces the CI jobs the current change selects, using
the same classifier as the `Select CI scope` job. Run it before opening a pull
request. It names every missing prerequisite before it spends time and never
reports a pass for a job it could not run; `--list` shows the plan and `--all`
runs every scope.

`cargo xtask preflight` (Shopify scope) and `cargo xtask release check` require
a disposable PostgreSQL database in `PIQAE_TEST_DATABASE_URL`. Without it
`release check` fails immediately. See `docs/contributing/ci.md`.

Use `cargo xtask dev` for the demo dashboard or `cargo xtask dev agent` for the
fake local printer path. `cargo xtask fixture reset` removes only disposable
repository-local state.

Runner capacity is a repository variable, never a workflow edit: every
`runs-on:` resolves through a `vars.PIQAE_*_RUNNER` indirection, and
`release/tools/check_workflow_runners.py` enforces it.

## Safety

- Never print to physical hardware without explicit user authorization naming
  the printer and expected fixture. Normal tests must use fake or virtual
  printers.
- Never set `PIQAE_ALLOW_PHYSICAL_TESTS=1` on the user's behalf unless that
  authorization is present in the active request.
- Never expose enrollment tokens, API keys, device keys, print documents, local
  bearer tokens, or unredacted production logs.
- Do not silently fall back from a pinned native profile to generic printing.
- Do not delete queues, profiles, state directories, or databases unless the
  exact destructive scope was requested and verified.

## Code quality

- Rust uses stable 1.88, edition 2024, rustfmt, and workspace Clippy lints.
- Avoid `unwrap`/`expect` in production code. Bound files, responses, waits,
  subprocesses, and queues.
- Keep platform-specific unsafe code isolated, documented, and fail-closed.
- TypeScript must pass the workspace checks and tests.
- Add failure-path and restart/durability coverage when behavior crosses a queue
  or process boundary.
- Behavior that varies by deployment configuration must be tested in every
  configuration it ships in, not only the default one. A handler that depends on
  the identity provider, the deployment kind, or an optional service belongs in
  `crates/control-plane/tests/identity_provider_matrix.rs` or an equivalent
  matrix. One passing configuration is not evidence for another.
- Use conventional, focused commits with DCO sign-off.

## Documentation and support truth

Update documentation with behavior. Do not call a platform production-ready
because it compiles or because a virtual spooler accepted a job. The checked-in
support matrix is authoritative. Keep legacy compatibility claims scoped to
tested endpoints and response behavior.
