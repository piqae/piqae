import type { DesignSpecification, Stock } from "@piqae/sdk";
import type { ShopifyPrintTarget } from "./shopify-print-targets";

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
  const projection = selected?.media_compatibility;
  const status = projection?.status ?? "not_reported";
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
      reasons:
        projection?.reasons ??
        (status === "ready"
          ? []
          : ["Loaded media has not been reported by this destination"]),
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
      source: loaded?.source ?? null,
      confidence: loaded?.confidence ?? null,
      observedAt: loaded?.observed_at ?? null,
      freshUntil: loaded?.fresh_until ?? null,
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
