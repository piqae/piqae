import type { NormalizedOrder } from "./orders.server";

export const PRINT_GROUPING_DIMENSIONS = [
  "primary_product",
  "product_mix",
  "taxonomy",
  "customer",
  "vendor",
  "product_type",
  "product_tags",
  "order_tags",
] as const;

export type PrintGroupingDimension = (typeof PRINT_GROUPING_DIMENSIONS)[number];
export type PrintOrderSettings = {
  hierarchy: PrintGroupingDimension[];
  taxonomyDepth: "broad" | "family" | "specific";
  mixedOrderMode: "dominant" | "contains";
};

export const DEFAULT_PRINT_ORDER_SETTINGS: PrintOrderSettings = {
  hierarchy: [],
  taxonomyDepth: "family",
  mixedOrderMode: "dominant",
};

export function parsePrintOrderSettings(value: unknown): PrintOrderSettings {
  if (!value || typeof value !== "object" || Array.isArray(value))
    return structuredClone(DEFAULT_PRINT_ORDER_SETTINGS);
  const candidate = value as Record<string, unknown>;
  const hierarchy = Array.isArray(candidate.hierarchy)
    ? candidate.hierarchy.filter(
        (item, index, values): item is PrintGroupingDimension =>
          typeof item === "string" &&
          PRINT_GROUPING_DIMENSIONS.includes(item as PrintGroupingDimension) &&
          values.indexOf(item) === index,
      )
    : [];
  if (hierarchy.length > PRINT_GROUPING_DIMENSIONS.length)
    throw new Error("Print grouping hierarchy is too large");
  const taxonomyDepth = ["broad", "family", "specific"].includes(
    String(candidate.taxonomyDepth),
  )
    ? (candidate.taxonomyDepth as PrintOrderSettings["taxonomyDepth"])
    : DEFAULT_PRINT_ORDER_SETTINGS.taxonomyDepth;
  const mixedOrderMode = ["dominant", "contains"].includes(
    String(candidate.mixedOrderMode),
  )
    ? (candidate.mixedOrderMode as PrintOrderSettings["mixedOrderMode"])
    : DEFAULT_PRINT_ORDER_SETTINGS.mixedOrderMode;
  return { hierarchy, taxonomyDepth, mixedOrderMode };
}

export function parsePrintOrderForm(value: FormDataEntryValue | null) {
  if (typeof value !== "string" || value.length > 4096)
    throw new Error("Print order settings are invalid");
  let parsed: unknown;
  try {
    parsed = JSON.parse(value);
  } catch {
    throw new Error("Print order settings are invalid");
  }
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed))
    throw new Error("Print order settings are invalid");
  const rawHierarchy = (parsed as Record<string, unknown>).hierarchy;
  if (
    !Array.isArray(rawHierarchy) ||
    rawHierarchy.some(
      (item) =>
        typeof item !== "string" ||
        !PRINT_GROUPING_DIMENSIONS.includes(item as PrintGroupingDimension),
    ) ||
    new Set(rawHierarchy).size !== rawHierarchy.length
  )
    throw new Error("Print grouping hierarchy is invalid");
  return parsePrintOrderSettings(parsed);
}

export function orderPrintSequence(
  orders: NormalizedOrder[],
  settings: PrintOrderSettings,
): NormalizedOrder[] {
  if (settings.hierarchy.length === 0) return [...orders];
  return orders
    .map((order, originalIndex) => ({
      order,
      originalIndex,
      keys: settings.hierarchy.map((dimension) =>
        groupingKey(order, dimension, settings),
      ),
    }))
    .sort((left, right) => {
      for (let index = 0; index < left.keys.length; index += 1) {
        const compared = compareKey(left.keys[index]!, right.keys[index]!);
        if (compared) return compared;
      }
      return left.originalIndex - right.originalIndex;
    })
    .map(({ order }) => order);
}

function groupingKey(
  order: NormalizedOrder,
  dimension: PrintGroupingDimension,
  settings: PrintOrderSettings,
): string {
  if (dimension === "customer")
    return order.customer
      ? `${normalizeKey(order.customer.displayName)}\u001e${normalizeKey(order.customer.id)}`
      : "";
  if (dimension === "order_tags")
    return multiValueKey(order.tags, settings.mixedOrderMode);

  const weighted = new Map<string, number>();
  const add = (value: string | undefined, weight: number) => {
    const key = normalizeKey(value);
    if (!key) return;
    weighted.set(key, (weighted.get(key) ?? 0) + Math.max(weight, 1));
  };
  for (const line of order.lineItems) {
    const weight = Number.isFinite(line.quantity) ? line.quantity : 1;
    const product = line.product;
    if (dimension === "primary_product" || dimension === "product_mix")
      add(
        product
          ? `${product.title || line.title}\u001e${product.id}`
          : line.title,
        weight,
      );
    else if (dimension === "taxonomy")
      add(
        taxonomyKey(product?.category?.fullName, settings.taxonomyDepth),
        weight,
      );
    else if (dimension === "vendor") add(product?.vendor, weight);
    else if (dimension === "product_type") add(product?.productType, weight);
    else if (dimension === "product_tags")
      for (const tag of product?.tags ?? []) add(tag, weight);
  }
  return weightedKey(
    weighted,
    dimension === "product_mix"
      ? "contains"
      : dimension === "primary_product"
        ? "dominant"
        : settings.mixedOrderMode,
  );
}

function taxonomyKey(
  fullName: string | undefined,
  depth: PrintOrderSettings["taxonomyDepth"],
): string {
  const parts = String(fullName ?? "")
    .split(/\s*(?:>|\/|›)\s*/)
    .map(normalizeKey)
    .filter(Boolean);
  if (depth === "broad") return parts[0] ?? "";
  if (depth === "family") return parts.slice(0, 2).join(" > ");
  return parts.join(" > ");
}

function multiValueKey(
  values: string[],
  mode: PrintOrderSettings["mixedOrderMode"],
): string {
  const weighted = new Map<string, number>();
  for (const value of values) {
    const key = normalizeKey(value);
    if (key) weighted.set(key, (weighted.get(key) ?? 0) + 1);
  }
  return weightedKey(weighted, mode);
}

function weightedKey(
  values: Map<string, number>,
  mode: PrintOrderSettings["mixedOrderMode"],
): string {
  const entries = [...values].sort(
    ([leftKey, leftWeight], [rightKey, rightWeight]) =>
      mode === "dominant" && leftWeight !== rightWeight
        ? rightWeight - leftWeight
        : compareKey(leftKey, rightKey),
  );
  if (mode === "dominant") return entries[0]?.[0] ?? "";
  return entries.map(([key]) => key).join("\u001f");
}

function normalizeKey(value: unknown): string {
  return String(value ?? "")
    .normalize("NFKD")
    .replace(/[\u0300-\u036f]/g, "")
    .trim()
    .toLocaleLowerCase("en");
}

function compareKey(left: string, right: string): number {
  if (!left && right) return 1;
  if (left && !right) return -1;
  return left < right ? -1 : left > right ? 1 : 0;
}
