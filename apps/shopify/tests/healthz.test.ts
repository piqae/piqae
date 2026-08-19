import { afterEach, describe, expect, it } from "vitest";
import { loader } from "../app/routes/healthz";

const originalReleaseSha = process.env.PIQAE_RELEASE_SHA;
const originalRailwaySha = process.env.RAILWAY_GIT_COMMIT_SHA;

afterEach(() => {
  if (originalReleaseSha === undefined) delete process.env.PIQAE_RELEASE_SHA;
  else process.env.PIQAE_RELEASE_SHA = originalReleaseSha;
  if (originalRailwaySha === undefined)
    delete process.env.RAILWAY_GIT_COMMIT_SHA;
  else process.env.RAILWAY_GIT_COMMIT_SHA = originalRailwaySha;
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
});
