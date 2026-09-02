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
      if (query.includes("PiqaeProductLabelCurrency"))
        return Response.json({ data: { shop: { currencyCode: "NZD" } } });
      const id = (options?.variables as { id?: string })?.id ?? "";
      return Response.json({
        data: {
          node: id.includes("ProductVariant")
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
                  pageInfo: { hasNextPage: false, endCursor: null },
                },
              },
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
    expect(result.warnings).toEqual([]);
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

  it("uses baseline product data with a warning when optional taxonomy is rejected", async () => {
    const admin = productAdmin();
    const baseGraphql = admin.graphql as ReturnType<typeof vi.fn>;
    const original = baseGraphql.getMockImplementation() as (
      query: string,
      options?: { variables?: unknown },
    ) => Promise<Response>;
    baseGraphql.mockImplementation(
      async (query: string, options?: { variables?: unknown }) => {
        if (
          query.includes("PiqaeProductLabelResource(") &&
          !query.includes("Baseline")
        )
          return Response.json({
            errors: [
              {
                message: "Optional taxonomy field unavailable",
                extensions: { code: "GRAPHQL_VALIDATION_FAILED" },
              },
            ],
          });
        return original(query, options);
      },
    );

    const result = await fetchProductDocumentInput(admin, shop, [
      "gid://shopify/Product/7",
    ]);
    expect(result.documentCount).toBe(2);
    expect(result.warnings).toEqual([
      {
        code: "optional_product_data_unavailable",
        message: expect.stringContaining("standard Shopify product data"),
      },
    ]);
    expect(
      baseGraphql.mock.calls.some(([query]) =>
        query.includes("PiqaeProductLabelResourceBaseline"),
      ),
    ).toBe(true);
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
    ).rejects.toThrow("Select between 1 and 100 Shopify products or variants");
    expect(admin.graphql).not.toHaveBeenCalled();
  });

  it("paginates every variant instead of silently truncating a product", async () => {
    const admin = productAdmin();
    const baseGraphql = admin.graphql as ReturnType<typeof vi.fn>;
    baseGraphql.mockImplementation(
      async (query: string, options?: { variables?: unknown }) => {
        if (query.includes("PiqaeShopPrintIdentity"))
          return Response.json({
            data: {
              shop: {
                name: "Label Shop",
                contactEmail: null,
                primaryDomain: null,
                billingAddress: null,
              },
            },
          });
        if (query.includes("PiqaeProductLabelCurrency"))
          return Response.json({ data: { shop: { currencyCode: "NZD" } } });
        const after = (options?.variables as { after?: string | null })?.after;
        return Response.json({
          data: {
            node: {
              __typename: "Product",
              id: "gid://shopify/Product/7",
              title: "Krank Blend",
              vendor: "C4 Coffee",
              productType: "Coffee",
              tags: [],
              category: null,
              variants: after
                ? {
                    nodes: [
                      {
                        id: "gid://shopify/ProductVariant/12",
                        title: "500 g",
                        sku: "COF-500",
                        barcode: "942000000001",
                        price: "19.50",
                      },
                    ],
                    pageInfo: { hasNextPage: false, endCursor: null },
                  }
                : {
                    nodes: [
                      {
                        id: "gid://shopify/ProductVariant/11",
                        title: "250 g",
                        sku: "COF-250",
                        barcode: "",
                        price: "12.00",
                      },
                    ],
                    pageInfo: { hasNextPage: true, endCursor: "page-2" },
                  },
            },
          },
        });
      },
    );

    const result = await fetchProductDocumentInput(admin, shop, [
      "gid://shopify/Product/7",
    ]);

    expect(result.documentCount).toBe(2);
    expect(result.input.orders[0]?.lineItems.map(({ id }) => id)).toEqual([
      "gid://shopify/ProductVariant/11",
      "gid://shopify/ProductVariant/12",
    ]);
    const productQueries = baseGraphql.mock.calls.filter(([query]) =>
      query.includes("PiqaeProductLabelResource"),
    );
    expect(productQueries).toHaveLength(2);
    expect(productQueries[1]?.[1]).toMatchObject({
      variables: { id: "gid://shopify/Product/7", after: "page-2" },
    });
  });

  it("fails the complete selection when Shopify returns a missing resource", async () => {
    const admin = productAdmin();
    const baseGraphql = admin.graphql as ReturnType<typeof vi.fn>;
    const original = baseGraphql.getMockImplementation() as (
      query: string,
      options?: { variables?: unknown },
    ) => Promise<Response>;
    baseGraphql.mockImplementation(
      async (query: string, options?: { variables?: unknown }) => {
        if (
          query.includes("PiqaeProductLabelResource") &&
          (options?.variables as { id?: string })?.id ===
            "gid://shopify/Product/8"
        )
          return Response.json({ data: { node: null } });
        return original(query, options);
      },
    );

    await expect(
      fetchProductDocumentInput(admin, shop, [
        "gid://shopify/Product/7",
        "gid://shopify/Product/8",
      ]),
    ).rejects.toThrow("products or variants are unavailable");
  });

  it("uses one bounded resource query per selected product, not one nested bulk query", async () => {
    const admin = productAdmin();
    await fetchProductDocumentInput(admin, shop, [
      "gid://shopify/Product/7",
      "gid://shopify/Product/8",
    ]);

    const calls = (admin.graphql as ReturnType<typeof vi.fn>).mock.calls;
    expect(
      calls.filter(([query]) => query.includes("PiqaeProductLabelResource")),
    ).toHaveLength(2);
    expect(calls.some(([query]) => query.includes("nodes(ids:"))).toBe(false);
  });
});
