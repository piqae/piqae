# Package publishing

Piqae has three distribution channels:

| Channel | Names | Release trigger |
| --- | --- | --- |
| npm | `@piqae/sdk` | `sdk-vX.Y.Z` tag |
| GitHub Container Registry | `ghcr.io/piqae/piqae/server`, `ghcr.io/piqae/piqae/migrate`, `ghcr.io/piqae/piqae/web` | `vX.Y.Z` tag |
| GitHub Releases | Piqae native archives, installers, checksums, and provenance | `vX.Y.Z` tag |

The SDK workflow verifies generated OpenAPI types, TypeScript, tests, package
exports, and a clean-consumer install before publishing. npm publication uses
GitHub Actions trusted publishing and provenance; no long-lived npm token
belongs in the repository.

The preview release workflow publishes the three container images. The
platform-specific native workflows attach only signed Windows assets and
signed-and-notarised macOS assets to a **draft** GitHub Release. A maintainer
must review the support matrix and physical-printer evidence before making a
release public. Unsigned cross-platform preview bundles remain workflow
artifacts and never become GitHub Release assets.

## One-time setup

1. Verify the canonical repository is `https://github.com/piqae/piqae`.
2. Make the repository public only after the full-history secret scan passes.
3. Reserve the `@piqae` npm organisation and configure a trusted publisher for
   package `@piqae/sdk`, repository `piqae/piqae`, workflow
   `.github/workflows/sdk-release.yml`, and the release environment if one is
   later added.
4. Run the SDK workflow manually. It produces a tarball but does not publish
   without a tag.
5. Push `sdk-v0.1.0` for the first npm release.
6. Push `v0.1.0` only when the native/container preview candidate is intended.
7. Mark the three GHCR packages public after their first successful publish.

Both publishing workflows reject release attempts from any repository other
than `piqae/piqae`. This keeps package provenance bound to the final project
identity.

## Version checks

The SDK tag must equal `sdk-v` followed by the version in
`sdk/typescript/package.json`. Native/container versions use the `vX.Y.Z` tag
and remain governed by the release gates and support matrix.

Before creating either tag, run:

```console
cargo xtask release check
pnpm --filter @piqae/sdk generate:check
pnpm --filter @piqae/sdk check
pnpm --filter @piqae/sdk test
pnpm --filter @piqae/sdk build
pnpm --filter @piqae/sdk lint
pnpm --filter @piqae/sdk smoke:package
```

## Namespace cutover

This repository uses only the Piqae namespace. Credentials use `piq_*`,
headers use `X-Piqae-*`, configuration uses `PIQAE_*`, and native files and
services use `piqae-*`. Existing installations using different product
identifiers are not read or upgraded automatically; operators must enrol a
new node or deliberately move data and configuration into the documented
Piqae paths.
