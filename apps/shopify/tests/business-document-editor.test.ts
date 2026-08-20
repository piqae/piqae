import { describe, expect, it } from "vitest";
import {
  blocksToDoc,
  docToBlocks,
} from "../app/components/BusinessDocumentEditor";
import type { Block } from "../app/core/template-model";

describe("business document editor serialization", () => {
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
});
