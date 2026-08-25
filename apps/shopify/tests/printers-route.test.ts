import { beforeEach, describe, expect, it, vi } from "vitest";

const { authenticateAdmin } = vi.hoisted(() => ({
  authenticateAdmin: vi.fn(),
}));

vi.mock("../app/shopify.server", () => ({
  default: { authenticate: { admin: authenticateAdmin } },
}));

import { loader } from "../app/routes/app.settings";

describe("legacy Shopify settings route", () => {
  beforeEach(() => {
    authenticateAdmin.mockReset();
    authenticateAdmin.mockResolvedValue({
      session: { shop: "fixture.myshopify.com" },
    });
  });

  it("authenticates before preserving query context in the printers redirect", async () => {
    const request = new Request(
      "https://shopify.piqae.com/app/settings?host=safe-host&embedded=1",
    );
    const response = await loader({
      request,
      params: {},
      context: {},
      unstable_pattern: "/app/settings",
    });

    expect(authenticateAdmin).toHaveBeenCalledWith(request);
    expect(response.status).toBe(302);
    expect(response.headers.get("location")).toBe(
      "/app/printers?host=safe-host&embedded=1",
    );
  });
});
