import { beforeEach, describe, expect, it, vi } from "vitest";

const { authenticateAdmin } = vi.hoisted(() => ({
  authenticateAdmin: vi.fn(),
}));

vi.mock("../app/shopify.server", () => ({
  default: { authenticate: { admin: authenticateAdmin } },
}));

import { loader } from "../app/routes/connect.complete";

describe("node connection completion", () => {
  beforeEach(() => {
    authenticateAdmin.mockReset();
    authenticateAdmin.mockResolvedValue({
      session: { shop: "c4beta.myshopify.com" },
    });
  });

  it("authenticates and returns to the fixed embedded settings route", async () => {
    const request = new Request(
      "https://shopify.piqae.com/connect/complete?shop=c4beta.myshopify.com",
    );
    const response = await loader({
      request,
      params: {},
      context: {},
      unstable_pattern: "/connect/complete",
    });

    expect(authenticateAdmin).toHaveBeenCalledWith(request);
    expect(response.status).toBe(302);
    expect(response.headers.get("location")).toBe(
      "/app/settings?shop=c4beta.myshopify.com",
    );
  });

  it("rejects a completion URL for a different authenticated store", async () => {
    const request = new Request(
      "https://shopify.piqae.com/connect/complete?shop=other.myshopify.com",
    );

    await expect(
      loader({
        request,
        params: {},
        context: {},
        unstable_pattern: "/connect/complete",
      }),
    ).rejects.toMatchObject({ status: 403 });
  });

  it("ignores redirect-shaped query parameters", async () => {
    const response = await loader({
      request: new Request(
        "https://shopify.piqae.com/connect/complete?shop=c4beta.myshopify.com&return_url=https://attacker.example",
      ),
      params: {},
      context: {},
      unstable_pattern: "/connect/complete",
    });

    expect(response.headers.get("location")).toBe(
      "/app/settings?shop=c4beta.myshopify.com",
    );
  });
});
