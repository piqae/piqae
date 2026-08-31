import { describe, expect, it } from "vitest";

import { adminExtensionPreflight } from "../app/core/admin-extension-cors.server";

function request(
  origin: string,
  headers = "authorization, content-type, idempotency-key",
) {
  return new Request("https://shopify.piqae.com/api/print/admin/previews", {
    method: "OPTIONS",
    headers: {
      origin,
      "access-control-request-method": "POST",
      "access-control-request-headers": headers,
    },
  });
}

describe("Admin extension CORS preflight", () => {
  it("allows only the headers used by authenticated preview actions", () => {
    const response = adminExtensionPreflight(
      request("https://extensions.shopifycdn.com"),
    );
    expect(response?.status).toBe(204);
    expect(response?.headers.get("access-control-allow-origin")).toBe(
      "https://extensions.shopifycdn.com",
    );
    expect(response?.headers.get("access-control-allow-headers")).toContain(
      "idempotency-key",
    );
  });

  it("rejects other origins and unexpected headers", () => {
    expect(
      adminExtensionPreflight(request("https://attacker.example"))?.status,
    ).toBe(403);
    expect(
      adminExtensionPreflight(
        request("https://extensions.shopifycdn.com", "authorization, x-unsafe"),
      )?.status,
    ).toBe(400);
  });

  it("rejects preflights for methods other than POST", () => {
    const unsafe = request("https://extensions.shopifycdn.com");
    unsafe.headers.set("access-control-request-method", "DELETE");

    expect(adminExtensionPreflight(unsafe)?.status).toBe(405);
  });

  it("leaves authenticated non-preflight requests to the route action", () => {
    expect(
      adminExtensionPreflight(
        new Request("https://shopify.piqae.com/api/print/admin/previews", {
          method: "POST",
        }),
      ),
    ).toBeNull();
  });
});
