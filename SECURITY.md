# Security policy

## Reporting a vulnerability

Do not report a suspected vulnerability in a public issue, discussion, pull
request, chat, or support bundle. Use this repository's GitHub **Report a
vulnerability** form so maintainers can coordinate privately. If private
reporting is unavailable, open a public issue containing only the words
“security contact required”; do not include technical detail.

Include the affected version or commit, platform, impact, the smallest safe
reproduction, and any suggested mitigation. Replace tokens, device identities,
customer metadata, printer profiles, and print documents with synthetic values.
Do not test against infrastructure or devices you do not own or have explicit
permission to assess.

Maintainers will acknowledge a complete report as capacity permits, establish a
private coordination channel, assess supported versions, and credit reporters
who request it. We do not promise a fixed remediation deadline before triage;
severity, exploitability, affected release tiers, and downstream coordination
determine timing.

## Supported versions

The checked-in [`release/support-matrix.yaml`](release/support-matrix.yaml) is
authoritative. A Preview or Disabled platform has not passed every production
release gate. Security fixes are prioritized for the newest supported release;
older, preview, and unreleased builds may require upgrading rather than a
backport.

## Scope and safe research

Good-faith research is welcome when it:

- uses your own tenant, node, printer, and synthetic documents;
- avoids privacy violations, persistence, service disruption, social
  engineering, and destructive or physical-print tests;
- stops after demonstrating the minimum impact; and
- gives maintainers reasonable time to investigate before disclosure.

Printing is a physical side effect. Never use a vulnerability report to send a
job to somebody else's printer or to include a captured customer document.

## Supply-chain response

Release candidates must carry SHA-256 checksums, an SPDX SBOM, and
repository-bound build provenance. If signing credentials, publishing tokens,
or build infrastructure may be compromised, maintainers will stop publishing,
revoke or rotate affected credentials, identify impacted artifacts, and publish
recovery guidance before resuming releases.

No vulnerability, secret, or license-policy exception is silent. A temporary
exception must name the advisory or finding, document its risk and owner, and
have a review date.
