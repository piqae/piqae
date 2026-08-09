# Native bundle status

The `v0.1.11` GitHub prerelease is the first published signed native release
from this repository. Its universal macOS application and installer are
Developer ID signed, Apple notarised and stapled; the release also includes a
signed Sparkle appcast, checksum, SBOM and repository-bound provenance. These
are Preview artifacts, not evidence that the remaining clean-install, rollback
or physical-printer gates passed.

| Platform | Agent/executor | Shell | Service template | Signed installer |
| --- | --- | --- | --- | --- |
| Linux | Preview; `v0.1.11` x86_64 and aarch64 bundles published with checksums, SBOMs and provenance | Preview; loopback status; no native connect-link consent | Preview systemd template | No |
| macOS aarch64 | Preview | Preview; authenticated status, native profiles, and connect-link consent | Per-user LaunchAgents | `v0.1.11` Developer ID signed, notarised and stapled |
| macOS x86_64 | Preview | Preview; authenticated status, native profiles, and connect-link consent | Per-user LaunchAgents | `v0.1.11` Developer ID signed, notarised and stapled |
| Windows | Disabled for production; preview-only evaluation | Disabled for production; development tray status and native profiles; no native connect-link consent | Per-user login launcher; no SCM service | Unsigned build is available for evaluation; Microsoft Artifact Signing identity validation and signed release evidence remain open |

Every archive includes its platform template/readme, the detailed installation
notes, support matrix, SPDX JSON SBOM, and a SHA-256 sidecar. The checksum
authenticates transfer only when obtained from a trusted release channel; it is
not a code-signing substitute.

CUPS PDF submission maps the V1 options to standard IPP names; RAW submission
ignores rendering options. The Disabled Windows PDF path uses bundled PDFium
rendering and GDI replay against an immutable public/private DEVMODE snapshot.
Physical HP and OKI release matrices, spooler-restart recovery, and long-run
duplicate-handoff evidence have not yet passed, so the path remains Disabled
for production regardless of source completeness.

The recorded publication evidence is GitHub Actions run `31287558654` at
commit `ce309cecd7740a7d8b43bc79aa8ecf67671e6a21` and the `v0.1.11` GitHub
prerelease. The run completed its macOS signing/notarisation, evidence audit,
protected promotion, public-feed smoke checks and Linux bundle jobs. Its
Windows release job was skipped because Windows is Disabled in the support
matrix.
