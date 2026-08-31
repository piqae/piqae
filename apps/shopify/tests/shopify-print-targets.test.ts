import { describe, expect, it } from "vitest";

import {
  loadShopifyPrintTargets,
  mapDesignSpecification,
} from "../app/core/shopify-print-targets.server";
import {
  selectTargetDestination,
  targetDocumentCompatibility,
  targetSupportsDocument,
} from "../app/core/shopify-print-targets";

const base = {
  target: {
    id: "tgt_labels",
    name: "Product labels",
    description: null,
    stock_id: "stock_100x50",
    enabled: true,
    routing_policy: "primary_then_standby",
    created_at: "2026-08-28T00:00:00Z",
    updated_at: "2026-08-28T00:00:00Z",
  },
  stock: {
    id: "stock_100x50",
    name: "100 × 50 label",
    sku: null,
    description: null,
    attributes: {
      kind: "label",
      width_mm: 100,
      height_mm: 50,
      gap_mm: 3,
      safe_area_mm: { top: 2, right: 2, bottom: 2, left: 2 },
    },
    archived: false,
    created_at: "2026-08-28T00:00:00Z",
    updated_at: "2026-08-28T00:00:00Z",
  },
  readiness: {
    target_id: "tgt_labels",
    status: "ready",
    selected_binding_id: "binding_primary",
    bindings: [
      {
        binding: { id: "binding_primary" },
        status: "ready",
        reasons: [],
      },
    ],
  },
  destinations: [
    {
      binding: {
        id: "binding_primary",
        destination_id: "pdst_labels",
        route_id: "route_labels",
        role: "primary",
        enabled: true,
      },
      printer: { id: "printer_zebra", name: "Zebra ZD421" },
      profile: { name: "100x50 direct thermal" },
      media_compatibility: {
        status: "ready",
        configuration_status: "configured",
        capability_status: "supported",
        loaded_media_status: "ready",
        reasons: [],
        profile_dimensions_mm: { width_mm: 100, height_mm: 50 },
        loaded_media: {
          source: "node",
          confidence: "operator_confirmed",
          observed_at: "2026-08-28T00:00:00Z",
          fresh_until: "2026-08-28T00:15:00Z",
          stock: { id: "stock_100x50", revision: 2 },
        },
      },
    },
  ],
  specification_revision: "spec_labels_2",
};

