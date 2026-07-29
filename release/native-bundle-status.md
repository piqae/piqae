# Native bundle status

No signed native release has been published from this repository. The release
workflows can produce signed candidates only when every platform credential is
present; otherwise they emit explicitly named unsigned Preview artifacts.

| Platform | Agent/executor | Shell | Service template | Signed installer |
| --- | --- | --- | --- | --- |
| Linux | Preview | Preview; loopback status | Preview systemd template | No |
| macOS aarch64 | Preview | Preview; authenticated status and native profile capture | Per-user LaunchAgents | Workflow implemented; no signed artifact published |
| macOS x86_64 | Preview | Preview; authenticated status and native profile capture | Per-user LaunchAgents | Workflow implemented; no signed artifact published |
| Windows | Development only | Development; status and native profile capture | Per-user login launcher; no SCM service | Workflow implemented; no signed artifact published |

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
