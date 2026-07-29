# Contributing to Spool

Thank you for helping build reliable, open printing infrastructure. Spool is
licensed under Apache-2.0, and contributions are accepted under the same terms.
Every commit must include a
[Developer Certificate of Origin](https://developercertificate.org/) sign-off:

```console
git commit --signoff
```

## Start here

Install the pinned toolchain with [mise](https://mise.jdx.dev/) or install the
versions in `mise.toml` yourself. Docker is optional for agent and UI work but
required for the complete self-hosted stack.

```console
mise install
cargo xtask doctor
pnpm install --frozen-lockfile
cargo xtask test changed
```

See [development](docs/contributing/development.md),
[testing](docs/contributing/testing.md), and
[releases](docs/contributing/releases.md) for the detailed workflows.

## Contribution expectations

1. Open an issue or discussion before substantial behavior, protocol, data
   model, or compatibility changes.
2. Keep pull requests focused and explain the observable user outcome.
3. Add tests at the narrowest useful layer and run `cargo xtask test changed`.
4. Preserve backward compatibility within the documented support window.
5. Never include customer documents, credentials, private keys, enrollment
   tokens, or production logs in issues, commits, fixtures, or support bundles.
6. Treat physical printing as a side effect. Automated and contributor tests
   use fake or virtual printers unless a human explicitly opts into a named
   physical device.
7. Update user-facing documentation when behavior or support claims change.

## Pull requests

PRs should state what changed, why, how it was tested, and any platform or
printer evidence. A successful build is not evidence of physical printing.
Keep support claims aligned with `release/support-matrix.yaml`.

Maintainers may ask for smaller commits, additional failure-path tests, or
hardware evidence before merging platform-specific changes.

See `GOVERNANCE.md`, `CODE_OF_CONDUCT.md`, `SECURITY.md`, `DCO`, and
`AGENTS.md`.
