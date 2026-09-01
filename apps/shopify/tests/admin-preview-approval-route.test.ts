import { describe, expect, it } from "vitest";
import { PiqaeError } from "@piqae/sdk";

import {
  approvalErrorMessage,
  approvalErrorStatus,
} from "../app/routes/api.print.preview-approve";

describe("admin preview approval errors", () => {
  it("does not expose an upstream gateway status as the merchant diagnosis", () => {
    expect(approvalErrorMessage(new Error("Bad Gateway"))).toBe(
      "Piqae could not reach the print service. The PDF is still available to download; try direct printing again in a moment.",
    );
    expect(
      approvalErrorMessage(
        new PiqaeError(502, {
          code: "unexpected_response",
          message: "Bad Gateway",
          retryable: true,
        }),
      ),
    ).toBe(
      "Piqae could not reach the print service. The PDF is still available to download; try direct printing again in a moment.",
    );
    expect(approvalErrorStatus(new Error("Bad Gateway"))).toBe(502);
    expect(
      approvalErrorStatus(
        new PiqaeError(503, {
          code: "unexpected_response",
          message: "unavailable",
        }),
      ),
    ).toBe(502);
  });

  it("keeps actionable Node readiness failures specific", () => {
    expect(
      approvalErrorMessage(
        new PiqaeError(409, {
          code: "node_render_not_ready",
          message: "not ready",
        }),
      ),
    ).toBe(
      "The selected Node is not ready for this document. Check the Node and try again.",
    );
    expect(
      approvalErrorStatus(
        new PiqaeError(409, {
          code: "node_render_not_ready",
          message: "not ready",
        }),
      ),
    ).toBe(409);
  });
});
