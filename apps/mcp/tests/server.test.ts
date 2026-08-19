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
  it("keeps intent reads separate from confirmed workflow writes", async () => {
    const fetchMock = vi.fn<typeof fetch>().mockResolvedValue(
      Response.json({
        status: "valid",
        capability_revision: 1,
        errors: [],
        warnings: [],
      }),
    );
    globalThis.fetch = fetchMock as typeof fetch;
    const { client, server } = await linked(config());
    const intent = {
      schema_version: 1,
      printer_id: "ptr_test",
      capability_revision: 1,
      portable_options: {},
      semantic_options: {},
      document_manifest: {
        page_count: 1,
        page_boxes: [{ width_mm: 100, height_mm: 150 }],
        color_spaces: [],
        separations: [],
        scaling: "none",
      },
    };
    const validated = await client.callTool({
      name: "piqae_print_intents",
      arguments: { action: "validate", intent },
    });
    expect(validated.isError).not.toBe(true);
    expect(new URL(String(fetchMock.mock.calls[0]?.[0])).pathname).toBe(
      "/v1/print-intents/validate",
    );

    fetchMock.mockClear();
    const denied = await client.callTool({
      name: "piqae_workflows",
      arguments: {
        action: "create",
        name: "Labels",
        printer_id: "ptr_test",
        capability_revision: 1,
        definition: intent,
        safe_overrides: [],
        confirm: "some_other_printer",
      },
    });
    expect(denied.isError).toBe(true);
    expect(fetchMock).not.toHaveBeenCalled();
    await client.close();
    await server.close();
  });

  it("rejects native driver escapes before intent requests reach the API", async () => {
    const fetchMock = vi.fn();
    globalThis.fetch = fetchMock as typeof fetch;
    const { client, server } = await linked(config());
    const response = await client.callTool({
      name: "piqae_print_intents",
      arguments: {
        action: "resolve",
        intent: {
          schema_version: 1,
          printer_id: "ptr_test",
          capability_revision: 1,
          portable_options: { native_options: { PrivateByte: "1" } },
          semantic_options: {},
          document_manifest: {
            page_count: 1,
            page_boxes: [{ width_mm: 100, height_mm: 150 }],
            color_spaces: [],
            separations: [],
            scaling: "none",
          },
        },
      },
    });
    expect(response.isError).toBe(true);
    expect(JSON.stringify(response.structuredContent)).toContain(
      "Driver-native intent field is forbidden",
    );
    expect(fetchMock).not.toHaveBeenCalled();
    await client.close();
    await server.close();
  });

  it("rejects nested camel-case native blobs before workflow creation", async () => {
    const fetchMock = vi.fn();
    globalThis.fetch = fetchMock as typeof fetch;
    const { client, server } = await linked(config());
    const response = await client.callTool({
      name: "piqae_workflows",
      arguments: {
        action: "create",
        name: "Unsafe",
        printer_id: "ptr_test",
        capability_revision: 1,
        definition: { semantic_options: { effect: { nativeBlob: "opaque" } } },
        safe_overrides: [],
        published: false,
        confirm: "ptr_test",
      },
    });
    expect(response.isError).toBe(true);
    expect(fetchMock).not.toHaveBeenCalled();
    await client.close();
    await server.close();
  });

  it("rejects exact native fields with the caller's root path", async () => {
    const fetchMock = vi.fn();
    globalThis.fetch = fetchMock as typeof fetch;
    const { client, server } = await linked(config());
    const response = await client.callTool({
      name: "piqae_workflows",
      arguments: {
        action: "create",
        name: "Unsafe",
        printer_id: "ptr_test",
        capability_revision: 1,
        definition: { semantic_options: { effect: { native: "opaque" } } },
        safe_overrides: [],
        published: false,
        confirm: "ptr_test",
      },
    });
    expect(response.isError).toBe(true);
    expect(JSON.stringify(response.structuredContent)).toContain(
      "definition.semantic_options.effect.native",
    );
    expect(fetchMock).not.toHaveBeenCalled();
    await client.close();
    await server.close();
  });

  it.each([
    ["driver-native", ["portable_options.native_options.InputSlot"]],
    ["invalid prefix", ["document_manifest.scaling"]],
    ["duplicate", ["portable_options.copies", "portable_options.copies"]],
  ])("rejects %s workflow safe overrides", async (_name, safeOverrides) => {
    const fetchMock = vi.fn();
    globalThis.fetch = fetchMock as typeof fetch;
    const { client, server } = await linked(config());
    const response = await client.callTool({
      name: "piqae_workflows",
      arguments: {
        action: "create",
        name: "Unsafe",
        printer_id: "ptr_test",
        capability_revision: 1,
        definition: {},
        safe_overrides: safeOverrides,
        published: false,
        confirm: "ptr_test",
      },
    });
    expect(response.isError).toBe(true);
    expect(fetchMock).not.toHaveBeenCalled();
    await client.close();
    await server.close();
  });

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
        "piqae_business_documents",
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

  it("keeps document print submission disabled by default", async () => {
    const fetchMock = vi.fn();
    globalThis.fetch = fetchMock as typeof fetch;
    const { client, server } = await linked(config());
    const response = await client.callTool({
      name: "piqae_business_documents",
      arguments: {
        action: "print",
        render_id: "drnd_test",
        target_id: "tgt_test",
        title: "Fake receipt",
        idempotency_key: "print-fixture-0001",
        confirm_destination: "tgt_test",
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
