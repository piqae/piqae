import { describe, expect, it, vi } from "vitest";
import { CredentialVault } from "../app/core/credentials.server";
import { fetchOrders, normalizeOrderGid } from "../app/core/orders.server";
import { MemoryShopRepository, normalizeShopDomain } from "../app/core/model";
import { ShopifyPrintingService } from "../app/core/printing.server";
import { renderShopifyLiquid } from "../app/core/liquid-template.server";
import { EntitlementService } from "../app/core/entitlements.server";
import { DownloadTokenVault } from "../app/core/download-token.server";
import { WebhookReconciliationWorker } from "../app/core/webhook-worker.server";
import {
  CloudflareEmailClient,
  EmailDeliveryError,
} from "../app/core/cloudflare-email.server";
import {
  confirmManagedPlan,
  hostedPricingUrl,
} from "../app/core/shopify-app-pricing.server";
import { editorDocument } from "../app/components/shopify-ui";
import { liquidCompatibilityNotice } from "../app/routes/app.templates.$templateId";
import { templates } from "../app/routes/app.templates";
import { selectedOrderIds } from "../app/routes/app.print";
import { starterTemplates } from "../app/core/starter-templates";

const shop = "fixture-shop.myshopify.com";
const order = {
  id: "gid://shopify/Order/42",
  name: "#1042",
  createdAt: "2026-08-09T00:00:00Z",
  currencyCode: "NZD",
  customer: null,
  shippingAddress: null,
  lineItems: {
    nodes: [
      {
        title: "Coffee",
        sku: "COF",
        quantity: 2,
        originalUnitPriceSet: { shopMoney: { amount: "10.00" } },
        discountedTotalSet: { shopMoney: { amount: "20.00" } },
      },
    ],
  },
  subtotalPriceSet: { shopMoney: { amount: "20.00" } },
  totalTaxSet: { shopMoney: { amount: "3.00" } },
  totalPriceSet: { shopMoney: { amount: "23.00" } },
};
const admin = {
  graphql: vi.fn(async () => Response.json({ data: { order } })),
};

