import { describe, expect, it, vi } from "vitest";

import { fetchProductDocumentInput } from "../app/core/products.server";
import type { AdminGraphql } from "../app/core/orders.server";

const shop = "labels.myshopify.com";

function productAdmin(): AdminGraphql {
  return {
    graphql: vi.fn(async (query: string, options?: { variables?: unknown }) => {
      if (query.includes("PiqaeShopPrintIdentity"))
        return Response.json({
          data: {
            shop: {
              name: "Label Shop",
              contactEmail: "labels@example.com",
              primaryDomain: { host: "labels.example.com" },
              billingAddress: null,
            },
          },
        });
      const ids = (options?.variables as { ids?: string[] })?.ids ?? [];
      return Response.json({
        data: {
          shop: { currencyCode: "NZD" },
          nodes: ids.map((id) =>
            id.includes("ProductVariant")
              ? {
                  __typename: "ProductVariant",
                  id,
                  title: "500 g",
                  sku: "COF-500",
                  barcode: "942000000001",
                  price: "19.50",
                  product: {
                    id: "gid://shopify/Product/7",
                    title: "Krank Blend",
                    vendor: "C4 Coffee",
                    productType: "Coffee",
                    tags: ["coffee", "retail"],
                    category: {
                      id: "gid://shopify/TaxonomyCategory/aa-1",
                      name: "Coffee",
                      fullName: "Food > Beverages > Coffee",
                      level: 3,
                      ancestorIds: ["gid://shopify/TaxonomyCategory/aa"],
                    },
                  },
                }
              : {
                  __typename: "Product",
                  id,
                  title: "Krank Blend",
                  vendor: "C4 Coffee",
                  productType: "Coffee",
                  tags: ["retail", "coffee", "coffee"],
                  category: null,
                  variants: {
                    nodes: [
                      {
                        id: "gid://shopify/ProductVariant/11",
                        title: "250 g",
                        sku: "COF-250",
                        barcode: "",
                        price: "12.00",
                      },
                      {
                        id: "gid://shopify/ProductVariant/12",
                        title: "500 g",
                        sku: "COF-500",
                        barcode: "942000000001",
                        price: "19.50",
                      },
                    ],
                  },
                },
          ),
        },
      });
    }) as AdminGraphql["graphql"],
  };
}

describe("Shopify product label data", () => {
  it("expands a selected product into one deterministic label per variant", async () => {
    const result = await fetchProductDocumentInput(productAdmin(), shop, [
      "gid://shopify/Product/7",
      "gid://shopify/Product/7",
    ]);

    expect(result.documentCount).toBe(2);
    expect(result.input.shop).toMatchObject({
      name: "Label Shop",
      domain: shop,
      primaryDomain: "labels.example.com",
    });
    expect(result.input.orders[0]?.lineItems).toMatchObject([
      {
        title: "Krank Blend",
        sku: "COF-250",
        labelCode128: "COF-250",
        quantity: 1,
        currency: "NZD",
      },
      {
        title: "Krank Blend",
        sku: "COF-500",
        labelCode128: "942000000001",
        quantity: 1,
        currency: "NZD",
      },
    ]);
  });

  it("prints only the selected variant from product-variant and POS actions", async () => {
    const result = await fetchProductDocumentInput(productAdmin(), shop, [
      "gid://shopify/ProductVariant/12",
    ]);

    expect(result.documentCount).toBe(1);
    expect(result.input.orders[0]?.lineItems[0]).toMatchObject({
      id: "gid://shopify/ProductVariant/12",
      title: "Krank Blend",
      sku: "COF-500",
      labelCode128: "942000000001",
      unitPrice: 19.5,
    });
  });

  it("rejects IDs outside Shopify products and variants before GraphQL", async () => {
    const admin = productAdmin();
    await expect(
      fetchProductDocumentInput(admin, shop, ["gid://shopify/Order/42"]),
    ).rejects.toThrow("Select at least one Shopify product or variant");
    expect(admin.graphql).not.toHaveBeenCalled();
  });
});
