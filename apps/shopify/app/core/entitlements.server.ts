import { createHash } from "node:crypto";

export type Entitlement =
  | { mode: "existing_piqae"; accountId: string }
  | { mode: "shopify_child"; accountId: string; planHandle: string };

export interface PartnerPricingClient {
  activeSubscription(input: {
    appId: string;
    shopId: string;
  }): Promise<{ status: string; planHandle: string | null } | null>;
}
export interface ExistingPiqaeVerifier {
  verify(token: string): Promise<{ accountId: string; active: boolean }>;
}
export interface ChildTenantProvisioner {
  provision(input: {
    shop: string;
    planHandle: string;
    idempotencyKey: string;
  }): Promise<{ accountId: string; credential: string }>;
}

export class EntitlementService {
  constructor(
    private readonly pricing: PartnerPricingClient,
    private readonly verifier: ExistingPiqaeVerifier,
    private readonly provisioner: ChildTenantProvisioner,
    private readonly appId: string,
    private readonly allowedPlans: ReadonlySet<string>,
  ) {}
  async linkExisting(token: string): Promise<Entitlement> {
    const account = await this.verifier.verify(token);
    if (!account.active) throw new Error("PIQAE_SUBSCRIPTION_INACTIVE");
    return { mode: "existing_piqae", accountId: account.accountId };
  }
  async provisionChild(input: {
    shop: string;
    shopId: string;
    redirectPlanHandle?: string;
  }): Promise<Entitlement & { credential: string }> {
    const subscription = await this.pricing.activeSubscription({
      appId: this.appId,
      shopId: input.shopId,
    });
    if (
      !subscription ||
      subscription.status !== "ACTIVE" ||
      !subscription.planHandle
    )
      throw new Error("SHOPIFY_SUBSCRIPTION_INACTIVE");
    if (
      input.redirectPlanHandle &&
      input.redirectPlanHandle !== subscription.planHandle
    )
      throw new Error("PLAN_HANDLE_MISMATCH");
    if (!this.allowedPlans.has(subscription.planHandle))
      throw new Error("PLAN_HANDLE_NOT_ALLOWED");
    const idempotencyKey = `shopify-child-${createHash("sha256").update(`${this.appId}\0${input.shop}`).digest("hex")}`;
    const child = await this.provisioner.provision({
      shop: input.shop,
      planHandle: subscription.planHandle,
      idempotencyKey,
    });
    return {
      mode: "shopify_child",
      accountId: child.accountId,
      planHandle: subscription.planHandle,
      credential: child.credential,
    };
  }
}

export class ShopifyPartnerGraphqlClient implements PartnerPricingClient {
  constructor(
    private readonly endpoint: string,
    private readonly token: string,
    private readonly fetcher: typeof fetch = fetch,
    private readonly timeoutMs = 10_000,
  ) {}
  async activeSubscription(input: { appId: string; shopId: string }) {
    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), this.timeoutMs);
    let response: Response;
    try {
      response = await this.fetcher(this.endpoint, {
        method: "POST",
        signal: controller.signal,
        headers: {
          "X-Shopify-Access-Token": this.token,
          "content-type": "application/json",
        },
        body: JSON.stringify({
          query: `query PiqaeActiveSubscription($appId: ID!, $shopId: ID!) { activeSubscription(appId: $appId, shopId: $shopId) { status plan { handle } } }`,
          variables: input,
        }),
      });
    } catch {
      throw new Error("PARTNER_API_TRANSPORT_FAILED");
    } finally {
      clearTimeout(timeout);
    }
    const body = (await response.json()) as any;
    if (!response.ok || body.errors?.length)
      throw new Error("PARTNER_API_FAILED");
    const subscription = body.data?.activeSubscription;
    return subscription
      ? {
          status: String(subscription.status),
          planHandle: subscription.plan?.handle
            ? String(subscription.plan.handle)
            : null,
        }
      : null;
  }
}
