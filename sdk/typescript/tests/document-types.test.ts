import { describe, expect, it } from "vitest";
import type { DocumentSpec } from "../src/index.js";

describe("document pointer contract", () => {
  it("types root and repeat-relative value bindings", () => {
    const specification = {
      spec_version: "piqae.document/v1",
      page: { size: "a4" },
      body: [
        {
          type: "repeat",
          pointer: "/items",
          children: [
            { type: "text", value: { pointer: "." } },
            { type: "text", value: { pointer: "./title" } },
            { type: "text", value: { pointer: "/shop/name" } },
          ],
        },
      ],
    } satisfies DocumentSpec;

    const [repeat] = specification.body;
    if (!repeat) throw new Error("repeat fixture is missing");
    expect(repeat.children[1]?.value).toEqual({
      pointer: "./title",
    });
  });
});