describe("Shopify boundary", () => {
  it("uses named checkbox values and enforces the Admin bulk limit", () => {
    const form = new FormData();
    form.append("orderIds", "gid://shopify/Order/1");
    form.append("orderIds", "gid://shopify/Order/2");
    form.append("orderIds", "gid://shopify/Order/1");
    expect(selectedOrderIds(form)).toEqual([
      "gid://shopify/Order/1",
      "gid://shopify/Order/2",
    ]);
    expect(() => selectedOrderIds(new FormData())).toThrow("1 and 50");
    const tooMany = new FormData();
    for (let index = 1; index <= 51; index += 1)
      tooMany.append("orderIds", `gid://shopify/Order/${index}`);
    expect(() => selectedOrderIds(tooMany)).toThrow("1 and 50");
  });
  it("accepts only canonical numeric order GIDs", () => {
    expect(normalizeOrderGid("42")).toBe("gid://shopify/Order/42");
    expect(() => normalizeOrderGid("gid://shopify/Product/42")).toThrow(
      "invalid",
    );
  });
  it("normalizes GraphQL money without trusting client order content", async () => {
    const [value] = await fetchOrders(admin, ["42", "42"]);
    expect(admin.graphql).toHaveBeenCalledTimes(1);
    expect(value?.lineItems[0]?.total).toBe("20.00");
    expect(value?.total).toBe("23.00");
  });
  it("binds encrypted Piqae credentials to the shop", () => {
    const vault = new CredentialVault(Buffer.alloc(32, 7));
    const sealed = vault.seal("secret-token", shop);
    expect(vault.open(sealed, shop)).toBe("secret-token");
    expect(() => vault.open(sealed, "other-shop.myshopify.com")).toThrow();
  });
  it("rejects malformed shops and credential-envelope tampering", () => {
    expect(normalizeShopDomain(" FIXTURE-SHOP.MYSHOPIFY.COM ")).toBe(shop);
    for (const invalid of [
      "fixture-shop.myshopify.com.evil.test",
      "https://fixture-shop.myshopify.com",
      "fixture_shop.myshopify.com",
      ".myshopify.com",
    ]) {
      expect(() => normalizeShopDomain(invalid)).toThrow("invalid");
    }

    const vault = new CredentialVault(Buffer.alloc(32, 7));
    const sealed = vault.seal("secret-token", shop);
    const parts = sealed.split(".");
    expect(() => vault.open(`v2.${parts.slice(1).join(".")}`, shop)).toThrow(
      "invalid credential envelope",
    );
    expect(() =>
      vault.open(
        `${sealed.slice(0, -1)}${sealed.endsWith("A") ? "B" : "A"}`,
        shop,
      ),
    ).toThrow();
    expect(() => vault.open(`${sealed}.trailing`, shop)).toThrow(
      "invalid credential envelope",
    );
  });
  it("deduplicates webhook claims without crossing repository data", async () => {
    const repository = new MemoryShopRepository();
    expect(
      await repository.claimWebhook("hook-1", {
        shop,
        topic: "ORDERS_UPDATED",
        resourceId: "gid://shopify/Order/42",
      }),
    ).toBe(true);
    expect(await repository.claimWebhook("hook-1")).toBe(false);
    expect(await repository.get(shop)).toBeNull();
  });
  it("rejects empty and oversized order selections before GraphQL", async () => {
    const boundary = { graphql: vi.fn() };
    await expect(fetchOrders(boundary, [])).rejects.toThrow(
      "select between 1 and 250 orders",
    );
    await expect(
      fetchOrders(
        boundary,
        Array.from({ length: 251 }, (_, index) => String(index + 1)),
      ),
    ).rejects.toThrow("select between 1 and 250 orders");
    expect(boundary.graphql).not.toHaveBeenCalled();
  });
  it("uses stable idempotency and prefers direct printing when a printer is selected", async () => {
    const repository = new MemoryShopRepository();
    const vault = new CredentialVault(Buffer.alloc(32, 9));
    await repository.put({
      shop,
      piqaeAccountId: "acct_1",
      encryptedCredential: vault.seal("token", shop),
      templateRevisionId: "rev_1",
      createdAt: new Date().toISOString(),
    });
    const create = vi.fn(async (..._args: unknown[]) => ({
      id: "render_1",
      state: "completed",
      failure_code: null,
    }));
    const print = vi.fn(async (..._args: unknown[]) => ({ id: "job_1" }));
    const service = new ShopifyPrintingService(
      repository,
      vault,
      () =>
        ({
          documents: {
            renders: { create, print, retrieve: vi.fn() },
            templates: {},
            conversions: {},
            renderAndPrint: vi.fn(),
          },
        }) as never,
      "https://app.example",
    );
    const result = await service.printOrders({
      admin,
      shop,
      orderIds: ["42"],
      printerId: "printer_1",
      requestKey: "click-1",
    });
    expect(result).toEqual({
      mode: "direct",
      renderId: "render_1",
      jobId: "job_1",
    });
    expect(create.mock.calls[0]?.[1]).toMatch(/^shopify-render-[a-f0-9]{64}$/);
    expect(print.mock.calls[0]?.[2]).toMatch(
      /^shopify-print-[a-f0-9]{64}-printer_1$/,
    );
  });
});

describe("bounded Liquid subset", () => {
  it("escapes untrusted values while preserving QR data", async () => {
    const result = await renderShopifyLiquid(
      "{{ order.name }} {{ order.qr }}",
      { order: { name: "<script>", qr: "https://example.test/q/42" } },
    );
    expect(result.output).toContain("&lt;script&gt;");
    expect(result.output).toContain("https://example.test/q/42");
  });
  it("rejects external composition and oversized sources", async () => {
    await expect(
      renderShopifyLiquid("{% include '/etc/passwd' %}", {}),
    ).rejects.toThrow("FORBIDDEN");
    await expect(
      renderShopifyLiquid("{%- render 'secret' -%}", {}),
    ).rejects.toThrow("FORBIDDEN");
    await expect(renderShopifyLiquid("x".repeat(70_000), {})).rejects.toThrow(
      "SOURCE_LIMIT",
    );
  });
});

