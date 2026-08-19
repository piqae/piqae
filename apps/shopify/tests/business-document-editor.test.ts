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
});
