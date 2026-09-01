import { describe, expect, it } from "vitest";

import {
  adminExtensionCors,
  adminExtensionPreflight,
  isAdminExtensionPreflightPath,
} from "../server/admin-extension-cors.mjs";

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
  it("intercepts only Shopify extension action and print-source routes", () => {
    expect(isAdminExtensionPreflightPath("/api/print/admin/previews")).toBe(
      true,
    );
    expect(
      isAdminExtensionPreflightPath("/api/print/previews/preview_1/approve"),
    ).toBe(true);
    expect(isAdminExtensionPreflightPath("/api/public/previews/image")).toBe(
      true,
    );
    expect(isAdminExtensionPreflightPath("/api/print/admin/options")).toBe(
      false,
    );
    expect(isAdminExtensionPreflightPath("/app/templates")).toBe(false);
  });

  it("allows Shopify to fetch public print sources", () => {
    const response = adminExtensionPreflight(
      new Request(
        "https://shopify.piqae.com/api/public/previews/artifact?token=fixture",
        {
          method: "OPTIONS",
          headers: {
            origin: "https://extensions.shopifycdn.com",
            "access-control-request-method": "GET",
            "access-control-request-headers": "authorization, x-requested-with",
          },
        },
      ),
    );

    expect(response?.status).toBe(204);
    expect(response?.headers.get("access-control-allow-methods")).toContain(
      "GET",
    );
    expect(response?.headers.get("access-control-allow-origin")).toBe(
      "https://extensions.shopifycdn.com",
    );
  });

  it("allows the Shopify Admin host to fetch public print sources", () => {
    const response = adminExtensionPreflight(
      new Request(
        "https://shopify.piqae.com/api/public/previews/artifact?token=fixture",
        {
          method: "OPTIONS",
          headers: {
            origin: "https://admin.shopify.com",
            "access-control-request-method": "GET",
            "access-control-request-headers": "authorization",
          },
        },
      ),
    );

    expect(response?.status).toBe(204);
    expect(response?.headers.get("access-control-allow-origin")).toBe(
      "https://admin.shopify.com",
    );
  });

  it("adds the trusted origin to printable responses", () => {
    const response = adminExtensionCors(
      new Response("preview", { headers: { vary: "Accept-Encoding" } }),
    );
    expect(response.headers.get("access-control-allow-origin")).toBe(
      "https://extensions.shopifycdn.com",
    );
    expect(response.headers.get("vary")).toContain("Accept-Encoding");
    expect(response.headers.get("vary")).toContain("Origin");
  });

  it("echoes the Shopify Admin origin on printable responses", () => {
    const request = new Request(
      "https://shopify.piqae.com/api/public/previews/artifact?token=fixture",
      { headers: { origin: "https://admin.shopify.com" } },
    );
    const response = adminExtensionCors(new Response("preview"), request);
    expect(response.headers.get("access-control-allow-origin")).toBe(
      "https://admin.shopify.com",
    );
  });

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

  it("does not allow the Shopify Admin host to call authenticated actions", () => {
    expect(
      adminExtensionPreflight(request("https://admin.shopify.com"))?.status,
    ).toBe(403);
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