describe("entitlements", () => {
  it("allows an active existing Piqae subscription without Shopify child billing", async () => {
    const service = new EntitlementService(
      { activeSubscription: vi.fn() },
      {
        verify: vi.fn(async () => ({
          accountId: "acct_existing",
          active: true,
        })),
      },
      { provision: vi.fn() },
      "app_1",
      new Set(["standard"]),
    );
    await expect(service.linkExisting("token")).resolves.toEqual({
      mode: "existing_piqae",
      accountId: "acct_existing",
    });
  });
  it("verifies Partner API state and provisions child tenants idempotently", async () => {
    let capturedKey = "";
    const provision = vi.fn(
      async (input: {
        shop: string;
        planHandle: string;
        idempotencyKey: string;
      }) => {
        capturedKey = input.idempotencyKey;
        return {
          accountId: "acct_child",
          credential: "child-secret",
        };
      },
    );
    const service = new EntitlementService(
      {
        activeSubscription: vi.fn(async () => ({
          status: "ACTIVE",
          planHandle: "standard",
        })),
      },
      { verify: vi.fn() },
      { provision },
      "app_1",
      new Set(["standard"]),
    );
    await service.provisionChild({
      shop,
      shopId: "gid://shopify/Shop/1",
      redirectPlanHandle: "standard",
    });
    expect(capturedKey).toMatch(/^shopify-child-[a-f0-9]{64}$/);
  });
});

describe("customer download grants", () => {
  it("encrypts bound short-lived grants and rejects tampering", () => {
    const vault = new DownloadTokenVault(Buffer.alloc(32, 3));
    const token = vault.issue({
      shop,
      renderId: "render_1",
      orderGid: "gid://shopify/Order/42",
      customerGid: "gid://shopify/Customer/7",
    });
    expect(vault.open(token).renderId).toBe("render_1");
    expect(() => vault.open(`${token.slice(0, -1)}A`)).toThrow();
  });
});

describe("automation delivery", () => {
  it("fails closed and schedules retry when email provider is absent", async () => {
    const query = vi
      .fn()
      .mockResolvedValueOnce({
        rows: [
          {
            webhook_id: "wh_1",
            shop,
            topic: "ORDERS_PAID",
            resource_id: "gid://shopify/Order/42",
            attempts: 1,
          },
        ],
      })
      .mockResolvedValueOnce({ rows: [] });
    const workflow = {
      listAutomations: vi.fn(async () => [
        {
          id: "00000000-0000-4000-8000-000000000001",
          name: "Email invoice",
          trigger: "order_paid",
          delivery: "email",
          templateId: "00000000-0000-4000-8000-000000000002",
          destination: "buyer@example.test",
          enabled: true,
          updatedAt: new Date().toISOString(),
        },
      ]),
      recordActivity: vi.fn(),
    } as any;
    const worker = new WebhookReconciliationWorker(
      { query } as any,
      {
        forShop: vi.fn(async () => ({
          graphql: vi.fn(async () =>
            Response.json({ data: { node: { id: "gid://shopify/Order/42" } } }),
          ),
        })),
      },
      workflow,
      { print: vi.fn() },
    );
    await worker.runOnce();
    expect(query.mock.calls.at(-1)?.[1]?.at(-1)).toBe(
      "EMAIL_PROVIDER_NOT_CONFIGURED",
    );
    expect(workflow.recordActivity).toHaveBeenCalledWith(
      shop,
      expect.objectContaining({ state: "failed" }),
    );
  });
});

