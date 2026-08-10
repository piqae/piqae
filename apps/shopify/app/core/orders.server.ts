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
  lineItems: Array<{
    title: string;
    sku: string;
    quantity: number;
    unitPrice: string;
    total: string;
  }>;
  subtotal: string;
  tax: string;
  total: string;
}

const ORDER_QUERY = `#graphql
 query PiqaePrintableOrder($id: ID!, $after: String) {
   order(id: $id) {
     id name createdAt currencyCode
     customer { id displayName email }
     shippingAddress { name company address1 address2 city province zip country phone }
     lineItems(first: 100, after: $after) { nodes { title sku quantity originalUnitPriceSet { shopMoney { amount } } discountedTotalSet { shopMoney { amount } } } pageInfo { hasNextPage endCursor } }
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
 query PiqaePrintableDraftOrder($id: ID!, $after: String) {
   draftOrder(id: $id) {
     id name createdAt currencyCode email
     shippingAddress { name company address1 address2 city province zip country phone }
     lineItems(first: 100, after: $after) { nodes { title sku quantity originalUnitPriceSet { shopMoney { amount } } discountedTotalSet { shopMoney { amount } } } pageInfo { hasNextPage endCursor } }
     subtotalPriceSet { shopMoney { amount } } totalTaxSet { shopMoney { amount } } totalPriceSet { shopMoney { amount } }
   }
 }`;

export async function fetchDraftOrders(
  admin: AdminGraphql,
  ids: string[],
): Promise<NormalizedOrder[]> {
  const unique = [...new Set(ids.map(normalizeDraftOrderGid))];
  if (unique.length < 1 || unique.length > 250)
    throw new Error("select between 1 and 250 draft orders");
  return boundedMap(unique, 8, async (id) => {
    const first = await graphqlWithRetry(admin, DRAFT_ORDER_QUERY, {
      id,
      after: null,
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
      shippingAddress: draft.shippingAddress ?? null,
      lineItems: lines.map((line: any) => ({
        title: line.title,
        sku: line.sku ?? "",
        quantity: line.quantity,
        unitPrice: money(line.originalUnitPriceSet),
        total: money(line.discountedTotalSet),
      })),
      subtotal: money(draft.subtotalPriceSet),
      tax: money(draft.totalTaxSet),
      total: money(draft.totalPriceSet),
    };
  });
}

export async function fetchOrders(
  admin: AdminGraphql,
  ids: string[],
): Promise<NormalizedOrder[]> {
  const unique = [...new Set(ids.map(normalizeOrderGid))];
  if (unique.length < 1 || unique.length > 250)
    throw new Error("select between 1 and 250 orders");
  return boundedMap(unique, 8, async (id) => {
    const first = await graphqlWithRetry(admin, ORDER_QUERY, {
      id,
      after: null,
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
      shippingAddress: order.shippingAddress ?? null,
      lineItems: allLines.map((line: any) => ({
        title: line.title,
        sku: line.sku ?? "",
        quantity: line.quantity,
        unitPrice: money(line.originalUnitPriceSet),
        total: money(line.discountedTotalSet),
      })),
      subtotal: money(order.subtotalPriceSet),
      tax: money(order.totalTaxSet),
      total: money(order.totalPriceSet),
    };
  });
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
