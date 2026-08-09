export interface ShopLink {
  shop: string;
  piqaeAccountId: string;
  encryptedCredential: string;
  templateRevisionId: string;
  entitlementMode?: "existing_piqae" | "shopify_child";
  planHandle?: string | null;
  createdAt: string;
}

export interface ShopRepository {
  get(shop: string): Promise<ShopLink | null>;
  put(link: ShopLink): Promise<void>;
  deleteShop(shop: string): Promise<void>;
  redactCustomer(shop: string, customerId: string): Promise<void>;
  claimWebhook(
    id: string,
    event?: { shop: string; topic: string; resourceId?: string },
  ): Promise<boolean>;
  recordRender(
    shop: string,
    renderId: string,
    idempotencyKey: string,
    order?: { orderGid: string; customerGid?: string },
  ): Promise<void>;
  ownsRender(shop: string, renderId: string): Promise<boolean>;
  ownsCustomerRender(
    shop: string,
    renderId: string,
    orderGid: string,
    customerGid: string,
  ): Promise<boolean>;
  latestCustomerRender(
    shop: string,
    orderGid: string,
    customerGid: string,
  ): Promise<string | null>;
}

export class MemoryShopRepository implements ShopRepository {
  private readonly links = new Map<string, ShopLink>();
  private readonly webhooks = new Set<string>();
  private readonly renders = new Map<
    string,
    { shop: string; orderGid?: string; customerGid?: string }
  >();
  async get(shop: string) {
    return this.links.get(shop) ?? null;
  }
  async put(link: ShopLink) {
    this.links.set(link.shop, structuredClone(link));
  }
  async deleteShop(shop: string) {
    this.links.delete(shop);
  }
  async redactCustomer(shop: string, customerId: string) {
    const customerGid = customerId.startsWith("gid://shopify/Customer/")
      ? customerId
      : `gid://shopify/Customer/${customerId}`;
    for (const [id, value] of this.renders)
      if (
        value.shop === normalizeShopDomain(shop) &&
        value.customerGid === customerGid
      )
        this.renders.delete(id);
  }
  async claimWebhook(
    id: string,
    _event?: { shop: string; topic: string; resourceId?: string },
  ) {
    if (this.webhooks.has(id)) return false;
    this.webhooks.add(id);
    return true;
  }
  async recordRender(
    shop: string,
    renderId: string,
    _idempotencyKey: string,
    order?: { orderGid: string; customerGid?: string },
  ) {
    this.renders.set(renderId, { shop: normalizeShopDomain(shop), ...order });
  }
  async ownsRender(shop: string, renderId: string) {
    return this.renders.get(renderId)?.shop === normalizeShopDomain(shop);
  }
  async ownsCustomerRender(
    shop: string,
    renderId: string,
    orderGid: string,
    customerGid: string,
  ) {
    const value = this.renders.get(renderId);
    return (
      value?.shop === normalizeShopDomain(shop) &&
      value.orderGid === orderGid &&
      value.customerGid === customerGid
    );
  }
  async latestCustomerRender(
    shop: string,
    orderGid: string,
    customerGid: string,
  ) {
    return (
      [...this.renders.entries()]
        .reverse()
        .find(
          ([, value]) =>
            value.shop === normalizeShopDomain(shop) &&
            value.orderGid === orderGid &&
            value.customerGid === customerGid,
        )?.[0] ?? null
    );
  }
}

export function normalizeShopDomain(shop: string): string {
  const normalized = shop.trim().toLowerCase();
  if (!/^[a-z0-9][a-z0-9-]*\.myshopify\.com$/.test(normalized))
    throw new Error("invalid Shopify shop domain");
  return normalized;
}
