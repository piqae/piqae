import {
  isBindingName,
  parseShopifyDataBindings,
  type ShopifyDataBindings,
} from "./shopify-data-bindings";
import { canonicalDataBytes, type JsonObject } from "@printpacket/core";
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

/**
 * Preserve Shopify's HTTP status and structured error body across the order
 * normalization boundary. Preview routes use this metadata to distinguish a
 * revoked offline credential from an ordinary permission failure and can then
 * perform one bounded token-exchange retry.
 */
export class ShopifyAdminApiError extends Error {
  readonly response: { code: number; body: unknown };

  constructor(status: number, body: unknown) {
    super(`Shopify Admin API failed (${status})`);
    this.name = "ShopifyAdminApiError";
    this.response = { code: status, body };
  }
}

export interface NormalizedOrder {
  id: string;
  name: string;
  createdAt: string;
  currency: string;
  customer: { id: string; displayName: string; email: string } | null;
  shippingAddress: Record<string, string> | null;
  billingAddress: Record<string, string> | null;
  note: string;
  shippingMethod: string;
  statusUrl: string;
  metafields: Record<string, Record<string, NormalizedMetafield>>;
  lineItems: Array<{
    id: string;
    title: string;
    sku: string;
    labelCode128: string | null;
    quantity: number;
    unitPrice: number;
    total: number;
    currency: string;
    product: NormalizedProduct | null;
    variant: NormalizedVariant | null;
  }>;
  subtotal: number;
  tax: number;
  total: number;
}