describe("Shopify print target projection", () => {
  it("keeps profile, stock, and loaded-media truth on the target", () => {
    const target = mapDesignSpecification(base as never);
    expect(target).toMatchObject({
      id: "tgt_labels",
      hasMediaCandidate: true,
      stock: { widthMm: 100, heightMm: 50, gapMm: 3 },
      destinations: [
        {
          printerId: "printer_zebra",
          destinationId: "pdst_labels",
          routeId: "route_labels",
          profileName: "100x50 direct thermal",
          mediaCompatibility: {
            status: "ready",
            confidence: "operator_confirmed",
            profileDimensionsMm: { widthMm: 100, heightMm: 50 },
          },
        },
      ],
    });
    expect(
      targetSupportsDocument(target, {
        format: "printpacket/v1",
        media: { kind: "label", width_mm: 100, height_mm: 50 },
        body: [],
      }),
    ).toBe(true);
    expect(
      targetSupportsDocument(target, {
        format: "printpacket/v1",
        media: { kind: "continuous", width_mm: 80 },
        body: [],
      }),
    ).toBe(false);

    const malformedSafeArea = structuredClone(base);
    malformedSafeArea.stock.attributes.safe_area_mm = {
      top: 2,
      right: Number.POSITIVE_INFINITY,
      bottom: 2,
      left: 2,
    };
    expect(
      mapDesignSpecification(malformedSafeArea as never).stock,
    ).toMatchObject({ safeAreaMm: null });
  });

  it("preserves successful target projections while reporting partial failure", async () => {
    const loaded = await loadShopifyPrintTargets({
      targets: {
        list: async () => [
          { id: "tgt_labels", enabled: true },
          { id: "tgt_unavailable", enabled: true },
        ],
        designSpecification: async (id) => {
          if (id === "tgt_unavailable") throw new Error("temporary outage");
          return base as never;
        },
      },
    });
    expect(loaded.partial).toBe(true);
    expect(loaded.targets).toHaveLength(1);
    expect(loaded.targets[0]?.id).toBe("tgt_labels");
  });

  it("matches the server's stock kinds, orientation, and label rotation", () => {
    const mapped = mapDesignSpecification(base as never);
    const receipt = structuredClone(mapped);
    receipt.stock = {
      ...receipt.stock!,
      kind: "receipt",
      widthMm: 80,
      heightMm: null,
    };
    receipt.destinations[0]!.mediaCompatibility.profileDimensionsMm = {
      widthMm: 80,
      heightMm: 120,
    };
    expect(
      targetSupportsDocument(receipt, {
        format: "printpacket/v1",
        media: { kind: "continuous", width_mm: 80 },
        body: [],
      }),
    ).toBe(true);

    const rollLabel = structuredClone(mapped);
    rollLabel.stock = {
      ...rollLabel.stock!,
      kind: "roll_label",
      widthMm: 50,
      heightMm: 100,
      rotatable: true,
    };
    rollLabel.destinations[0]!.mediaCompatibility.profileDimensionsMm = {
      widthMm: 50,
      heightMm: 100,
    };
    const landscapeLabel = {
      format: "printpacket/v1" as const,
      media: { kind: "label" as const, width_mm: 100, height_mm: 50 },
      body: [],
    };
    expect(targetSupportsDocument(rollLabel, landscapeLabel)).toBe(true);
    rollLabel.stock.rotatable = false;
    expect(targetSupportsDocument(rollLabel, landscapeLabel)).toBe(false);

    const sheet = structuredClone(mapped);
    sheet.stock = {
      ...sheet.stock!,
      kind: "sheet",
      widthMm: 210,
      heightMm: 297,
      orientation: "portrait",
    };
    sheet.destinations[0]!.mediaCompatibility.profileDimensionsMm = {
      widthMm: 210,
      heightMm: 297,
    };
    const landscapeA4 = {
      format: "printpacket/v1" as const,
      media: {
        kind: "paged" as const,
        size: "a4" as const,
        orientation: "landscape" as const,
      },
      body: [],
    };
    expect(targetSupportsDocument(sheet, landscapeA4)).toBe(false);
    sheet.stock.orientation = "either";
    expect(targetSupportsDocument(sheet, landscapeA4)).toBe(true);
    sheet.stock.kind = "card";
    expect(targetSupportsDocument(sheet, landscapeA4)).toBe(false);
    sheet.stock.kind = "envelope";
    expect(targetSupportsDocument(sheet, landscapeA4)).toBe(false);
  });

  it("treats not-reported loaded-media evidence as not ready", () => {
    const specification = {
      ...structuredClone(base),
      destinations: [
        {
          ...structuredClone(base.destinations[0]),
          media_compatibility: {
            status: "not_reported",
            configuration_status: "configured",
            capability_status: "supported",
            loaded_media_status: "unknown",
            reasons: ["stock_not_loaded"],
            profile_dimensions_mm: { width_mm: 100, height_mm: 50 },
            loaded_media: null,
          },
        },
      ],
    };
    const target = mapDesignSpecification(specification as never);
    expect(target.hasMediaCandidate).toBe(false);
    expect(target.destinations[0]?.mediaCompatibility).toMatchObject({
      status: "not_reported",
      configurationStatus: "configured",
      capabilityStatus: "supported",
      loadedMediaStatus: "unknown",
      observedAt: null,
    });
    expect(
      targetDocumentCompatibility(target, {
        format: "printpacket/v1",
        media: { kind: "label", width_mm: 100, height_mm: 50 },
        body: [],
      }),
    ).toBe("unverified");
  });

  it("reserves incompatible for a proven document or profile mismatch", () => {
    const target = mapDesignSpecification(base as never);
    expect(
      targetDocumentCompatibility(target, {
        format: "printpacket/v1",
        media: { kind: "label", width_mm: 62, height_mm: 29 },
        body: [],
      }),
    ).toBe("incompatible");
  });

  it("skips a stale primary and advises the compatible exact standby", () => {
    const specification = {
      ...structuredClone(base),
      readiness: {
        ...structuredClone(base.readiness),
        bindings: [
          {
            binding: {
              id: "binding_primary",
              destination_id: "pdst_labels",
              route_id: "route_labels",
            },
            status: "not_ready",
            reasons: ["printer_offline"],
          },
          {
            binding: {
              id: "binding_standby",
              destination_id: "pdst_standby",
              route_id: "route_standby",
            },
            status: "ready",
            reasons: [],
          },
        ],
      },
      destinations: [
        {
          ...structuredClone(base.destinations[0]!),
          media_compatibility: {
            ...structuredClone(base.destinations[0]!.media_compatibility),
            status: "stale",
            loaded_media_status: "stale",
            reasons: ["loaded_media_stale"],
          },
        },
        {
          ...structuredClone(base.destinations[0]!),
          binding: {
            id: "binding_standby",
            destination_id: "pdst_standby",
            route_id: "route_standby",
            role: "standby",
            enabled: true,
          },
          printer: { id: "printer_standby", name: "Standby Zebra" },
          media_compatibility: {
            ...structuredClone(base.destinations[0]!.media_compatibility),
            status: "ready",
          },
        },
      ],
    };
    const target = mapDesignSpecification(specification as never);
    const document = {
      format: "printpacket/v1" as const,
      media: { kind: "label" as const, width_mm: 100, height_mm: 50 },
      body: [],
    };
    expect(target.destinations).toHaveLength(2);
    expect(selectTargetDestination(target, document)).toMatchObject({
      role: "standby",
      destinationId: "pdst_standby",
      routeId: "route_standby",
      printerId: "printer_standby",
    });
    expect(targetSupportsDocument(target, document)).toBe(true);
    expect(target.hasMediaCandidate).toBe(true);
  });
});
