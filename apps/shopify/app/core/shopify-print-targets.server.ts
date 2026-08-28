import type { DesignSpecification, Stock } from "@piqae/sdk";
import type {
  ShopifyPrintTarget,
  ShopifyPrintTargetDestination,
} from "./shopify-print-targets";

export async function loadShopifyPrintTargets(client: {
  targets: {
    list(): Promise<Array<{ id: string; enabled: boolean }>>;
    designSpecification(id: string): Promise<DesignSpecification>;
  };
}): Promise<{ targets: ShopifyPrintTarget[]; partial: boolean }> {
  const targets = await client.targets.list();
  const results = await Promise.allSettled(
    targets
      .filter((target) => target.enabled)
      .slice(0, 100)
      .map((target) => client.targets.designSpecification(target.id)),
  );
  return {
    targets: results.flatMap((result) =>
      result.status === "fulfilled"
        ? [mapDesignSpecification(result.value)]
        : [],
    ),
    partial: results.some((result) => result.status === "rejected"),
  };
}

export function mapDesignSpecification(
  specification: DesignSpecification,
): ShopifyPrintTarget {
  const destinations = specification.destinations.map((destination) => {
    const readiness = specification.readiness.bindings.find(
      ({ binding }) => binding.id === destination.binding.id,
    );
    return destinationProjection(destination, readiness);
  });
  // This is an advisory display candidate only. Never project the generic
  // selected_binding_id as an exact PrintPacket destination: it is not aware
  // of this document's media or requested renderer policy.
  const hasMediaCandidate = destinations.some(
    (destination) =>
      destination.enabled &&
      destination.destinationId !== null &&
      destination.routeId !== null &&
      destination.mediaCompatibility.status === "ready",
  );
  return {
    id: specification.target.id,
    name: specification.target.name,
    specificationRevision: specification.specification_revision,
    hasMediaCandidate,
    configurationReasons: hasMediaCandidate
      ? []
      : destinations.flatMap((destination) => [
          ...destination.readinessReasons,
          ...destination.mediaCompatibility.reasons,
        ]),
    destinations,
    stock: stockProjection(specification.stock),
  };
}

function destinationProjection(
  destination: DesignSpecification["destinations"][number],
  readiness: DesignSpecification["readiness"]["bindings"][number] | undefined,
): ShopifyPrintTargetDestination {
  const projection = destination.media_compatibility;
  const loaded = projection.loaded_media;
  const width = numberOrNull(projection.profile_dimensions_mm?.width_mm);
  const height = numberOrNull(projection.profile_dimensions_mm?.height_mm);
  return {
    bindingId: destination.binding.id,
    role: destination.binding.role,
    enabled: destination.binding.enabled,
    destinationId: destination.binding.destination_id,
    routeId: destination.binding.route_id,
    printerId: destination.printer.id,
    printerName: destination.printer.name,
    profileName: destination.profile.name,
    readinessStatus: readiness?.status ?? "destination_missing",
    readinessReasons: readiness?.reasons ?? ["destination_readiness_missing"],
    mediaCompatibility: {
      status: projection.status,
      reasons: projection.reasons,
      profileDimensionsMm:
        width !== null && height !== null
          ? { widthMm: width, heightMm: height }
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
    orientation:
      value.orientation === "portrait" ||
      value.orientation === "landscape" ||
      value.orientation === "either"
        ? value.orientation
        : null,
    rotatable: value.rotatable === true,
    widthMm: numberOrNull(value.width_mm),
    heightMm: numberOrNull(value.height_mm ?? value.length_mm),
    gapMm: numberOrNull(value.gap_mm),
    markIntervalMm: numberOrNull(value.mark_interval_mm),
    safeAreaMm: safeAreaOrNull(value.safe_area_mm),
  };
}

function safeAreaOrNull(value: unknown) {
  if (
    !value ||
    typeof value !== "object" ||
    !("top" in value) ||
    !("right" in value) ||
    !("bottom" in value) ||
    !("left" in value)
  )
    return null;
  const top = numberOrNull(value.top);
  const right = numberOrNull(value.right);
  const bottom = numberOrNull(value.bottom);
  const left = numberOrNull(value.left);
  return top === null || right === null || bottom === null || left === null
    ? null
    : { top, right, bottom, left };
}

function numberOrNull(value: unknown) {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}
