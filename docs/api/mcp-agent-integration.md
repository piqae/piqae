# MCP and coding-agent integration

Piqae includes `@piqae/mcp-server`, a Model Context Protocol resource server
for coding agents and operator assistants. It exposes the same public API and
authorization boundaries as `@piqae/sdk`; it is not a privileged database
administration path.

The supported local transport is stdio. The supported remote transport is
stateless Streamable HTTP. Legacy HTTP+SSE is intentionally not added. Remote
deployments publish RFC 9728 protected-resource metadata and delegate OAuth to
an operator-selected authorization server rather than storing OAuth clients,
authorization codes, refresh tokens, or consent state inside Piqae MCP.

See [Install and use the Piqae MCP server](mcp-installation.md) for copy-ready
Codex, Claude Code, VS Code/Copilot, Windows and generic stdio configurations,
authentication choices, first-use verification, and troubleshooting. See
[`apps/mcp/README.md`](../../apps/mcp/README.md) for remote OAuth deployment,
tool coverage, secret delivery, and job-safety policy.

## Authorization boundary

Every MCP operation calls the normal Piqae HTTP API. The control plane remains
authoritative for credential verification, workspace/environment isolation,
scope checks, revocation, request IDs, and audit records. A platform credential
must include an exact workspace/environment selection; an ordinary tenant key
cannot use those headers.

Remote OAuth access tokens must be accepted by the Piqae API and contain the
exact MCP resource URL in `aud`. The MCP validates the audience after the Piqae
control plane has validated the token signature, issuer, application binding,
tenant, membership, and permissions. This prevents a bearer issued only for a
different resource from being replayed at the MCP endpoint.

For local stdio, configure a separate least-privilege Piqae API key in the MCP
process environment. Never put a key in MCP arguments, URLs, repository config,
tool inputs, print metadata, or logs.

## One-time credentials

Creating API keys, webhooks, node enrolments, connect sessions, or platform mode
can return a one-time capability. By default the MCP writes it to a new `0600`
file under a preconfigured `0700` non-symlink directory. The tool result contains
only the path and non-secret metadata. Transcript delivery is disabled unless
the server operator and the individual call both opt in.

This makes MCP-assisted developer setup possible without making the normal
agent conversation a secret store. Move the resulting value into the target
environment's secret manager, verify the integration, then remove the staging
file. Revoke abandoned credentials through the normal API.

## Capability truth

The MCP exposes operations implemented by the current public API. It does not
claim unsupported member mutation, credential recovery, unarchive, physical
delivery proof, or production-ready platform/node support. The checked-in
support matrix remains authoritative.
