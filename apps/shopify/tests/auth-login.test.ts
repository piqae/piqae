import { beforeEach, describe, expect, it, vi } from "vitest";
import { readFile } from "node:fs/promises";

const { login, authenticateAdmin } = vi.hoisted(() => ({
  login: vi.fn(),
  authenticateAdmin: vi.fn(),
}));

vi.mock("../app/shopify.server", () => ({
  default: {
    login,
    authenticate: { admin: authenticateAdmin },
  },
}));

import { action, loader } from "../app/routes/auth.login";
import routes from "../app/routes";

describe("Shopify installation login", () => {
  beforeEach(() => {
    login.mockReset();
    authenticateAdmin.mockReset();
    login.mockResolvedValue({});
  });

  it("registers the canonical login route before the auth callback wildcard", () => {
    expect(routes.slice(0, 4)).toEqual([
      { path: "healthz", file: "routes/healthz.ts" },
      { index: true, file: "routes/_index.tsx" },
      { path: "auth/login", file: "routes/auth.login.tsx" },
      { path: "auth/*", file: "routes/auth.$.tsx" },
    ]);
  });

  it.each([
    ["GET", loader],
    ["POST", action],
  ])("delegates %s requests to shopify.login", async (method, handler) => {
    const request = new Request(
      "https://shopify.piqae.com/auth/login?shop=c4beta.myshopify.com",
      { method },
    );

    await handler({
      request,
      params: {},
      context: {},
      unstable_pattern: "/auth/login",
    });

    expect(login).toHaveBeenCalledOnce();
    expect(login).toHaveBeenCalledWith(request);
    expect(authenticateAdmin).not.toHaveBeenCalled();
  });

  it("opts new and returning installs into refreshable offline tokens", async () => {
    const configuration = await readFile(
      new URL("../app/shopify.server.ts", import.meta.url),
      "utf8",
    );

    expect(configuration).toMatch(
      /future:\s*\{\s*expiringOfflineAccessTokens:\s*true,?\s*\}/,
    );
  });
});
