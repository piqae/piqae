import { createHash } from "node:crypto";
import { PiqaeClient, PiqaePlatform, type PiqaeAccount } from "@piqae/sdk";
import type { ShopLink, ShopRepository } from "./model";
import { normalizeShopDomain } from "./model";
import { starterTemplates } from "./starter-templates";
import {
  parseTemplateEnvelope,
  serializeTemplateEnvelope,
} from "./template-model";
import { seedStarterTemplates } from "./template-index.server";
import type { WorkflowRepository } from "./workflows.server";

export class ManagedPiqaeAccountService {
  private readonly platform: PiqaePlatform;
  private readonly platformKey: string;

  constructor(
    private readonly shops: ShopRepository,
    private readonly workflows: WorkflowRepository,
    platformKey: string,
    baseUrl: string,
    fetcher: typeof globalThis.fetch = globalThis.fetch,
  ) {
    this.platformKey = platformKey;
    this.platform = new PiqaePlatform({ platformKey, baseUrl, fetch: fetcher });
  }

  async ensure(rawShop: string): Promise<ShopLink> {
    const shop = normalizeShopDomain(rawShop);
    const existing = await this.shops.get(shop);
    if (existing?.entitlementMode === "shopify_child") return existing;

    const account = await this.platform.accounts.getOrCreate(shop, {
      name: shop.replace(/\.myshopify\.com$/, ""),
      metadata: { source: "shopify", shop },
    });
    const templateRevisionId = await this.publishStarters(shop, account.live);
    const link: ShopLink = {
      shop,
      piqaeAccountId: account.id,
      piqaeLiveEnvironmentId: account.environments.live.id,
      piqaeTestEnvironmentId: account.environments.test.id,
      encryptedCredential: "",
      templateRevisionId,
      entitlementMode: "shopify_child",
      planHandle: existing?.planHandle ?? "development",
      createdAt: existing?.createdAt ?? new Date().toISOString(),
    };
    await this.shops.put(link);
    return link;
  }

  client(link: ShopLink): PiqaeClient {
    if (
      link.entitlementMode !== "shopify_child" ||
      !link.piqaeLiveEnvironmentId
    )
      throw new Error("PIQAE_MANAGED_ACCOUNT_NOT_READY");
    return new PiqaeClient({
      platformKey: this.platformKey,
      platformContext: {
        workspaceId: link.piqaeAccountId,
        environmentId: link.piqaeLiveEnvironmentId,
      },
      baseUrl: this.platform.baseUrl,
    });
  }

  private async publishStarters(
    shop: string,
    client: PiqaeAccount["live"],
  ): Promise<string> {
    await seedStarterTemplates(this.workflows, shop);
    const stored = await this.workflows.listTemplates(shop);
    let defaultRevisionId: string | undefined;
    for (const starter of starterTemplates) {
      const digest = createHash("sha256")
        .update(`${shop}\0${starter.id}`)
        .digest("hex");
      const template = await client.businessDocuments.templates.create(
        {
          name: `Shopify ${starter.name}`,
          specification: starter.specification,
        },
        `shopify-managed-template-${digest}`,
      );
      const revision = await client.businessDocuments.templates.publish(
        template.id,
        starter.specification,
        `shopify-managed-publish-${digest}`,
      );
      const local = stored.find((candidate) => {
        try {
          return (
            parseTemplateEnvelope(candidate.source).system?.key === starter.id
          );
        } catch {
          return false;
        }
      });
      if (!local) throw new Error("PIQAE_DEFAULT_TEMPLATE_MISSING");
      const envelope = parseTemplateEnvelope(local.source);
      envelope.published = {
        piqaeTemplateId: template.id,
        piqaeRevisionId: revision.id,
        canonicalDigest: createHash("sha256")
          .update(JSON.stringify(starter.specification))
          .digest("hex"),
      };
      await this.workflows.saveTemplate(shop, {
        ...local,
        source: serializeTemplateEnvelope(envelope),
      });
      if (starter.id === "invoice") defaultRevisionId = revision.id;
    }
    if (!defaultRevisionId) throw new Error("PIQAE_DEFAULT_TEMPLATE_MISSING");
    return defaultRevisionId;
  }
}
