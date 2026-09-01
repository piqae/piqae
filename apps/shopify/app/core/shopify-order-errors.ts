export class ShopifyOrderUnavailableError extends Error {
  readonly reason: "standard_history_only" | "unavailable";

  constructor(reason: "standard_history_only" | "unavailable") {
    super(
      reason === "standard_history_only"
        ? "Shopify order is unavailable with standard order-history access"
        : "Shopify order is unavailable to the app",
    );
    this.name = "ShopifyOrderUnavailableError";
    this.reason = reason;
  }
}
