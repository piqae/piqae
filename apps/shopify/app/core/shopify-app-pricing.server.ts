import type { PartnerPricingClient } from "./entitlements.server";
import { normalizeShopDomain } from "./model";

export const MANAGED_PLANS = {
  starter: { limit: 500 },
  growth: { limit: 5000 },
  scale: { limit: 2_147_483_647 },
} as const;
export type ManagedPlanHandle = keyof typeof MANAGED_PLANS;

export function hostedPricingUrl(shop: string, appHandle: string): string {
  const store = normalizeShopDomain(shop).replace(/\.myshopify\.com$/, "");
  if (!/^[a-z0-9][a-z0-9-]*$/.test(appHandle))
    throw new Error("SHOPIFY_APP_HANDLE is invalid");
  return `https://admin.shopify.com/store/${encodeURIComponent(store)}/charges/${encodeURIComponent(appHandle)}/pricing_plans`;
}

export async function confirmManagedPlan(
  client: PartnerPricingClient,
  input: { appId: string; shopId: string; returnedHandle: string },
): Promise<ManagedPlanHandle> {
  if (!(input.returnedHandle in MANAGED_PLANS))
    throw new Error("PLAN_HANDLE_NOT_ALLOWED");
  const active = await client.activeSubscription({
    appId: input.appId,
    shopId: input.shopId,
  });
  if (
    !active ||
    active.status !== "ACTIVE" ||
    active.planHandle !== input.returnedHandle
  )
    throw new Error("SHOPIFY_SUBSCRIPTION_NOT_CONFIRMED");
  return input.returnedHandle as ManagedPlanHandle;
}
