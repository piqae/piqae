import {
  isBindingName,
  parseShopifyDataBindings,
  type ShopifyDataBindings,
} from "./shopify-data-bindings";
export {
  parseShopifyDataBindings,
  type ShopifyDataBindings,
} from "./shopify-data-bindings";

export interface AdminGraphql {
  graphql(
    query: string,
    options?: { variables?: Record<string, unknown> },
  ): Promise<Response>;
}

export interface NormalizedOrder {
  id: string;
  name: string;
  createdAt: string;
  currency: string;
  customer: { id: string; displayName: string; email: string } | null;
  shippingAddress: Record<string, string> | null;
  metafields: Record<string, Record<string, NormalizedMetafield>>;
  lineItems: Array<{
    id: string;
    title: string;
    sku: string;
    quantity: number;
    unitPrice: string;
    total: string;
    currency: string;
    product: NormalizedProduct | null;
    variant: NormalizedVariant | null;
  }>;
  subtotal: string;
  tax: string;
  total: string;
}

export type NormalizedMetaobject = {
  id: string;
  type: string;
  handle: string;
  displayName: string;
  fields: Record<string, unknown>;
};

export type NormalizedMetafield = {
  type: string;
  value: unknown;
  reference?: NormalizedMetaobject;
};

export type NormalizedTaxonomyCategory = {
  id: string;
  name: string;
  fullName: string;
  level: number;
  ancestorIds: string[];
};

export type NormalizedProduct = {
  id: string;
  title: string;
  vendor: string;
  productType: string;
  category: NormalizedTaxonomyCategory | null;
  metafields: Record<string, Record<string, NormalizedMetafield>>;
};

export type NormalizedVariant = {
  id: string;
  title: string;
  barcode: string;
  metafields: Record<string, Record<string, NormalizedMetafield>>;
};

const MAX_METAFIELDS_PER_OWNER = 20;
const MAX_METAOBJECT_FIELDS = 24;
const MAX_METAOBJECT_VALUE_BYTES = 16 * 1024;
const MAX_NORMALIZED_PAYLOAD_BYTES = 8 * 1024 * 1024;

const ORDER_QUERY = `#graphql
 query PiqaePrintableOrder($id: ID!, $after: String, $orderFields: [HasMetafieldsIdentifier!]!, $productFields: [HasMetafieldsIdentifier!]!, $variantFields: [HasMetafieldsIdentifier!]!) {
   order(id: $id) {
     id name createdAt currencyCode
     customer { id displayName email }
     shippingAddress { name company address1 address2 city province zip country phone }
     metafieldsByIdentifiers(identifiers: $orderFields) { namespace key type jsonValue reference { ... on Metaobject { id type handle displayName fields { key value jsonValue } } } }
     lineItems(first: 100, after: $after) { nodes { id title sku quantity originalUnitPriceSet { shopMoney { amount } } discountedTotalSet { shopMoney { amount } } product { id title vendor productType category { id name fullName level ancestorIds } metafieldsByIdentifiers(identifiers: $productFields) { namespace key type jsonValue reference { ... on Metaobject { id type handle displayName fields { key value jsonValue } } } } } variant { id title barcode metafieldsByIdentifiers(identifiers: $variantFields) { namespace key type jsonValue reference { ... on Metaobject { id type handle displayName fields { key value jsonValue } } } } } } pageInfo { hasNextPage endCursor } }
     subtotalPriceSet { shopMoney { amount } }
     totalTaxSet { shopMoney { amount } }
     totalPriceSet { shopMoney { amount } }
   }
 }`;

export function normalizeOrderGid(value: string): string {
  const candidate = value.startsWith("gid://")
    ? value
    : `gid://shopify/Order/${value}`;
  if (!/^gid:\/\/shopify\/Order\/[1-9][0-9]*$/.test(candidate))
    throw new Error("invalid Shopify order ID");
  return candidate;
}

export function normalizeDraftOrderGid(value: string): string {
  const candidate = value.startsWith("gid://")
    ? value
    : `gid://shopify/DraftOrder/${value}`;
  if (!/^gid:\/\/shopify\/DraftOrder\/[1-9][0-9]*$/.test(candidate))
    throw new Error("invalid Shopify draft order ID");
  return candidate;
}

