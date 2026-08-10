import type { Pool } from "pg";
import type { ShopLink, ShopRepository } from "./model";
import { normalizeShopDomain } from "./model";

export class PostgresShopRepository implements ShopRepository {
  constructor(private readonly pool: Pool) {}
  async get(rawShop: string): Promise<ShopLink | null> {
    const shop = normalizeShopDomain(rawShop);
    const result = await this.pool.query(
      "SELECT shop, piqae_account_id, encrypted_credential, template_revision_id, entitlement_mode, plan_handle, created_at FROM shopify_shop_links WHERE shop = $1",
      [shop],
    );
    const row = result.rows[0];
    return row
      ? {
          shop: row.shop,
          piqaeAccountId: row.piqae_account_id,
          encryptedCredential: row.encrypted_credential,
          templateRevisionId: row.template_revision_id,
          entitlementMode: row.entitlement_mode,
          planHandle: row.plan_handle,
          createdAt: new Date(row.created_at).toISOString(),
        }
      : null;
  }
  async put(link: ShopLink) {
    const shop = normalizeShopDomain(link.shop);
    await this.pool.query(
      "INSERT INTO shopify_shop_links (shop, piqae_account_id, encrypted_credential, template_revision_id) VALUES ($1,$2,$3,$4) ON CONFLICT (shop) DO UPDATE SET piqae_account_id=EXCLUDED.piqae_account_id, encrypted_credential=EXCLUDED.encrypted_credential, template_revision_id=EXCLUDED.template_revision_id, updated_at=now()",
      [
        shop,
        link.piqaeAccountId,
        link.encryptedCredential,
        link.templateRevisionId,
      ],
    );
  }
  async deleteShop(rawShop: string) {
    await this.pool.query("DELETE FROM shopify_shop_links WHERE shop=$1", [
      normalizeShopDomain(rawShop),
    ]);
  }
  async redactCustomer(rawShop: string, customerId: string) {
    const customerGid = customerId.startsWith("gid://shopify/Customer/")
      ? customerId
      : `gid://shopify/Customer/${customerId}`;
    await this.pool.query(
      "DELETE FROM shopify_render_ownership WHERE shop=$1 AND customer_gid=$2",
      [normalizeShopDomain(rawShop), customerGid],
    );
    await this.pool.query(
      "UPDATE shopify_webhook_inbox SET payload=NULL WHERE shop=$1 AND customer_id=$2",
      [normalizeShopDomain(rawShop), customerId],
    );
  }
  async claimWebhook(
    id: string,
    event?: { shop: string; topic: string; resourceId?: string },
  ) {
    const result = await this.pool.query(
      "INSERT INTO shopify_webhook_inbox (webhook_id, shop, topic, resource_id) VALUES ($1,$2,$3,$4) ON CONFLICT DO NOTHING RETURNING webhook_id",
      [
        id,
        event ? normalizeShopDomain(event.shop) : null,
        event?.topic ?? null,
        event?.resourceId ?? null,
      ],
    );
    return result.rowCount === 1;
  }
  async recordRender(
    shop: string,
    renderId: string,
    idempotencyKey: string,
    order?: { orderGid: string; customerGid?: string },
  ) {
    await this.pool.query(
      "INSERT INTO shopify_render_ownership(shop,render_id,idempotency_key,order_gid,customer_gid) VALUES($1,$2,$3,$4,$5) ON CONFLICT(shop,idempotency_key) DO NOTHING",
      [
        normalizeShopDomain(shop),
        renderId,
        idempotencyKey,
        order?.orderGid ?? null,
        order?.customerGid ?? null,
      ],
    );
  }
  async ownsRender(shop: string, renderId: string) {
    const result = await this.pool.query(
      "SELECT 1 FROM shopify_render_ownership WHERE shop=$1 AND render_id=$2",
      [normalizeShopDomain(shop), renderId],
    );
    return result.rowCount === 1;
  }
  async ownsCustomerRender(
    shop: string,
    renderId: string,
    orderGid: string,
    customerGid: string,
  ) {
    const result = await this.pool.query(
      "SELECT 1 FROM shopify_render_ownership WHERE shop=$1 AND render_id=$2 AND order_gid=$3 AND customer_gid=$4",
      [normalizeShopDomain(shop), renderId, orderGid, customerGid],
    );
    return result.rowCount === 1;
  }
  async latestCustomerRender(
    shop: string,
    orderGid: string,
    customerGid: string,
  ) {
    const result = await this.pool.query(
      "SELECT render_id FROM shopify_render_ownership WHERE shop=$1 AND order_gid=$2 AND customer_gid=$3 ORDER BY created_at DESC LIMIT 1",
      [normalizeShopDomain(shop), orderGid, customerGid],
    );
    return result.rows[0]?.render_id ?? null;
  }
}
