import { parseShopifyDataBindings } from "./shopify-data-bindings";

export type ShopifyDocumentField = {
  label: string;
  path: string;
  group:
    | "Shop"
    | "Order"
    | "Financial"
    | "Customer"
    | "Shipping"
    | "Item"
    | "Product"
    | "Shopify custom data";
  conditionable?: boolean;
};

/**
 * Shopify-specific authoring help. These paths compile to ordinary Piqae path
 * expressions; the canonical document format remains commerce-platform neutral.
 */
export const SHOPIFY_DOCUMENT_FIELDS: readonly ShopifyDocumentField[] = [
  {
    label: "Shop name",
    path: "shop.name",
    group: "Shop",
    conditionable: true,
  },
  {
    label: "Shop domain",
    path: "shop.domain",
    group: "Shop",
    conditionable: true,
  },
  {
    label: "Shop primary domain",
    path: "shop.primaryDomain",
    group: "Shop",
    conditionable: true,
  },
  {
    label: "Shop contact email",
    path: "shop.email",
    group: "Shop",
    conditionable: true,
  },
  {
    label: "Shop business address",
    path: "shop.address.formatted",
    group: "Shop",
    conditionable: true,
  },
  {
    label: "Order number",
    path: "order.name",
    group: "Order",
    conditionable: true,
  },
  {
    label: "Order date",
    path: "order.createdAt",
    group: "Order",
    conditionable: true,
  },
  {
    label: "Order currency",
    path: "order.currency",
    group: "Financial",
    conditionable: true,
  },
  {
    label: "Subtotal",
    path: "order.subtotal",
    group: "Financial",
    conditionable: true,
  },
  {
    label: "Tax",
    path: "order.tax",
    group: "Financial",
    conditionable: true,
  },
  {
    label: "Total",
    path: "order.total",
    group: "Financial",
    conditionable: true,
  },
  {
    label: "Customer name",
    path: "order.customer.displayName",
    group: "Customer",
    conditionable: true,
  },
  {
    label: "Customer email",
    path: "order.customer.email",
    group: "Customer",
    conditionable: true,
  },
  {
    label: "Shipping address",
    path: "order.shippingAddress.formatted",
    group: "Shipping",
    conditionable: true,
  },
  {
    label: "Billing address",
    path: "order.billingAddress.formatted",
    group: "Shipping",
    conditionable: true,
  },
  {
    label: "Shipping method",
    path: "order.shippingMethod",
    group: "Shipping",
    conditionable: true,
  },
  {
    label: "Order note",
    path: "order.note",
    group: "Order",
    conditionable: true,
  },
  {
    label: "Order status URL",
    path: "order.statusUrl",
    group: "Order",
    conditionable: true,
  },
  {
    label: "Item title",
    path: "item.title",
    group: "Item",
    conditionable: true,
  },
  { label: "Item SKU", path: "item.sku", group: "Item", conditionable: true },
  {
    label: "Label barcode (Code 128 safe)",
    path: "item.labelCode128",
    group: "Item",
    conditionable: true,
  },
  {
    label: "Item quantity",
    path: "item.quantity",
    group: "Item",
    conditionable: true,
  },
  {
    label: "Item unit price",
    path: "item.unitPrice",
    group: "Financial",
    conditionable: true,
  },
  {
    label: "Item total",
    path: "item.total",
    group: "Financial",
    conditionable: true,
  },
  {
    label: "Item currency",
    path: "item.currency",
    group: "Financial",
    conditionable: true,
  },
  {
    label: "Variant title",
    path: "item.variant.title",
    group: "Item",
    conditionable: true,
  },
  {
    label: "Variant barcode",
    path: "item.variant.barcode",
    group: "Item",
    conditionable: true,
  },
  {
    label: "Product vendor",
    path: "item.product.vendor",
    group: "Product",
    conditionable: true,
  },
  {
    label: "Product type",
    path: "item.product.productType",
    group: "Product",
    conditionable: true,
  },
  {
    label: "Shopify category",
    path: "item.product.category.name",
    group: "Product",
    conditionable: true,
  },
  {
    label: "Full Shopify category",
    path: "item.product.category.fullName",
    group: "Product",
    conditionable: true,
  },
  {
    label: "Shopify category ID",
    path: "item.product.category.id",
    group: "Product",
    conditionable: true,
  },
] as const;

export function shopifyCustomDocumentFields(
  allowlist: string[],
): ShopifyDocumentField[] {
  const bindings = parseShopifyDataBindings(allowlist);
  const result: ShopifyDocumentField[] = [];
  for (const owner of ["order", "product", "variant"] as const) {
    for (const identifier of bindings[owner] ?? []) {
      const prefix =
        owner === "order"
          ? "order"
          : owner === "product"
            ? "item.product"
            : "item.variant";
      result.push({
        label: `${owner[0]!.toUpperCase()}${owner.slice(1)} · ${identifier}`,
        path: `${prefix}.metafields.${identifier}.value`,
        group: "Shopify custom data",
        conditionable: true,
      });
      for (const field of bindings.metaobjectFields?.[
        `${owner}:${identifier}`
      ] ?? []) {
        result.push({
          label: `${owner[0]!.toUpperCase()}${owner.slice(1)} · ${identifier} · ${field}`,
          path: `${prefix}.metafields.${identifier}.reference.fields.${field}`,
          group: "Shopify custom data",
          conditionable: true,
        });
      }
    }
  }
  return result;
}
