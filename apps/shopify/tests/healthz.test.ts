import { afterEach, describe, expect, it } from "vitest";
import { loader } from "../app/routes/healthz";

const originalReleaseSha = process.env.PIQAE_RELEASE_SHA;
const originalRailwaySha = process.env.RAILWAY_GIT_COMMIT_SHA;
const originalNodeEnv = process.env.NODE_ENV;

afterEach(() => {
  if (originalReleaseSha === undefined) delete process.env.PIQAE_RELEASE_SHA;
  else process.env.PIQAE_RELEASE_SHA = originalReleaseSha;
  if (originalRailwaySha === undefined)
    delete process.env.RAILWAY_GIT_COMMIT_SHA;
  else process.env.RAILWAY_GIT_COMMIT_SHA = originalRailwaySha;
  if (originalNodeEnv === undefined) delete process.env.NODE_ENV;
  else process.env.NODE_ENV = originalNodeEnv;
});

describe("Shopify health endpoint", () => {
  it("binds health evidence to the deployed commit", async () => {
    process.env.RAILWAY_GIT_COMMIT_SHA = "A".repeat(40);
    delete process.env.PIQAE_RELEASE_SHA;

    const response = loader({} as never);

    expect(response.headers.get("cache-control")).toBe("no-store");
    expect(await response.json()).toEqual({
      status: "ok",
      service: "piqae-shopify",
      version: "0.2.0",
      revision: "a".repeat(40),
    });
  });

  it("does not claim an invalid deployment revision", async () => {
    process.env.PIQAE_RELEASE_SHA = "main";
    delete process.env.RAILWAY_GIT_COMMIT_SHA;

    const response = loader({} as never);

    expect(await response.json()).toMatchObject({ revision: "unknown" });
  });

  it("does not trust mutable runtime revision variables in production", async () => {
    process.env.NODE_ENV = "production";
    process.env.PIQAE_RELEASE_SHA = "b".repeat(40);
    process.env.RAILWAY_GIT_COMMIT_SHA = "c".repeat(40);

    const response = loader({} as never);

    expect(await response.json()).toMatchObject({ revision: "unknown" });
  });
});
