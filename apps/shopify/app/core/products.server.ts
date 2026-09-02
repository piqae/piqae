import {
  fetchShopPrintIdentity,
  normalizedLabelCode128Candidate,
  normalizeShopifyGraphqlRejection,
  normalizeMoneyAmount,
  ShopifyAdminApiError,
  ShopifyAdminGraphqlError,
  shopifyDocumentInput,
  type AdminGraphql,
  type NormalizedOrder,
  type NormalizedProduct,
  type NormalizedVariant,
  type ShopifyDocumentInput,
  type ShopifyDocumentWarning,
} from "./orders.server";

const PRODUCT_QUERY_CONCURRENCY = 6;
const PRODUCT_VARIANT_PAGE_SIZE = 100;
const MAX_PRODUCT_VARIANT_PAGES = 100;
const MAX_LABELS_PER_PREVIEW = 10_000;

const SHOP_CURRENCY_QUERY = `#graphql
  query PiqaeProductLabelCurrency {
    shop { currencyCode }
  }
`;

const PRODUCT_PRINT_QUERY = `#graphql
  query PiqaeProductLabelResource($id: ID!, $after: String) {
    node(id: $id) {
      ... on Product {
        __typename id title vendor productType tags
        category { id name fullName level ancestorIds }
        variants(first: 100, after: $after) {
          nodes { id title sku barcode price }
          pageInfo { hasNextPage endCursor }
        }
      }
      ... on ProductVariant {
        __typename id title sku barcode price
        product { id title vendor productType tags category { id name fullName level ancestorIds } }
      }
    }
  }
`;

const BASELINE_PRODUCT_PRINT_QUERY = `#graphql
  query PiqaeProductLabelResourceBaseline($id: ID!, $after: String) {
    node(id: $id) {
      ... on Product {
        __typename id title vendor productType
        variants(first: 100, after: $after) {
          nodes { id title sku barcode price }
          pageInfo { hasNextPage endCursor }
        }
      }
      ... on ProductVariant {
        __typename id title sku barcode price
        product { id title vendor productType }
      }
    }
  }
`;

export async function fetchProductDocumentInput(
  admin: AdminGraphql,
  shop: string,
  ids: string[],
): Promise<{
  input: ShopifyDocumentInput;
  documentCount: number;
  warnings: ShopifyDocumentWarning[];
}> {
  const selected = [...new Set(ids)];
  if (
    !selected.length ||
    selected.length > 100 ||
    selected.some(
      (id) => !/^gid:\/\/shopify\/(?:Product|ProductVariant)\/\d+$/.test(id),
    )
  )
    throw new Error("Select between 1 and 100 Shopify products or variants");

  const currencyPayload = await productGraphql(admin, SHOP_CURRENCY_QUERY, {});
  const currency = String(currencyPayload.data?.shop?.currencyCode ?? "");
  if (!/^[A-Z]{3}$/.test(currency))
    throw new Error("Shopify product currency is unavailable");

  const lineItems: ReturnType<typeof labelLine>[] = [];
  let usedBaselineData = false;
  for (
    let offset = 0;
    offset < selected.length;
    offset += PRODUCT_QUERY_CONCURRENCY
  ) {
    const batch = await Promise.all(
      selected
        .slice(offset, offset + PRODUCT_QUERY_CONCURRENCY)
        .map((id) => fetchProductLabelsWithFallback(admin, id, currency)),
    );
    for (const result of batch) {
      const { labels } = result;
      usedBaselineData ||= result.usedBaselineData;
      if (lineItems.length + labels.length > MAX_LABELS_PER_PREVIEW)
        throw new Error(
          `The selected products exceed the ${MAX_LABELS_PER_PREVIEW} label preview limit`,
        );
      lineItems.push(...labels);
    }
  }
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
    warnings: usedBaselineData
      ? ([
          {
            code: "optional_product_data_unavailable",
            message:
              "These labels used standard Shopify product data because optional categories or tags were unavailable.",
          },
        ] satisfies ShopifyDocumentWarning[])
      : [],
  };
}

