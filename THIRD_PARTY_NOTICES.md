# Third-party notices

Piqae depends on third-party software distributed under its own licences.
Apache-2.0 applies only to Piqae-authored material and does not replace those
terms.

Release artifacts must include:

- the dependency licence report produced from the locked Rust and JavaScript
  dependency graphs;
- an SPDX or CycloneDX SBOM;
- notices and source-offer material required by bundled native components;
- PDFium notices in Windows packages that contain PDFium;
- Sparkle and WinSparkle notices in packages that contain those updaters.

The generated report and SBOM for a specific artifact are authoritative because
the exact dependency graph differs by platform and release. Maintainers must
run `cargo xtask release check` before publishing and must not remove a
third-party notice merely because Piqae itself is Apache-2.0.

Embedded native SDK archives use `THIRD_PARTY_LICENSES.json`. The release gate
regenerates it from the locked target-resolved dependency graph and requires
exact bundled licence/attribution text for every reachable third-party package;
it fails closed when package source has no reliable text. Windows NuGet evidence
also includes the exact pinned managed dependency package licence.
