import type { PrintPacketV1 } from "@piqae/sdk";

export type MediaCompatibilityStatus =
  | "ready"
  | "not_reported"
  | "stale"
  | "untrusted"
  | "incompatible";
export type MediaConfigurationStatus =
  | "configured"
  | "not_configured"
  | "incompatible";
export type MediaCapabilityStatus = "supported" | "unsupported" | "unknown";
export type LoadedMediaStatus =
  | "ready"
  | "unknown"
  | "stale"
  | "untrusted"
  | "incompatible";

export type ShopifyPrintTargetDestination = {
  bindingId: string;
  role: "primary" | "standby";
  enabled: boolean;
  destinationId: string | null;
  routeId: string | null;
  printerId: string;
  printerName: string;
  profileName: string;
  readinessStatus: string;
  readinessReasons: string[];
  mediaCompatibility: {
    configurationStatus: MediaConfigurationStatus;
    capabilityStatus: MediaCapabilityStatus;
    loadedMediaStatus: LoadedMediaStatus;
    status: MediaCompatibilityStatus;
    reasons: string[];
    profileDimensionsMm: { widthMm: number; heightMm: number } | null;
    source: string | null;
    confidence: string | null;
    observedAt: string | null;
    freshUntil: string | null;
  };
};

export type ShopifyPrintTarget = {
  id: string;
  name: string;
  specificationRevision: string;
  hasMediaCandidate: boolean;
  configurationReasons: string[];
  /** Ordered primary-then-standby candidates. Core remains routing authority. */
  destinations: ShopifyPrintTargetDestination[];
  stock: null | {
    id: string;
    name: string;
    kind: string | null;
    orientation: "portrait" | "landscape" | "either" | null;
    rotatable: boolean;
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
};

export function targetSupportsDocument(
  target: ShopifyPrintTarget,
  document: PrintPacketV1,
): boolean {
  return selectTargetDestination(target, document) !== null;
}

export type TargetDocumentCompatibility =
  | "ready"
  | "unverified"
  | "incompatible";

/**
 * Separates a proven media mismatch from missing or stale loaded-media
 * evidence. Both remain fail-closed for an immutable target binding, but the
 * merchant guidance must not claim that unknown physical stock is
 * incompatible.
 */
export function targetDocumentCompatibility(
  target: ShopifyPrintTarget,
  document: PrintPacketV1,
): TargetDocumentCompatibility {
  const configuredDestinations = target.destinations.filter(
    (destination) =>
      destination.enabled &&
      destination.destinationId !== null &&
      destination.routeId !== null &&
      destination.mediaCompatibility.configurationStatus === "configured" &&
      mediaMatchesDocument(
        target.stock,
        destination.mediaCompatibility.profileDimensionsMm,
        document,
      ),
  );
  if (
    configuredDestinations.some(
      (destination) =>
        destination.mediaCompatibility.capabilityStatus === "supported" &&
        destination.mediaCompatibility.loadedMediaStatus === "ready" &&
        destination.mediaCompatibility.status === "ready",
    )
  )
    return "ready";
  if (
    configuredDestinations.some(
      (destination) =>
        destination.mediaCompatibility.capabilityStatus === "unknown" ||
        ["unknown", "stale", "untrusted"].includes(
          destination.mediaCompatibility.loadedMediaStatus,
        ),
    ) ||
    target.destinations.some(
      (destination) =>
        destination.enabled &&
        (destination.mediaCompatibility.configurationStatus ===
          "not_configured" ||
          destination.mediaCompatibility.capabilityStatus === "unknown"),
    )
  )
    return "unverified";
  return "incompatible";
}

/**
 * Selects an advisory exact binding for editor/display purposes. The target
 * print request deliberately carries only target + specification revision;
 * core re-evaluates renderer capability, topology and liveness at handoff.
 */
export function selectTargetDestination(
  target: ShopifyPrintTarget,
  document: PrintPacketV1,
): ShopifyPrintTargetDestination | null {
  return (
    target.destinations.find(
      (destination) =>
        destination.enabled &&
        destination.destinationId !== null &&
        destination.routeId !== null &&
        destination.mediaCompatibility.status === "ready" &&
        mediaMatchesDocument(
          target.stock,
          destination.mediaCompatibility.profileDimensionsMm,
          document,
        ),
    ) ?? null
  );
}

function mediaMatchesDocument(
  stock: ShopifyPrintTarget["stock"],
  profileDimensions: { widthMm: number; heightMm: number } | null,
  document: PrintPacketV1,
): boolean {
  if (!stock?.kind) return false;
  if (document.media.kind === "paged") {
    if (stock.kind !== "sheet") return false;
    const dimensions = pagedDimensions(
      document.media.size,
      document.media.orientation,
    );
    const orientation = document.media.orientation ?? "portrait";
    return (
      (stock.orientation === null ||
        stock.orientation === "either" ||
        stock.orientation === orientation) &&
      dimensionsMatchUnordered(
        stock.widthMm,
        stock.heightMm,
        dimensions.widthMm,
        dimensions.heightMm,
      ) &&
      dimensionsMatchUnordered(
        profileDimensions?.widthMm ?? null,
        profileDimensions?.heightMm ?? null,
        dimensions.widthMm,
        dimensions.heightMm,
      )
    );
  }
  if (document.media.kind === "continuous")
    return (
      ["roll", "continuous", "receipt"].includes(stock.kind) &&
      numberMatches(stock.widthMm, document.media.width_mm) &&
      numberMatches(profileDimensions?.widthMm ?? null, document.media.width_mm)
    );
  if (!["label", "roll_label"].includes(stock.kind)) return false;
  const ordered =
    dimensionsMatchOrdered(
      stock.widthMm,
      stock.heightMm,
      document.media.width_mm,
      document.media.height_mm,
    ) &&
    dimensionsMatchOrdered(
      profileDimensions?.widthMm ?? null,
      profileDimensions?.heightMm ?? null,
      document.media.width_mm,
      document.media.height_mm,
    );
  const rotated =
    stock.rotatable &&
    dimensionsMatchUnordered(
      stock.widthMm,
      stock.heightMm,
      document.media.width_mm,
      document.media.height_mm,
    ) &&
    dimensionsMatchUnordered(
      profileDimensions?.widthMm ?? null,
      profileDimensions?.heightMm ?? null,
      document.media.width_mm,
      document.media.height_mm,
    );
  return ordered || rotated;
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

function dimensionsMatchUnordered(
  actualWidth: number | null,
  actualHeight: number | null,
  widthMm: number,
  heightMm: number,
) {
  return (
    dimensionsMatchOrdered(actualWidth, actualHeight, widthMm, heightMm) ||
    dimensionsMatchOrdered(actualWidth, actualHeight, heightMm, widthMm)
  );
}

function dimensionsMatchOrdered(
  actualWidth: number | null,
  actualHeight: number | null,
  widthMm: number,
  heightMm: number,
) {
  return (
    numberMatches(actualWidth, widthMm) && numberMatches(actualHeight, heightMm)
  );
}

function numberMatches(actual: number | null, expected: number) {
  return actual !== null && Math.abs(actual - expected) <= 0.5;
}
