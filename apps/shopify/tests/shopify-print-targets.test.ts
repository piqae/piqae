import { describe, expect, it } from "vitest";

import { mapDesignSpecification } from "../app/core/shopify-print-targets.server";
import { targetSupportsDocument } from "../app/core/shopify-print-targets";

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
      binding: { id: "binding_primary" },
      printer: { id: "printer_zebra", name: "Zebra ZD421" },
      profile: { name: "100x50 direct thermal" },
      media_compatibility: {
        status: "ready",
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
} as never;

describe("Shopify print target projection", () => {
  it("keeps profile, stock, and loaded-media truth on the target", () => {
    const target = mapDesignSpecification(base);
    expect(target).toMatchObject({
      id: "tgt_labels",
      ready: true,
      selectedPrinterId: "printer_zebra",
      selectedProfileName: "100x50 direct thermal",
      stock: { widthMm: 100, heightMm: 50, gapMm: 3 },
      mediaCompatibility: {
        status: "ready",
        confidence: "operator_confirmed",
        profileDimensionsMm: { widthMm: 100, heightMm: 50 },
      },
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
  });

  it("treats missing loaded-media evidence as not reported and not ready", () => {
    const specification = structuredClone(base as object) as any;
    delete specification.destinations[0].media_compatibility;
    const target = mapDesignSpecification(specification);
    expect(target.ready).toBe(false);
    expect(target.mediaCompatibility).toMatchObject({
      status: "not_reported",
      observedAt: null,
    });
  });
});
