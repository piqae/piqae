import { describe, expect, it } from "vitest";
import { PDF_BASE14_V1, canonicalData, definePacket, preflightPacket, renderCacheKey, requiredFeatures } from "../src/index.js";

describe("PrintPacket developer contract", () => {
  it("types and preflights one fixed barcode label", () => {
    const label = definePacket({
      format: "printpacket/v1",
      media: { kind: "label", width_mm: 50, height_mm: 30 },
      body: [{ type: "barcode", value: { type: "path", path: ["sku"] }, symbology: "code128", width_mm: 35, height_mm: 10 }]
    });
    preflightPacket(label);
    expect(requiredFeatures(label)).toContain("media_label");
    expect(requiredFeatures(label)).toContain("barcode_code128");
    expect(PDF_BASE14_V1).toBe("printpacket.pdf-base14/v1");
  });

  it("canonicalizes data independently of object insertion order", () => {
    expect(canonicalData({ b: 2, a: { d: 4, c: 3 } })).toBe(canonicalData({ a: { c: 3, d: 4 }, b: 2 }));
  });

  it("matches the Rust reference receipt cache identity", async () => {
    await expect(renderCacheKey({
      standard: "PrintPacket",
      specification_version: "printpacket/v1",
      canonical_json: "printpacket.canonical-json/v1",
      canonical_sha256: "93554bac881890441088531a9030a2c7b89e7221400b1b8226aed33a86a25a1b",
      canonical_bytes: 1,
      required_features: [],
      resource_count: 0,
      resource_bytes: 0
    }, {
      lines: [{ name: "Flat white", total: "$5.50" }, { name: "Bagel", total: "$8.00" }],
      receipt_url: "https://example.invalid/r/R-1042"
    })).resolves.toBe("511ba47dd2e29e4df9c5d116ae58f6c3fa5ebc0ec1feb06c9430b78743f1dc19");
  });

  it("rejects excessive nesting before an API or SDK call", () => {
    let children: unknown[] = [];
    for (let index = 0; index < 34; index += 1) children = [{ type: "section", children }];
    expect(() => preflightPacket({ format: "printpacket/v1", media: { kind: "paged", size: "a4" }, body: children as never })).toThrow("nesting");
  });
});
