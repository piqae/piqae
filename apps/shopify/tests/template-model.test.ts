import { describe, expect, it } from "vitest";
import {
  canonicalToVisual,
  visualCompatibility,
  visualFields,
  visualTemplate,
  visualToCanonical,
  type PdfmeVisualModel,
} from "../app/core/template-model";

function model(types: string[] = ["text", "qrcode", "line"]): PdfmeVisualModel {
  return {
    schema: "pdfme-compatible/v1",
    page: "A4",
    fields: [],
    template: {
      basePdf: { width: 210, height: 297, padding: [0, 0, 0, 0] },
      schemas: [
        types.map((type, index) => ({
          name: `field-${index}`,
          type,
          position: { x: 10 + index, y: 20 + index },
          width: 40,
          height: 10,
          content: "literal",
        })),
      ],
    },
  };
}

describe("PDFme visual adapter", () => {
  it("maps the supported subset to an exact ordered canvas", () => {
    const source = model();
    expect(visualCompatibility(source)).toEqual({
      roundTrip: "lossless",
      warnings: [],
    });
    const canonical = visualToCanonical(source);
    expect(canonical.body[0]).toMatchObject({
      type: "canvas",
      children: [
        { type: "text", x_mm: 10, y_mm: 20, width_mm: 40, height_mm: 10 },
        { type: "qr", x_mm: 11, y_mm: 21, width_mm: 40, height_mm: 10 },
        { type: "line", x_mm: 12, y_mm: 22, width_mm: 40, height_mm: 10 },
      ],
    });
  });

  it("truthfully rejects plugins and lossy transforms", () => {
    const unsupported = model(["image"]);
    expect(visualCompatibility(unsupported).roundTrip).toBe("unsupported");
    expect(() => visualToCanonical(unsupported)).toThrow(
      "not exactly supported",
    );
    const rotated = model(["text"]);
    rotated.template!.schemas[0]![0]!.rotate = 30;
    expect(visualCompatibility(rotated).roundTrip).toBe("lossy");
    expect(() => visualToCanonical(rotated)).toThrow("not exactly supported");
  });

  it("preserves page ordering with explicit page breaks", () => {
    const source = model(["text"]);
    source.template!.schemas.push(source.template!.schemas[0]!);
    expect(visualToCanonical(source).body.map((node) => node.type)).toEqual([
      "canvas",
      "page_break",
      "canvas",
    ]);
  });

  it("keeps Piqae literal-versus-binding intent across PDFme changes", () => {
    const source: PdfmeVisualModel = {
      schema: "pdfme-compatible/v1",
      page: "A4",
      fields: [
        {
          id: "title",
          type: "text",
          x: 1,
          y: 1,
          width: 50,
          height: 8,
          text: "Invoice",
        },
        {
          id: "number",
          type: "text",
          x: 1,
          y: 10,
          width: 50,
          height: 8,
          binding: "/order/name",
        },
      ],
    };
    const native = visualTemplate(source);
    expect(native.schemas[0]![1]!.content).toBe("{{ order.name }}");
    const roundTrip = { ...source, fields: [], template: native };
    expect(
      visualFields(roundTrip).map(({ text, binding }) => ({ text, binding })),
    ).toEqual([
      { text: "Invoice", binding: undefined },
      { text: undefined, binding: "/order/name" },
    ]);
  });

  it("round-trips the canonical canvas back into PDFme data", () => {
    const source = model();
    const canonical = visualToCanonical(source);
    expect(visualToCanonical(canonicalToVisual(canonical))).toEqual(canonical);
  });
});
