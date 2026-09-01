import { describe, expect, it } from "vitest";

import { loader } from "../app/routes/api.public.print-placeholder";

async function load(state: string) {
  const response = loader({
    request: new Request(
      `https://shopify.example.com/api/public/print-placeholder?state=${state}`,
    ),
  } as never);
  return { response, body: await response.text() };
}

async function loadFromAdmin(state: string) {
  const response = loader({
    request: new Request(
      `https://shopify.example.com/api/public/print-placeholder?state=${state}`,
      { headers: { origin: "https://admin.shopify.com" } },
    ),
  } as never);
  return { response, body: await response.text() };
}

describe("admin print preview placeholder", () => {
  it("shows an accessible shimmer while the real order PDF renders", async () => {
    const { response, body } = await load("loading");
    expect(response.status).toBe(200);
    expect(response.headers.get("access-control-allow-origin")).toBe(
      "https://extensions.shopifycdn.com",
    );
    expect(response.headers.get("cache-control")).toContain("no-store");
    expect(body).toContain('aria-label="Generating document preview"');
    expect(body).toContain('aria-busy="true"');
    expect(body).toContain("@keyframes shimmer");
    expect(body).not.toContain("Preview unavailable");
  });

  it("shows failure copy only after the preview request fails", async () => {
    const loading = await load("loading");
    const failed = await load("error");
    expect(loading.body).not.toContain("Preview unavailable");
    expect(failed.body).toContain("Preview unavailable");
  });

  it("can be embedded by the Shopify Admin print host", async () => {
    const { response } = await loadFromAdmin("loading");
    expect(response.headers.get("access-control-allow-origin")).toBe(
      "https://admin.shopify.com",
    );
  });
});
