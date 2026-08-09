import { describe, expect, it } from "vitest";
import {
  PrintIntentBuilder,
  preliminarilyValidatePrintIntent,
  type CapabilityDocument,
  type DocumentManifest,
} from "../src/index.js";

const manifest: DocumentManifest = {
  page_count: 1,
  page_boxes: [{ width_mm: 100, height_mm: 150, bleed_mm: 2 }],
  color_spaces: ["DeviceCMYK"],
  separations: ["White"],
  scaling: "none",
  pdf_version: "1.7",
  has_transparency: false,
};

const capabilities: CapabilityDocument = {
  schema_version: 1,
  printer_id: "ptr_1",
  revision: 7,
  driver_fingerprint_sha256: "a".repeat(64),
  facets: {
    "media.sensing": {
      type: "enum",
      mutability: "job_override",
      supported: true,
      values: ["gap", "black_mark"],
      evidence: { level: "mapped", source: "fixture" },
    },
    "effects.white.coverage": {
      type: "number",
      mutability: "job_override",
      supported: true,
      minimum: 0,
      maximum: 1,
      evidence: { level: "replay_tested", source: "fixture" },
    },
  },
  created_at: "2026-08-09T00:00:00Z",
};

describe("PrintIntentBuilder", () => {
  it("builds immutable normalized job options and validates them locally", () => {
    const base = PrintIntentBuilder.create({
      printerId: "ptr_1",
      capabilityRevision: 7,
      documentManifest: manifest,
    });
    const intent = base
      .portable({ copies: 50 })
      .semantic("media.sensing", "black_mark")
      .semantic("effects.white.coverage", 0.8)
      .stock("stk_1", 3)
      .build();
    expect(base.build().semantic_options).toEqual({});
    expect(
      preliminarilyValidatePrintIntent(intent, capabilities),
    ).toMatchObject({
      status: "valid",
      errors: [],
      normalized_intent: intent,
    });
  });

  it("rejects native escapes, stale revisions, and unsupported values", () => {
    expect(() =>
      PrintIntentBuilder.create({
        printerId: "ptr_1",
        capabilityRevision: 7,
        documentManifest: manifest,
      }).portable({ native_options: { undocumented: "value" } }),
    ).toThrow(/driver-native/);
    expect(() =>
      PrintIntentBuilder.create({
        printerId: "ptr_1",
        capabilityRevision: 7,
        documentManifest: manifest,
      }).semantic("effects.white.config", { nativeBlob: "opaque" }),
    ).toThrow(/Driver-native field/);
    const intent = PrintIntentBuilder.create({
      printerId: "ptr_1",
      capabilityRevision: 6,
      documentManifest: manifest,
    })
      .semantic("media.sensing", "continuous")
      .build();
    expect(
      preliminarilyValidatePrintIntent(intent, capabilities).errors.map(
        ({ code }) => code,
      ),
    ).toEqual(["stale_capability_revision", "facet_value_not_allowed"]);
  });

  it("checks declared dependencies and conflicts without pretending to be authoritative", () => {
    const constrained = structuredClone(capabilities);
    constrained.facets["media.sensing"]!.dependencies = ["media.stock"];
    constrained.facets["media.sensing"]!.conflicts = ["media.continuous"];
    constrained.facets["media.continuous"] = {
      type: "boolean",
      mutability: "job_override",
      supported: true,
      evidence: { level: "discovered", source: "fixture" },
    };
    const intent = PrintIntentBuilder.create({
      printerId: "ptr_1",
      capabilityRevision: 7,
      documentManifest: manifest,
    })
      .semantic("media.sensing", "gap")
      .semantic("media.continuous", true)
      .build();
    expect(
      preliminarilyValidatePrintIntent(intent, constrained).errors.map(
        ({ code }) => code,
      ),
    ).toEqual(["facet_dependency_missing", "facet_conflict"]);
  });

  it("leaves operator authorization to the server and node", () => {
    const operatorOnly = structuredClone(capabilities);
    operatorOnly.facets["media.sensing"]!.mutability = "operator_only";
    const intent = PrintIntentBuilder.create({
      printerId: "ptr_1",
      capabilityRevision: 7,
      documentManifest: manifest,
    })
      .semantic("media.sensing", "gap")
      .build();
    expect(
      preliminarilyValidatePrintIntent(intent, operatorOnly),
    ).toMatchObject({
      status: "operator_action_required",
      warnings: [{ code: "operator_authorization_required" }],
    });
  });
});
