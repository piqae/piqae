import { describe, expect, it } from "vitest";
import { pdfmeAdapter, pdfmeCompatibility } from "../src/index.js";
import basic from "./fixtures/pdfme-basic.json" with { type: "json" };

describe("pdfme adapter", () => {
  it("converts the declared flow-compatible subset in strict mode", () => {
    const result = pdfmeAdapter.convert(basic);
    expect(result.errors).toEqual([]);
    expect(result.fidelity).toBe("exact");
    expect(result.document?.body).toEqual([
      { type: "text", value: { pointer: "/order~1name" }, font_size: 14 },
      { type: "qr", value: { pointer: "/qr~0value" }, size_mm: 24 }
    ]);
  });

  it("requires explicit opt-in when absolute layout information is discarded", () => {
    const positioned = {
      basePdf: { width: 210, height: 297 },
      schemas: [[{ name: "order", type: "text", position: { x: 10, y: 10 }, width: 190, height: 8 }]]
    };
    const rejected = pdfmeAdapter.convert(positioned);
    expect(rejected.document).toBeUndefined();
    expect(rejected.errors.map((issue) => issue.code)).toContain("PDFME_LAYOUT_LOSSY");
    const result = pdfmeAdapter.convert(positioned, { strict: false });
    expect(result.errors).toEqual([]);
    expect(result.fidelity).toBe("lossy");
    expect(result.document).toEqual({
      spec_version: "piqae.document/v1",
      page: { size: "a4", margin_mm: 0 },
      body: [
        { type: "spacer", height_mm: 10 },
        { type: "text", value: { pointer: "/order" }, font_size: 10 }
      ]
    });
  });

  it("rejects background PDFs, unknown page sizes, and plugins without execution", () => {
    const background = pdfmeAdapter.convert({ basePdf: "data:application/pdf;base64,AAAA", schemas: [] }, { strict: false });
    expect(background.errors.map((issue) => issue.code)).toEqual(expect.arrayContaining([
      "PDFME_BASE_PDF_UNSUPPORTED",
      "PDFME_PAGE_SIZE_UNSUPPORTED"
    ]));
    const plugin = pdfmeAdapter.convert({
      basePdf: { width: 210, height: 297 },
      schemas: [[{ name: "x", type: "customPlugin", position: { x: 0, y: 0 } }]]
    }, { strict: false });
    expect(plugin.errors.some((issue) => issue.code === "PDFME_SCHEMA_TYPE_UNSUPPORTED")).toBe(true);
  });

  it("publishes explicit compatibility rather than claiming broad pdfme parity", () => {
    expect(pdfmeCompatibility.execution).toBe("conversion-only");
    expect(pdfmeCompatibility.features.find((feature) => feature.feature === "plugins")?.level).toBe("unsupported");
    expect(pdfmeCompatibility.features.find((feature) => feature.feature === "absolute-positioning")?.level).toBe("lossy");
  });

  it("bounds roll height and preserves source indexes after layout sorting", () => {
    for (const height of [0, -1, 2001, Number.POSITIVE_INFINITY, Number.NaN]) {
      const result = pdfmeAdapter.convert({ basePdf: { width: 58, height }, schemas: [] });
      expect(result.errors.map((issue) => issue.code)).toContain("PDFME_PAGE_SIZE_UNSUPPORTED");
    }
    expect(pdfmeAdapter.convert({ basePdf: { width: 80, height: 500 }, schemas: [] }).document?.page.size).toBe("roll80mm");
    const sorted = pdfmeAdapter.convert({
      basePdf: { width: 210, height: 297 },
      schemas: [[
        { name: "late", type: "customPlugin", position: { x: 0, y: 20 } },
        { name: "early", type: "text", position: { x: 0, y: 10 } }
      ]]
    }, { strict: false });
    const unsupported = sorted.errors.find((issue) => issue.code === "PDFME_SCHEMA_TYPE_UNSUPPORTED");
    expect(unsupported?.path).toBe("$.schemas[0][0].type");
    expect(unsupported?.feature).toBe("plugins");
    expect(unsupported?.message).toContain("customPlugin");
  });
});
