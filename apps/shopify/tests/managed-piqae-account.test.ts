import { describe, expect, it, vi } from "vitest";

import { ManagedPiqaeAccountService } from "../app/core/managed-piqae-account.server";
import { MemoryShopRepository } from "../app/core/model";
import { starterTemplates } from "../app/core/starter-templates";
import { MemoryWorkflowRepository } from "../app/core/workflows.server";

const shop = "managed-shop.myshopify.com";
const fixturePlatformKey = ["piq", "platform", "fixture"].join("_");

describe("managed Piqae Shopify accounts", () => {
  it("provisions an isolated account and publishes the starter documents", async () => {
    let template = 0;
    let revision = 0;
    const fetcher = vi.fn(
      async (input: string | URL | Request, init?: RequestInit) => {
        const url = new URL(String(input));
        const headers = new Headers(init?.headers);
        expect(headers.get("authorization")).toBe(
          `Bearer ${fixturePlatformKey}`,
        );
        if (url.pathname === `/v1/platform/accounts/${shop}`) {
          expect(init?.method).toBe("PUT");
          return Response.json({
            id: "ws_managed",
            external_id: shop,
            name: "managed-shop",
            status: "active",
            metadata: { source: "shopify", shop },
            environments: {
              live: { id: "env_live", kind: "live" },
              test: { id: "env_test", kind: "test" },
            },
            created_at: "2026-08-19T00:00:00Z",
            updated_at: "2026-08-19T00:00:00Z",
          });
        }
        expect(headers.get("x-piqae-workspace-id")).toBe("ws_managed");
        expect(headers.get("x-piqae-environment-id")).toBe("env_live");
        if (url.pathname === "/v1/business-document-templates") {
          template += 1;
          return Response.json({ id: `tpl_${template}` });
        }
        if (
          /^\/v1\/business-document-templates\/tpl_\d+\/publish$/.test(
            url.pathname,
          )
        ) {
          revision += 1;
          return Response.json({ id: `rev_${revision}` });
        }
        return Response.json(
          { error: { code: "unexpected", message: url.pathname } },
          { status: 500 },
        );
      },
    );
    const shops = new MemoryShopRepository();
    const workflows = new MemoryWorkflowRepository();
    const service = new ManagedPiqaeAccountService(
      shops,
      workflows,
      fixturePlatformKey,
      "https://api.piqae.test",
      fetcher as typeof fetch,
    );

    const link = await service.ensure(shop);
    expect(link).toMatchObject({
      shop,
      piqaeAccountId: "ws_managed",
      piqaeLiveEnvironmentId: "env_live",
      piqaeTestEnvironmentId: "env_test",
      encryptedCredential: "",
      entitlementMode: "shopify_child",
      planHandle: "development",
    });
    expect(template).toBe(starterTemplates.length);
    expect(revision).toBe(starterTemplates.length);
    expect(
      (await workflows.listTemplates(shop)).every(
        (value) => value.state === "published",
      ),
    ).toBe(true);

    await service.ensure(shop);
    expect(fetcher).toHaveBeenCalledTimes(1 + starterTemplates.length * 2);
  });
});
