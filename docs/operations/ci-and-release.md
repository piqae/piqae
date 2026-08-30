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
history checks.

Both required workflows also subscribe to GitHub's `merge_group` event. This is
required when merge queue is enabled: otherwise the queue creates a synthetic
merge commit for which the required contexts are never reported. The
classifier uses the merge group's explicit base and head SHAs, so queue runs
remain path-scoped instead of falling back to the full matrix.

The weekly supply-chain workflow performs the slower advisory scan, source
SBOM, complete secret-history scan, and public updater-feed smoke test. PRs scan
only their changed Git history and run dependency policy only when dependency
or workflow files changed.

Dependency license/source policy has one owner: `Supply-chain policy`. Do not
add `cargo deny` back to the ordinary CI workflow. Running it in both workflows
duplicated the tool installation and dependency graph scan without adding an
independent gate. The aggregate `Supply-chain result` carries that result into
branch protection.

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

Start the Blacksmith trial with `PIQAE_CI_LINUX_RUNNER`; Rust, web, and package
verification account for most Linux compute and benefit from colocated caches.
Keep `PIQAE_CI_LIGHT_RUNNER` on GitHub initially because the classifier and
aggregate jobs are only a few seconds long, and move macOS/Windows only after
their runner labels and signing tooling have passed an unsigned candidate run.
Blacksmith transparently accelerates the standard cache actions used here, so
do not replace them with archived provider-specific cache actions.

## Release flow

Shopify follows an independent hosted-app release lane using one public app.
Development uses Shopify CLI previews on a Dev Store; it does not release an
app version. Production is a manual, protected deployment that first requires
Railway `/healthz` to report the exact reviewed commit, then creates and
releases one Shopify app version. Successful
production releases use `shopify-v*` tags and do not trigger the native `v*`
release workflow. See [Shopify release operations](shopify-release.md).

`release/product-release.yaml` is the machine-readable product release set.
It binds the web, API/control plane, SDK, MCP server, desktop node, and Shopify
app to explicit API/node contracts and a single deployment order. Structural
CI validates it before the Shopify component lands; a `v*` product release is
fail-closed unless every required component is present and the tag version
matches the Cargo workspace version.

The release workflow is a dependency graph, not a chain of workflows that
dispatch and poll one another. Reusable workflows return to their direct
caller. Environment approvals protect promotion jobs only; build and audit
jobs must finish before an approval is requested so approval time does not
consume a runner or trigger recursive `workflow_run`/`gh run watch` loops.

The `Piqae release` workflow is the sole `v*` tag trigger:

1. Resolve and validate the version, build number, tag, and membership of the
   source commit in `main`.
2. Build and audit platforms enabled by `release/support-matrix.yaml`; a
   Disabled desktop platform is skipped and cannot block an enabled platform.
   Build the Apple and Windows embedded-node SDK candidates independently of
   the desktop support tier so their ABI/package checks cannot be skipped.
3. Require complete signing credentials for tagged native candidates.
4. Let each selected publisher pause at the protected `native-release`
   environment independently.
5. Publish immutable packages before signed appcasts and promote the shared
   manifest last for each platform.
6. Dereference each platform's public feed before that platform reports
   success.
7. Publish a successful macOS or Windows lane as a narrowly labelled Preview
   prerelease without waiting for unrelated candidates. For `all`, separately
   fail aggregate certification unless every effective selected lane and
   requested publisher succeeds; update the prerelease notes and attach
   candidate-only evidence only after that aggregate passes.

The macOS and enabled Windows GitHub prerelease finalizers use the same
per-tag serialization group and idempotent state marker. Each accepts either
the original draft or an already published prerelease, so either platform can
finish first and one platform's failure cannot strand the successful sibling in
a draft. The aggregate job always records Passed or Failed in that public state
before returning; candidate-only aggregate assets are attached only on success.
Direct `windows-release.yml` dispatch is candidate/unsigned-preview only and
cannot bypass the canonical release workflow's shared core, product contract,
support-tier, source-identity, and ancestry gates.

The platform `v*` release keeps one immutable source identity: server, web,
migration image, native applications, embedded-node SDK candidates, manifest,
and appcasts are all built from the same commit and version by `release.yml`.
Publication completion is recorded per platform rather than inferred from the
slowest sibling; the optional `all` result remains the coordinated
cross-platform certification unit. SDK candidates
include checksums, SPDX SBOMs, and build provenance. They remain unsigned
Preview artifacts and are not pushed to Swift or NuGet registries until their
separate signing and registry-publication gates are enabled. Their clean
consumer gates already resolve the exact staged assets, execute the packaged
native ABI, and verify the managed/native dependency evidence. A consumer
requiring a new API must not be released before that platform tag completes.
The TypeScript SDK and MCP server
remain independently versioned with `sdk-v*` and `mcp-v*`; publish them only
after the compatible platform release and express their minimum supported API
contract in code and release notes. Do not create all three tags concurrently.

Production deployment is a separate protected `production` environment. Keep
required reviewers, prevent self-review, restrict deployment branches/tags,
and store production-only secrets on that environment. Release publication and
production promotion share no workflow-wait polling: approval is represented
by the GitHub environment gate, and deployment serialization by the
`piqae-production-promotion` concurrency group.

The only branch-protection contexts should be `CI result` and
`Supply-chain result`. Requiring leaf jobs or an entire path-filtered workflow
causes permanently pending checks when a job is intentionally skipped. Both
aggregate jobs use `always()` and explicitly reject failed or cancelled selected
jobs, so dependency failures cannot skip the required result.

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
