import { PiqaeClient } from "@piqae/sdk";
import { CredentialVault } from "./core/credentials.server";
import { MemoryShopRepository, type ShopRepository } from "./core/model";
import { ShopifyPrintingService } from "./core/printing.server";
import { PostgresShopRepository } from "./core/postgres-shop-repository.server";
import pg from "pg";
import { DownloadTokenVault } from "./core/download-token.server";
import { CloudflareEmailClient, EmailDeliveryError } from "./core/cloudflare-email.server";
import { ProductionAutomationDelivery } from "./core/production-automation-delivery.server";

let injected: ShopRepository | undefined;
let memoizedProduction: ReturnType<typeof buildServices> | undefined;
export function setRepositoryForTests(repository: ShopRepository | undefined) {
  injected = repository;
}
function required(name: string) {
  const value = process.env[name];
  if (!value) throw new Error(`${name} is required`);
  return value;
}

export function createProductionServices() {
  if (!injected && process.env.NODE_ENV === "production" && memoizedProduction)
    return memoizedProduction;
  const value = buildServices();
  if (!injected && process.env.NODE_ENV === "production")
    memoizedProduction = value;
  return value;
}

function buildServices() {
  const repository =
    injected ??
    (process.env.NODE_ENV === "production"
      ? new PostgresShopRepository(
          new pg.Pool({
            connectionString: required("DATABASE_URL"),
            max: 10,
            statement_timeout: 10_000,
          }),
        )
      : new MemoryShopRepository());
  const vault = CredentialVault.fromBase64(
    required("PIQAE_SHOPIFY_CREDENTIAL_KEY"),
  );
  const downloadTokens = new DownloadTokenVault(
    Buffer.from(required("PIQAE_SHOPIFY_DOWNLOAD_KEY"), "base64"),
  );
  const baseUrl = process.env.PIQAE_API_URL ?? "https://api.piqae.com";
  const printing = new ShopifyPrintingService(
    repository,
    vault,
    (token) => new PiqaeClient({ baseUrl, accessToken: () => token }),
    required("SHOPIFY_APP_URL"),
  );
  const emailConfigured =
    process.env.CLOUDFLARE_ACCOUNT_ID &&
    process.env.CLOUDFLARE_EMAIL_API_TOKEN &&
    process.env.CLOUDFLARE_EMAIL_FROM_ADDRESS;
  const automationDelivery = emailConfigured
    ? new ProductionAutomationDelivery(
        printing,
        new CloudflareEmailClient({
          accountId: process.env.CLOUDFLARE_ACCOUNT_ID!,
          token: process.env.CLOUDFLARE_EMAIL_API_TOKEN!,
          fromAddress: process.env.CLOUDFLARE_EMAIL_FROM_ADDRESS!,
          fromName:
            process.env.CLOUDFLARE_EMAIL_FROM_NAME ?? "Piqae Order Printing",
          replyTo: process.env.CLOUDFLARE_EMAIL_REPLY_TO,
        }),
        {
          load: async (shop, renderId) => {
            const link = await repository.get(shop);
            if (!link) throw new Error("PIQAE_LINK_NOT_FOUND");
            const credential = vault.open(link.encryptedCredential, shop);
            for (let attempt = 0; attempt < 20; attempt += 1) {
              const response = await fetch(
                `${baseUrl}/v1/document-renders/${encodeURIComponent(renderId)}/artifact`,
                { headers: { authorization: `Bearer ${credential}` }, signal: AbortSignal.timeout(10_000) },
              );
              if (response.ok)
                return new Uint8Array(await response.arrayBuffer());
              if (response.status !== 409)
                throw new Error(`PIQAE_ARTIFACT_FAILED_${response.status}`);
              await new Promise((resolve) => setTimeout(resolve, Math.min(2_000, 100 * 2 ** attempt)));
            }
            throw new Error("PIQAE_ARTIFACT_TIMEOUT");
          },
        },
      )
    : new ProductionAutomationDelivery(printing, undefined, { load: async () => { throw new EmailDeliveryError("email provider is not configured", false); } });
  return {
    repository,
    vault,
    downloadTokens,
    baseUrl,
    printing,
    automationDelivery,
  };
}
