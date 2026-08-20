import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { importOrderPrinterProTemplate } from "../app/core/order-printer-pro-import.server";
import {
  parseTemplateEnvelope,
  serializeTemplateEnvelope,
} from "../app/core/template-model";

const fixture = readFileSync(
  new URL("./fixtures/order-printer-pro-packing-slip.liquid", import.meta.url),
  "utf8",
);

describe("Order Printer Pro import", () => {
  it("maps a representative packing slip into a bounded canonical document", () => {
    const result = importOrderPrinterProTemplate(fixture);
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.document.format).toBe("piqae.business-document/v1");
    expect(result.document.theme?.font_size_pt).toBe(9);
    expect(result.document.body[0]).toMatchObject({
      type: "repeat",
      items: { type: "path", path: ["orders"] },
    });
    expect(JSON.stringify(result.document)).toContain('"type":"barcode"');
    expect(JSON.stringify(result.document)).toContain('"type":"qr"');
    expect(result.diagnostics.map(({ fidelity }) => fidelity)).toEqual(
      expect.arrayContaining(["exact", "mapped", "lossy"]),
    );
    expect(result.originalSource).toBe(fixture);
    const envelope = parseTemplateEnvelope(
      serializeTemplateEnvelope({
        schema: "piqae.shopify-business-template/v1",
        document: result.document,
        editor: {
          mode: "visual",
          liquid: result.normalizedLiquid,
          roundTrip: "lossless",
          warnings: [],
          import: {
            format: "order_printer_pro",
            originalSource: result.originalSource,
            diagnostics: result.diagnostics,
          },
        },
        assets: [],
      }),
    );
    expect(envelope.editor.import?.originalSource).toBe(fixture);
  });

  it("never executes or imports active/networked constructs", () => {
    for (const source of [
      "{% render 'remote' %}",
      '<script src="https://example.com/x.js"></script>',
      '<div onclick="print()">Print</div>',
      "<style>body{background-image:url(https://example.com/x)}</style>",
    ]) {
      const result = importOrderPrinterProTemplate(source);
      expect(result.ok).toBe(false);
      expect(result.diagnostics[0]?.code).toBe("unsafe_construct");
    }
  });

  it("reports unsupported Liquid instead of interpreting it", () => {
    const result = importOrderPrinterProTemplate(
      "<p>{% capture secret %}x{% endcapture %}</p>",
    );
    expect(result.ok).toBe(false);
    expect(
      result.diagnostics.some(({ fidelity }) => fidelity === "unsupported"),
    ).toBe(true);
  });
});
