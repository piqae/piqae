import { describe, expect, it, vi } from "vitest";

import { migrateLegacyOfflineSessionWith } from "../app/core/legacy-offline-session.server";

describe("legacy Shopify offline-session migration", () => {
  it("migrates once and persists the expiring token pair", async () => {
    const migrated = {
      shop: "store.myshopify.com",
      accessToken: "expiring-access-token",
      refreshToken: "rotating-refresh-token",
    };
    const migrate = vi.fn(async () => migrated);
    const store = vi.fn(async () => true);

    await expect(
      migrateLegacyOfflineSessionWith(
        {
          shop: "store.myshopify.com",
          accessToken: "legacy-offline-token",
        },
        migrate,
        store,
      ),
    ).resolves.toBe(migrated);
    expect(migrate).toHaveBeenCalledOnce();
    expect(store).toHaveBeenCalledExactlyOnceWith(migrated);
  });

  it("does not call the irreversible grant without the legacy token", async () => {
    const migrate = vi.fn();
    const store = vi.fn();
    await expect(
      migrateLegacyOfflineSessionWith(
        { shop: "store.myshopify.com" },
        migrate,
        store,
      ),
    ).rejects.toThrow("no access token");
    expect(migrate).not.toHaveBeenCalled();
    expect(store).not.toHaveBeenCalled();
  });

  it("does not replay a completed migration when persistence fails", async () => {
    const migrate = vi.fn(async () => ({
      shop: "store.myshopify.com",
      accessToken: "expiring-access-token",
      refreshToken: "rotating-refresh-token",
    }));
    const store = vi.fn(async () => false);
    await expect(
      migrateLegacyOfflineSessionWith(
        {
          shop: "store.myshopify.com",
          accessToken: "legacy-offline-token",
        },
        migrate,
        store,
      ),
    ).rejects.toThrow("could not be stored");
    expect(migrate).toHaveBeenCalledOnce();
    expect(store).toHaveBeenCalledOnce();
  });
});
