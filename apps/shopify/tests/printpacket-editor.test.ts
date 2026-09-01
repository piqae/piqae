import { describe, expect, it } from "vitest";
import {
  SHOPIFY_VARIABLES,
  blocksToDoc,
  completeExpression,
  contextualFieldSuggestions,
  docToBlocks,
  incompleteExpressionQuery,
  insertBlockAfterPath,
  moveBlockAtPath,
  parseContextualInline,
  removeBlockAtPath,
  replaceBlockAtPath,
  searchDocumentFields,
  barcodeCanvasStyle,
  canvasContentStyle,
  canvasGeometry,
  canvasStyle,
  compactCanvasAnnotations,
  documentMargins,
  maximumDocumentMargin,
  responsiveCanvasGeometry,
  qrCanvasStyle,
  safeAreaStyle,
  withDocumentMargin,
} from "../app/components/PrintPacketEditor";
import {
  authoringPathExpression,
  canonicalizeShopifyEditorBody,
} from "../app/core/shopify-editor-scopes";
import type { ShopifyDocumentField } from "../app/core/shopify-document-fields";
import type { Block, PrintPacket } from "../app/core/template-model";
import editorGeneratedPacket from "./fixtures/printpacket/editor-generated-packet.json";
import { starterTemplates } from "../app/core/starter-templates";

