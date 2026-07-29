# Native bundle status

The native release archives are unsigned source-built artifacts.

| Platform | Agent/executor | Shell | Service template | Signed installer |
| --- | --- | --- | --- | --- |
| Linux | Preview | Preview; loopback status | Preview systemd template | No |
| macOS aarch64 | Preview | Preview; status unavailable | Preview LaunchDaemon template | No |
| macOS x86_64 | Preview | Preview; status unavailable | Preview LaunchDaemon template | No |
| Windows | Development only | Development; status and native profile capture | Per-user login launcher; no SCM service | Unsigned Inno Setup package |

Every archive includes its platform template/readme, the detailed installation
notes, support matrix, SPDX JSON SBOM, and a SHA-256 sidecar. The checksum
authenticates transfer only when obtained from a trusted release channel; it is
not a code-signing substitute.

CUPS PDF submission maps the V1 options to standard IPP names; RAW submission
ignores rendering options. The Disabled Windows PDF path maps only SumatraPDF's
documented pages, copies, colour, collation, duplex, bin, paper, scaling, and
rotation settings. It rejects `dpi`, `media`, and `nup` before handoff instead
of silently ignoring them.
