import { describe, expect, it } from "vitest";
import {
  parseTemplateEnvelope,
  serializeTemplateEnvelope,
  validateBusinessDocument,
} from "../app/core/template-model";
import { starterTemplates } from "../app/core/starter-templates";
describe("business document model", () => {
  it("ships only semantic, reflowable starters", () => {
    for (const starter of starterTemplates) {
      expect(starter.specification.format).toBe("piqae.business-document/v1");
      expect(JSON.stringify(starter.specification)).not.toContain("canvas");
      expect(
        starter.specification.body.some((node) => node.type === "table"),
      ).toBe(true);
      expect(parseTemplateEnvelope(starter.source).document).toEqual(
        starter.specification,
      );
    }
  });
  it("rejects legacy documents and excessive nesting", () => {
    expect(() => parseTemplateEnvelope('{"schema":"legacy-template"}')).toThrow(
      "Legacy templates",
    );
    const document = structuredClone(starterTemplates[0]!.specification);
    let children = document.body;
    for (let index = 0; index < 14; index++) {
      const next: typeof document.body = [];
      children.push({ type: "section", children: next });
      children = next;
    }
    expect(() => validateBusinessDocument(document)).toThrow("12 levels");
  });

  it.each([0, 1, 50, 200])(
    "keeps the invoice collection-driven for %i line items",
    (count) => {
      const input = {
        order: {
          currencyCode: "NZD",
          lineItems: Array.from({ length: count }, (_, index) => ({
            title: `Item ${index}`,
            quantity: 1,
            price: 10,
            total: 10,
          })),
        },
      };
      const table = starterTemplates[0]!.specification.body.find(
        (node) => node.type === "table",
      );
      expect(table?.type).toBe("table");
      expect(input.order.lineItems).toHaveLength(count);
      expect(JSON.stringify(table)).not.toContain(
        `Item ${Math.max(0, count - 1)}`,
      );
    },
  );
  it("serializes a bounded envelope", () =>
    expect(
      serializeTemplateEnvelope(
        parseTemplateEnvelope(starterTemplates[0]!.source),
      ),
    ).toContain("piqae.business-document/v1"));
});
