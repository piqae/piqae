import type { Pool } from "pg";
import type { ShopLink, ShopRepository } from "./model";
import { normalizeShopDomain } from "./model";

const poolShopLocks = new WeakMap<Pool, Map<string, Promise<void>>>();

export class PostgresShopRepository implements ShopRepository {
  constructor(
    private readonly pool: Pool,
    private readonly lockPool: Pool,
  ) {}
  async get(rawShop: string): Promise<ShopLink | null> {
    const shop = normalizeShopDomain(rawShop);
    const result = await this.pool.query(
      "SELECT shop, piqae_account_id, encrypted_credential, piqae_live_environment_id, piqae_test_environment_id, template_revision_id, entitlement_mode, plan_handle, created_at, xmin::text AS repository_revision FROM shopify_shop_links WHERE shop = $1",
      [shop],
    );
    const row = result.rows[0];
    return row
      ? {
          shop: row.shop,
          piqaeAccountId: row.piqae_account_id,
          encryptedCredential: row.encrypted_credential ?? "",
          piqaeLiveEnvironmentId: row.piqae_live_environment_id,
          piqaeTestEnvironmentId: row.piqae_test_environment_id,
          templateRevisionId: row.template_revision_id,
          entitlementMode: row.entitlement_mode,
          planHandle: row.plan_handle,
          createdAt: new Date(row.created_at).toISOString(),
          repositoryRevision: row.repository_revision,
        }
      : null;
  }
  async put(link: ShopLink) {
    const shop = normalizeShopDomain(link.shop);
    await this.pool.query(
      `INSERT INTO shopify_shop_links
        (shop, piqae_account_id, encrypted_credential, piqae_live_environment_id,
         piqae_test_environment_id, template_revision_id, entitlement_mode, plan_handle)
       VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
       ON CONFLICT (shop) DO UPDATE SET
         piqae_account_id=EXCLUDED.piqae_account_id,
         encrypted_credential=EXCLUDED.encrypted_credential,
         piqae_live_environment_id=EXCLUDED.piqae_live_environment_id,
         piqae_test_environment_id=EXCLUDED.piqae_test_environment_id,
         template_revision_id=EXCLUDED.template_revision_id,
         entitlement_mode=EXCLUDED.entitlement_mode,
         plan_handle=EXCLUDED.plan_handle,
         updated_at=now()`,
      [
        shop,
        link.piqaeAccountId,
        link.entitlementMode === "shopify_child"
          ? null
          : link.encryptedCredential,
        link.piqaeLiveEnvironmentId ?? null,
        link.piqaeTestEnvironmentId ?? null,
        link.templateRevisionId,
        link.entitlementMode ?? "existing_piqae",
        link.planHandle ?? null,
      ],
    );
  }
  async withShopLock<T>(rawShop: string, action: () => Promise<T>): Promise<T> {
    const shop = normalizeShopDomain(rawShop);
    let locks = poolShopLocks.get(this.lockPool);
    if (!locks) {
      locks = new Map();
      poolShopLocks.set(this.lockPool, locks);
    }
    return withQueuedLock(locks, shop, () =>
      this.withDatabaseShopLock(shop, action),
    );
  }
  private async withDatabaseShopLock<T>(
    shop: string,
    action: () => Promise<T>,
  ): Promise<T> {
    const client = await this.lockPool.connect();
    let locked = false;
    try {
      await client.query("SELECT pg_advisory_lock(hashtextextended($1, 0))", [
        shop,
      ]);
      locked = true;
      return await action();
    } finally {
      if (locked) {
        try {
          const result = await client.query(
            "SELECT pg_advisory_unlock(hashtextextended($1, 0))",
            [shop],
          );
          if (result.rows[0]?.pg_advisory_unlock !== true)
            throw new Error("PIQAE_SHOP_LOCK_LOST");
        } catch (error) {
          client.release(true);
          throw error;
        }
      }
      client.release();
    }
  }
  async putIfCurrentMatches(link: ShopLink, expected: ShopLink | null) {
    const shop = normalizeShopDomain(link.shop);
    const next = linkValues(link);
    if (!expected) {
      const result = await this.pool.query(
        `INSERT INTO shopify_shop_links
          (shop,piqae_account_id,encrypted_credential,piqae_live_environment_id,
           piqae_test_environment_id,template_revision_id,entitlement_mode,plan_handle)
         VALUES($1,$2,$3,$4,$5,$6,$7,$8)
         ON CONFLICT(shop) DO NOTHING RETURNING shop`,
        [shop, ...next],
      );
      return result.rowCount === 1;
    }
    if (!expected.repositoryRevision) return false;
    const prior = linkValues(expected);
    const result = await this.pool.query(
      `UPDATE shopify_shop_links SET
         piqae_account_id=$2,encrypted_credential=$3,
         piqae_live_environment_id=$4,piqae_test_environment_id=$5,
         template_revision_id=$6,entitlement_mode=$7,plan_handle=$8,
         updated_at=clock_timestamp()
       WHERE shop=$1
         AND piqae_account_id=$9
         AND encrypted_credential IS NOT DISTINCT FROM $10
         AND piqae_live_environment_id IS NOT DISTINCT FROM $11
         AND piqae_test_environment_id IS NOT DISTINCT FROM $12
         AND template_revision_id=$13
         AND entitlement_mode=$14
         AND plan_handle IS NOT DISTINCT FROM $15
         AND xmin::text=$16
       RETURNING shop`,
      [shop, ...next, ...prior, expected.repositoryRevision],
    );
    return result.rowCount === 1;
  }
  async deleteShop(rawShop: string) {
    const shop = normalizeShopDomain(rawShop);
    await this.withShopLock(shop, async () => {
      await this.pool.query("DELETE FROM shopify_shop_links WHERE shop=$1", [
        shop,
      ]);
    });
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

function linkValues(link: ShopLink) {
  return [
    link.piqaeAccountId,
    link.entitlementMode === "shopify_child" ? null : link.encryptedCredential,
    link.piqaeLiveEnvironmentId ?? null,
    link.piqaeTestEnvironmentId ?? null,
    link.templateRevisionId,
    link.entitlementMode ?? "existing_piqae",
    link.planHandle ?? null,
  ];
}

async function withQueuedLock<T>(
  locks: Map<string, Promise<void>>,
  key: string,
  action: () => Promise<T>,
): Promise<T> {
  const previous = locks.get(key) ?? Promise.resolve();
  let release!: () => void;
  const current = new Promise<void>((resolve) => {
    release = resolve;
  });
  const tail = previous.then(() => current);
  locks.set(key, tail);
  await previous;
  try {
    return await action();
  } finally {
    release();
    if (locks.get(key) === tail) locks.delete(key);
  }
}
