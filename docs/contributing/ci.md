# Continuous integration

**Status:** implemented. Runner capacity, deployment origins, and the
identity-provider matrix are configuration; the gates themselves are code.

## Reproduce CI before opening a pull request

```console
cargo xtask preflight
```

`preflight` classifies the change with `release/tools/ci_changed_paths.py` —
the same script the `Select CI scope` job runs — and then executes the CI jobs
that scope selects, with the same commands and the same flags. Because the
scope comes from one shared classifier, local and CI selection cannot drift.

```console
cargo xtask preflight --list   # show the plan without running it
cargo xtask preflight --all    # every scope, as a scheduled run would
```

It refuses to imply coverage it did not give:

- A missing prerequisite is named **before** any time is spent, with the
  command that fixes it. Everything else still runs, and `preflight` exits
  non-zero listing what it could not verify rather than quietly skipping it.
- A job that cannot run on this operating system is reported as skipped and
  attributed to the CI job that does cover it.
- Jobs that only exist to produce release artifacts are listed as `ci-only`
  rather than silently dropped.
- The changed-history Gitleaks scan is also listed as `ci-only`: reproducing it
  faithfully depends on GitHub's event base/head boundary. Local preflight
  still runs the release policy and dependency checks; GitHub remains the
  authority for the bounded secret-history result.

### PostgreSQL

`CI / Rust (PostgreSQL evidence)`, `CI / Shopify`, and
`cargo xtask release check` need a disposable PostgreSQL database.

Every database-backed Rust suite answers `skipped:` and reports a pass when
`PIQAE_TEST_DATABASE_URL` is unset, so an ordinary `cargo test` run proves
nothing about schema upgrades, the WorkOS identity projection, cloud billing,
cross-node reroute fencing, or platform-account authorization. CI now runs them
through `release/tools/check_postgres_release_tests.py`, which fails closed when
a required test is missing, skipped, filtered to zero tests, or unsuccessful.
Add a gate there when you add a boundary that only exists against a real
database.

A missing database is the most common opaque local failure, so `preflight` and
`release check` both name it with the command that fixes it:

```console
docker run --rm -d -p 5432:5432 -e POSTGRES_PASSWORD=postgres \
  -e POSTGRES_DB=piqae_test --name piqae-preflight-db postgres:16
export PIQAE_TEST_DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5432/piqae_test
```

Point it only at a database you can afford to lose. The suites create and drop
their own schemas.

## Configuration coverage

Piqae ships one binary that runs under several identity providers:
`local_owner` for self-hosted installs, `workos` or generic OIDC for the
managed cloud. A suite that only ever builds one of them cannot see a handler
that reaches for provider-specific state — that handler passes every check and
then answers `503` in the deployment it was never exercised in.

`crates/control-plane/tests/identity_provider_matrix.rs` builds the real router
once per provider configuration and asserts the provider-independent contract
in each. The route list is parsed from the router source, so a new identity
route must either enter the matrix or be declared local-owner-only under
`/v1/identity/local/`.

When you add a route that behaves differently by deployment, add the
configuration to that matrix in the same change.

## Runner capacity is a repository variable, never a workflow edit

Every `runs-on:` resolves through a `vars.PIQAE_*_RUNNER` variable with a
GitHub-hosted default. Changing the runner fleet is a variable change: no pull
request, no review cycle, and an instant revert by clearing the variable.
`release/tools/check_workflow_runners.py` fails the supply-chain policy job if
a job ever hardcodes a runner label.