export interface ShopifyDocumentInput extends Record<string, unknown> {
  shop: {
    name: string;
    domain: string;
  };
  orders: NormalizedOrder[];
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
const MONEY_SCALE = 6;
const MONEY_SCALE_FACTOR = 1_000_000;
const CODE128_MAX_INPUT_BYTES = 80;
const CODE128_FIXED_MODULES = 55;
const CODE128_MODULES_PER_BYTE = 11;
const CODE128_MIN_MODULE_WIDTH_POINTS = 0.45;
const PRODUCT_LABEL_BARCODE_WIDTH_MM = 70;
const POINTS_PER_MM = 72 / 25.4;

const ORDER_QUERY = `#graphql
 query PiqaePrintableOrder($id: ID!, $after: String, $orderFields: [HasMetafieldsIdentifier!]!, $productFields: [HasMetafieldsIdentifier!]!, $variantFields: [HasMetafieldsIdentifier!]!) {
   order(id: $id) {
     id name createdAt currencyCode
     customer { id displayName email }
     shippingAddress { name company address1 address2 city province zip country phone }
     billingAddress { name company address1 address2 city province zip country phone }
     note statusPageUrl shippingLine { title }
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
     id name createdAt currencyCode email note
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
    const money = (set: any) => normalizeMoneyAmount(set?.shopMoney?.amount);
    return {
      id: stringValue(draft.id),
      name: stringValue(draft.name),
      createdAt: normalizedDateTime(draft.createdAt),
      currency: normalizedCurrency(draft.currencyCode),
      customer: draft.email
        ? {
            id: "",
            displayName: draft.shippingAddress?.name ?? "",
            email: draft.email,
          }
        : null,
      shippingAddress: normalizedAddress(draft.shippingAddress),
      billingAddress: null,
      note: stringValue(draft.note),
      shippingMethod: "",
      statusUrl: "",
      metafields: normalizeMetafields(
        draft.metafieldsByIdentifiers,
        selection,
        "order",
      ),
      lineItems: lines.map((line: any) => ({
        id: stringValue(line.id),
        title: stringValue(line.title),
        sku: stringValue(line.sku),
        labelCode128: normalizedLabelCode128Candidate(
          line.variant?.barcode,
          line.sku,
        ),
        quantity: normalizedQuantity(line.quantity),
        unitPrice: money(line.originalUnitPriceSet),
        total: money(line.discountedTotalSet),
        currency: normalizedCurrency(draft.currencyCode),
        product: normalizeProduct(line.product, selection, "product"),
        variant: normalizeVariant(line.variant, selection, "variant"),
      })),
      subtotal: money(draft.subtotalPriceSet),
      tax: money(draft.totalTaxSet),
      total: money(draft.totalPriceSet),
    } satisfies NormalizedOrder;
  });
  assertOrdersPayload(orders);
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
    const money = (set: any) => normalizeMoneyAmount(set?.shopMoney?.amount);
    return {
      id: stringValue(order.id),
      name: stringValue(order.name),
      createdAt: normalizedDateTime(order.createdAt),
      currency: normalizedCurrency(order.currencyCode),
      customer: order.customer
        ? {
            displayName: stringValue(order.customer.displayName),
            email: stringValue(order.customer.email),
            id: stringValue(order.customer.id),
          }
        : null,
      shippingAddress: normalizedAddress(order.shippingAddress),
      billingAddress: normalizedAddress(order.billingAddress),
      note: stringValue(order.note),
      shippingMethod: stringValue(order.shippingLine?.title),
      statusUrl: stringValue(order.statusPageUrl),
      metafields: normalizeMetafields(
        order.metafieldsByIdentifiers,
        selection,
        "order",
      ),
      lineItems: allLines.map((line: any) => ({
        id: stringValue(line.id),
        title: stringValue(line.title),
        sku: stringValue(line.sku),
        labelCode128: normalizedLabelCode128Candidate(
          line.variant?.barcode,
          line.sku,
        ),
        quantity: normalizedQuantity(line.quantity),
        unitPrice: money(line.originalUnitPriceSet),
        total: money(line.discountedTotalSet),
        currency: normalizedCurrency(order.currencyCode),
        product: normalizeProduct(line.product, selection, "product"),
        variant: normalizeVariant(line.variant, selection, "variant"),
      })),
      subtotal: money(order.subtotalPriceSet),
      tax: money(order.totalTaxSet),
      total: money(order.totalPriceSet),
    } satisfies NormalizedOrder;
  });
  assertOrdersPayload(orders);
  return orders;
}

/**
 * Build the only Shopify render-data shape submitted to PrintPacket. Money is
 * numeric before this boundary and the canonical encoder proves the exact
 * binary64/cache contract and 4 MiB limit used by every node renderer.
 */
export function shopifyDocumentInput(
  shopDomain: string,
  orders: NormalizedOrder[],
  shopName?: string,
): ShopifyDocumentInput {
  const domain = shopDomain.trim().toLowerCase();
  if (!/^[a-z0-9][a-z0-9-]*\.myshopify\.com$/.test(domain))
    throw new Error("invalid Shopify shop domain");
  const fallbackName = domain.slice(0, -".myshopify.com".length);
  const name = stringValue(shopName?.trim() || fallbackName);
  if (!name) throw new Error("Shopify shop name is required");
  const input = {
    shop: { name, domain },
    orders,
  } satisfies ShopifyDocumentInput;
  try {
    canonicalDataBytes(input as unknown as JsonObject);
  } catch {
    throw new Error(
      "Shopify document data exceeds the PrintPacket data contract",
    );
  }
  return input;
}

/**
 * Shopify GraphQL exposes Decimal money as a string. PrintPacket money is a
 * JSON number, so accept at most six decimal places, reject exponent/ambiguous
 * spellings, and require the scaled integer to remain JavaScript-safe. This
 * makes the resulting binary64 value deterministic across cloud and nodes.
 */
export function normalizeMoneyAmount(value: unknown): number {
  if (typeof value !== "string" && typeof value !== "number")
    throw new Error("Shopify money amount is invalid");
  const raw = String(value);
  const match = /^(-?)(0|[1-9][0-9]*)(?:\.([0-9]{1,6}))?$/.exec(raw);
  if (!match) throw new Error("Shopify money amount is invalid");
  const fraction = (match[3] ?? "").padEnd(MONEY_SCALE, "0");
  const magnitude = BigInt(`${match[2]}${fraction}`);
  if (magnitude > BigInt(Number.MAX_SAFE_INTEGER))
    throw new Error("Shopify money amount exceeds the PrintPacket safe range");
  const normalizedScaled = match[1] === "-" ? -magnitude : magnitude;
  const amount = Number(normalizedScaled) / MONEY_SCALE_FACTOR;
  if (!Number.isFinite(amount))
    throw new Error("Shopify money amount is invalid");
  const canonical = `${match[1] ?? ""}${match[2]}.${fraction}`;
  if (magnitude !== 0n && amount.toFixed(MONEY_SCALE) !== canonical)
    throw new Error("Shopify money amount exceeds exact PrintPacket precision");
  return Object.is(amount, -0) ? 0 : amount;
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

/**
 * Select the first candidate that the canonical renderer can encode at the
 * starter label's fixed 70 mm width. Code 128-B uses 55 fixed modules plus 11
 * per input byte, including the mandatory quiet zones used by the renderer.
 */
export function normalizedLabelCode128Candidate(
  barcode: unknown,
  sku: unknown,
): string | null {
  for (const source of [barcode, sku]) {
    if (typeof source !== "string") continue;
    const candidate = source.trim();
    if (
      candidate.length < 1 ||
      candidate.length > CODE128_MAX_INPUT_BYTES ||
      !/^[\x20-\x7e]+$/.test(candidate)
    )
      continue;
    const modules =
      CODE128_FIXED_MODULES + candidate.length * CODE128_MODULES_PER_BYTE;
    const moduleWidth =
      (PRODUCT_LABEL_BARCODE_WIDTH_MM * POINTS_PER_MM) / modules;
    if (moduleWidth >= CODE128_MIN_MODULE_WIDTH_POINTS) return candidate;
  }
  return null;
}

function stringValue(value: unknown): string {
  return typeof value === "string"
    ? value.slice(0, MAX_METAOBJECT_VALUE_BYTES)
    : "";
}

function normalizedCurrency(value: unknown): string {
  const currency = stringValue(value);
  if (!/^[A-Z]{3}$/.test(currency))
    throw new Error("Shopify currency code is invalid");
  return currency;
}

function normalizedQuantity(value: unknown): number {
  if (
    !Number.isSafeInteger(value) ||
    Number(value) < 0 ||
    Number(value) > 1_000_000_000
  )
    throw new Error("Shopify line-item quantity is invalid");
  return Number(value);
}

function normalizedDateTime(value: unknown): string {
  const timestamp = stringValue(value);
  const match =
    /^(\d{4})-(\d{2})-(\d{2})T\d{2}:\d{2}:\d{2}(?:\.\d{1,6})?(?:Z|[+-]\d{2}:\d{2})$/.exec(
      timestamp,
    );
  const year = Number(match?.[1]);
  const month = Number(match?.[2]);
  const day = Number(match?.[3]);
  const leap = year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0);
  const daysInMonth = [
    31,
    leap ? 29 : 28,
    31,
    30,
    31,
    30,
    31,
    31,
    30,
    31,
    30,
    31,
  ];
  if (
    !match ||
    month < 1 ||
    month > 12 ||
    day < 1 ||
    day > (daysInMonth[month - 1] ?? 0) ||
    !Number.isFinite(Date.parse(timestamp))
  )
    throw new Error("Shopify order timestamp is invalid");
  return timestamp;
}

function boundedJsonValue(value: unknown): unknown {
  const serialized = JSON.stringify(value ?? null) ?? "null";
  if (Buffer.byteLength(serialized, "utf8") > MAX_METAOBJECT_VALUE_BYTES)
    throw new Error(
      "Shopify custom-data value exceeds the document binding limit",
    );
  return JSON.parse(serialized) as unknown;
}

function assertOrdersPayload(orders: NormalizedOrder[]) {
  // A complete shop wrapper is checked immediately before submission. This
  // catches an oversized order selection at the fetch boundary as well.
  try {
    canonicalDataBytes({ orders } as unknown as JsonObject);
  } catch {
    throw new Error(
      "Shopify document data exceeds the PrintPacket data contract",
    );
  }
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
    const graphqlErrors = Array.isArray(body.errors) ? body.errors : [];
    const throttled =
      response.status === 429 ||
      graphqlErrors.some(
        (error: any) => error.extensions?.code === "THROTTLED",
      );
    if (!throttled) {
      if (!response.ok) throw new ShopifyAdminApiError(response.status, body);
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
