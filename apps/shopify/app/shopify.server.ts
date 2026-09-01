import {
  ApiVersion,
  shopifyApp,
} from "@shopify/shopify-app-react-router/server";
import { shopifyApi } from "@shopify/shopify-api";
import { MemorySessionStorage } from "@shopify/shopify-app-session-storage-memory";
import { PostgreSQLSessionStorage } from "@shopify/shopify-app-session-storage-postgresql";
import { recordInstallation } from "./core/installations.server";
import { migrateLegacyOfflineSessionWith } from "./core/legacy-offline-session.server";
import { configuredShopifyDistribution } from "./core/shopify-distribution.server";
import { configuredShopifyScopes } from "./core/shopify-scopes.server";

function required(name: string): string {
  const value = process.env[name];
  if (!value) throw new Error(`${name} is required`);
  return value;
}

const apiKey = required("SHOPIFY_API_KEY");
const apiSecretKey = required("SHOPIFY_API_SECRET");
const appUrl = required("SHOPIFY_APP_URL");
const scopes = configuredShopifyScopes(process.env.SCOPES);

// Memory storage is development-only. Production must inject a durable SessionStorage
// implementation before importing this module.
const sessionStorage =
  process.env.NODE_ENV === "production"
    ? new PostgreSQLSessionStorage(required("DATABASE_URL"), {
        sessionTableName: "shopify_sessions",
      })
    : new MemorySessionStorage();

const legacyTokenMigrationApi = shopifyApi({
  apiKey,
  apiSecretKey,
  apiVersion: ApiVersion.July26,
  hostName: new URL(appUrl).host,
  isEmbeddedApp: true,
  scopes,
});

/**
 * Shopify now rejects legacy non-expiring offline tokens at the Admin API.
 * Its regular request-token exchange cannot migrate those stored credentials;
 * the dedicated migration grant must use the legacy offline token itself.
 *
 * The migration is intentionally attempted exactly once. Shopify revokes the
 * old token when it issues the expiring access/refresh pair, so callers must
 * surface a reauthorization state instead of blindly replaying a failed grant.
 */
export async function migrateLegacyOfflineSession(session: {
  shop: string;
  accessToken?: string;
}) {
  return migrateLegacyOfflineSessionWith(
    session,
    async (input) =>
      (await legacyTokenMigrationApi.auth.migrateToExpiringToken(input))
        .session,
    (migrated) => sessionStorage.storeSession(migrated),
  );
}

const shopify = shopifyApp({
  apiKey,
  apiSecretKey,
  appUrl,
  apiVersion: ApiVersion.July26,
  scopes,
  distribution: configuredShopifyDistribution(),
  future: {
    expiringOfflineAccessTokens: true,
  },
  sessionStorage,
  hooks: {
    afterAuth: async ({ session }) => {
      await recordInstallation(session.shop, session.scope ?? "");
      // Webhooks are app-specific subscriptions managed by the released
      // shopify.app TOML. registerWebhooks is only for shop-specific runtime
      // subscriptions and can fail an otherwise successful installation.
      try {
        const { createProductionServices } = await import("./services.server");
        await createProductionServices().managedAccounts.ensure(session.shop);
      } catch {
        // Installation must remain recoverable during a transient Piqae outage.
        // Authenticated app loaders retry the idempotent provisioning operation.
        console.error("Managed Piqae account provisioning was deferred");
      }
    },
  },
});

export default shopify;
export const authenticate = shopify.authenticate;
