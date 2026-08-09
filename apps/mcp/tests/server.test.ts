import { afterEach, describe, expect, it, vi } from "vitest";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { InMemoryTransport } from "@modelcontextprotocol/sdk/inMemory.js";
import type { Transport } from "@modelcontextprotocol/sdk/shared/transport.js";
import { createPiqaeMcpServer } from "../src/server.js";
import { loadConfig, type McpConfig } from "../src/config.js";

const originalFetch = globalThis.fetch;

afterEach(() => {
  globalThis.fetch = originalFetch;
  vi.restoreAllMocks();
});

describe("Piqae MCP server", () => {
  it("advertises the complete grouped tool and knowledge surface", async () => {
    const { client, server } = await linked(config());
    const tools = await client.listTools();
    expect(tools.tools.map((tool) => tool.name)).toEqual(
      expect.arrayContaining([
        "piqae_context",
        "piqae_api_keys",
        "piqae_nodes",
        "piqae_node_onboarding",
        "piqae_printers",
        "piqae_stocks",
        "piqae_targets",
        "piqae_uploads",
        "piqae_jobs",
        "piqae_webhooks",
        "piqae_platform_accounts",
        "piqae_search_docs",
      ]),
    );
    const resources = await client.listResources();
    expect(resources.resources.map((resource) => resource.uri)).toContain(
      "piqae://openapi/v1",
    );
    await client.close();
    await server.close();
  });

  it("calls public health without forwarding the configured credential", async () => {
    globalThis.fetch = vi.fn(
      async (_input: string | URL | Request, init?: RequestInit) => {
        const headers = new Headers(init?.headers);
        expect(headers.has("authorization")).toBe(false);
        return Response.json({ status: "ok" });
      },
    ) as typeof fetch;
    const { client, server } = await linked(config());
    const response = await client.callTool({
      name: "piqae_context",
      arguments: { action: "health" },
    });
    expect(response.isError).not.toBe(true);
    expect(response.structuredContent).toEqual({ status: "ok" });
    await client.close();
    await server.close();
  });

  it("keeps job submission disabled by default", async () => {
    const fetchMock = vi.fn();
    globalThis.fetch = fetchMock as typeof fetch;
    const { client, server } = await linked(config());
    const response = await client.callTool({
      name: "piqae_jobs",
      arguments: {
        action: "create",
        printer_id: "ptr_test",
        title: "Fake fixture",
        content_type: "pdf",
        uri: "https://objects.example/fake.pdf",
        idempotency_key: "fixture-job-0001",
        confirm_destination: "ptr_test",
        fixture: "deterministic virtual printer",
      },
    });
    expect(response.isError).toBe(true);
    expect(JSON.stringify(response.structuredContent)).toContain(
      "Job submission is disabled",
    );
    expect(fetchMock).not.toHaveBeenCalled();
    await client.close();
    await server.close();
  });

  it("redacts secrets nested in API error details", async () => {
    const leaked = "piq_unknown_super-secret-value";
    globalThis.fetch = vi.fn(async () =>
      Response.json(
        {
          error: {
            code: "denied",
            message: `Credential ${leaked} was denied`,
            request_id: "req_test",
            retryable: false,
            details: { nested: { access_token: leaked } },
          },
        },
        { status: 403 },
      ),
    ) as typeof fetch;
    const { client, server } = await linked(config());
    const response = await client.callTool({
      name: "piqae_context",
      arguments: { action: "identity" },
    });
    const serialized = JSON.stringify(response.structuredContent);
    expect(serialized).not.toContain(leaked);
    expect(serialized).toContain("REDACTED_PIQAE_SECRET");
    await client.close();
    await server.close();
  });
});

describe("configuration", () => {
  it("defaults to loopback, no print submission, and no transcript secret output", () => {
    const value = loadConfig({});
    expect(value.publicUrl).toBe("http://127.0.0.1:39300/mcp");
    expect(value.jobSubmission).toBe("disabled");
    expect(value.allowSecretOutput).toBe(false);
  });

  it("requires platform selection values as a pair", () => {
    expect(() => loadConfig({ PIQAE_WORKSPACE_ID: "wsp_test" })).toThrow(
      /PIQAE_WORKSPACE_ID and PIQAE_ENVIRONMENT_ID/,
    );
  });

  it("constructs a valid bracketed IPv6 loopback URL", () => {
    expect(loadConfig({ PIQAE_MCP_BIND_HOST: "::1" }).publicUrl).toBe(
      "http://[::1]:39300/mcp",
    );
  });
});

function config(): McpConfig {
  return loadConfig({
    PIQAE_API_ORIGIN: "https://api.example.test",
    PIQAE_API_KEY: "piq_test_example-not-a-real-secret",
  });
}

async function linked(configuration: McpConfig) {
  const [clientTransport, serverTransport] =
    InMemoryTransport.createLinkedPair();
  const server = createPiqaeMcpServer(configuration);
  const client = new Client({ name: "piqae-mcp-test", version: "1.0.0" });
  await server.connect(serverTransport as unknown as Transport);
  await client.connect(clientTransport as unknown as Transport);
  return { client, server };
}
