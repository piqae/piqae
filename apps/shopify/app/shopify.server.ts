import {
  ApiVersion,
  AppDistribution,
  shopifyApp,
} from "@shopify/shopify-app-react-router/server";
import { MemorySessionStorage } from "@shopify/shopify-app-session-storage-memory";
import { PostgreSQLSessionStorage } from "@shopify/shopify-app-session-storage-postgresql";
import { recordInstallation } from "./core/installations.server";

function required(name: string): string {
  const value = process.env[name];
  if (!value) throw new Error(`${name} is required`);
  return value;
}

// Memory storage is development-only. Production must inject a durable SessionStorage
// implementation before importing this module.
const sessionStorage =
  process.env.NODE_ENV === "production"
    ? new PostgreSQLSessionStorage(required("DATABASE_URL"), {
        sessionTableName: "shopify_sessions",
      })
    : new MemorySessionStorage();

const shopify = shopifyApp({
  apiKey: required("SHOPIFY_API_KEY"),
  apiSecretKey: required("SHOPIFY_API_SECRET"),
  appUrl: required("SHOPIFY_APP_URL"),
  apiVersion: ApiVersion.July26,
  scopes: (
    process.env.SCOPES ??
    "read_orders,read_all_orders,read_draft_orders,read_products,read_customers"
  )
    .split(",")
    .map((scope) => scope.trim())
    .filter(Boolean),
  distribution: AppDistribution.AppStore,
  sessionStorage,
  hooks: {
    afterAuth: async ({ session }) => {
      await recordInstallation(session.shop, session.scope ?? "");
      await shopify.registerWebhooks({ session });
    },
  },
});

export default shopify;
export const authenticate = shopify.authenticate;
