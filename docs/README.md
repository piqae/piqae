# Spool documentation

Spool is an open-source, self-hostable print control plane and local print-node
agent. Start with the journey matching your job:

- [Cloud evaluation](getting-started/cloud.md)
- [Self-hosted Docker Compose](getting-started/self-hosted-compose.md)
- [Self-hosted Kubernetes](getting-started/self-hosted-kubernetes.md)
- [Local-only node](getting-started/local-only.md)
- [Developer setup](getting-started/development.md)

## Status language

Every operational page uses these terms literally:

- **Implemented**: code exists in this repository.
- **Tested**: the named automated or physical test has actually run.
- **Preview**: usable for evaluation, but release gates remain.
- **Supported**: covered by a published stable support tier.
- **Disabled**: built or documented for development only; not a production
  release claim.
- **Planned**: design only.

No native platform is currently a stable Supported release. The authoritative
release tiers are in
[`release/support-matrix.yaml`](../release/support-matrix.yaml) and
[`release/native-bundle-status.md`](../release/native-bundle-status.md).

## Raw Markdown URL convention

Documentation links use repository-relative paths with the literal `.md`
extension. This keeps links usable in GitHub, source archives, raw Markdown
readers, and generated `llms-full.txt`. Do not use documentation-site routes
such as `/docs/nodes/macos`; write `../nodes/macos.md`. External links use full
HTTPS URLs. Never place credentials, signed URLs, native profile blobs, or
unredacted support bundles in a URL.

The numbered documents remain the product and architecture record. Journey
pages explain how to operate what is currently real and link back to those
authoritative specifications.
