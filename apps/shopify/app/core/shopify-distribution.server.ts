import { AppDistribution } from "@shopify/shopify-app-react-router/server";

export function configuredShopifyDistribution(
  value = process.env.SHOPIFY_DISTRIBUTION,
): AppDistribution {
  switch (value?.trim() || "app_store") {
    case "app_store":
      return AppDistribution.AppStore;
    case "single_merchant":
      return AppDistribution.SingleMerchant;
    default:
      throw new Error(
        "SHOPIFY_DISTRIBUTION must be app_store or single_merchant",
      );
  }
}
