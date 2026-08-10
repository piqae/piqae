import { describe, expect, it } from "vitest";
import { isTrustedDownloadUrl } from "./OrderDocument.jsx";

describe("customer document downloads", () => {
  it("accepts only the canonical short-lived grant endpoint", () => {
    expect(
      isTrustedDownloadUrl(
        "https://shopify.piqae.com/api/public/documents/download?token=opaque",
        "https://shopify.piqae.com",
      ),
    ).toBe(true);
    for (const value of [
      "https://evil.test/api/public/documents/download?token=opaque",
      "https://shopify.piqae.com.evil.test/api/public/documents/download?token=opaque",
      "https://shopify.piqae.com/api/public/documents/download",
      "https://shopify.piqae.com/api/public/documents/other?token=opaque",
      "javascript:alert(1)",
    ]) {
      expect(isTrustedDownloadUrl(value, "https://shopify.piqae.com")).toBe(
        false,
      );
    }
  });
});
