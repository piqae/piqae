import { afterEach, describe, expect, it, vi } from "vitest";
import { verifyBearer } from "../src/api.js";
import { loadConfig } from "../src/config.js";

const originalFetch = globalThis.fetch;

afterEach(() => {
  globalThis.fetch = originalFetch;
  vi.restoreAllMocks();
});

describe("remote OAuth bearer verification", () => {
  it("requires the exact MCP resource audience after upstream signature validation", async () => {
    globalThis.fetch = vi.fn(async () =>
      Response.json({
        id: "usr_test",
        workspace_id: "wsp_test",
        environment_id: "env_test",
      }),
    ) as typeof fetch;
    const config = loadConfig({
      PIQAE_API_ORIGIN: "https://api.example.test",
      PIQAE_MCP_PUBLIC_URL: "https://mcp.example.test/mcp",
      PIQAE_MCP_AUTHORIZATION_SERVER: "https://identity.example.test",
    });
    await expect(
      verifyBearer(
        config,
        jwt({
          aud: "https://different.example.test",
          permissions: ["jobs_read"],
        }),
      ),
    ).rejects.toThrow(/audience-bound/);
    await expect(
      verifyBearer(
        config,
        jwt({
          aud: ["https://api.example.test", "https://mcp.example.test/mcp"],
          azp: "coding-agent",
          permissions: ["jobs_read"],
        }),
      ),
    ).resolves.toMatchObject({
      clientId: "coding-agent",
      scopes: ["jobs_read"],
    });
  });
});

function jwt(payload: Record<string, unknown>): string {
  return [
    Buffer.from(JSON.stringify({ alg: "RS256", typ: "JWT" })).toString(
      "base64url",
    ),
    Buffer.from(JSON.stringify(payload)).toString("base64url"),
    "upstream-verified-signature",
  ].join(".");
}
