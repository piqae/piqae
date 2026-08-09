import pg from "pg";
import { normalizeShopDomain } from "./model";

const development = new Map<
  string,
  { state: "installed" | "uninstalled"; scopes: string }
>();
let pool: pg.Pool | undefined;
function postgres(): pg.Pool {
  const connectionString = process.env.DATABASE_URL;
  if (!connectionString)
    throw new Error(
      "DATABASE_URL is required for durable Shopify installations",
    );
  return (pool ??= new pg.Pool({
    connectionString,
    max: 5,
    statement_timeout: 10_000,
  }));
}

export async function recordInstallation(
  rawShop: string,
  scopes: string,
): Promise<void> {
  const shop = normalizeShopDomain(rawShop);
  if (
    process.env.NODE_ENV === "test" ||
    process.env.NODE_ENV === "development"
  ) {
    development.set(shop, { state: "installed", scopes });
    return;
  }
  await postgres().query(
    `INSERT INTO shopify_installations(shop,state,scopes) VALUES($1,'installed',$2)
    ON CONFLICT(shop) DO UPDATE SET state='installed',scopes=EXCLUDED.scopes,uninstalled_at=NULL,updated_at=now()`,
    [shop, scopes],
  );
}

export async function markInstallationUninstalled(
  rawShop: string,
): Promise<void> {
  const shop = normalizeShopDomain(rawShop);
  if (
    process.env.NODE_ENV === "test" ||
    process.env.NODE_ENV === "development"
  ) {
    const current = development.get(shop);
    if (current) development.set(shop, { ...current, state: "uninstalled" });
    return;
  }
  await postgres().query(
    "UPDATE shopify_installations SET state='uninstalled',uninstalled_at=now(),updated_at=now() WHERE shop=$1",
    [shop],
  );
}

export async function redactInstallation(rawShop: string): Promise<void> {
  const shop = normalizeShopDomain(rawShop);
  if (
    process.env.NODE_ENV === "test" ||
    process.env.NODE_ENV === "development"
  ) {
    development.delete(shop);
    return;
  }
  await postgres().query("DELETE FROM shopify_installations WHERE shop=$1", [
    shop,
  ]);
}
