# Install and use the Piqae MCP server

`@piqae/mcp-server` lets an MCP-compatible agent inspect and operate Piqae
through the same public API, tenant isolation, scopes, and audit trail as the
TypeScript SDK. It is not a privileged database connection.

## Choose an authentication model

| Use case                                            | Credential                                                             | Required context                                                        |
| --------------------------------------------------- | ---------------------------------------------------------------------- | ----------------------------------------------------------------------- |
| One Piqae workspace and environment                 | Dedicated `piq_test_...` or `piq_live_...` API key                     | None; the verified key owns its tenant                                  |
| Integrator operating one customer account at a time | Server-only `piq_platform_...` credential                              | Exact `PIQAE_WORKSPACE_ID` and `PIQAE_ENVIRONMENT_ID` for that customer |
| Centrally hosted remote MCP                         | OAuth access token accepted by Piqae and audience-bound to the MCP URL | Selected by the authenticated token/grants                              |

For local agents, create a separate least-privilege key under **Dashboard →
Settings → API keys**. Start with read scopes for discovery. Add mutation scopes
only for tools the agent must use. Never reuse a platform production credential
as a general developer key.

The examples below read `PIQAE_API_KEY` from the agent process environment. Set
it in the operating system, agent secret store, or a private environment file;
do not commit it, paste it into prompts, or put it in command arguments.

Prerequisites are Node.js 20 or newer and an MCP client that supports stdio.
`npx -y` downloads the published package on first use and then runs it over
standard input/output.

## Codex CLI, IDE extension, and ChatGPT desktop

Codex clients on the same host share `~/.codex/config.toml`. Use `env_vars` to
forward an existing environment variable without writing its value into the
configuration:

```toml
[mcp_servers.piqae]
command = "npx"
args = ["-y", "@piqae/mcp-server", "--stdio"]
env_vars = ["PIQAE_API_KEY"]
required = true
default_tools_approval_mode = "writes"

[mcp_servers.piqae.env]
PIQAE_API_ORIGIN = "https://api.piqae.com"
PIQAE_MCP_JOB_SUBMISSION = "disabled"
PIQAE_MCP_SECRET_DIRECTORY = "/absolute/private/path/piqae-agent-secrets"
```

Restart the client, then run `codex mcp list` or use `/mcp`. The desktop and IDE
settings also provide **MCP servers → Add server** for the same stdio command.

## Claude Code

Create a private wrapper environment or configure `.mcp.json` with environment
expansion. This keeps the secret value outside the checked-in file:

```json
{
  "mcpServers": {
    "piqae": {
      "type": "stdio",
      "command": "npx",
      "args": ["-y", "@piqae/mcp-server", "--stdio"],
      "env": {
        "PIQAE_API_ORIGIN": "https://api.piqae.com",
        "PIQAE_API_KEY": "${PIQAE_API_KEY}",
        "PIQAE_MCP_JOB_SUBMISSION": "disabled",
        "PIQAE_MCP_SECRET_DIRECTORY": "${PIQAE_MCP_SECRET_DIRECTORY}"
      }
    }
  }
}
```

Use `claude mcp list`, `claude mcp get piqae`, or `/mcp` to verify it. On native
Windows, Claude Code requires `command: "cmd"` and
`args: ["/c", "npx", "-y", "@piqae/mcp-server", "--stdio"]` for npm-based
stdio servers.

## VS Code and GitHub Copilot agent mode

Open **MCP: Open User Configuration** or create `.vscode/mcp.json`. Prefer a
password input or a private `envFile` instead of a literal key:

```json
{
  "inputs": [
    {
      "type": "promptString",
      "id": "piqae-api-key",
      "description": "Piqae API key",
      "password": true
    }
  ],
  "servers": {
    "piqae": {
      "type": "stdio",
      "command": "npx",
      "args": ["-y", "@piqae/mcp-server", "--stdio"],
      "env": {
        "PIQAE_API_ORIGIN": "https://api.piqae.com",
        "PIQAE_API_KEY": "${input:piqae-api-key}",
        "PIQAE_MCP_JOB_SUBMISSION": "disabled"
      }
    }
  }
}
```

Run **MCP: List Servers**, start `piqae`, and inspect its output if discovery
fails. A workspace configuration is shareable only when it contains no secret.

## Other MCP clients

Use the client's stdio server form with this process contract:

```json
{
  "command": "npx",
  "args": ["-y", "@piqae/mcp-server", "--stdio"],
  "env": {
    "PIQAE_API_ORIGIN": "https://api.piqae.com",
    "PIQAE_API_KEY": "<read from the client secret store>",
    "PIQAE_MCP_JOB_SUBMISSION": "disabled"
  }
}
```

Some clients use `servers` while others use `mcpServers`; follow that client's
schema. The process contract and Piqae authentication variables do not change.

## Platform/customer-account context

An integrator should normally run one MCP process for one selected customer
context, or restart it when an authorised operator switches customers:

```text
PIQAE_PLATFORM_KEY=piq_platform_...
PIQAE_WORKSPACE_ID=wsp_customer...
PIQAE_ENVIRONMENT_ID=env_customer_live...
```

Do not let a model, URL parameter, printer ID, or user-entered workspace ID
choose these values. Resolve them in trusted application code from the signed-in
integrator user and the integrator's immutable customer mapping. For broad
account provisioning, use the `piqae_platform_accounts` tool; for account-scoped
printing, use a process already bound to that account.

## First safe session

Ask the agent to perform these operations in order:

1. Call `piqae_context` with `identity`, then `workspace`.
2. Call `piqae_printers` with `list`; retrieve one printer only after selecting
   its exact returned ID.
3. Inspect targets and their `readiness` or `design_specification` before
   preparing a print workflow.
4. Keep job submission disabled until a human has confirmed the destination,
   fixture, Test/Live environment, and expected media.

The main tools are `piqae_context`, `piqae_nodes`, `piqae_node_onboarding`,
`piqae_printers`, `piqae_stocks`, `piqae_targets`, `piqae_uploads`,
`piqae_jobs`, `piqae_webhooks`, `piqae_platform_accounts`, and
`piqae_search_docs`.

To permit deliberate Test printing:

```text
PIQAE_MCP_JOB_SUBMISSION=test_only
```

The agent must still provide a stable idempotency key, the exact destination ID
as confirmation, and a named fixture. `all` is an explicit production operator
decision, not an installation default. Accepted or spooler-complete does not
prove physical delivery.

## One-time secrets and remote OAuth

Configure `PIQAE_MCP_SECRET_DIRECTORY` as an existing absolute, non-symlink
directory with mode `0700`. Created credentials are written to new `0600` files
and only their paths are returned to the agent. Transcript delivery requires
both server-wide and per-call opt-in and is not recommended.

For a centrally hosted MCP, use Streamable HTTP and the OAuth deployment model
in the [MCP server README](../../apps/mcp/README.md). The Piqae MCP process is a
resource server, not an authorization server. Its access token must be accepted
by the configured Piqae control plane and have the exact MCP public URL in its
audience.

## Troubleshooting

- `401` or `403`: call `piqae_context` with `identity`; check key revocation,
  environment, and scopes without printing the key.
- Empty printers: confirm the node connector grant and selected environment;
  another tenant's printers are intentionally invisible.
- Platform context error: set workspace and environment together and use IDs
  returned by the platform account response.
- Server exits immediately: verify Node.js 20+, `npx`, the absolute secret
  directory, and the client's MCP output log.
- A write is refused: confirm the API-key scope, MCP job policy, and any exact-ID
  confirmation required by the tool.
