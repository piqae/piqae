import { describe, expect, it } from "vitest";

import {
  orderPrintSequence,
  parsePrintOrderSettings,
} from "../app/core/print-order";
import type { NormalizedOrder } from "../app/core/orders.server";

function order(
  id: string,
  customer: string,
  lines: Array<{
    title: string;
    quantity: number;
    category: string;
    tags?: string[];
  }>,
  tags: string[] = [],
): NormalizedOrder {
  return {
    id,
    name: id,
    createdAt: "2026-09-02T00:00:00Z",
    currency: "NZD",
    customer: { id: `customer-${customer}`, displayName: customer, email: "" },
    shippingAddress: null,
    billingAddress: null,
    note: "",
    shippingMethod: "",
    statusUrl: "",
    tags,
    metafields: {},
    lineItems: lines.map((line, index) => ({
      id: `${id}-${index}`,
      title: line.title,
      sku: "",
      labelCode128: null,
      quantity: line.quantity,
      unitPrice: 1,
      total: line.quantity,
      currency: "NZD",
      product: {
        id: `product-${line.title}`,
        title: line.title,
        vendor: "C4",
        productType: "Coffee",
        tags: line.tags ?? [],
        category: {
          id: `category-${line.category}`,
          name: line.category.split(" > ").at(-1)!,
          fullName: line.category,
          level: line.category.split(" > ").length,
          ancestorIds: [],
        },
        metafields: {},
      },
      variant: null,
    })),
    subtotal: 1,
    tax: 0,
    total: 1,
  };
}

describe("print order grouping", () => {
  it("preserves selection order when no hierarchy is enabled", () => {
    const selected = [order("second", "B", []), order("first", "A", [])];
    expect(
      orderPrintSequence(selected, {
        hierarchy: [],
        taxonomyDepth: "family",
        mixedOrderMode: "dominant",
      }).map(({ id }) => id),
    ).toEqual(["second", "first"]);
  });

  it("uses item quantity for a mixed order's dominant taxonomy group", () => {
    const selected = [
      order("tea", "A", [
        {
          title: "Breakfast tea",
          quantity: 1,
          category: "Food > Beverages > Tea",
        },
      ]),
      order("mostly-coffee", "B", [
        {
          title: "Krank",
          quantity: 4,
          category: "Food > Beverages > Coffee",
        },
        {
          title: "Cup",
          quantity: 1,
          category: "Home > Kitchen > Drinkware",
        },
      ]),
      order("home", "C", [
        {
          title: "Cup",
          quantity: 2,
          category: "Home > Kitchen > Drinkware",
        },
      ]),
    ];
    expect(
      orderPrintSequence(selected, {
        hierarchy: ["taxonomy"],
        taxonomyDepth: "specific",
        mixedOrderMode: "dominant",
      }).map(({ id }) => id),
    ).toEqual(["mostly-coffee", "tea", "home"]);
  });

  it("applies the configured hierarchy in order and remains stable on ties", () => {
    const selected = [
      order("b", "Zed", [
        { title: "Coffee", quantity: 1, category: "Food > Beverages" },
      ]),
      order("a", "Amy", [
        { title: "Coffee", quantity: 1, category: "Food > Beverages" },
      ]),
      order("c", "Amy", [
        { title: "Tea", quantity: 1, category: "Food > Beverages" },
      ]),
    ];
    expect(
      orderPrintSequence(selected, {
        hierarchy: ["primary_product", "customer"],
        taxonomyDepth: "family",
        mixedOrderMode: "contains",
      }).map(({ id }) => id),
    ).toEqual(["a", "b", "c"]);
  });

  it("keeps primary product dominant even when mixed groups use contains", () => {
    const selected = [
      order("mixed-first", "A", [
        { title: "Coffee", quantity: 3, category: "Food > Beverages" },
        { title: "Tea", quantity: 1, category: "Food > Beverages" },
      ]),
      order("coffee", "B", [
        { title: "Coffee", quantity: 1, category: "Food > Beverages" },
      ]),
      order("tea", "C", [
        { title: "Tea", quantity: 1, category: "Food > Beverages" },
      ]),
    ];
    expect(
      orderPrintSequence(selected, {
        hierarchy: ["primary_product"],
        taxonomyDepth: "family",
        mixedOrderMode: "contains",
      }).map(({ id }) => id),
    ).toEqual(["mixed-first", "coffee", "tea"]);
  });

  it("does not merge different customers who share a display name", () => {
    const firstTea = order("first-tea", "Same name", [
      { title: "Tea", quantity: 1, category: "Food > Beverages" },
    ]);
    const secondCoffee = order("second-coffee", "Same name", [
      { title: "Coffee", quantity: 1, category: "Food > Beverages" },
    ]);
    const firstCoffee = order("first-coffee", "Same name", [
      { title: "Coffee", quantity: 1, category: "Food > Beverages" },
    ]);
    firstTea.customer!.id = "customer-1";
    firstCoffee.customer!.id = "customer-1";
    secondCoffee.customer!.id = "customer-2";
    expect(
      orderPrintSequence([firstTea, secondCoffee, firstCoffee], {
        hierarchy: ["customer", "primary_product"],
        taxonomyDepth: "family",
        mixedOrderMode: "dominant",
      }).map(({ id }) => id),
    ).toEqual(["first-coffee", "first-tea", "second-coffee"]);
  });

  it("bounds and normalizes persisted settings", () => {
    expect(
      parsePrintOrderSettings({
        hierarchy: ["taxonomy", "taxonomy", "customer", "unknown"],
        taxonomyDepth: "specific",
        mixedOrderMode: "contains",
      }),
    ).toEqual({
      hierarchy: ["taxonomy", "customer"],
      taxonomyDepth: "specific",
      mixedOrderMode: "contains",
    });
  });
});