const DRAFT_ORDER_QUERY = `#graphql
 query PiqaePrintableDraftOrder($id: ID!, $after: String, $orderFields: [HasMetafieldsIdentifier!]!, $productFields: [HasMetafieldsIdentifier!]!, $variantFields: [HasMetafieldsIdentifier!]!) {
   draftOrder(id: $id) {
     id name createdAt currencyCode email
     metafieldsByIdentifiers(identifiers: $orderFields) { namespace key type jsonValue reference { ... on Metaobject { id type handle displayName fields { key value jsonValue } } } }
     shippingAddress { name company address1 address2 city province zip country phone }
     lineItems(first: 100, after: $after) { nodes { id title sku quantity originalUnitPriceSet { shopMoney { amount } } discountedTotalSet { shopMoney { amount } } product { id title vendor productType category { id name fullName level ancestorIds } metafieldsByIdentifiers(identifiers: $productFields) { namespace key type jsonValue reference { ... on Metaobject { id type handle displayName fields { key value jsonValue } } } } } variant { id title barcode metafieldsByIdentifiers(identifiers: $variantFields) { namespace key type jsonValue reference { ... on Metaobject { id type handle displayName fields { key value jsonValue } } } } } } pageInfo { hasNextPage endCursor } }
     subtotalPriceSet { shopMoney { amount } } totalTaxSet { shopMoney { amount } } totalPriceSet { shopMoney { amount } }
   }
 }`;

export async function fetchDraftOrders(
  admin: AdminGraphql,
  ids: string[],
  bindings: ShopifyDataBindings = {},
): Promise<NormalizedOrder[]> {
  const selection = normalizeBindings(bindings);
  const unique = [...new Set(ids.map(normalizeDraftOrderGid))];
  if (unique.length < 1 || unique.length > 250)
    throw new Error("select between 1 and 250 draft orders");
  const orders = await boundedMap(unique, 8, async (id) => {
    const first = await graphqlWithRetry(admin, DRAFT_ORDER_QUERY, {
      id,
      after: null,
      orderFields: selection.order,
      productFields: selection.product,
      variantFields: selection.variant,
    });
    if (first.errors?.length)
      throw new Error("Shopify Admin API rejected the draft order query");
    const draft = first.data?.draftOrder;
    if (!draft) throw new Error(`draft order not found: ${id}`);
    const lines = [...draft.lineItems.nodes];
    let pageInfo = draft.lineItems.pageInfo;
    for (let page = 1; pageInfo?.hasNextPage; page += 1) {
      if (page >= 100 || !pageInfo.endCursor)
        throw new Error(`draft order pagination limit reached: ${id}`);
      const next = await graphqlWithRetry(admin, DRAFT_ORDER_QUERY, {
        id,
        after: pageInfo.endCursor,
        orderFields: selection.order,
        productFields: selection.product,
        variantFields: selection.variant,
      });
      const connection = next.data?.draftOrder?.lineItems;
      if (!connection)
        throw new Error(`draft order changed while paginating: ${id}`);
      lines.push(...connection.nodes);
      pageInfo = connection.pageInfo;
    }
    const money = (set: any) => String(set?.shopMoney?.amount ?? "0");
    return {
      id: draft.id,
      name: draft.name,
      createdAt: draft.createdAt,
      currency: draft.currencyCode,
      customer: draft.email
        ? {
            id: "",
            displayName: draft.shippingAddress?.name ?? "",
            email: draft.email,
          }
        : null,
      shippingAddress: normalizedAddress(draft.shippingAddress),
      metafields: normalizeMetafields(
        draft.metafieldsByIdentifiers,
        selection,
        "order",
      ),
      lineItems: lines.map((line: any) => ({
        id: stringValue(line.id),
        title: line.title,
        sku: line.sku ?? "",
        quantity: line.quantity,
        unitPrice: money(line.originalUnitPriceSet),
        total: money(line.discountedTotalSet),
        currency: draft.currencyCode,
        product: normalizeProduct(line.product, selection, "product"),
        variant: normalizeVariant(line.variant, selection, "variant"),
      })),
      subtotal: money(draft.subtotalPriceSet),
      tax: money(draft.totalTaxSet),
      total: money(draft.totalPriceSet),
    } satisfies NormalizedOrder;
  });
  assertPayloadSize(orders);
  return orders;
}

