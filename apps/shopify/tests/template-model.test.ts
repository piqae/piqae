import { describe, expect, it } from "vitest";
import {
  parseTemplateEnvelope,
  serializeTemplateEnvelope,
  validatePrintPacket,
  documentHasPageBreak,
} from "../app/core/template-model";
import { starterTemplates } from "../app/core/starter-templates";
import starterSpecifications from "./fixtures/printpacket/starter-specifications.json";
const allNodes = (nodes: any[]): any[] =>
  nodes.flatMap((node) => [
    node,
    ...(Array.isArray(node.children) ? allNodes(node.children) : []),
    ...(Array.isArray(node.then) ? allNodes(node.then) : []),
    ...(Array.isArray(node.else) ? allNodes(node.else) : []),
  ]);
describe("PrintPacket model", () => {
  it("keeps the checked cross-runtime renderer fixtures identical to every starter", () => {
    expect(
      Object.fromEntries(
        starterTemplates.map(({ id, specification }) => [id, specification]),
      ),
    ).toEqual(starterSpecifications);
  });
  it("ships only semantic, reflowable starters", () => {
    for (const starter of starterTemplates) {
      expect(starter.specification.format).toBe("printpacket/v1");
      expect(JSON.stringify(starter.specification)).not.toContain("canvas");
      const nodes = allNodes(starter.specification.body);
      expect(nodes.some((node) => node.type === "repeat")).toBe(true);
      expect(
        nodes.some((node) =>
          starter.kind === "label"
            ? node.type === "barcode"
            : node.type === "table",
        ),
      ).toBe(true);
      expect(parseTemplateEnvelope(starter.source).document).toEqual(
        starter.specification,
      );
    }
  });
  it("ships a valid thermal product label with variant, price, and a safe Code 128 value", () => {
    const label = starterTemplates.find(
      ({ id }) => id === "product-label",
    )!.specification;
    expect(() => validatePrintPacket(label)).not.toThrow();
    expect(label.media).toEqual({
      kind: "label",
      width_mm: 100,
      height_mm: 50,
      margins: { top_mm: 3, right_mm: 3, bottom_mm: 3, left_mm: 3 },
    });

    const nodes = allNodes(label.body);
    expect(
      nodes.find(
        (node) =>
          node.type === "conditional" &&
          node.condition?.type === "exists" &&
          node.condition.value?.type === "current_path" &&
          node.condition.value.path?.join(".") === "variant.title",
      ),
    ).toBeDefined();
    expect(
      nodes.find(
        (node) =>
          node.type === "paragraph" &&
          node.content?.some(
            (inline: any) =>
              inline.type === "value" &&
              inline.value?.type === "format_money" &&
              inline.value.amount?.type === "current_path" &&
              inline.value.amount.path?.join(".") === "unitPrice" &&
              inline.value.currency?.type === "current_path" &&
              inline.value.currency.path?.join(".") === "currency",
          ),
      ),
    ).toBeDefined();
    expect(nodes.find((node) => node.type === "barcode")).toMatchObject({
      value: { type: "current_path", path: ["labelCode128"] },
      symbology: "code128",
      width_mm: 70,
      height_mm: 12,
      human_readable: true,
    });
  });
  it("rejects noncanonical packets and excessive nesting", () => {
    expect(() => parseTemplateEnvelope('{"schema":"unsupported"}')).toThrow(
      "piqae.shopify-printpacket-template/v1",
    );
    const removed = structuredClone(starterTemplates[0]!.specification);
    expect(() =>
      validatePrintPacket({
        ...removed,
        format: "piqae.business-document/v1",
      } as never),
    ).toThrow("printpacket/v1");
    const document = structuredClone(starterTemplates[0]!.specification);
    let children = document.body;
    for (let index = 0; index < 14; index++) {
      const next: typeof document.body = [];
      children.push({ type: "section", children: next });
      children = next;
    }
    expect(() => validatePrintPacket(document)).toThrow("12 levels");
  });

  it("rejects missing media and malformed page regions before traversal", () => {
    const invoice = structuredClone(starterTemplates[0]!.specification);
    expect(() =>
      validatePrintPacket({ ...invoice, media: undefined } as never),
    ).toThrow("Document media is invalid");
    expect(() =>
      validatePrintPacket({
        ...invoice,
        header: { default: "not-an-array" },
      } as never),
    ).toThrow("Document page region is invalid");
    expect(() =>
      validatePrintPacket({ ...invoice, body: [null] } as never),
    ).toThrow("Document block is invalid");
  });

  it("rejects page breaks recursively in every continuous-media region", () => {
    const receipt = structuredClone(
      starterTemplates.find(({ id }) => id === "receipt")!.specification,
    );
    receipt.header = { default: [{ type: "page_break" }] };
    expect(documentHasPageBreak(receipt)).toBe(true);
    expect(() => validatePrintPacket(receipt)).toThrow(
      "not supported on continuous media",
    );

    const label = structuredClone(
      starterTemplates.find(({ id }) => id === "product-label")!.specification,
    );
    label.header = {
      last: [
        {
          type: "conditional",
          condition: { type: "path", path: ["order", "id"] },
          then: [{ type: "section", children: [{ type: "page_break" }] }],
        },
      ],
    };
    expect(documentHasPageBreak(label)).toBe(true);
    expect(() => validatePrintPacket(label)).toThrow(
      "not supported on label media",
    );

    const footerFirst = structuredClone(label);
    footerFirst.header = {};
    footerFirst.footer = { first: [{ type: "page_break" }] };
    expect(() => validatePrintPacket(footerFirst)).toThrow(
      "not supported on label media",
    );
  });

  it.each([
    ["header", "first"],
    ["header", "default"],
    ["header", "last"],
    ["footer", "first"],
    ["footer", "default"],
    ["footer", "last"],
  ] as const)("scans the %s.%s page region", (region, variant) => {
    const label = structuredClone(
      starterTemplates.find(({ id }) => id === "product-label")!.specification,
    );
    const nestedBreak = [
      {
        type: "conditional" as const,
        condition: { type: "path" as const, path: ["order", "id"] },
        then: [
          {
            type: "section" as const,
            children: [{ type: "page_break" as const }],
          },
        ],
      },
    ];
    const variants =
      variant === "first"
        ? { first: nestedBreak }
        : variant === "default"
          ? { default: nestedBreak }
          : { last: nestedBreak };
    if (region === "header") label.header = variants;
    else label.footer = variants;
    expect(documentHasPageBreak(label)).toBe(true);
    expect(() => validatePrintPacket(label)).toThrow(
      "not supported on label media",
    );
  });

  it.each([0, 1, 50, 200])(
    "keeps the invoice collection-driven for %i line items",
    (count) => {
      const input = {
        orders: [
          {
            currency: "NZD",
            lineItems: Array.from({ length: count }, (_, index) => ({
              title: `Item ${index}`,
              quantity: 1,
              unitPrice: 10,
              total: 10,
              currency: "NZD",
            })),
          },
        ],
      };
      const table = allNodes(starterTemplates[0]!.specification.body).find(
        (node) => node.type === "table",
      );
      expect(table?.type).toBe("table");
      expect(input.orders[0]!.lineItems).toHaveLength(count);
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
    ).toContain("printpacket/v1"));
});
