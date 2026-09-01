const DEFAULT_SHOPIFY_SCOPES =
  "read_orders,read_all_orders,read_draft_orders,read_products,read_customers,read_metaobjects";

export function configuredShopifyScopes(value?: string): string[] {
  const scopes = (value ?? DEFAULT_SHOPIFY_SCOPES)
    .split(",")
    .map((scope) => scope.trim())
    .filter(Boolean);
  if (!scopes.includes("read_all_orders"))
    throw new Error("SCOPES must include Shopify-approved read_all_orders");
  if (
    !scopes.some((scope) => scope === "read_orders" || scope === "write_orders")
  )
    throw new Error(
      "SCOPES must pair read_all_orders with read_orders or write_orders",
    );
  return [...new Set(scopes)];
}
