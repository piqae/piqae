import { describe, expect, it } from "vitest";

import { MemoryShopRepository, type ShopLink } from "../app/core/model";

const shop = "render-lifecycle.myshopify.com";

function link(account = "account_first"): ShopLink {
  return {
    shop,
    piqaeAccountId: account,
    encryptedCredential: `encrypted-${account}`,
    templateRevisionId: `revision-${account}`,
    createdAt: "2026-08-30T00:00:00.000Z",
  };
}

function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

describe("in-memory Shopify render ownership lifecycle", () => {
  it("requires a current shop link and preserves only exact idempotent retries", async () => {
    const repository = new MemoryShopRepository();
    await expect(
      repository.recordRender(shop, "render_1", "ownership-key-1"),
    ).rejects.toThrow("SHOPIFY_RENDER_OWNERSHIP_UNAVAILABLE");

    await repository.put(link());
    await repository.recordRender(shop, "render_1", "ownership-key-1", {
      orderGid: "gid://shopify/Order/10",
      customerGid: "gid://shopify/Customer/20",
    });
    await expect(
      repository.recordRender(shop, "render_1", "ownership-key-1", {
        orderGid: "gid://shopify/Order/10",
        customerGid: "gid://shopify/Customer/20",
      }),
    ).resolves.toBeUndefined();
    await expect(
      repository.recordRender(shop, "render_other", "ownership-key-1"),
    ).rejects.toThrow("SHOPIFY_RENDER_OWNERSHIP_CONFLICT");
    expect(await repository.ownsRender(shop, "render_1")).toBe(true);
  });

  it("serializes uninstall ahead of a waiting render registration", async () => {
    const repository = new MemoryShopRepository();
    await repository.put(link());
    const entered = deferred();
    const release = deferred();
    const blocker = repository.withShopLock(shop, async () => {
      entered.resolve();
      await release.promise;
    });
    await entered.promise;

    const uninstall = repository.deleteShop(shop);
    const lateRender = repository.recordRender(
      shop,
      "render_after_uninstall",
      "ownership-key-after-uninstall",
    );
    const lateRenderExpectation = expect(lateRender).rejects.toThrow(
      "SHOPIFY_RENDER_OWNERSHIP_UNAVAILABLE",
    );
    release.resolve();

    await blocker;
    await uninstall;
    await lateRenderExpectation;
    expect(await repository.ownsRender(shop, "render_after_uninstall")).toBe(
      false,
    );
  });

  it("does not restore old render ownership after reinstall", async () => {
    const repository = new MemoryShopRepository();
    await repository.put(link());
    await repository.recordRender(shop, "render_old", "ownership-key-old");

    await repository.deleteShop(shop);
    expect(await repository.ownsRender(shop, "render_old")).toBe(false);
    await repository.put(link("account_reinstalled"));
    expect(await repository.ownsRender(shop, "render_old")).toBe(false);

    await repository.recordRender(shop, "render_new", "ownership-key-new");
    expect(await repository.ownsRender(shop, "render_new")).toBe(true);
  });

  it("revokes only ownership stored for a redacted customer GID", async () => {
    const repository = new MemoryShopRepository();
    await repository.put(link());
    await repository.recordRender(shop, "render_redacted", "ownership-key-1", {
      orderGid: "gid://shopify/Order/1",
      customerGid: "gid://shopify/Customer/42",
    });
    await repository.recordRender(shop, "render_retained", "ownership-key-2", {
      orderGid: "gid://shopify/Order/2",
      customerGid: "gid://shopify/Customer/43",
    });

    await repository.redactCustomer(shop, "42");
    expect(await repository.ownsRender(shop, "render_redacted")).toBe(false);
    expect(await repository.ownsRender(shop, "render_retained")).toBe(true);
    expect(
      await repository.ownsCustomerRender(
        shop,
        "render_retained",
        "gid://shopify/Order/2",
        "gid://shopify/Customer/43",
      ),
    ).toBe(true);

    await repository.redactCustomer(shop, "gid://shopify/Customer/43");
    expect(await repository.ownsRender(shop, "render_retained")).toBe(false);
  });
});
