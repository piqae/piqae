import { describe, expect, it, vi } from "vitest";
import { CredentialVault } from "../app/core/credentials.server";
import {
  fetchOrders,
  normalizedLabelCode128Candidate,
  normalizeMoneyAmount,
  normalizeOrderGid,
  parseShopifyDataBindings,
  shopifyDocumentInput,
  type AdminGraphql,
} from "../app/core/orders.server";
import {
  SHOPIFY_DOCUMENT_FIELDS,
  shopifyCustomDocumentFields,
} from "../app/core/shopify-document-fields";
import { MemoryShopRepository, normalizeShopDomain } from "../app/core/model";
import {
  parseRenderCost,
  ShopifyPrintingService,
} from "../app/core/printing.server";
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
import {
  canSubmitTemplateMode,
  customizedTemplateName,
  editorLiquidForMode,
  liquidCompatibilityNotice,
  mediaForPageSize,
  mediaPresetForDocument,
  pageSizeForDocument,
} from "../app/routes/app.templates.$templateId";
import { customizedSystemDraft, templates } from "../app/routes/app.templates";
import { selectedOrderIds } from "../app/routes/app.print";
import { starterTemplates } from "../app/core/starter-templates";
import { MemoryWorkflowRepository } from "../app/core/workflows.server";
import {
  parseTemplateEnvelope,
  removeSystemOwnership,
  serializeTemplateEnvelope,
} from "../app/core/template-model";
import { templateDigest } from "../app/core/template-digest.server";
import { seedStarterTemplates } from "../app/core/template-index.server";
import { ACCOUNT_DEFAULT_DOCUMENT_ID } from "../app/core/admin-print-options.server";

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
        id: "gid://shopify/LineItem/8",
        title: "Coffee",
        sku: "COF",
        quantity: 2,
        originalUnitPriceSet: { shopMoney: { amount: "10.00" } },
        discountedTotalSet: { shopMoney: { amount: "20.00" } },
        product: {
          id: "gid://shopify/Product/7",
          title: "Coffee",
          vendor: "C4 Coffee",
          productType: "Coffee",
          category: {
            id: "gid://shopify/TaxonomyCategory/aa-1",
            name: "Coffee",
            fullName: "Food > Beverages > Coffee",
            level: 3,
            ancestorIds: ["gid://shopify/TaxonomyCategory/aa"],
          },
          metafieldsByIdentifiers: [
            {
              namespace: "custom",
              key: "origin",
              type: "metaobject_reference",
              jsonValue: "gid://shopify/Metaobject/9",
              reference: {
                id: "gid://shopify/Metaobject/9",
                type: "coffee_origin",
                handle: "ethiopia",
                displayName: "Ethiopia",
                fields: [
                  {
                    key: "country",
                    type: "single_line_text_field",
                    jsonValue: "Ethiopia",
                  },
                  {
                    key: "internal_cost",
                    type: "number_decimal",
                    jsonValue: 4.2,
                  },
                ],
              },
            },
          ],
        },
        variant: {
          id: "gid://shopify/ProductVariant/11",
          title: "500g / Whole Beans",
          barcode: "942000000001",
          metafieldsByIdentifiers: [],
        },
      },
    ],
  },
  metafieldsByIdentifiers: [],
  subtotalPriceSet: { shopMoney: { amount: "20.00" } },
  totalTaxSet: { shopMoney: { amount: "3.00" } },
  totalPriceSet: { shopMoney: { amount: "23.00" } },
};
const admin = {
  graphql: vi.fn<AdminGraphql["graphql"]>(async () =>
    Response.json({ data: { order } }),
  ),
};

