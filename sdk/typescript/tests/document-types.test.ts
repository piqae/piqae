import { describe, expect, it } from "vitest";
import type { PrintPacketV1 } from "../src/index.js";

describe("PrintPacket expression contract", () => {
  it("types root and current-item paths", () => {
    const specification = {
      format: "printpacket/v1",
      media: { kind: "paged", size: "a4" },
      body: [
        {
          type: "repeat",
          items: { type: "path", path: ["items"] },
          children: [
            {
              type: "paragraph",
              content: [{ type: "value", value: { type: "current_path", path: ["title"] } }]
            }
          ],
        },
      ],
    } satisfies PrintPacketV1;

    const [repeat] = specification.body;
    if (!repeat) throw new Error("repeat fixture is missing");
    expect(repeat.children[0]).toMatchObject({ type: "paragraph" });
  });
});
