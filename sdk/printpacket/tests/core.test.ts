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

  it("negotiates features used only in page regions", () => {
    const packet = definePacket({
      format: "printpacket/v1",
      media: { kind: "paged", size: "a4" },
      header: {
        first: [{ type: "qr", value: { type: "literal", value: "first" }, size_mm: 12 }],
        default: [{ type: "grid", columns: [1], children: [] }]
      },
      body: [],
      footer: {
        default: [{ type: "barcode", value: { type: "literal", value: "ABC" }, symbology: "code128", width_mm: 30, height_mm: 10 }],
        last: [{ type: "keep_together", children: [] }]
      }
    });
    expect(requiredFeatures(packet)).toEqual(expect.arrayContaining([
      "layout_regions", "layout_grid", "layout_keep_together", "barcode_qr", "barcode_code128"
    ]));
  });

  it("canonicalizes data independently of object insertion order", () => {
    expect(canonicalData({ b: 2, a: { d: 4, c: 3 } })).toBe(canonicalData({ a: { c: 3, d: 4 }, b: 2 }));
  });

  it("canonicalizes binary64 numbers and UTF-8 keys without runtime spelling", () => {
    const canonical = canonicalData({ "\u{10000}": -0, "\ue000": 1.5 });
    expect(canonical).toContain("d0000000000000000");
    expect(canonical).toContain("d3ff8000000000000");
    expect(canonical.indexOf("\ue000")).toBeLessThan(canonical.indexOf("\u{10000}"));
    expect(() => canonicalData({ unsafe: 9_007_199_254_740_992 })).toThrow("safe integers");
    expect(() => canonicalData({ invalid: "\ud800" })).toThrow("Unicode scalar");
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
    })).resolves.toBe("67ee0fdf2f856773a6fd7ef05af4823ac98884db695ea2073fe4b5be07a09eb9");
  });

  it("rejects excessive nesting before an API or SDK call", () => {
    let children: unknown[] = [];
    for (let index = 0; index < 34; index += 1) children = [{ type: "section", children }];
    expect(() => preflightPacket({ format: "printpacket/v1", media: { kind: "paged", size: "a4" }, body: children as never })).toThrow("nesting");
  });

  it("accepts 100 resources and rejects 101 before an API or SDK call", () => {
    const resource = { type: "image" as const, digest: `sha256:${"a".repeat(64)}` as const, media_type: "image/jpeg" as const, byte_length: 1 };
    const packet = definePacket({
      format: "printpacket/v1",
      media: { kind: "paged", size: "a4" },
      body: [],
      resources: Object.fromEntries(Array.from({ length: 100 }, (_, index) => [`image_${index}`, resource]))
    });
    expect(() => preflightPacket(packet)).not.toThrow();
    expect(() => preflightPacket({
      ...packet,
      resources: { ...packet.resources, image_100: resource }
    })).toThrow("100 resources");
  });
});
