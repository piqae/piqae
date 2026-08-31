import { describe, expect, it } from "vitest";

import { classifyAdminPreviewFailure } from "../app/routes/api.print.admin-previews";

describe("admin preview failure classification", () => {
  it("turns stale publications into a useful republish instruction", () => {
    expect(
      classifyAdminPreviewFailure(
        new Error("The published packing slip has no pinned Piqae revision"),
      ),
    ).toEqual({
      code: "document_publication",
      message:
        "This document publication is no longer available. Open the document, publish it again, then retry the preview.",
    });
  });

  it("does not expose unknown upstream error text", () => {
    expect(
      classifyAdminPreviewFailure(new Error("sensitive upstream body")),
    ).toEqual({
      code: "preview_failed",
      message: "Piqae could not generate this preview. Try again in a moment.",
    });
  });
});