async function fetchProductLabelsWithFallback(
  admin: AdminGraphql,
  id: string,
  currency: string,
): Promise<{
  labels: ReturnType<typeof labelLine>[];
  usedBaselineData: boolean;
}> {
  try {
    return {
      labels: await fetchProductLabels(admin, id, currency, "rich"),
      usedBaselineData: false,
    };
  } catch (error) {
    if (!isOptionalProductEnrichmentFailure(error)) throw error;
    return {
      labels: await fetchProductLabels(admin, id, currency, "baseline"),
      usedBaselineData: true,
    };
  }
}

async function fetchProductLabels(
  admin: AdminGraphql,
  id: string,
  currency: string,
  mode: "rich" | "baseline",
) {
  const expectedType = id.includes("/ProductVariant/")
    ? "ProductVariant"
    : "Product";
  const variants: any[] = [];
  const variantIds = new Set<string>();
  let product: NormalizedProduct | null = null;
  let after: string | null = null;

  for (let page = 0; page < MAX_PRODUCT_VARIANT_PAGES; page += 1) {
    const payload = await productGraphql(
      admin,
      mode === "rich" ? PRODUCT_PRINT_QUERY : BASELINE_PRODUCT_PRINT_QUERY,
      { id, after },
    );
    const node = payload.data?.node;
    if (
      !node ||
      node.__typename !== expectedType ||
      String(node.id ?? "") !== id
    )
      throw new Error(
        "One or more selected Shopify products or variants are unavailable",
      );

    if (expectedType === "ProductVariant")
      return [labelLine(normalizedProduct(node.product), node, currency)];

    product ??= normalizedProduct(node);
    const pageVariants = Array.isArray(node.variants?.nodes)
      ? node.variants.nodes
      : [];
    for (const variant of pageVariants) {
      const variantId = String(variant?.id ?? "");
      if (
        !/^gid:\/\/shopify\/ProductVariant\/\d+$/.test(variantId) ||
        variantIds.has(variantId)
      )
        throw new Error(
          "A selected Shopify product changed while its variants were loading",
        );
      variantIds.add(variantId);
      variants.push(variant);
    }

    if (!node.variants?.pageInfo?.hasNextPage) break;
    after = String(node.variants.pageInfo.endCursor ?? "");
    if (!after || page === MAX_PRODUCT_VARIANT_PAGES - 1)
      throw new Error("Shopify product variant pagination limit reached");
  }

  if (!product || !variants.length)
    throw new Error("A selected Shopify product has no printable variants");
  return variants.map((variant) => labelLine(product!, variant, currency));
}

async function productGraphql(
  admin: AdminGraphql,
  query: string,
  variables: Record<string, unknown>,
): Promise<any> {
  for (let attempt = 0; attempt < 5; attempt += 1) {
    let response: Response;
    try {
      response = await admin.graphql(query, { variables });
    } catch (error) {
      throw await normalizeShopifyGraphqlRejection(error);
    }
    let body: any;
    try {
      body = await response.json();
    } catch {
      throw new Error("Shopify Admin API returned invalid JSON");
    }
    const errors =
      body && typeof body === "object" && Array.isArray(body.errors)
        ? body.errors
        : [];
    const throttled =
      response.status === 429 ||
      errors.some((error: any) => error?.extensions?.code === "THROTTLED");
    if (!throttled) {
      if (!response.ok) throw new ShopifyAdminApiError(response.status, body);
      if (errors.length) throw new ShopifyAdminGraphqlError();
      return body;
    }
    if (attempt === 4)
      throw new Error("Shopify Admin API throttle retry exhausted");
    const available = Number(
      body.extensions?.cost?.throttleStatus?.currentlyAvailable ?? 0,
    );
    const restoreRate = Math.max(
      1,
      Number(body.extensions?.cost?.throttleStatus?.restoreRate ?? 50),
    );
    const delay = Math.min(
      2_000,
      Math.max(
        100,
        ((PRODUCT_VARIANT_PAGE_SIZE - available) / restoreRate) * 1_000,
      ),
    );
    await new Promise((resolve) => setTimeout(resolve, delay));
  }
  throw new Error("unreachable");
}

function isOptionalProductEnrichmentFailure(error: unknown): boolean {
  if (error instanceof ShopifyAdminGraphqlError) return true;
  return (
    error instanceof ShopifyAdminApiError &&
    (error.response.code === 400 || error.response.code === 422) &&
    Boolean(
      error.response.body &&
      typeof error.response.body === "object" &&
      Array.isArray((error.response.body as { errors?: unknown }).errors),
    )
  );
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