| Variable | Default | Covers |
| --- | --- | --- |
| `PIQAE_CI_RUST_RUNNER` | falls back to `PIQAE_CI_LINUX_RUNNER` | `Rust (ubuntu-latest)` and its `otlp` sibling |
| `PIQAE_CI_LINUX_RUNNER` | `ubuntu-latest` | Web, SDK, MCP, Shopify, Rust dependency policy |
| `PIQAE_CI_LIGHT_RUNNER` | `ubuntu-latest` | scope selection, policy, result, contract, Terraform |
| `PIQAE_CI_MACOS_RUNNER` | `macos-latest` | `Rust (macos-latest)`, macOS menu shell |
| `PIQAE_CI_WINDOWS_RUNNER` | `windows-latest` | `Rust (windows-latest)` |
| `PIQAE_RELEASE_LINUX_RUNNER` | `ubuntu-latest` | release container, bundle, and evidence jobs |
| `PIQAE_RELEASE_LINUX_ARM_RUNNER` | `ubuntu-24.04-arm` | aarch64 release bundles |
| `PIQAE_RELEASE_MACOS_RUNNER` | `macos-15` | **signing and notarization** |
| `PIQAE_RELEASE_WINDOWS_RUNNER` | `windows-latest` | **signing** |

The production promotion job runs on a protected `[self-hosted,
piqae-production]` pool on purpose and is deliberately not relocatable by a
variable.

Do not move the signing runners. Code-signing and notarization depend on the
platform vendors' own toolchains and on secrets scoped to a protected
environment; relocating them changes the trust boundary of a release.

### Third-party runners

`PIQAE_CI_RUST_RUNNER` exists so the compile-bound Rust jobs can be sized
without also buying cores for the install-bound Node jobs, which would spend
budget where it does not convert into wall-clock.

This repository is public, so GitHub-hosted standard runners are free. Any
third-party runner is therefore net-new spend bought purely for developer
wall-clock time, not a saving. Measure before and after with the job durations
in the Actions UI, and clear the variable if the change does not pay for
itself.

To move the Rust jobs onto Blacksmith, install its GitHub App first — a runner
label with no fleet behind it leaves jobs queued indefinitely — and then set
one variable:

```console
gh variable set PIQAE_CI_RUST_RUNNER --body blacksmith-4vcpu-ubuntu-2404
gh variable delete PIQAE_CI_RUST_RUNNER   # revert
```

Start at 4 vCPU. Blacksmith bills free and paid minutes in proportion to vCPU
count, so a larger runner has to convert the extra cores into proportionally
less wall-clock to break even, and `rustc` does not scale linearly. Keep
`Swatinem/rust-cache` as it is: Blacksmith's own fork of it was archived, and
its colocated cache accelerates the upstream action with no workflow change.

Leave the light, macOS, Windows, and release runner variables unset. Those jobs
are install-bound or on the signing path, where a third-party runner buys
little and risks more.

## Post-deploy verification

A health probe that answers `200` proves a process started. It does not prove
the reviewed commit is live, and it does not prove the endpoints work in the
deployment's own configuration.

Both the control plane and the web application report `service` and `revision`
in their health documents, where `revision` is the full commit the build came
from, or `unknown`. `release/tools/post_deploy_smoke.py` asserts the origin is
serving the expected service at the expected commit, and then exercises real
endpoints from `release/post-deploy-probes.json`:

- endpoints that require authentication must answer `401` or `403`; a `5xx`
  means the route is deployed but structurally unavailable in this
  configuration, and a `2xx` would be an authentication bypass;
- public endpoints must answer `200`.

```console
python3 release/tools/post_deploy_smoke.py \
  --origin https://api.example.com \
  --service piqae-control-plane \
  --revision "$(git rev-parse HEAD)"
```

The `Post-deploy smoke` workflow runs this on demand or from another workflow
with a revision, and on a schedule without one as a standing production
surface check. Set `PIQAE_API_ORIGIN` and `PIQAE_WEB_ORIGIN` to enable it; the
scheduled check reports and skips when an origin is not configured.

The probe list is representative, not exhaustive, and no probe is
authenticated: it proves routes are reachable and configured, not that their
responses are correct. `identity_provider_matrix.rs` asserts the same
expectations against the real router, so the deploy gate cannot drift into
being either a false alarm or a rubber stamp.
