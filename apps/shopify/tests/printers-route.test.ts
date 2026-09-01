import { beforeEach, describe, expect, it, vi } from "vitest";

const { authenticateAdmin, getSettings, updateSettings } = vi.hoisted(() => ({
  authenticateAdmin: vi.fn(),
  getSettings: vi.fn(),
  updateSettings: vi.fn(),
}));

vi.mock("../app/shopify.server", () => ({
  default: { authenticate: { admin: authenticateAdmin } },
}));
vi.mock("../app/core/workflows.server", async (importOriginal) => {
  const original =
    await importOriginal<typeof import("../app/core/workflows.server")>();
  return {
    ...original,
    workflows: () => ({ getSettings, updateSettings }),
  };
});

import { action, loader } from "../app/routes/app.settings";

describe("Shopify settings route", () => {
  const settings = {
    defaultPrinterId: "",
    defaultTemplateId: "",
    preferDirect: true,
    offerPdf: true,
    metafieldAllowlist: [],
    retentionDays: 30,
    renderExecutionPolicy: "automatic",
    printOrder: {
      hierarchy: [],
      taxonomyDepth: "family",
      mixedOrderMode: "dominant",
    },
  };

  beforeEach(() => {
    authenticateAdmin.mockReset();
    getSettings.mockReset();
    updateSettings.mockReset();
    authenticateAdmin.mockResolvedValue({
      session: { shop: "fixture.myshopify.com" },
    });
    getSettings.mockResolvedValue(settings);
  });

  it("authenticates and loads the merchant print hierarchy", async () => {
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
    expect(response.settings.printOrder.taxonomyDepth).toBe("family");
  });

  it("saves a bounded ordered grouping strategy without replacing other settings", async () => {
    const request = new Request("https://shopify.piqae.com/app/settings", {
      method: "POST",
      body: new URLSearchParams({
        intent: "save-print-order",
        printOrder: JSON.stringify({
          hierarchy: ["taxonomy", "primary_product"],
          taxonomyDepth: "specific",
          mixedOrderMode: "contains",
        }),
      }),
    });
    const response = await action({
      request,
      params: {},
      context: {},
      unstable_pattern: "/app/settings",
    });

    expect(response).toEqual({ ok: true, error: "" });
    expect(updateSettings).toHaveBeenCalledWith("fixture.myshopify.com", {
      printOrder: {
        hierarchy: ["taxonomy", "primary_product"],
        taxonomyDepth: "specific",
        mixedOrderMode: "contains",
      },
    });
  });
});