export async function fetchOrders(
  admin: AdminGraphql,
  ids: string[],
  bindings: ShopifyDataBindings = {},
): Promise<NormalizedOrder[]> {
  const selection = normalizeBindings(bindings);
  const unique = [...new Set(ids.map(normalizeOrderGid))];
  if (unique.length < 1 || unique.length > 250)
    throw new Error("select between 1 and 250 orders");
  const orders = await boundedMap(unique, 8, async (id) => {
    const first = await graphqlWithRetry(admin, ORDER_QUERY, {
      id,
      after: null,
      orderFields: selection.order,
      productFields: selection.product,
      variantFields: selection.variant,
    });
    const body = first;
    if (body.errors?.length)
      throw new Error("Shopify Admin API rejected the order query");
    const order = body.data?.order;
    if (!order) throw new Error(`order not found: ${id}`);
    const allLines = [...order.lineItems.nodes];
    let pageInfo = order.lineItems.pageInfo;
    for (let page = 1; pageInfo?.hasNextPage; page += 1) {
      if (page >= 100 || !pageInfo.endCursor)
        throw new Error(`order line-item pagination limit reached: ${id}`);
      const next = await graphqlWithRetry(admin, ORDER_QUERY, {
        id,
        after: pageInfo.endCursor,
        orderFields: selection.order,
        productFields: selection.product,
        variantFields: selection.variant,
      });
      const connection = next.data?.order?.lineItems;
      if (!connection) throw new Error(`order changed while paginating: ${id}`);
      allLines.push(...connection.nodes);
      pageInfo = connection.pageInfo;
    }
    const money = (set: any) => String(set?.shopMoney?.amount ?? "0");
    return {
      id: order.id,
      name: order.name,
      createdAt: order.createdAt,
      currency: order.currencyCode,
      customer: order.customer
        ? {
            displayName: order.customer.displayName ?? "",
            email: order.customer.email ?? "",
            id: order.customer.id,
          }
        : null,
      shippingAddress: normalizedAddress(order.shippingAddress),
      metafields: normalizeMetafields(
        order.metafieldsByIdentifiers,
        selection,
        "order",
      ),
      lineItems: allLines.map((line: any) => ({
        id: stringValue(line.id),
        title: line.title,
        sku: line.sku ?? "",
        quantity: line.quantity,
        unitPrice: money(line.originalUnitPriceSet),
        total: money(line.discountedTotalSet),
        currency: order.currencyCode,
        product: normalizeProduct(line.product, selection, "product"),
        variant: normalizeVariant(line.variant, selection, "variant"),
      })),
      subtotal: money(order.subtotalPriceSet),
      tax: money(order.totalTaxSet),
      total: money(order.totalPriceSet),
    } satisfies NormalizedOrder;
  });
  assertPayloadSize(orders);
  return orders;
}

type NormalizedBindingSelection = {
  order: Array<{ namespace: string; key: string }>;
  product: Array<{ namespace: string; key: string }>;
  variant: Array<{ namespace: string; key: string }>;
  metaobjectFields: Map<string, Set<string>>;
};