describe("PrintPacket editor serialization", () => {
  it("keeps paged and label canvases at their physical aspect ratio", () => {
    const a4 = structuredClone(starterTemplates[0]!.specification);
    expect(canvasGeometry(a4)).toEqual({ widthMm: 210, heightMm: 297 });
    expect(canvasStyle(a4).aspectRatio).toBe("210 / 297");
    if (a4.media.kind !== "paged") throw new Error("invoice must be paged");
    a4.media.orientation = "landscape";
    expect(canvasGeometry(a4)).toEqual({ widthMm: 297, heightMm: 210 });
    expect(canvasStyle(a4).aspectRatio).toBe("297 / 210");

    const label = starterTemplates.find(
      ({ id }) => id === "product-label",
    )!.specification;
    expect(canvasGeometry(label)).toEqual({ widthMm: 100, heightMm: 50 });
    expect(canvasStyle(label).aspectRatio).toBe("100 / 50");
    expect(compactCanvasAnnotations(label)).toBe(true);
    expect(compactCanvasAnnotations(a4)).toBe(true);
    expect(
      safeAreaStyle(label, { top: 2, right: 4, bottom: 3, left: 5 }),
    ).toEqual({ top: "4%", right: "4%", bottom: "6%", left: "5%" });
  });
  it("uses renderer-default margins and reflects bounded margin edits", () => {
    const document: PrintPacket = {
      format: "printpacket/v1",
      media: { kind: "label", width_mm: 100, height_mm: 50 },
      body: [],
    };

    expect(documentMargins(document)).toEqual({
      top_mm: 10,
      right_mm: 10,
      bottom_mm: 10,
      left_mm: 10,
    });
    expect(canvasStyle(document)).toMatchObject({
      padding: 0,
      "--piqae-mm": "calc(100cqw / 100)",
      "--piqae-margin-top": "calc(10 * var(--piqae-mm))",
      "--piqae-margin-right": "calc(10 * var(--piqae-mm))",
      "--piqae-margin-bottom": "calc(10 * var(--piqae-mm))",
      "--piqae-margin-left": "calc(10 * var(--piqae-mm))",
    });
    // Vertical percentages are resolved against the media height, while
    // horizontal percentages are resolved against the media width. CSS
    // percentage padding cannot provide that physical guarantee because all
    // four padding percentages resolve against the containing width.
    expect(canvasContentStyle(document)).toEqual({
      top: "20%",
      right: "10%",
      bottom: "20%",
      left: "10%",
    });

    const edited = withDocumentMargin(document, "left", 4.5);
    expect(edited.media.margins).toEqual({
      top_mm: 10,
      right_mm: 10,
      bottom_mm: 10,
      left_mm: 4.5,
    });
    expect(canvasStyle(edited).padding).toBe(0);
    expect(canvasContentStyle(edited)).toEqual({
      top: "20%",
      right: "10%",
      bottom: "20%",
      left: "4.5%",
    });
    expect(maximumDocumentMargin(edited, "top")).toBe(39);
    expect(withDocumentMargin(edited, "top", 500).media.margins?.top_mm).toBe(
      39,
    );
  });
  it("keeps continuous-media margins in the same physical scale as its content", () => {
    const receipt: PrintPacket = {
      format: "printpacket/v1",
      media: {
        kind: "continuous",
        width_mm: 80,
        margins: { top_mm: 3, right_mm: 4, bottom_mm: 5, left_mm: 6 },
      },
      body: [],
    };

    expect(canvasStyle(receipt)).toMatchObject({
      aspectRatio: undefined,
      "--piqae-mm": "calc(100cqw / 80)",
      "--piqae-media-min-height": "120mm",
    });
    expect(canvasContentStyle(receipt)).toEqual({
      margin:
        "calc(3 * var(--piqae-mm)) calc(4 * var(--piqae-mm)) calc(5 * var(--piqae-mm)) calc(6 * var(--piqae-mm))",
    });
  });
  it.each([320, 768])(
    "preserves A4 portrait, landscape, and label ratios at a %i px browser width",
    (browserWidth) => {
      const portrait = structuredClone(starterTemplates[0]!.specification);
      const landscape = structuredClone(portrait);
      if (landscape.media.kind !== "paged")
        throw new Error("invoice must be paged");
      landscape.media.orientation = "landscape";
      const label = starterTemplates.find(
        ({ id }) => id === "product-label",
      )!.specification;

      for (const [document, expectedRatio] of [
        [portrait, 210 / 297],
        [landscape, 297 / 210],
        [label, 100 / 50],
      ] as const) {
        const geometry = responsiveCanvasGeometry(document, browserWidth);
        expect(geometry.widthPx).toBeLessThanOrEqual(browserWidth - 32);
        expect(geometry.heightPx).not.toBeNull();
        expect(geometry.widthPx / geometry.heightPx!).toBeCloseTo(
          expectedRatio,
          8,
        );
      }
    },
  );
  it("sizes and aligns barcodes from the same physical footprint used by the renderer", () => {
    const label = starterTemplates.find(
      ({ id }) => id === "product-label",
    )!.specification;
    const barcode = {
      type: "barcode",
      value: { type: "literal", value: "942000000001" },
      symbology: "code128",
      width_mm: 88,
      height_mm: 16,
      human_readable: true,
      align: "center",
      padding_mm: 1,
      gap_mm: 2.5,
    } as const;
    expect(barcodeCanvasStyle(barcode, label.media.kind)).toMatchObject({
      width: "calc(90 * var(--piqae-mm))",
      maxWidth: "100%",
      marginInline: "auto",
      "--piqae-barcode-width": "calc(88 * var(--piqae-mm))",
      "--piqae-barcode-height": "calc(16 * var(--piqae-mm))",
      "--piqae-code-padding": "calc(1 * var(--piqae-mm))",
      "--piqae-code-gap": "calc(2.5 * var(--piqae-mm))",
    });
    expect(
      barcodeCanvasStyle(
        {
          ...barcode,
          width_mm: 1,
          height_mm: 1,
          padding_mm: -1,
          gap_mm: 200,
          align: "right",
        },
        label.media.kind,
      ),
    ).toMatchObject({
      width: "calc(20 * var(--piqae-mm))",
      maxWidth: "100%",
      marginInline: "auto 0",
      "--piqae-barcode-width": "calc(20 * var(--piqae-mm))",
      "--piqae-barcode-height": "calc(8 * var(--piqae-mm))",
      "--piqae-code-padding": "calc(0 * var(--piqae-mm))",
      "--piqae-code-gap": "calc(20 * var(--piqae-mm))",
    });
  });
  it("gives QR placeholders their declared physical square size", () => {
    const qr = {
      type: "qr",
      value: { type: "literal", value: "https://piqae.com" },
      size_mm: 24,
      error_correction: "Q",
    } as const;

    expect(qrCanvasStyle(qr)).toMatchObject({
      width: "calc(24 * var(--piqae-mm))",
      height: "calc(24 * var(--piqae-mm))",
    });
    expect(qrCanvasStyle({ ...qr, size_mm: 1 })).toMatchObject({
      width: "calc(8 * var(--piqae-mm))",
      height: "calc(8 * var(--piqae-mm))",
    });
  });
  it("preserves computed values, line breaks, and non-text layout blocks", () => {
    const blocks: Block[] = [
      {
        type: "paragraph",
        content: [
          {
            type: "value",
            value: {
              type: "format_money",
              amount: { type: "path", path: ["order", "total"] },
              currency: { type: "path", path: ["order", "currencyCode"] },
            },
          },
          { type: "line_break" },
          { type: "text", value: "Due" },
        ],
      },
      {
        type: "grid",
        columns: [1, 1],
        children: [
          { type: "paragraph", content: [{ type: "text", value: "Left" }] },
          { type: "paragraph", content: [{ type: "text", value: "Right" }] },
        ],
      },
      { type: "page_break" },
    ];

    expect(docToBlocks(blocksToDoc(blocks))).toEqual(blocks);
  });

  it("round-trips editable tables, branches, media and nested layouts losslessly", () => {
    const blocks: Block[] = [
      {
        type: "table",
        items: { type: "path", path: ["order", "lineItems"] },
        repeat_header: true,
        columns: [
          {
            header: [{ type: "text", value: "Product", style: { bold: true } }],
            cell: [
              {
                type: "value",
                value: { type: "current_path", path: ["title"] },
              },
            ],
            width: 4,
          },
          {
            header: [{ type: "text", value: "Qty" }],
            cell: [
              {
                type: "value",
                value: { type: "current_path", path: ["quantity"] },
              },
            ],
            width: 1,
            align: "right",
          },
        ],
      },
      {
        type: "conditional",
        condition: { type: "path", path: ["order", "note"] },
        then: [
          {
            type: "paragraph",
            content: [
              {
                type: "value",
                value: { type: "path", path: ["order", "note"] },
              },
            ],
          },
        ],
        else: [
          { type: "paragraph", content: [{ type: "text", value: "No notes" }] },
        ],
      },
      {
        type: "row",
        gap_mm: 5,
        children: [
          {
            type: "image",
            resource: "shop.logo",
            width_mm: 40,
            height_mm: 16,
            fit: "contain",
          },
          {
            type: "qr",
            value: { type: "path", path: ["order", "statusUrl"] },
            size_mm: 24,
            error_correction: "Q",
          },
        ],
      },
    ];

    expect(docToBlocks(blocksToDoc(blocks))).toEqual(blocks);
  });

  it("updates and removes nested canvas blocks without changing their siblings", () => {
    const blocks: Block[] = [
      {
        type: "conditional",
        condition: { type: "path", path: ["order", "note"] },
        then: [
          { type: "paragraph", content: [{ type: "text", value: "Original" }] },
          { type: "divider" },
        ],
        else: [],
      },
    ];
    const path = [
      { branch: "root" as const, index: 0 },
      { branch: "then" as const, index: 0 },
    ];
    const replacement: Block = {
      type: "heading",
      level: 2,
      content: [{ type: "text", value: "Updated" }],
    };
    const updated = replaceBlockAtPath(blocks, path, replacement);
    expect(
      (updated[0] as Extract<Block, { type: "conditional" }>).then,
    ).toEqual([replacement, { type: "divider" }]);
    expect(
      (
        removeBlockAtPath(updated, path)[0] as Extract<
          Block,
          { type: "conditional" }
        >
      ).then,
    ).toEqual([{ type: "divider" }]);
  });

  it("inserts after the selected block without replacing existing content", () => {
    const first: Block = {
      type: "paragraph",
      content: [{ type: "text", value: "First" }],
    };
    const second: Block = { type: "divider" };
    const inserted: Block = {
      type: "paragraph",
      content: [{ type: "text", value: "Inserted" }],
    };
    expect(
      insertBlockAfterPath(
        [first, second],
        [{ branch: "root", index: 0 }],
        inserted,
      ),
    ).toEqual([first, inserted, second]);
  });

  it("moves a selected nested block without moving its container", () => {
    const first: Block = {
      type: "paragraph",
      content: [{ type: "text", value: "First" }],
    };
    const second: Block = { type: "divider" };
    const blocks: Block[] = [
      { type: "stack", children: [first, second] },
      { type: "page_break" },
    ];
    expect(
      moveBlockAtPath(
        blocks,
        [
          { branch: "root", index: 0 },
          { branch: "children", index: 1 },
        ],
        -1,
      ),
    ).toEqual([
      { type: "stack", children: [second, first] },
      { type: "page_break" },
    ]);
  });

  it("converts contextual item expressions while retaining root shop data", () => {
    expect(
      parseContextualInline(
        "{{ item.title }} — {{ shop.name }} — {{ item.product.metafields.custom.bin.value }}",
        [],
        "item",
      ),
    ).toEqual([
      {
        type: "value",
        value: { type: "current_path", path: ["title"] },
      },
      { type: "text", value: " — " },
      {
        type: "value",
        value: { type: "path", path: ["shop", "name"] },
      },
      { type: "text", value: " — " },
      {
        type: "value",
        value: {
          type: "current_path",
          path: ["product", "metafields", "custom", "bin", "value"],
        },
      },
    ]);
  });

  it("compiles the friendly order and item aliases into canonical renderer scopes", () => {
    expect(authoringPathExpression("order.name", "order")).toEqual({
      type: "current_path",
      path: ["name"],
    });
    expect(authoringPathExpression("item.sku", "item")).toEqual({
      type: "current_path",
      path: ["sku"],
    });
    expect(authoringPathExpression("shop.name", "item")).toEqual({
      type: "path",
      path: ["shop", "name"],
    });
    expect(SHOPIFY_VARIABLES).toContain("order.tax");
    expect(SHOPIFY_VARIABLES).not.toContain("order.taxTotal");
  });

  it("emits the exact batch-safe packet consumed by the real renderer fixture", () => {
    const authored: PrintPacket = {
      format: "printpacket/v1",
      media: { kind: "paged", size: "a4" },
      theme: { font_size_pt: 10, line_height: 1.25 },
      resources: {},
      body: [
        {
          type: "heading",
          level: 1,
          content: [
            {
              type: "value",
              value: { type: "path", path: ["shop", "name"] },
            },
          ],
        },
        {
          type: "paragraph",
          content: [
            {
              type: "value",
              value: { type: "path", path: ["order", "name"] },
            },
          ],
        },
        {
          type: "table",
          items: { type: "path", path: ["order", "lineItems"] },
          columns: [
            {
              header: [{ type: "text", value: "Item" }],
              cell: [
                {
                  type: "value",
                  value: { type: "path", path: ["item", "title"] },
                },
              ],
            },
            {
              header: [{ type: "text", value: "SKU" }],
              cell: [
                {
                  type: "value",
                  value: { type: "path", path: ["item", "sku"] },
                },
              ],
            },
            {
              header: [{ type: "text", value: "Total" }],
              cell: [
                {
                  type: "value",
                  value: {
                    type: "format_money",
                    amount: { type: "path", path: ["item", "total"] },
                    currency: {
                      type: "path",
                      path: ["item", "currency"],
                    },
                  },
                },
              ],
            },
          ],
        },
        { type: "page_break" },
      ],
    };
    expect({
      ...authored,
      body: canonicalizeShopifyEditorBody(authored.body),
    }).toEqual(editorGeneratedPacket);
  });

  it("finds and completes expressions from a contextual Shopify field catalogue", () => {
    const fields: ShopifyDocumentField[] = [
      { label: "Order name", path: "order.name", group: "Order" },
      { label: "Item SKU", path: "item.sku", group: "Item" },
      {
        label: "Product · custom.origin",
        path: "item.product.metafields.custom.origin.value",
        group: "Shopify custom data",
      },
    ];
    expect(
      contextualFieldSuggestions(fields, "item").map((field) => field.path),
    ).toEqual(["item.sku", "item.product.metafields.custom.origin.value"]);
    expect(searchDocumentFields(fields, "custom origin")[0]?.path).toBe(
      "item.product.metafields.custom.origin.value",
    );
    expect(incompleteExpressionQuery("SKU: {{ item.sk")).toBe("item.sk");
    expect(incompleteExpressionQuery("{{ item.sku }}")).toBeNull();
    expect(completeExpression("SKU: {{ item.sk", "item.sku")).toBe(
      "SKU: {{ item.sku }}",
    );
  });
});
