export type PiqaeRuntimeMode = "fake" | "local" | "live";

export interface PiqaeRuntime {
  mode: PiqaeRuntimeMode;
  baseUrl: string;
  permitsPhysicalPrinting: boolean;
}

export type ShopifyStorageMode = "memory" | "postgres";

export function resolveShopifyStorage(
  environment: NodeJS.ProcessEnv = process.env,
): ShopifyStorageMode {
  const configured = environment.PIQAE_SHOPIFY_STORAGE;
  if (
    configured !== undefined &&
    configured !== "memory" &&
    configured !== "postgres"
  )
    throw new Error("PIQAE_SHOPIFY_STORAGE must be memory or postgres");
  const mode =
    configured ?? (environment.NODE_ENV === "test" ? "memory" : "postgres");
  if (mode === "memory" && environment.NODE_ENV === "production")
    throw new Error("memory Shopify storage cannot run in production");
  if (mode === "postgres" && !environment.DATABASE_URL)
    throw new Error("DATABASE_URL is required for PostgreSQL Shopify storage");
  return mode;
}

export function resolvePiqaeRuntime(
  environment: NodeJS.ProcessEnv = process.env,
): PiqaeRuntime {
  const mode = (environment.PIQAE_SHOPIFY_RUNTIME ??
    (environment.NODE_ENV === "production" ? "live" : "fake")) as
    | PiqaeRuntimeMode
    | string;
  if (!(["fake", "local", "live"] as const).includes(mode as PiqaeRuntimeMode))
    throw new Error("PIQAE_SHOPIFY_RUNTIME must be fake, local, or live");

  const fallback =
    mode === "live" ? "https://api.piqae.com" : "http://127.0.0.1:8080";
  const url = new URL(environment.PIQAE_API_URL ?? fallback);
  const loopback =
    url.hostname === "127.0.0.1" ||
    url.hostname === "localhost" ||
    url.hostname === "[::1]";

  if (url.username || url.password || url.search || url.hash)
    throw new Error(
      "PIQAE_API_URL must not contain credentials, query, or fragment",
    );
  if (mode === "fake" && (!loopback || environment.NODE_ENV === "production"))
    throw new Error(
      "fake Piqae runtime is loopback-only and cannot run in production",
    );
  if (
    mode === "local" &&
    url.protocol !== "https:" &&
    !(loopback && url.protocol === "http:")
  )
    throw new Error(
      "local Piqae runtime requires HTTPS or a loopback HTTP URL",
    );
  if (mode === "live" && url.protocol !== "https:")
    throw new Error("live Piqae runtime requires HTTPS");

  return {
    mode: mode as PiqaeRuntimeMode,
    baseUrl: url.toString().replace(/\/$/, ""),
    // This is descriptive only. Physical tests still require the agent-side
    // explicit printer/fixture authorization gate.
    permitsPhysicalPrinting: mode !== "fake",
  };
}
