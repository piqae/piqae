export type LegacyOfflineSession = {
  shop: string;
  accessToken?: string;
};

export async function migrateLegacyOfflineSessionWith<T>(
  session: LegacyOfflineSession,
  migrate: (input: {
    shop: string;
    nonExpiringOfflineAccessToken: string;
  }) => Promise<T>,
  store: (session: T) => Promise<boolean>,
): Promise<T> {
  if (!session.accessToken)
    throw new Error("Shopify offline session has no access token to migrate");

  // Shopify revokes the legacy credential when this one-time grant succeeds.
  // Never replay it here: persist the returned pair immediately and let the
  // caller require reauthorization if either operation has an uncertain result.
  const migrated = await migrate({
    shop: session.shop,
    nonExpiringOfflineAccessToken: session.accessToken,
  });
  const stored = await store(migrated);
  if (!stored)
    throw new Error("Shopify expiring offline session could not be stored");
  return migrated;
}
