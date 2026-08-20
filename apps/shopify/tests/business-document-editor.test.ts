import { describe, expect, it } from "vitest";
import {
  blocksToDoc,
  docToBlocks,
  insertBlockAfterPath,
  removeBlockAtPath,
  replaceBlockAtPath,
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
});
