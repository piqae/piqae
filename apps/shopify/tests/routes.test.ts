import { describe, expect, it } from "vitest";
import routes from "../app/routes";

describe("public route registration", () => {
  it("registers the node connection completion callback", () => {
    expect(routes).toContainEqual({
      path: "connect/complete",
      file: "routes/connect.complete.tsx",
    });
  });
});
