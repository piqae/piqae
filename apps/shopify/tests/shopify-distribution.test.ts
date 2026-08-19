import { AppDistribution } from "@shopify/shopify-app-react-router/server";
import { describe, expect, it } from "vitest";
import { configuredShopifyDistribution } from "../app/core/shopify-distribution.server";

describe("Shopify distribution policy", () => {
  it("defaults to the permanent App Store registration", () => {
    expect(configuredShopifyDistribution()).toBe(AppDistribution.AppStore);
    expect(configuredShopifyDistribution("app_store")).toBe(
      AppDistribution.AppStore,
    );
  });

  it("allows an explicit single-store pilot registration", () => {
    expect(configuredShopifyDistribution("single_merchant")).toBe(
      AppDistribution.SingleMerchant,
    );
  });

  it("rejects ambiguous distribution modes", () => {
    expect(() => configuredShopifyDistribution("custom")).toThrow(
      "SHOPIFY_DISTRIBUTION must be app_store or single_merchant",
    );
  });
});
