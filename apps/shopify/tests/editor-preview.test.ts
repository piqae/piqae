import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it, vi } from "vitest";

import {
  createEditorDraftPreview,
  EDITOR_PREVIEW_EXPIRES_SECONDS,
  fetchLatestOrderSummary,
} from "../app/core/editor-preview.server";
import { starterTemplates } from "../app/core/starter-templates";
import type { AdminGraphql } from "../app/core/orders.server";
import { EDITOR_PREVIEW_CLIENT_ERROR } from "../app/routes/app.templates.$templateId";

const shop = "fixture-shop.myshopify.com";
const latest = { id: "gid://shopify/Order/42" };
const order = {
  id: latest.id,
  name: "#1042",
  createdAt: "2026-08-30T00:00:00Z",
  currencyCode: "NZD",
  customer: null,
  shippingAddress: null,
  billingAddress: null,
  note: null,
  statusPageUrl: null,
  shippingLine: null,
  metafieldsByIdentifiers: [],
  lineItems: {
    nodes: [],
    pageInfo: { hasNextPage: false, endCursor: null },
  },
  subtotalPriceSet: { shopMoney: { amount: "0.00" } },
  totalTaxSet: { shopMoney: { amount: "0.00" } },
  totalPriceSet: { shopMoney: { amount: "0.00" } },
};

const render = (state: "registered" | "rendering" | "completed") => ({
  id: "pprv_1",
  purpose: "preview" as const,
  state,
  failure_code: null,
  expires_at: "2026-08-30T00:05:00Z",
  created_at: "2026-08-30T00:00:00Z",
  updated_at: "2026-08-30T00:00:00Z",
});

function adminForPreview(customerId?: string) {
  return {
    graphql: vi.fn<AdminGraphql["graphql"]>(async (query) =>
      Response.json(
        query.includes("PiqaeLatestPreviewOrder")
          ? { data: { orders: { nodes: [latest] } } }
          : {
              data: {
                order: customerId
                  ? {
                      ...order,
                      customer: {
                        id: customerId,
                        displayName: "Private buyer",
                        email: "private@example.test",
                      },
                    }
                  : order,
              },
            },
      ),
    ),
  };
}

describe("Shopify editor PDF preview", () => {
  it("keeps order hydration out of the editor loader and uses a private preview proxy", () => {
    const editorRoute = readFileSync(
      join(process.cwd(), "app/routes/app.templates.$templateId.tsx"),
      "utf8",
    );
    const loaderSource = editorRoute.slice(
      editorRoute.indexOf("export async function loader"),
      editorRoute.indexOf("export async function action"),
    );
    expect(loaderSource).not.toContain("fetchLatestOrderSummary");
    expect(EDITOR_PREVIEW_CLIENT_ERROR).not.toMatch(/connect|order|customer/i);

    const proxy = readFileSync(
      join(
        process.cwd(),
        "app/routes/api.editor-preview-renders.$renderId.artifact.tsx",
      ),
      "utf8",
    );
    expect(proxy).toContain("downloadPreviewDraftArtifact");
    expect(proxy).not.toContain("printPackets.renders.download(");
    expect(proxy).toContain('"cache-control": "private, no-store"');
    expect(proxy).toContain('"referrer-policy": "no-referrer"');
    expect(proxy).toContain('"x-content-type-options": "nosniff"');
  });

  it("queries only the newest order identifier", async () => {
    const admin = adminForPreview();

    await expect(fetchLatestOrderSummary(admin)).resolves.toEqual(latest);
    const query = admin.graphql.mock.calls[0]?.[0] ?? "";
    expect(query).toContain(
      "orders(first: 1, sortKey: CREATED_AT, reverse: true)",
    );
    expect(query).toContain("nodes { id }");
    expect(query).not.toMatch(
      /\bname\b|customer|email|address|lineItems|metafield|note|statusPage/i,
    );
  });

  it("returns no order without hydrating any order data", async () => {
    const admin = {
      graphql: vi.fn<AdminGraphql["graphql"]>(async () =>
        Response.json({ data: { orders: { nodes: [] } } }),
      ),
    };

    await expect(fetchLatestOrderSummary(admin)).resolves.toBeNull();
    expect(admin.graphql).toHaveBeenCalledTimes(1);
  });

  it("uploads draft assets before creating and waiting for a purpose-fenced preview", async () => {
    const customerId = "gid://shopify/Customer/7";
    const admin = adminForPreview(customerId);
    const calls: string[] = [];
    const bytes = new Uint8Array([1, 2, 3]);
    const digest = createHash("sha256").update(bytes).digest("hex");
    const putJpeg = vi.fn(async () => {
      calls.push("asset");
    });
    const createPreviewDraft = vi.fn(async () => {
      calls.push("create");
      return render("registered");
    });
    const retrievePreviewDraft = vi.fn(async () => {
      calls.push("retrieve");
      return render("completed");
    });
    const recordRender = vi.fn(async () => {
      calls.push("record");
    });
    const specification = structuredClone(starterTemplates[0]!.specification);

    await expect(
      createEditorDraftPreview({
        admin,
        shop,
        latestOrder: latest,
        specification,
        assets: [
          {
            id: "logo",
            digest,
            mediaType: "image/jpeg",
            bytes: bytes.byteLength,
            sourceUrl: "https://cdn.shopify.com/s/files/logo.jpg",
          },
        ],
        requestKey: "00000000-0000-4000-8000-000000000001",
        metafieldAllowlist: [],
        client: {
          printPackets: {
            resources: { putJpeg },
            renders: {
              createPreviewDraft,
              retrievePreviewDraft,
            },
          },
        } as never,
        renders: { recordRender },
        assetFetcher: vi.fn(async () => bytes),
        sleep: vi.fn(async () => undefined),
      }),
    ).resolves.toEqual({ renderId: "pprv_1" });

    expect(calls).toEqual(["asset", "create", "record", "retrieve"]);
    expect(recordRender).toHaveBeenCalledWith(
      shop,
      "pprv_1",
      expect.stringMatching(/^shopify-editor-preview-[a-f0-9]{64}$/),
      { orderGid: latest.id, customerGid: customerId },
    );
    expect(createPreviewDraft).toHaveBeenCalledWith(
      {
        specification,
        input: expect.objectContaining({
          shop: { name: "fixture-shop", domain: shop },
          orders: [expect.objectContaining({ id: latest.id })],
        }),
        expires_in_seconds: EDITOR_PREVIEW_EXPIRES_SECONDS,
      },
      expect.stringMatching(/^shopify-editor-preview-[a-f0-9]{64}$/),
    );
  });

  it("stops bounded polling when the browser request is aborted", async () => {
    const admin = adminForPreview();
    const controller = new AbortController();
    const retrievePreviewDraft = vi.fn(async () => render("rendering"));
    const recordRender = vi.fn(async () => undefined);

    await expect(
      createEditorDraftPreview({
        admin,
        shop,
        latestOrder: latest,
        specification: structuredClone(starterTemplates[0]!.specification),
        assets: [],
        requestKey: "00000000-0000-4000-8000-000000000002",
        metafieldAllowlist: [],
        client: {
          printPackets: {
            resources: { putJpeg: vi.fn() },
            renders: {
              createPreviewDraft: vi.fn(async () => render("registered")),
              retrievePreviewDraft,
            },
          },
        } as never,
        renders: { recordRender },
        signal: controller.signal,
        sleep: vi.fn(async () => controller.abort("left preview")),
      }),
    ).rejects.toBe("left preview");

    expect(recordRender).toHaveBeenCalledOnce();
    expect(retrievePreviewDraft).not.toHaveBeenCalled();
  });
});
