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
| Web | web only |
| TypeScript SDK | SDK plus downstream MCP and Shopify consumers |
| MCP | MCP only |
| Shopify app | Shopify checks, build, and PostgreSQL migration gate |
| OpenAPI contract | API contract and all web/SDK/MCP/Shopify consumers |
| Terraform | Terraform only |
| macOS shell/packaging | macOS Swift or packaging validation only |
| Windows shell/packaging | Windows tray or installer validation only |
| Shared native Rust | Linux plus affected macOS and Windows Rust jobs |
| Server/database Rust | Linux/server Rust only |
| Dependency manifests | applicable build plus dependency policy |

New pushes cancel superseded PR work. Rust caches are restored on PRs but saved
only from `main`, development installers are retained for three days, release
candidates for fourteen days, and the weekly source SBOM for seven days.
Dependabot groups Actions and container updates and limits each ecosystem to
three open pull requests so one weekly dependency burst does not multiply full
CI runs.

The supply-chain workflow is the sole owner of dependency policy and secret
history checks. Its weekly run also performs the slower advisory scan, source
SBOM, complete secret-history scan, and public updater-feed smoke test. PRs scan
only their changed Git history and run dependency policy only when dependency
or workflow files changed.

Only `CI result` and `Supply-chain result` should be required after the first
green `main` run containing the aggregate jobs. Individual platform jobs are
selected by the dependency-aware classifier and must not be required contexts,
because an intentionally unselected job reports `skipped`.

Changes to `.github/workflows/ci.yml` or the classifier and its tests exercise
every scope. Other workflow changes select release tooling plus the affected
platform or package, not unrelated application builds. A weekly Monday run
forces the full matrix so a path-classification mistake cannot hide platform
drift indefinitely.

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

Do not switch every runner class at once. A Blacksmith pilot should change only
`PIQAE_CI_LINUX_RUNNER`; lightweight policy jobs and the macOS, Windows, and
release jobs remain on GitHub-hosted runners. This isolates the provider change
to the compute-heavy Linux CI jobs and leaves a useful control group.

Before installing the integration or changing a variable:

1. Record 20--30 recent, completed pull-request runs of `CI` from the Actions
   UI or API. For each run, record the commit, conclusion, queue duration
   (`started_at - created_at`), Linux job duration
   (`completed_at - started_at`), and whether a rerun was needed.
2. Record GitHub-hosted runner cost for the same period. Public-repository
   usage may have no marginal Actions charge, but it is still the baseline for
   evaluating a paid provider.
3. Choose a current Blacksmith Ubuntu label with the same architecture and at
   least the resources required by the Linux jobs. Confirm the label in the
   Blacksmith installation rather than copying an example from this document.
4. Set an end date (normally two weeks or 20--30 comparable runs) and an owner
   responsible for rollback.

Repository variables are the narrowest scope and are preferred for the pilot.
After installing the Blacksmith GitHub integration, set the selected label:

```console
gh variable set PIQAE_CI_LINUX_RUNNER --repo OWNER/REPOSITORY --body SELECTED_RUNNER_LABEL
```

If centrally managed organization variables are required instead, restrict the
variable to this repository during the pilot; do not expose it to every
repository by default:

```console
gh variable set PIQAE_CI_LINUX_RUNNER --org ORGANIZATION \
  --repos REPOSITORY --body SELECTED_RUNNER_LABEL
```

Run the same measurement for the experiment window and compare medians and
95th percentiles, not only the fastest run. Review queue duration, Linux job
duration, end-to-end workflow duration, provider/infrastructure failure rate,
cache hit behavior, rerun rate, and actual invoice cost. Separate failures
caused by the tested commit from runner or network failures. Accept the pilot
only if the improvement is repeatable and the reliability and cost are
acceptable; document the measured result before considering other runner
classes or release jobs.

Rollback is immediate and does not require a workflow edit. Delete the variable
at the same scope where it was created, then rerun one failed or representative
workflow to prove that GitHub's fallback is active:

```console
gh variable delete PIQAE_CI_LINUX_RUNNER --repo OWNER/REPOSITORY
# For an organization-scoped pilot:
gh variable delete PIQAE_CI_LINUX_RUNNER --org ORGANIZATION
```

The workflow expression falls back to `ubuntu-latest` when the variable is
absent. Do not set the variable to an empty string as a rollback mechanism.
Blacksmith is available only to GitHub organizations. Its supported runner
labels, security model, fork behavior, and prices must be rechecked before
enabling it:

- [Blacksmith quickstart and runner labels](https://docs.blacksmith.sh/introduction/quickstart)
- [Blacksmith pricing](https://www.blacksmith.sh/pricing)
- [GitHub Actions billing](https://docs.github.com/en/billing/concepts/product-billing/github-actions)

## Release flow

Shopify follows an independent hosted-app release lane. A successful `main` CI
run deploys the exact reviewed commit to Shopify staging. Production is a
manual, protected deployment that first requires Railway `/healthz` to report
the same commit, then creates and releases one Shopify app version. Successful
production releases use `shopify-v*` tags and do not trigger the native `v*`
release workflow. See [Shopify release operations](shopify-release.md).

The `Piqae release` workflow is the sole `v*` tag trigger:

1. Resolve and validate the version, build number, tag, and membership of the
   source commit in `main`.
2. Build and audit platforms enabled by `release/support-matrix.yaml`; a
   Disabled platform is skipped and cannot block an enabled platform.
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
