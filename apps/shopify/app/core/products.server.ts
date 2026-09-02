import {
  fetchShopPrintIdentity,
  normalizedLabelCode128Candidate,
  normalizeMoneyAmount,
  shopifyDocumentInput,
  type AdminGraphql,
  type NormalizedOrder,
  type NormalizedProduct,
  type NormalizedVariant,
  type ShopifyDocumentInput,
} from "./orders.server";

const PRODUCT_PRINT_QUERY = `#graphql
  query PiqaeProductLabels($ids: [ID!]!) {
    shop { currencyCode }
    nodes(ids: $ids) {
      ... on Product {
        __typename id title vendor productType tags
        category { id name fullName level ancestorIds }
        variants(first: 100) { nodes { id title sku barcode price } }
      }
      ... on ProductVariant {
        __typename id title sku barcode price
        product { id title vendor productType tags category { id name fullName level ancestorIds } }
      }
    }
  }
`;

export async function fetchProductDocumentInput(
  admin: AdminGraphql,
  shop: string,
  ids: string[],
): Promise<{ input: ShopifyDocumentInput; documentCount: number }> {
  const selected = [...new Set(ids)].slice(0, 100);
  if (
    !selected.length ||
    selected.some(
      (id) => !/^gid:\/\/shopify\/(?:Product|ProductVariant)\/\d+$/.test(id),
    )
  )
    throw new Error("Select at least one Shopify product or variant");
  const response = await admin.graphql(PRODUCT_PRINT_QUERY, {
    variables: { ids: selected },
  });
  const payload = (await response.json()) as any;
  if (!response.ok || payload.errors?.length)
    throw new Error("Shopify could not load the selected product data");
  const currency = String(payload.data?.shop?.currencyCode ?? "");
  if (!/^[A-Z]{3}$/.test(currency))
    throw new Error("Shopify product currency is unavailable");
  const lineItems = (payload.data?.nodes ?? []).flatMap((node: any) => {
    if (!node) return [];
    if (node.__typename === "Product") {
      const product = normalizedProduct(node);
      return (node.variants?.nodes ?? []).map((variant: any) =>
        labelLine(product, variant, currency),
      );
    }
    if (node.__typename === "ProductVariant")
      return [labelLine(normalizedProduct(node.product), node, currency)];
    return [];
  });
  if (!lineItems.length)
    throw new Error("The selected products do not have printable variants");
  const order: NormalizedOrder = {
    id: selected[0]!,
    name: "Product labels",
    referenceCode128: null,
    createdAt: new Date(0).toISOString(),
    currency,
    customer: null,
    shippingAddress: null,
    billingAddress: null,
    note: "",
    shippingMethod: "",
    statusUrl: "",
    tags: [],
    metafields: {},
    lineItems,
    subtotal: 0,
    tax: 0,
    total: 0,
  };
  const identity = await fetchShopPrintIdentity(admin, shop);
  return {
    input: shopifyDocumentInput(shop, [order], identity),
    documentCount: lineItems.length,
  };
}

function normalizedProduct(value: any): NormalizedProduct {
  const category = value?.category;
  return {
    id: String(value?.id ?? ""),
    title: String(value?.title ?? "").slice(0, 16_384),
    vendor: String(value?.vendor ?? "").slice(0, 16_384),
    productType: String(value?.productType ?? "").slice(0, 16_384),
    tags: Array.isArray(value?.tags)
      ? [...new Set<string>(value.tags.map((tag: unknown) => String(tag)))]
          .sort()
          .slice(0, 250)
      : [],
    category: category
      ? {
          id: String(category.id ?? ""),
          name: String(category.name ?? ""),
          fullName: String(category.fullName ?? ""),
          level: Number.isSafeInteger(category.level) ? category.level : 0,
          ancestorIds: Array.isArray(category.ancestorIds)
            ? category.ancestorIds.slice(0, 16).map(String)
            : [],
        }
      : null,
    metafields: {},
  };
}

function labelLine(product: NormalizedProduct, value: any, currency: string) {
  const variant: NormalizedVariant = {
    id: String(value?.id ?? ""),
    title: String(value?.title ?? "").slice(0, 16_384),
    barcode: String(value?.barcode ?? "").slice(0, 16_384),
    metafields: {},
  };
  const sku = String(value?.sku ?? "").slice(0, 16_384);
  const price = normalizeMoneyAmount(value?.price ?? "0");
  return {
    id: variant.id,
    title: product.title,
    sku,
    labelCode128: normalizedLabelCode128Candidate(variant.barcode, sku),
    quantity: 1,
    unitPrice: price,
    total: price,
    currency,
    product,
    variant,
  };
}
