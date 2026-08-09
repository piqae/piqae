# Piqae MCP server

`@piqae/mcp-server` gives coding agents a standards-based, typed interface to
Piqae's checked-in API and operational knowledge. It is a thin adapter over the
public Piqae API and TypeScript SDK: it does not connect to PostgreSQL, invent a
second authorization model, or bypass tenant and scope checks.

The optional `piqae_documents` tool validates and operates bounded
`piqae.document/v1` templates. It does not return template inputs or document
content, accept credentials as tool arguments, or bypass the existing explicit
authorization context. Printing a completed render remains behind the normal
job-submission policy and exact destination confirmation.

It exposes MCP tools for deployment/workspace context, API keys, nodes and node
onboarding, printers, stocks, targets and profile bindings, uploads, jobs,
webhooks, billing/usage, and platform/integrator accounts. It also exposes the
OpenAPI contract, SDK guide, authentication guide, lifecycle guidance, a docs
search tool, and an operator prompt.

For copy-ready setup for Codex, Claude Code, VS Code/Copilot, native Windows,
platform customer contexts, and a first safe tool session, see
[`docs/api/mcp-installation.md`](../../docs/api/mcp-installation.md).

## Local stdio

Use a separate, least-privilege key for each agent and environment:

```json
{
  "mcpServers": {
    "piqae": {
      "command": "npx",
      "args": ["-y", "@piqae/mcp-server", "--stdio"],
      "env": {
        "PIQAE_API_ORIGIN": "https://api.piqae.com",
        "PIQAE_API_KEY": "${PIQAE_API_KEY}",
        "PIQAE_MCP_SECRET_DIRECTORY": "/absolute/private/path/piqae-agent-secrets"
      }
    }
  }
}
```

The secret directory must be a real, non-symlink directory with mode `0700`.
One-time API-key, webhook, enrolment, and platform credentials are written to a
new `0600` JSON file by default. Returning a secret through the MCP/model
transcript requires both `PIQAE_MCP_ALLOW_SECRET_OUTPUT=true` on the server and
`delivery=response` on that specific tool call.

Platform service-account credentials also require an explicit account context:

```text
PIQAE_PLATFORM_KEY=piq_platform_...
PIQAE_WORKSPACE_ID=wsp_...
PIQAE_ENVIRONMENT_ID=env_...
```

Individual tool calls may provide `workspace_id` and `environment_id` instead.
Tenant credentials are rejected if they try to select a different tenant.

## Remote Streamable HTTP and OAuth

Remote mode uses stateless Streamable HTTP. It validates `Host` and `Origin`,
requires HTTPS off loopback, verifies each bearer against Piqae, publishes OAuth
Protected Resource Metadata (RFC 9728), and requires the OAuth JWT audience to
contain the exact MCP resource URL.

```text
PIQAE_API_ORIGIN=https://api.piqae.com
PIQAE_MCP_BIND_HOST=127.0.0.1
PIQAE_MCP_PORT=39300
PIQAE_MCP_PUBLIC_URL=https://api.example.com/mcp
PIQAE_MCP_AUTHORIZATION_SERVER=https://identity.example.com
piqae-mcp --http
```

The authorization server must publish RFC 8414/OIDC metadata, use Authorization
Code with PKCE for public clients, and issue an access token that is both:

- accepted by the configured Piqae control plane; and
- audience-bound to the exact `PIQAE_MCP_PUBLIC_URL` resource.

This server is only a resource server. It deliberately does not implement a
home-grown authorization server, dynamic registration database, token exchange,
or refresh-token store. Put those responsibilities in the selected standards-
compliant identity provider. The MCP authorization requirements are documented
in the [MCP authorization specification](https://modelcontextprotocol.io/specification/2025-11-25/basic/authorization),
and remote clients should use
[Streamable HTTP](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports).

Loopback HTTP can use a manually configured Piqae bearer without OAuth metadata,
but stdio is simpler and has a smaller attack surface for local coding agents.

## Safety policy

Job submission is disabled by default:

```text
PIQAE_MCP_JOB_SUBMISSION=disabled   # default
PIQAE_MCP_JOB_SUBMISSION=test_only  # accepts only piq_test_/spl_test_ keys
PIQAE_MCP_JOB_SUBMISSION=all        # explicit operator decision
```

Every job create still requires an idempotency key, an exact destination-ID
confirmation, and a named fixture. Cancellation, node/connector revocation,
target unbinding, webhook removal, rollback, key revocation, and account archive
require an exact identifier confirmation. A successful job response means
durable registration, not physical delivery.

The MCP never sends binary document content through a model. It can create
upload metadata, after which trusted application code uploads bytes directly
with the returned URL and headers. Device private keys remain node-owned.

## Development

```console
pnpm --filter @piqae/sdk build
pnpm --filter @piqae/mcp-server check
pnpm --filter @piqae/mcp-server test
pnpm --filter @piqae/mcp-server build
```

The package build copies the authoritative OpenAPI and selected checked-in
guides into `dist/knowledge`, so installed agents receive the same bounded
knowledge resources when they are not running inside a Piqae checkout.