function normalizeBindings(
  bindings: ShopifyDataBindings,
): NormalizedBindingSelection {
  const identifiers = (values: string[] | undefined, owner: string) => {
    const unique = [...new Set(values ?? [])];
    if (unique.length > MAX_METAFIELDS_PER_OWNER)
      throw new Error(`${owner} metafield binding limit exceeded`);
    return unique.map((value) => {
      const [namespace, key, extra] = value.split(".");
      if (extra || !isBindingName(namespace) || !isBindingName(key))
        throw new Error(`invalid ${owner} metafield binding: ${value}`);
      return { namespace: namespace!, key: key! };
    });
  };
  const metaobjectFields = new Map<string, Set<string>>();
  for (const [reference, fields] of Object.entries(
    bindings.metaobjectFields ?? {},
  )) {
    if (
      !/^(order|product|variant):[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+$/.test(
        reference,
      )
    )
      throw new Error(`invalid metaobject reference binding: ${reference}`);
    const unique = new Set(fields);
    if (
      unique.size > MAX_METAOBJECT_FIELDS ||
      [...unique].some((field) => !isBindingName(field))
    )
      throw new Error(`invalid metaobject field binding: ${reference}`);
    metaobjectFields.set(reference, unique);
  }
  return {
    order: identifiers(bindings.order, "order"),
    product: identifiers(bindings.product, "product"),
    variant: identifiers(bindings.variant, "variant"),
    metaobjectFields,
  };
}

function normalizeMetafields(
  values: unknown,
  selection: NormalizedBindingSelection,
  owner: "order" | "product" | "variant",
): Record<string, Record<string, NormalizedMetafield>> {
  if (!Array.isArray(values)) return {};
  const result: Record<string, Record<string, NormalizedMetafield>> = {};
  for (const field of values) {
    if (!field || typeof field !== "object") continue;
    const source = field as Record<string, unknown>;
    const namespace = stringValue(source.namespace);
    const key = stringValue(source.key);
    const identifier = `${namespace}.${key}`;
    if (
      !selection[owner].some(
        (allowed) => allowed.namespace === namespace && allowed.key === key,
      )
    )
      continue;
    const normalized: NormalizedMetafield = {
      type: stringValue(source.type),
      value: boundedJsonValue(source.jsonValue),
    };
    const allowedFields = selection.metaobjectFields.get(
      `${owner}:${identifier}`,
    );
    const reference = normalizeMetaobject(source.reference, allowedFields);
    if (reference) normalized.reference = reference;
    (result[namespace] ??= {})[key] = normalized;
  }
  return result;
}

function normalizeMetaobject(
  value: unknown,
  allowedFields?: Set<string>,
): NormalizedMetaobject | undefined {
  if (!allowedFields?.size || !value || typeof value !== "object")
    return undefined;
  const source = value as Record<string, unknown>;
  if (!String(source.id ?? "").startsWith("gid://shopify/Metaobject/"))
    return undefined;
  const fields: Record<string, unknown> = {};
  for (const candidate of Array.isArray(source.fields) ? source.fields : []) {
    if (!candidate || typeof candidate !== "object") continue;
    const field = candidate as Record<string, unknown>;
    const key = stringValue(field.key);
    if (allowedFields.has(key)) fields[key] = boundedJsonValue(field.jsonValue);
  }
  return {
    id: stringValue(source.id),
    type: stringValue(source.type),
    handle: stringValue(source.handle),
    displayName: stringValue(source.displayName),
    fields,
  };
}

function normalizeProduct(
  value: any,
  selection: NormalizedBindingSelection,
  owner: "product",
): NormalizedProduct | null {
  if (!value) return null;
  const category = value.category;
  return {
    id: stringValue(value.id),
    title: stringValue(value.title),
    vendor: stringValue(value.vendor),
    productType: stringValue(value.productType),
    category: category
      ? {
          id: stringValue(category.id),
          name: stringValue(category.name),
          fullName: stringValue(category.fullName),
          level: Number.isSafeInteger(category.level) ? category.level : 0,
          ancestorIds: Array.isArray(category.ancestorIds)
            ? category.ancestorIds
                .slice(0, 16)
                .map((id: unknown) => stringValue(id))
            : [],
        }
      : null,
    metafields: normalizeMetafields(
      value.metafieldsByIdentifiers,
      selection,
      owner,
    ),
  };
}

function normalizeVariant(
  value: any,
  selection: NormalizedBindingSelection,
  owner: "variant",
): NormalizedVariant | null {
  if (!value) return null;
  return {
    id: stringValue(value.id),
    title: stringValue(value.title),
    barcode: stringValue(value.barcode),
    metafields: normalizeMetafields(
      value.metafieldsByIdentifiers,
      selection,
      owner,
    ),
  };
}

function stringValue(value: unknown): string {
  return typeof value === "string"
    ? value.slice(0, MAX_METAOBJECT_VALUE_BYTES)
    : "";
}

function boundedJsonValue(value: unknown): unknown {
  const serialized = JSON.stringify(value ?? null) ?? "null";
  if (Buffer.byteLength(serialized, "utf8") > MAX_METAOBJECT_VALUE_BYTES)
    throw new Error(
      "Shopify custom-data value exceeds the document binding limit",
    );
  return JSON.parse(serialized) as unknown;
}

function assertPayloadSize(orders: NormalizedOrder[]) {
  if (
    Buffer.byteLength(JSON.stringify(orders), "utf8") >
    MAX_NORMALIZED_PAYLOAD_BYTES
  )
    throw new Error(
      "Shopify document data exceeds the normalized payload limit",
    );
}

function normalizedAddress(value: Record<string, unknown> | null | undefined) {
  if (!value) return null;
  const safe = Object.fromEntries(
    Object.entries(value).map(([key, item]) => [
      key,
      typeof item === "string" ? item : "",
    ]),
  ) as Record<string, string>;
  safe.formatted = [
    safe.name,
    safe.company,
    safe.address1,
    safe.address2,
    [safe.city, safe.province, safe.zip].filter(Boolean).join(" "),
    safe.country,
    safe.phone,
  ]
    .filter(Boolean)
    .join("\n");
  return safe;
}

async function graphqlWithRetry(
  admin: AdminGraphql,
  query: string,
  variables: Record<string, unknown>,
): Promise<any> {
  for (let attempt = 0; attempt < 5; attempt += 1) {
    const response = await admin.graphql(query, { variables });
    let body: any;
    try {
      body = await response.json();
    } catch {
      throw new Error("Shopify Admin API returned invalid JSON");
    }
    const throttled =
      response.status === 429 ||
      body.errors?.some((error: any) => error.extensions?.code === "THROTTLED");
    if (!throttled) {
      if (!response.ok)
        throw new Error(`Shopify Admin API failed (${response.status})`);
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
      Math.max(100, ((100 - available) / restoreRate) * 1_000),
    );
    await new Promise((resolve) => setTimeout(resolve, delay));
  }
  throw new Error("unreachable");
}

async function boundedMap<T, R>(
  values: T[],
  concurrency: number,
  operation: (value: T) => Promise<R>,
): Promise<R[]> {
  const results = new Array<R>(values.length);
  let cursor = 0;
  await Promise.all(
    Array.from({ length: Math.min(concurrency, values.length) }, async () => {
      for (;;) {
        const index = cursor++;
        if (index >= values.length) return;
        results[index] = await operation(values[index]!);
      }
    }),
  );
  return results;
}
