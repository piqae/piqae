import type { PrintPacketV1 } from "@piqae/sdk";

export type MediaCompatibilityStatus =
  | "ready"
  | "not_reported"
  | "stale"
  | "untrusted"
  | "incompatible";

export type ShopifyPrintTarget = {
  id: string;
  name: string;
  specificationRevision: string;
  ready: boolean;
  readinessReasons: string[];
  selectedPrinterId: string | null;
  selectedPrinterName: string | null;
  selectedProfileName: string | null;
  stock: null | {
    id: string;
    name: string;
    kind: string | null;
    widthMm: number | null;
    heightMm: number | null;
    gapMm: number | null;
    markIntervalMm: number | null;
    safeAreaMm: {
      top: number;
      right: number;
      bottom: number;
      left: number;
    } | null;
  };
  mediaCompatibility: {
    status: MediaCompatibilityStatus;
    reasons: string[];
    profileDimensionsMm: { widthMm: number; heightMm: number } | null;
    source: string | null;
    confidence: string | null;
    observedAt: string | null;
    freshUntil: string | null;
  };
};

export function targetSupportsDocument(
  target: ShopifyPrintTarget,
  document: PrintPacketV1,
): boolean {
  const stock = target.stock;
  if (!stock?.kind) return false;
  if (document.media.kind === "paged") {
    if (!["sheet", "card", "envelope"].includes(stock.kind)) return false;
    const dimensions = pagedDimensions(
      document.media.size,
      document.media.orientation,
    );
    return dimensionsMatch(stock, dimensions.widthMm, dimensions.heightMm);
  }
  if (document.media.kind === "continuous")
    return (
      ["roll", "continuous"].includes(stock.kind) &&
      numberMatches(stock.widthMm, document.media.width_mm)
    );
  return (
    stock.kind === "label" &&
    numberMatches(stock.widthMm, document.media.width_mm) &&
    numberMatches(stock.heightMm, document.media.height_mm)
  );
}

function pagedDimensions(
  size: "a4" | "a5" | "letter",
  orientation: "portrait" | "landscape" = "portrait",
) {
  const portrait =
    size === "a4"
      ? { widthMm: 210, heightMm: 297 }
      : size === "a5"
        ? { widthMm: 148, heightMm: 210 }
        : { widthMm: 215.9, heightMm: 279.4 };
  return orientation === "landscape"
    ? { widthMm: portrait.heightMm, heightMm: portrait.widthMm }
    : portrait;
}

function dimensionsMatch(
  stock: NonNullable<ShopifyPrintTarget["stock"]>,
  widthMm: number,
  heightMm: number,
) {
  return (
    (numberMatches(stock.widthMm, widthMm) &&
      numberMatches(stock.heightMm, heightMm)) ||
    (numberMatches(stock.widthMm, heightMm) &&
      numberMatches(stock.heightMm, widthMm))
  );
}

function numberMatches(actual: number | null, expected: number) {
  return actual !== null && Math.abs(actual - expected) <= 0.5;
}
