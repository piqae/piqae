import { request as httpRequest } from "node:http";
import { createServer } from "node:net";
import { afterEach, describe, expect, it, vi } from "vitest";
import { loadConfig } from "../src/config.js";
import { startHttpServer } from "../src/http.js";

const originalFetch = globalThis.fetch;
const closers: Array<() => Promise<void>> = [];

afterEach(async () => {
  globalThis.fetch = originalFetch;
  vi.restoreAllMocks();
  await Promise.all(closers.splice(0).map((close) => close()));
});

describe("Streamable HTTP", () => {
  it("publishes RFC 9728 metadata for the exact MCP resource", async () => {
    const port = await unusedPort();
    const publicUrl = `http://127.0.0.1:${port}/mcp`;
    closers.push(
      await startHttpServer(
        loadConfig({
          PIQAE_MCP_PORT: String(port),
          PIQAE_MCP_PUBLIC_URL: publicUrl,
          PIQAE_MCP_AUTHORIZATION_SERVER: "https://identity.example.com",
        }),
      ),
    );
    const response = await originalFetch(
      `http://127.0.0.1:${port}/.well-known/oauth-protected-resource/mcp`,
    );
    expect(response.status).toBe(200);
    expect(await response.json()).toMatchObject({
      resource: publicUrl,
      authorization_servers: ["https://identity.example.com"],
    });
  });

  it("requires a bearer before processing MCP JSON-RPC", async () => {
    const port = await unusedPort();
    closers.push(
      await startHttpServer(
        loadConfig({
          PIQAE_MCP_PORT: String(port),
          PIQAE_MCP_PUBLIC_URL: `http://127.0.0.1:${port}/mcp`,
        }),
      ),
    );
    const response = await originalFetch(`http://127.0.0.1:${port}/mcp`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(initializeRequest()),
    });
    expect(response.status).toBe(401);
    expect(response.headers.get("www-authenticate")).toContain("invalid_token");
  });

  it("verifies a loopback Piqae key and completes MCP initialization", async () => {
    const port = await unusedPort();
    const apiOrigin = "https://api.example.test";
    globalThis.fetch = vi.fn(
      async (input: string | URL | Request, init?: RequestInit) => {
        const url = String(input);
        if (url === `${apiOrigin}/v1/identity/me`) {
          expect(new Headers(init?.headers).get("authorization")).toBe(
            "Bearer piq_test_example",
          );
          return Response.json({
            id: "usr_test",
            workspace_id: "wsp_test",
            environment_id: "env_test",
          });
        }
        return originalFetch(input, init);
      },
    ) as typeof fetch;
    closers.push(
      await startHttpServer(
        loadConfig({
          PIQAE_API_ORIGIN: apiOrigin,
          PIQAE_MCP_PORT: String(port),
          PIQAE_MCP_PUBLIC_URL: `http://127.0.0.1:${port}/mcp`,
        }),
      ),
    );
    const response = await originalFetch(`http://127.0.0.1:${port}/mcp`, {
      method: "POST",
      headers: {
        accept: "application/json, text/event-stream",
        authorization: "Bearer piq_test_example",
        "content-type": "application/json",
      },
      body: JSON.stringify(initializeRequest()),
    });
    expect(response.status).toBe(200);
    expect(await response.json()).toMatchObject({
      jsonrpc: "2.0",
      id: 1,
      result: { serverInfo: { name: "piqae", version: "0.1.0" } },
    });
  });

  it("rejects an oversized chunked body without trusting Content-Length", async () => {
    const port = await unusedPort();
    const apiOrigin = "https://api.example.test";
    mockIdentity(apiOrigin);
    closers.push(
      await startHttpServer(
        loadConfig({
          PIQAE_API_ORIGIN: apiOrigin,
          PIQAE_MCP_PORT: String(port),
          PIQAE_MCP_PUBLIC_URL: `http://127.0.0.1:${port}/mcp`,
        }),
      ),
    );
    const response = await chunkedRequest(port, 17, 64 * 1024);
    expect(response.status).toBe(413);
    expect(response.body).toContain("request_too_large");
  });
});

function initializeRequest() {
  return {
    jsonrpc: "2.0",
    id: 1,
    method: "initialize",
    params: {
      protocolVersion: "2025-06-18",
      capabilities: {},
      clientInfo: { name: "http-test", version: "1.0.0" },
    },
  };
}

async function unusedPort(): Promise<number> {
  const server = createServer();
  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  const address = server.address();
  if (!address || typeof address === "string")
    throw new Error("failed to allocate test port");
  await new Promise<void>((resolve, reject) => {
    server.close((error) => (error ? reject(error) : resolve()));
  });
  return address.port;
}

function mockIdentity(apiOrigin: string): void {
  globalThis.fetch = vi.fn(
    async (input: string | URL | Request, init?: RequestInit) => {
      if (String(input) === `${apiOrigin}/v1/identity/me`) {
        return Response.json({
          id: "usr_test",
          workspace_id: "wsp_test",
          environment_id: "env_test",
        });
      }
      return originalFetch(input, init);
    },
  ) as typeof fetch;
}

async function chunkedRequest(
  port: number,
  chunks: number,
  chunkBytes: number,
): Promise<{ status: number; body: string }> {
  return new Promise((resolve, reject) => {
    const request = httpRequest(
      {
        host: "127.0.0.1",
        port,
        path: "/mcp",
        method: "POST",
        headers: {
          accept: "application/json, text/event-stream",
          authorization: "Bearer piq_test_example",
          "content-type": "application/json",
        },
      },
      (response) => {
        const body: Buffer[] = [];
        response.on("data", (chunk: Buffer) => body.push(chunk));
        response.on("end", () =>
          resolve({
            status: response.statusCode ?? 0,
            body: Buffer.concat(body).toString("utf8"),
          }),
        );
      },
    );
    request.once("error", reject);
    const body = Buffer.alloc(chunkBytes, 0x20);
    for (let index = 0; index < chunks; index += 1) request.write(body);
    request.end();
  });
}
