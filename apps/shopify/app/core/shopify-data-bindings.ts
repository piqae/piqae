export type ShopifyDataBindings = {
  order?: string[];
  product?: string[];
  variant?: string[];
  metaobjectFields?: Record<string, string[]>;
};

export function isBindingName(value: string | undefined): value is string {
  return Boolean(value && /^[A-Za-z0-9_-]{1,64}$/.test(value));
}

/** Pure, browser-safe parser shared by authoring UI and server hydration. */
export function parseShopifyDataBindings(
  values: string[],
): ShopifyDataBindings {
  const result: ShopifyDataBindings = {
    order: [],
    product: [],
    variant: [],
    metaobjectFields: {},
  };
  for (const raw of values) {
    const [possibleOwner, binding] = raw.trim().includes(":")
      ? raw.trim().split(":", 2)
      : ["order", raw.trim()];
    if (!["order", "product", "variant"].includes(possibleOwner ?? ""))
      throw new Error(`invalid Shopify metafield binding: ${raw}`);
    const owner = possibleOwner as "order" | "product" | "variant";
    const parts = (binding ?? "").split(".");
    if (parts.length < 2 || parts.length > 3)
      throw new Error(`invalid Shopify metafield binding: ${raw}`);
    const [namespace, key, metaobjectField] = parts;
    if (
      !isBindingName(namespace) ||
      !isBindingName(key) ||
      (metaobjectField && !isBindingName(metaobjectField))
    )
      throw new Error(`invalid Shopify metafield binding: ${raw}`);
    const identifier = `${namespace}.${key}`;
    result[owner]!.push(identifier);
    if (metaobjectField) {
      const referenceKey = `${owner}:${identifier}`;
      (result.metaobjectFields![referenceKey] ??= []).push(metaobjectField);
    }
  }
  return result;
}