describe("Cloudflare transactional email", () => {
  const message = {
    to: "buyer@example.test",
    subject: "Invoice",
    html: "<p>Attached</p>",
    text: "Attached",
    pdf: new Uint8Array([37, 80, 68, 70]),
    filename: "invoice.pdf",
  };
  it("sends REST fields and accepts queued recipients without exposing the token", async () => {
    const fetcher = vi.fn(async (_url: string, init: RequestInit) =>
      Response.json({
        success: true,
        result: { delivered: [], queued: [message.to], permanent_bounces: [] },
      }),
    );
    const client = new CloudflareEmailClient({
      accountId: "acct",
      token: "super-secret",
      fromAddress: "orders@example.test",
      fromName: "Piqae",
      replyTo: "help@example.test",
      fetch: fetcher as typeof fetch,
    });
    await expect(client.send(message)).resolves.toBe("queued");
    const body = JSON.parse(String(fetcher.mock.calls[0]?.[1]?.body));
    expect(body.from).toEqual({
      address: "orders@example.test",
      name: "Piqae",
    });
    expect(body.reply_to).toBe("help@example.test");
    expect(body.text).toBe("Attached");
    expect(body.attachments[0].content).toBe("JVBERg==");
  });
  it("retries only rate limits and server failures", async () => {
    for (const [status, retryable] of [
      [400, false],
      [401, false],
      [429, true],
      [500, true],
    ] as const) {
      const client = new CloudflareEmailClient({
        accountId: "acct",
        token: "secret",
        fromAddress: "orders@example.test",
        fromName: "Piqae",
        fetch: (async () =>
          Response.json({ success: false }, { status })) as typeof fetch,
      });
      await expect(client.send(message)).rejects.toMatchObject({ retryable });
    }
  });
});

describe("Shopify App Pricing", () => {
  it("builds the official hosted pricing URL", () => {
    expect(hostedPricingUrl(shop, "piqae-order-printing")).toBe(
      "https://admin.shopify.com/store/fixture-shop/charges/piqae-order-printing/pricing_plans",
    );
  });
  it("persists only a Partner-API-confirmed exact returned plan", async () => {
    await expect(
      confirmManagedPlan(
        {
          activeSubscription: vi.fn(async () => ({
            status: "ACTIVE",
            planHandle: "starter",
          })),
        },
        { appId: "app", shopId: "shop", returnedHandle: "starter" },
      ),
    ).resolves.toBe("starter");
    await expect(
      confirmManagedPlan(
        {
          activeSubscription: vi.fn(async () => ({
            status: "ACTIVE",
            planHandle: "growth",
          })),
        },
        { appId: "app", shopId: "shop", returnedHandle: "starter" },
      ),
    ).rejects.toThrow("NOT_CONFIRMED");
  });
});

describe("Shopify document experience", () => {
  it("offers the standard document library without mutating published revisions", () => {
    expect(templates.map(([name]) => name)).toEqual([
      "Invoice",
      "Packing slip",
      "Receipt",
      "Returns form",
      "Quote / pro forma",
      "Refund / credit note",
      "Gift receipt",
      "Delivery note",
    ]);
    expect(new Set(starterTemplates.map(({ id }) => id)).size).toBe(8);
    for (const template of starterTemplates) {
      expect(template.specification.spec_version).toBe("piqae.document/v1");
      expect(template.specification.body.length).toBeGreaterThan(0);
      expect(
        template.specification.body.some((node) => node.type === "repeat"),
      ).toBe(true);
    }
    expect(editorDocument.schema).toBe("piqae.document/v1");
    expect(editorDocument.nodes.some((node) => node.type === "repeat")).toBe(
      true,
    );
  });

  it("keeps advanced Liquid explicitly compatibility-gated", () => {
    expect(liquidCompatibilityNotice("visual")).toBeNull();
    expect(liquidCompatibilityNotice("liquid")).toContain(
      "Unsupported tags or filters",
    );
  });
});
