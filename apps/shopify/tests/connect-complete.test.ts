import { describe, expect, it } from "vitest";

import { loader } from "../app/routes/connect.complete";

describe("node connection completion", () => {
  it("renders a data-free success response outside embedded Admin auth", async () => {
    const response = loader();
    expect(response.status).toBe(200);
    await expect(response.json()).resolves.toEqual({ connected: true });
  });
});
