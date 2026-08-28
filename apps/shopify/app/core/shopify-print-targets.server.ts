import type { DesignSpecification, Stock } from "@piqae/sdk";
import type {
  MediaCompatibilityStatus,
  ShopifyPrintTarget,
} from "./shopify-print-targets";

type MediaProjection = {
  status?: unknown;
  reasons?: unknown;
  profile_dimensions_mm?: {
    width_mm?: unknown;
    height_mm?: unknown;
  } | null;
  loaded_media?: {
    source?: unknown;
    confidence?: unknown;
    observed_at?: unknown;
    fresh_until?: unknown;
  } | null;
};

export async function loadShopifyPrintTargets(client: {
  targets: {
    list(): Promise<Array<{ id: string; enabled: boolean }>>;
    designSpecification(id: string): Promise<DesignSpecification>;
  };
}): Promise<ShopifyPrintTarget[]> {
  const targets = await client.targets.list();
  const results = await Promise.allSettled(
    targets
      .filter((target) => target.enabled)
      .slice(0, 100)
      .map((target) => client.targets.designSpecification(target.id)),
  );
  return results.flatMap((result) =>
    result.status === "fulfilled" ? [mapDesignSpecification(result.value)] : [],
  );
}

export function mapDesignSpecification(
  specification: DesignSpecification,
): ShopifyPrintTarget {
  const selected = specification.destinations.find(
    ({ binding }) => binding.id === specification.readiness.selected_binding_id,
  );
  const destinationReadiness = selected
    ? specification.readiness.bindings.find(
        ({ binding }) => binding.id === selected.binding.id,
      )
    : undefined;
  const projection = (
    selected as unknown as
      | {
          media_compatibility?: MediaProjection;
        }
      | undefined
  )?.media_compatibility;
  const status = isMediaStatus(projection?.status)
    ? projection.status
    : "not_reported";
  const loaded = projection?.loaded_media;
  return {
    id: specification.target.id,
    name: specification.target.name,
    specificationRevision: specification.specification_revision,
    ready: specification.readiness.status === "ready" && status === "ready",
    readinessReasons: [
      ...(destinationReadiness?.reasons ?? []),
      ...(selected ? [] : ["target_has_no_ready_binding"]),
      ...(status === "ready" ? [] : [`media_${status}`]),
    ],
    selectedPrinterId: selected?.printer.id ?? null,
    selectedPrinterName: selected?.printer.name ?? null,
    selectedProfileName: selected?.profile.name ?? null,
    stock: stockProjection(specification.stock),
    mediaCompatibility: {
      status,
      reasons: Array.isArray(projection?.reasons)
        ? projection.reasons.filter(
            (reason): reason is string => typeof reason === "string",
          )
        : status === "ready"
          ? []
          : ["Loaded media has not been reported by this destination"],
      profileDimensionsMm:
        numberOrNull(projection?.profile_dimensions_mm?.width_mm) !== null &&
        numberOrNull(projection?.profile_dimensions_mm?.height_mm) !== null
          ? {
              widthMm: numberOrNull(
                projection?.profile_dimensions_mm?.width_mm,
              )!,
              heightMm: numberOrNull(
                projection?.profile_dimensions_mm?.height_mm,
              )!,
            }
          : null,
      source: stringOrNull(loaded?.source),
      confidence: stringOrNull(loaded?.confidence),
      observedAt: stringOrNull(loaded?.observed_at),
      freshUntil: stringOrNull(loaded?.fresh_until),
    },
  };
}

function stockProjection(stock: Stock | null): ShopifyPrintTarget["stock"] {
  if (!stock) return null;
  const value = stock.attributes;
  return {
    id: stock.id,
    name: stock.name,
    kind: typeof value.kind === "string" ? value.kind : null,
    widthMm: numberOrNull(value.width_mm),
    heightMm: numberOrNull(value.height_mm ?? value.length_mm),
    gapMm: numberOrNull(value.gap_mm),
    markIntervalMm: numberOrNull(value.mark_interval_mm),
    safeAreaMm: value.safe_area_mm ?? null,
  };
}

function numberOrNull(value: unknown) {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}
function stringOrNull(value: unknown) {
  return typeof value === "string" ? value : null;
}
function isMediaStatus(value: unknown): value is MediaCompatibilityStatus {
  return [
    "ready",
    "not_reported",
    "stale",
    "untrusted",
    "incompatible",
  ].includes(String(value));
}