async function publishPinnedDocument(
  workflow: MemoryWorkflowRepository,
  input: {
    id: string;
    piqaeRevisionId: string;
    targetId: string;
    specificationRevision: string;
    accountId?: string;
  },
) {
  const starter = starterTemplates[0]!;
  const envelope = parseTemplateEnvelope(starter.source);
  envelope.published = {
    piqaeAccountId: input.accountId ?? "acct_1",
    piqaeEnvironmentId: null,
    piqaeTemplateId: `piqae_${input.id}`,
    piqaeRevisionId: input.piqaeRevisionId,
    canonicalDigest: templateDigest(JSON.stringify(envelope.document)),
  };
  return workflow.saveTemplate(shop, {
    ...starter,
    id: input.id,
    source: serializeTemplateEnvelope(envelope),
    state: "published",
    revision: 1,
    designTargetId: input.targetId,
    designSpecificationRevision: input.specificationRevision,
  });
}

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
    expect(value?.lineItems[0]?.total).toBe(20);
    expect(value?.lineItems[0]?.labelCode128).toBe("942000000001");
    expect(value?.total).toBe(23);
    expect(typeof value?.total).toBe("number");
  });
  it("handles Shopify's string-form HTTP error without masking it as a TypeError", async () => {
    const rejectedAdmin = {
      graphql: vi.fn<AdminGraphql["graphql"]>(async () =>
        Response.json(
          { errors: "The stored Admin API credential is invalid" },
          { status: 403 },
        ),
      ),
    };
    await expect(fetchOrders(rejectedAdmin, ["42"])).rejects.toThrow(
      "Shopify Admin API failed (403)",
    );
  });
  it("normalizes only Code128 candidates that fit the fixed product label", () => {
    expect(normalizedLabelCode128Candidate("VALID-BARCODE", "VALID-SKU")).toBe(
      "VALID-BARCODE",
    );
    expect(normalizedLabelCode128Candidate("bār", "  VALID-SKU  ")).toBe(
      "VALID-SKU",
    );
    expect(normalizedLabelCode128Candidate("", "ÜNICODE-SKU")).toBeNull();
    expect(normalizedLabelCode128Candidate("A".repeat(35), "fallback")).toBe(
      "A".repeat(35),
    );
    expect(
      normalizedLabelCode128Candidate("A".repeat(36), "DENSE-FALLBACK"),
    ).toBe("DENSE-FALLBACK");
    expect(
      normalizedLabelCode128Candidate("A".repeat(36), "B".repeat(36)),
    ).toBeNull();
    expect(
      normalizedLabelCode128Candidate("A".repeat(81), "B".repeat(81)),
    ).toBeNull();
    expect(normalizedLabelCode128Candidate("\n", "")).toBeNull();
  });
  it("rejects an RFC3339-shaped order timestamp with an impossible calendar date", async () => {
    const invalidDateAdmin = {
      graphql: vi.fn<AdminGraphql["graphql"]>(async () =>
        Response.json({
          data: { order: { ...order, createdAt: "2026-02-30T00:00:00Z" } },
        }),
      ),
    };
    await expect(fetchOrders(invalidDateAdmin, ["42"])).rejects.toThrow(
      "timestamp is invalid",
    );
  });
  it("normalizes Shopify decimals into the bounded canonical numeric contract", () => {
    expect(normalizeMoneyAmount("0.000001")).toBe(0.000001);
    expect(normalizeMoneyAmount("900719925.474099")).toBe(900719925.474099);
    expect(normalizeMoneyAmount("-0.00")).toBe(0);
    for (const invalid of [
      "01.00",
      "1e3",
      "1.0000001",
      "9007199254.740991",
      Number.POSITIVE_INFINITY,
    ]) {
      expect(() => normalizeMoneyAmount(invalid)).toThrow("Shopify money");
    }
  });
  it("builds the canonical shop/orders render root and rejects non-Shopify identity", async () => {
    const [normalized] = await fetchOrders(admin, ["42"]);
    const input = shopifyDocumentInput(shop, [normalized!], "Fixture Shop");
    expect(input).toMatchObject({
      shop: { name: "Fixture Shop", domain: shop },
      orders: [{ name: "#1042", total: 23 }],
    });
    expect(() =>
      shopifyDocumentInput("attacker.example", [normalized!]),
    ).toThrow("invalid Shopify shop domain");
  });
  it("normalizes taxonomy and only explicitly allowlisted custom data", async () => {
    admin.graphql.mockClear();
    const bindings = parseShopifyDataBindings([
      "product:custom.origin.country",
    ]);
    const [value] = await fetchOrders(admin, ["42"], bindings);
    expect(value?.lineItems[0]?.product?.category?.fullName).toBe(
      "Food > Beverages > Coffee",
    );
    expect(
      value?.lineItems[0]?.product?.metafields.custom?.origin?.reference
        ?.fields,
    ).toEqual({ country: "Ethiopia" });
    expect(
      value?.lineItems[0]?.product?.metafields.custom?.origin?.reference?.fields
        .internal_cost,
    ).toBeUndefined();
    expect(admin.graphql.mock.calls[0]?.[1]).toMatchObject({
      variables: {
        productFields: [{ namespace: "custom", key: "origin" }],
      },
    });
  });

  it("rejects broad or excessive Shopify data bindings", async () => {
    expect(() => parseShopifyDataBindings(["product:*.origin"])).toThrow(
      "invalid Shopify metafield binding",
    );
    const tooMany = Array.from(
      { length: 21 },
      (_, index) => `custom.field_${index}`,
    );
    await expect(
      fetchOrders(admin, ["42"], { order: tooMany }),
    ).rejects.toThrow("order metafield binding limit exceeded");
  });

  it("exposes Shopify taxonomy and allowlisted custom data as generic document paths", () => {
    expect(
      SHOPIFY_DOCUMENT_FIELDS.find(({ label }) => label === "Shop name")?.path,
    ).toBe("shop.name");
    expect(
      SHOPIFY_DOCUMENT_FIELDS.find(({ label }) => label === "Item total")?.path,
    ).toBe("item.total");
    expect(
      SHOPIFY_DOCUMENT_FIELDS.find(
        ({ label }) => label === "Label barcode (Code 128 safe)",
      )?.path,
    ).toBe("item.labelCode128");
    expect(
      SHOPIFY_DOCUMENT_FIELDS.find(
        ({ label }) => label === "Shopify category ID",
      )?.path,
    ).toBe("item.product.category.id");
    expect(
      shopifyCustomDocumentFields(["product:custom.origin.country"]).map(
        ({ path }) => path,
      ),
    ).toEqual([
      "item.product.metafields.custom.origin.value",
      "item.product.metafields.custom.origin.reference.fields.country",
    ]);
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
    const workflow = new MemoryWorkflowRepository();
    const vault = new CredentialVault(Buffer.alloc(32, 9));
    await repository.put({
      shop,
      piqaeAccountId: "acct_1",
      encryptedCredential: vault.seal("token", shop),
      templateRevisionId: "rev_1",
      createdAt: new Date().toISOString(),
    });
    await publishPinnedDocument(workflow, {
      id: "invoice",
      piqaeRevisionId: "rev_1",
      targetId: "tgt_orders",
      specificationRevision: "spec_orders_4",
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
          printPackets: {
            renders: { create, print, retrieve: vi.fn() },
            templates: {},
            conversions: {},
            renderAndPrint: vi.fn(),
          },
        }) as never,
      "https://app.example",
      undefined,
      workflow,
    );
    const result = await service.printOrders({
      admin,
      shop,
      orderIds: ["42"],
      targetId: "tgt_orders",
      targetSpecificationRevision: "spec_orders_4",
      templateId: "invoice",
      requestKey: "click-1",
    });
    expect(result).toEqual({
      mode: "direct",
      renderId: "render_1",
      jobId: "job_1",
    });
    expect(create.mock.calls[0]?.[1]).toMatch(/^shopify-render-[a-f0-9]{64}$/);
    expect(print.mock.calls[0]?.[2]).toMatch(
      /^shopify-print-[a-f0-9]{64}-tgt_orders$/,
    );
    expect(print.mock.calls[0]?.[1]).toMatchObject({
      target_id: "tgt_orders",
      specification_revision: "spec_orders_4",
      render_policy: "automatic",
    });
    expect(create.mock.calls[0]?.[0]).toMatchObject({
      input: {
        shop: { name: "fixture-shop", domain: shop },
        orders: [{ name: "#1042", total: 23 }],
      },
    });
    await expect(
      service.printOrders({
        admin,
        shop,
        orderIds: ["42"],
        targetId: "tgt_orders",
        targetSpecificationRevision: "spec_orders_5",
        templateId: "invoice",
        requestKey: "changed-target",
      }),
    ).rejects.toThrow("changed after this document was published");
    expect(create).toHaveBeenCalledTimes(1);

    for (const renderExecutionPolicy of [
      "prefer_node",
      "require_node",
    ] as const) {
      await workflow.saveSettings(shop, {
        ...(await workflow.getSettings(shop)),
        renderExecutionPolicy,
      });
      await expect(
        service.printOrders({
          admin,
          shop,
          orderIds: ["42"],
          targetId: "tgt_orders",
          targetSpecificationRevision: "spec_orders_4",
          templateId: "invoice",
          requestKey: `policy-${renderExecutionPolicy}`,
        }),
      ).resolves.toMatchObject({ mode: "direct" });
      expect(print.mock.calls.at(-1)?.[1]).toMatchObject({
        target_id: "tgt_orders",
        specification_revision: "spec_orders_4",
        render_policy: renderExecutionPolicy,
      });
    }
  });
  it("fails closed when a selected published document has no pinned Piqae revision", async () => {
    const repository = new MemoryShopRepository();
    const workflow = new MemoryWorkflowRepository();
    const vault = new CredentialVault(Buffer.alloc(32, 9));
    await repository.put({
      shop,
      piqaeAccountId: "acct_1",
      encryptedCredential: vault.seal("token", shop),
      templateRevisionId: "fallback_revision",
      createdAt: new Date().toISOString(),
    });
    await workflow.saveTemplate(shop, {
      ...starterTemplates[0]!,
      state: "published",
      revision: 1,
    });
    const service = new ShopifyPrintingService(
      repository,
      vault,
      () => ({ printPackets: {} }) as never,
      "https://app.example",
      undefined,
      workflow,
    );
    await expect(
      service.previewOrders({
        admin,
        shop,
        orderIds: ["42"],
        templateId: starterTemplates[0]!.id,
        requestKey: "preview-unpinned",
      }),
    ).rejects.toThrow("has no pinned Piqae revision");
  });
  it.each([
    {
      accountId: "acct_other",
      environmentId: null,
      digest: "current",
      error: "different Piqae account",
    },
    {
      accountId: "acct_1",
      environmentId: null,
      digest: "b".repeat(64),
      error: "no longer matches its pinned Piqae revision",
    },
    {
      accountId: "acct_1",
      environmentId: "env_other",
      digest: "current",
      error: "different Piqae environment",
    },
  ])("fails closed on a mismatched publication context", async (fixture) => {
    const repository = new MemoryShopRepository();
    const workflow = new MemoryWorkflowRepository();
    const vault = new CredentialVault(Buffer.alloc(32, 9));
    await repository.put({
      shop,
      piqaeAccountId: "acct_1",
      encryptedCredential: vault.seal("token", shop),
      templateRevisionId: "fallback_revision",
      createdAt: new Date().toISOString(),
    });
    const starter = starterTemplates[0]!;
    const envelope = parseTemplateEnvelope(starter.source);
    envelope.published = {
      piqaeAccountId: fixture.accountId,
      piqaeEnvironmentId: fixture.environmentId,
      piqaeTemplateId: "template_context",
      piqaeRevisionId: "revision_context",
      canonicalDigest:
        fixture.digest === "current"
          ? templateDigest(JSON.stringify(envelope.document))
          : fixture.digest,
    };
    const saved = await workflow.saveTemplate(shop, {
      ...starter,
      id: "00000000-0000-4000-8000-000000008888",
      source: serializeTemplateEnvelope(envelope),
      state: "published",
      revision: 1,
    });
    const service = new ShopifyPrintingService(
      repository,
      vault,
      () => ({ printPackets: {} }) as never,
      "https://app.example",
      undefined,
      workflow,
    );
    await expect(
      service.previewOrders({
        admin,
        shop,
        orderIds: ["42"],
        templateId: saved.id,
        requestKey: "preview-context-mismatch",
      }),
    ).rejects.toThrow(fixture.error);
  });
  it("pins POS receipt rendering to the published canonical receipt revision", async () => {
    const repository = new MemoryShopRepository();
    const workflow = new MemoryWorkflowRepository();
    const vault = new CredentialVault(Buffer.alloc(32, 9));
    await repository.put({
      shop,
      piqaeAccountId: "acct_1",
      encryptedCredential: vault.seal("token", shop),
      templateRevisionId: "invoice_revision",
      createdAt: new Date().toISOString(),
    });
    const receipt = starterTemplates.find(({ id }) => id === "receipt")!;
    const envelope = parseTemplateEnvelope(receipt.source);
    envelope.published = {
      piqaeAccountId: "acct_1",
      piqaeEnvironmentId: null,
      piqaeTemplateId: "receipt_template",
      piqaeRevisionId: "receipt_revision",
      canonicalDigest: templateDigest(JSON.stringify(envelope.document)),
    };
    await workflow.saveTemplate(shop, {
      ...receipt,
      id: "00000000-0000-4000-8000-000000000003",
      source: serializeTemplateEnvelope(envelope),
      state: "published",
      revision: 1,
    });
    const cloneEnvelope = parseTemplateEnvelope(receipt.source);
    cloneEnvelope.published = {
      piqaeAccountId: "acct_1",
      piqaeEnvironmentId: null,
      piqaeTemplateId: "receipt_clone_template",
      piqaeRevisionId: "receipt_clone_revision",
      canonicalDigest: templateDigest(JSON.stringify(cloneEnvelope.document)),
    };
    await workflow.saveTemplate(shop, {
      ...receipt,
      id: "00000000-0000-4000-8000-000000009999",
      source: serializeTemplateEnvelope(cloneEnvelope),
      state: "published",
      revision: 1,
    });
    await seedStarterTemplates(workflow, shop);
    const create = vi.fn(async () => ({
      id: "receipt_render",
      state: "completed",
      failure_code: null,
    }));
    const service = new ShopifyPrintingService(
      repository,
      vault,
      () =>
        ({
          printPackets: {
            renders: { create },
          },
        }) as never,
      "https://app.example",
      undefined,
      workflow,
    );
    await expect(
      service.printOrders({
        admin,
        shop,
        orderIds: ["42"],
        systemTemplateKey: "receipt",
      }),
    ).resolves.toMatchObject({ mode: "download", renderId: "receipt_render" });
    expect(create).toHaveBeenCalledWith(
      expect.objectContaining({ template_revision_id: "receipt_revision" }),
      expect.stringMatching(/^shopify-render-/),
    );
  });
  it("approves the exact rendered preview without rendering again", async () => {
    const repository = new MemoryShopRepository();
    const workflow = new MemoryWorkflowRepository();
    const vault = new CredentialVault(Buffer.alloc(32, 7));
    await repository.put({
      shop,
      piqaeAccountId: "acct_preview",
      encryptedCredential: vault.seal("token", shop),
      templateRevisionId: "rev_preview",
      createdAt: new Date().toISOString(),
    });
    await publishPinnedDocument(workflow, {
      id: "preview-invoice",
      piqaeRevisionId: "rev_preview",
      targetId: "tgt_orders",
      specificationRevision: "spec_orders_4",
      accountId: "acct_preview",
    });
    const createRender = vi.fn(async () => ({
      id: "render_preview",
      state: "completed",
      failure_code: null,
      artifact_byte_length: 48_000,
      page_count: 2,
    }));
    const createPreview = vi.fn(async () => ({
      id: "preview_1",
      render_id: "render_preview",
      state: "awaiting_approval",
      expires_at: new Date(Date.now() + 60_000).toISOString(),
    }));
    const approve = vi.fn(async () => ({
      preview: { state: "approved" },
      job: { id: "job_preview" },
    }));
    const readiness = vi.fn(async () => ({
      requested_policy: "automatic",
      selected_mode: "cloud_pdf",
      reason: "automatic_missing_measurements",
      destination: {
        supported: true,
        ready: false,
        missing_resources: ["a".repeat(64)],
        reason: "resources_not_cached",
      },
      estimates: { cloud_ms: 0, node_ms: 0 },
    }));
    const client = {
      printPackets: {
        renders: {
          create: createRender,
          retrieve: vi.fn(async () => ({
            id: "render_preview",
            state: "completed",
            template_revision_id: "rev_preview",
          })),
          print: vi.fn(),
          readiness,
        },
        previews: {
          create: createPreview,
          retrieve: vi.fn(async () => ({ render_id: "render_preview" })),
          approve,
        },
      },
    } as never;
    const service = new ShopifyPrintingService(
      repository,
      vault,
      () => client,
      "https://app.example",
      undefined,
      workflow,
    );
    const preview = await service.previewOrders({
      admin,
      shop,
      orderIds: ["42"],
      templateId: "preview-invoice",
      requestKey: "preview-click",
    });
    const result = await service.approvePreview({
      shop,
      previewId: preview.previewId,
      renderId: preview.renderId,
      targetId: "tgt_orders",
      targetSpecificationRevision: "spec_orders_4",
      templateId: "preview-invoice",
      requestKey: "approve-click",
      renderCost: preview.renderCost,
    });
    expect(result).toEqual({ jobId: "job_preview", state: "approved" });
    expect(createRender).toHaveBeenCalledTimes(1);
    expect(createPreview).toHaveBeenCalledWith(
      "render_preview",
      { expires_in_seconds: 900 },
      expect.stringMatching(/^shopify-preview-/),
    );
    expect(approve).toHaveBeenCalledWith(
      "preview_1",
      expect.objectContaining({
        target_id: "tgt_orders",
        specification_revision: "spec_orders_4",
        render_policy: "automatic",
        render_cost: expect.objectContaining({
          document_count: 1,
          page_count: 2,
          pdf_bytes: 48_000,
        }),
      }),
      "approve-click",
    );
    await expect(
      service.renderReadiness({
        shop,
        renderId: preview.renderId,
        printerId: "printer_1",
        renderCost: preview.renderCost,
      }),
    ).resolves.toMatchObject({
      destination: { ready: false, reason: "resources_not_cached" },
    });
    expect(readiness).toHaveBeenCalledWith("render_preview", {
      printer_id: "printer_1",
      render_policy: "automatic",
      render_cost: expect.objectContaining({
        document_count: 1,
        page_count: 2,
        pdf_bytes: 48_000,
      }),
    });
    await expect(
      service.approvePreview({
        shop,
        previewId: preview.previewId,
        renderId: preview.renderId,
        printerId: "printer_1",
        templateId: "preview-invoice",
        requestKey: "approve-pinned-as-printer",
      }),
    ).rejects.toThrow("pinned to a print target");
    await expect(
      service.approvePreview({
        shop,
        previewId: preview.previewId,
        renderId: preview.renderId,
        targetId: "tgt_orders",
        targetSpecificationRevision: "spec_orders_5",
        templateId: "preview-invoice",
        requestKey: "approve-after-drift",
      }),
    ).rejects.toThrow("changed after this document was published");
    expect(approve).toHaveBeenCalledTimes(1);
  });

  it("approves an unpinned account document by printer with current defaults", async () => {
    const repository = new MemoryShopRepository();
    const workflow = new MemoryWorkflowRepository();
    const vault = new CredentialVault(Buffer.alloc(32, 6));
    await repository.put({
      shop,
      piqaeAccountId: "acct_current_defaults",
      encryptedCredential: vault.seal("token", shop),
      templateRevisionId: "rev_current_defaults",
      createdAt: new Date().toISOString(),
    });
    await repository.recordRender(
      shop,
      "render_current_defaults",
      "preview-current-defaults",
    );
    const approve = vi.fn(
      async (
        _previewId: string,
        _input: Record<string, unknown>,
        _requestKey: string,
      ) => ({
        preview: { state: "approved" },
        job: { id: "job_current_defaults" },
      }),
    );
    const service = new ShopifyPrintingService(
      repository,
      vault,
      () =>
        ({
          printPackets: {
            renders: {
              retrieve: vi.fn(async () => ({
                id: "render_current_defaults",
                state: "completed",
                template_revision_id: "rev_current_defaults",
              })),
            },
            previews: {
              retrieve: vi.fn(async () => ({
                render_id: "render_current_defaults",
              })),
              approve,
            },
          },
        }) as never,
      "https://app.example",
      undefined,
      workflow,
    );

    await expect(
      service.approvePreview({
        shop,
        previewId: "preview_current_defaults",
        renderId: "render_current_defaults",
        printerId: "printer_office",
        templateId: ACCOUNT_DEFAULT_DOCUMENT_ID,
        requestKey: "approve-current-defaults",
      }),
    ).resolves.toEqual({
      jobId: "job_current_defaults",
      state: "approved",
    });
    expect(approve).toHaveBeenCalledWith(
      "preview_current_defaults",
      expect.objectContaining({ printer_id: "printer_office" }),
      "approve-current-defaults",
    );
    expect(approve.mock.calls[0]?.[1]).not.toHaveProperty("target_id");
  });

  it("keeps canonical document media authoritative for editor sizing", () => {
    const label = mediaForPageSize("100x50mm");
    expect(label).toMatchObject({
      kind: "label",
      width_mm: 100,
      height_mm: 50,
    });
    expect(
      pageSizeForDocument({
        format: "printpacket/v1",
        media: label,
        body: [],
      }),
    ).toBe("100x50mm label");
    expect(
      mediaPresetForDocument({
        format: "printpacket/v1",
        media: label,
        body: [],
      }),
    ).toBe("100x50mm");
    expect(mediaForPageSize("80mm")).toMatchObject({
      kind: "continuous",
      width_mm: 80,
    });
    expect(mediaForPageSize("custom-label")).toMatchObject({
      kind: "label",
      width_mm: 62,
      height_mm: 29,
    });
  });

  it("accepts only bounded integral render measurements", () => {
    expect(
      parseRenderCost({
        document_count: 250,
        page_count: 300,
        pdf_bytes: 12_000_000,
        input_bytes: 800_000,
      }),
    ).toMatchObject({ document_count: 250, page_count: 300 });
    expect(() =>
      parseRenderCost({
        document_count: 10_001,
        page_count: 1,
        pdf_bytes: 1,
        input_bytes: 1,
      }),
    ).toThrow("render cost is invalid");
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
    const middle = Math.floor(token.length / 2);
    const tampered = `${token.slice(0, middle)}${token[middle] === "A" ? "B" : "A"}${token.slice(middle + 1)}`;
    expect(() => vault.open(tampered)).toThrow();
    const previewToken = vault.issuePreview({
      shop,
      renderId: "render_2",
      previewId: "preview_2",
    });
    expect(vault.openPreview(previewToken)).toMatchObject({
      shop,
      renderId: "render_2",
      previewId: "preview_2",
    });
    expect(() => vault.open(previewToken)).toThrow();
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
    ]);
    expect(new Set(starterTemplates.map(({ id }) => id)).size).toBe(4);
    for (const template of starterTemplates) {
      expect(template.specification.format).toBe("printpacket/v1");
      expect(template.specification.body.length).toBeGreaterThan(0);
    }
    expect(starterTemplates.find(({ id }) => id === "receipt")?.pageSize).toBe(
      "80mm",
    );
    expect(
      starterTemplates.find(({ id }) => id === "product-label")?.pageSize,
    ).toBe("100x50mm");
    expect(editorDocument.format).toBe("printpacket/v1");
  });

  it("customizes an immutable system document into an editable draft", () => {
    const starter = starterTemplates[0]!;
    const draft = customizedSystemDraft(
      {
        id: starter.id,
        name: starter.name,
        kind: starter.kind,
        pageSize: starter.pageSize,
        state: "published",
        source: starter.source,
        revision: 1,
        draftRevision: 1,
        published: {
          revision: 1,
          name: starter.name,
          kind: starter.kind,
          pageSize: starter.pageSize,
          source: starter.source,
          designTargetId: null,
          designSpecificationRevision: null,
          media: starter.specification.media,
        },
        updatedAt: "2026-08-10T00:00:00.000Z",
      },
      "draft-id",
    );
    expect(draft).toMatchObject({
      id: "draft-id",
      name: `${starter.name} — customized`,
      state: "draft",
      revision: 1,
    });
    const customized = parseTemplateEnvelope(draft.source);
    expect(customized.system).toBeUndefined();
    expect(customized.published).toBeUndefined();
    expect(() =>
      customizedSystemDraft(
        {
          ...draft,
          draftRevision: 1,
          published: null,
          updatedAt: "2026-08-10T00:00:00.000Z",
        },
        "again",
      ),
    ).toThrow("Only system documents");
  });

  it("keeps advanced Liquid explicitly compatibility-gated", () => {
    expect(liquidCompatibilityNotice("visual")).toBeNull();
    expect(liquidCompatibilityNotice("liquid")).toContain(
      "Unsupported constructs",
    );
    expect(canSubmitTemplateMode("visual", undefined)).toBe(false);
    expect(canSubmitTemplateMode("visual", {})).toBe(true);
    expect(canSubmitTemplateMode("source", undefined)).toBe(false);
    expect(customizedTemplateName("x".repeat(200))).toHaveLength(200);
    expect(customizedTemplateName("Invoice")).toBe("Invoice — customized");
    expect(editorLiquidForMode("visual", "{{ kept }}")).toBe("{{ kept }}");
    expect(editorLiquidForMode("source", "{{ kept }}")).toBe("{{ kept }}");
    expect(editorLiquidForMode("liquid", "{{ order.name }}")).toBe(
      "{{ order.name }}",
    );
    const ownedEnvelope = parseTemplateEnvelope(starterTemplates[0]!.source);
    ownedEnvelope.published = {
      piqaeAccountId: "account_owned",
      piqaeEnvironmentId: null,
      piqaeTemplateId: "template_owned",
      piqaeRevisionId: "revision_owned",
      canonicalDigest: templateDigest(JSON.stringify(ownedEnvelope.document)),
    };
    expect(ownedEnvelope.system).toBeDefined();
    const portable = removeSystemOwnership(ownedEnvelope);
    expect(portable.system).toBeUndefined();
    expect(portable.published).toBeUndefined();
  });
});
