# CI and release operations

**Decision:** keep GitHub Actions as the orchestrator and use standard
GitHub-hosted runners by default. Piqae is public, so those runners are free;
Blacksmith is an optional speed/capacity switch, not the default cost-saving
measure. Local machines provide preflight only and never provide published
release bytes.

Credential custody and signing-provider decisions are documented in
[Secrets, signing, and low-cost builds](secrets-and-signing.md).

## Pull-request CI

`release/tools/ci_changed_paths.py` assigns each change to a bounded test
group. Workflow and shared build changes deliberately select every group.

| Change | Expensive validation |
| --- | --- |
| Documentation only | policy checks; no Rust, web, macOS, or Windows build |
| Web or TypeScript | web/SDK only |
| OpenAPI contract | API contract and web/SDK |
| Terraform | Terraform only |
| macOS shell/packaging | combined macOS Rust and Swift job |
| Windows shell/packaging | combined Windows Rust and installer job |
| Shared Rust/workspace | Linux, macOS, and Windows Rust jobs |
| Dependency manifests | applicable build plus dependency policy |

New pushes cancel superseded PR work. Rust caches are restored on PRs but saved
only from `main`, development installers are retained for three days, release
candidates for fourteen days, and the weekly source SBOM for seven days.
Dependabot groups Actions and container updates and limits each ecosystem to
three open pull requests so one weekly dependency burst does not multiply full
CI runs.

The weekly supply-chain workflow performs the slower advisory scan, source
SBOM, complete secret-history scan, and public updater-feed smoke test. PRs scan
only their changed Git history and run dependency policy only when dependency
or workflow files changed.

The existing required-check names remain as compatibility jobs for the first
rollout. After these workflows have one green `main` run, replace the individual
branch-protection contexts with `CI result` and `Supply-chain result`, then
remove the `macOS menu shell` and `Windows development installer` compatibility
jobs. Do not change branch protection before those aggregate contexts exist on
`main`, or every pull request will be blocked.

## Runner provider switch

All runner choices are repository variables. Leaving them unset uses standard
GitHub runners:

| Variable | GitHub default |
| --- | --- |
| `PIQAE_CI_LIGHT_RUNNER` | `ubuntu-latest` |
| `PIQAE_CI_LINUX_RUNNER` | `ubuntu-latest` |
| `PIQAE_CI_MACOS_RUNNER` | `macos-latest` |
| `PIQAE_CI_WINDOWS_RUNNER` | `windows-latest` |
| `PIQAE_RELEASE_LINUX_RUNNER` | `ubuntu-latest` |
| `PIQAE_RELEASE_LINUX_ARM_RUNNER` | `ubuntu-24.04-arm` |
| `PIQAE_RELEASE_MACOS_RUNNER` | `macos-15` |
| `PIQAE_RELEASE_WINDOWS_RUNNER` | `windows-latest` |

After installing the Blacksmith GitHub integration, a controlled trial can set
only the ordinary CI variables, for example:

```console
gh variable set PIQAE_CI_LINUX_RUNNER --body blacksmith-2vcpu-ubuntu-2404
gh variable set PIQAE_CI_WINDOWS_RUNNER --body blacksmith-2vcpu-windows-2025
gh variable set PIQAE_CI_MACOS_RUNNER --body blacksmith-6vcpu-macos-15
```

Measure elapsed time, queue time, failure rate, and actual invoice cost for two
weeks before setting release-runner variables. Reverting is deletion of the
variables; no workflow edit is required. Blacksmith is available only to GitHub
organizations. Its current runner mappings and prices must be rechecked before
enabling it:

- [Blacksmith quickstart and runner labels](https://docs.blacksmith.sh/introduction/quickstart)
- [Blacksmith pricing](https://www.blacksmith.sh/pricing)
- [GitHub Actions billing](https://docs.github.com/en/billing/concepts/product-billing/github-actions)

## Release flow

The `Piqae release` workflow is the sole `v*` tag trigger:

1. Resolve and validate the version, build number, tag, and membership of the
   source commit in `main`.
2. Build and audit macOS, Windows, Linux, and container candidates once.
3. Require complete signing credentials for tagged native candidates.
4. Pause at the protected `native-release` environment.
5. Publish immutable packages before signed appcasts and promote the shared
   manifest last.
6. Dereference the public feeds.
7. Publish the draft GitHub release as a prerelease only after every platform
   stage succeeds.

Desktop nodes discover updates from the signed platform appcasts below
`https://downloads.piqae.com/releases/stable/`. GitHub Releases improves human
discovery but is not an updater feed. macOS currently offers a Sparkle-guided
coordinated app and native-component update. Windows remains preview/disabled
pending its documented release evidence. Neither path may claim Supported
automatic updates until its rollback and certification gates pass.

## Local workflow

Use local compute aggressively for feedback:

```console
cargo xtask doctor
cargo xtask test changed
```

Before a tag, run `cargo xtask release check` with the documented disposable
PostgreSQL test database. Local outputs may be inspected or installed on test
machines, but publication always rebuilds the tag in CI. This prevents an
unreviewed or unprovenanced developer-machine binary from entering an updater
channel.
