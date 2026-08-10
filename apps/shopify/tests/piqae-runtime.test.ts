import { describe, expect, it } from "vitest";
import {
  resolvePiqaeRuntime,
  resolveShopifyStorage,
} from "../app/core/piqae-runtime.server";

describe("Shopify Piqae runtime selection", () => {
  it("defaults development to a loopback fake runtime", () => {
    expect(resolvePiqaeRuntime({ NODE_ENV: "development" })).toEqual({
      mode: "fake",
      baseUrl: "http://127.0.0.1:8080",
      permitsPhysicalPrinting: false,
    });
  });

  it("requires HTTPS for the live network", () => {
    expect(() =>
      resolvePiqaeRuntime({
        NODE_ENV: "development",
        PIQAE_SHOPIFY_RUNTIME: "live",
        PIQAE_API_URL: "http://api.piqae.com",
      }),
    ).toThrow("live Piqae runtime requires HTTPS");
  });

  it("accepts an explicit live endpoint without authorizing hardware itself", () => {
    expect(
      resolvePiqaeRuntime({
        NODE_ENV: "development",
        PIQAE_SHOPIFY_RUNTIME: "live",
        PIQAE_API_URL: "https://api.piqae.com/",
      }),
    ).toEqual({
      mode: "live",
      baseUrl: "https://api.piqae.com",
      permitsPhysicalPrinting: true,
    });
  });

  it("rejects credentials embedded in configured URLs", () => {
    expect(() =>
      resolvePiqaeRuntime({
        NODE_ENV: "development",
        PIQAE_SHOPIFY_RUNTIME: "local",
        PIQAE_API_URL: "http://token@127.0.0.1:8080",
      }),
    ).toThrow("must not contain credentials");
  });
});

describe("Shopify state storage selection", () => {
  it("uses memory only for tests by default", () => {
    expect(resolveShopifyStorage({ NODE_ENV: "test" })).toBe("memory");
  });

  it("requires durable PostgreSQL for ordinary development", () => {
    expect(() => resolveShopifyStorage({ NODE_ENV: "development" })).toThrow(
      "DATABASE_URL is required",
    );
    expect(
      resolveShopifyStorage({
        NODE_ENV: "development",
        DATABASE_URL: "postgresql://configured",
      }),
    ).toBe("postgres");
  });

  it("never permits memory storage in production", () => {
    expect(() =>
      resolveShopifyStorage({
        NODE_ENV: "production",
        PIQAE_SHOPIFY_STORAGE: "memory",
      }),
    ).toThrow("cannot run in production");
  });
});
