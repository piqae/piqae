import { createHash } from "node:crypto";
import { PiqaeClient, PiqaePlatform } from "@piqae/sdk";
import type { ShopLink, ShopRepository } from "./model";
import { normalizeShopDomain } from "./model";
import { starterTemplates } from "./starter-templates";
import {
  parseTemplateEnvelope,
  serializeTemplateEnvelope,
} from "./template-model";
import {
  findSystemTemplate,
  seedStarterTemplates,
  systemTemplateId,
} from "./template-index.server";
import type {
  BillingState,
  SaveMerchantTemplate,
  WorkflowRepository,
} from "./workflows.server";

export class ManagedPiqaeAccountService {
  private readonly platform: PiqaePlatform;
  private readonly platformKey: string;

  constructor(
    private readonly shops: ShopRepository,
    private readonly workflows: WorkflowRepository,
    platformKey: string,
    baseUrl: string,
    private readonly fetcher: typeof globalThis.fetch = globalThis.fetch,
  ) {
    this.platformKey = platformKey;
    this.platform = new PiqaePlatform({ platformKey, baseUrl, fetch: fetcher });
  }

  async ensure(rawShop: string): Promise<ShopLink> {
    const shop = normalizeShopDomain(rawShop);
    return this.shops.withShopLock(shop, () => this.ensureLocked(shop));
  }

  async activatePlan(
    rawShop: string,
    plan: Exclude<BillingState["plan"], "free">,
    limit: number,
  ): Promise<ShopLink> {
    const shop = normalizeShopDomain(rawShop);
    return this.shops.withShopLock(shop, async () => {
      await this.ensureLocked(shop);
      const current = await this.shops.get(shop);
      if (!current || current.entitlementMode !== "shopify_child")
        throw new Error("PIQAE_MANAGED_ACCOUNT_NOT_READY");
      const updated = { ...current, planHandle: plan };
      if (!(await this.shops.putIfCurrentMatches(updated, current)))
        throw new Error("PIQAE_ACCOUNT_LINK_CHANGED");
      const previous = await this.workflows.getBilling(shop);
      await this.workflows.saveBilling(shop, {
        mode: "shopify_child",
        plan,
        used: previous.used,
        limit,
        status: "active",
      });
      return updated;
    });
  }

  private async ensureLocked(shop: string): Promise<ShopLink> {
    const existing = await this.shops.get(shop);
    if (existing?.entitlementMode === "shopify_child") {
      if (await this.starterGenerationCurrent(shop, existing)) return existing;
      if (!existing.piqaeLiveEnvironmentId)
        throw new Error("PIQAE_MANAGED_ACCOUNT_NOT_READY");
      const templateRevisionId = await this.publishStarters(
        shop,
        existing.piqaeAccountId,
        existing.piqaeLiveEnvironmentId,
        this.client(existing),
      );
      const repaired = { ...existing, templateRevisionId };
      if (!(await this.shops.putIfCurrentMatches(repaired, existing)))
        throw new Error("PIQAE_ACCOUNT_LINK_CHANGED");
      return repaired;
    }

    const account = await this.platform.accounts.getOrCreate(shop, {
      name: shop.replace(/\.myshopify\.com$/, ""),
      metadata: { source: "shopify", shop },
    });
    const templateRevisionId = await this.publishStarters(
      shop,
      account.id,
      account.environments.live.id,
      account.live,
    );
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
    if (!(await this.shops.putIfCurrentMatches(link, existing)))
      throw new Error("PIQAE_ACCOUNT_LINK_CHANGED");
    return link;
  }

  private async starterGenerationCurrent(
    shop: string,
    link: ShopLink,
  ): Promise<boolean> {
    const stored = await this.workflows.listTemplates(shop);
    for (const starter of starterTemplates) {
      const currentStarterDigest = createHash("sha256")
        .update(JSON.stringify(starter.specification))
        .digest("hex");
      const local =
        stored.find(
          (candidate) => candidate.id === systemTemplateId(starter.id),
        ) ?? findSystemTemplate(stored, starter.id);
      if (!local?.published) return false;
      try {
        const envelope = parseTemplateEnvelope(local.published.source);
        const published = envelope.published;
        if (
          envelope.system?.key !== starter.id ||
          envelope.system.immutable !== true ||
          !published ||
          published.piqaeAccountId !== link.piqaeAccountId ||
          published.piqaeEnvironmentId !== link.piqaeLiveEnvironmentId ||
          // A valid old publication is still old. Comparing the pin with the
          // checked-in starter digest lets immutable defaults receive layout
          // fixes once, while merchant-owned copies remain untouched.
          published.canonicalDigest !== currentStarterDigest ||
          published.canonicalDigest !==
            createHash("sha256")
              .update(JSON.stringify(envelope.document))
              .digest("hex") ||
          (starter.id === "invoice" &&
            published.piqaeRevisionId !== link.templateRevisionId)
        )
          return false;
      } catch {
        return false;
      }
    }
    return true;
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
      fetch: this.fetcher,
    });
  }

  private async publishStarters(
    shop: string,
    accountId: string,
    environmentId: string,
    client: Pick<PiqaeClient, "printPackets">,
  ): Promise<string> {
    await seedStarterTemplates(this.workflows, shop);
    const stored = await this.workflows.listTemplates(shop);
    const updates: SaveMerchantTemplate[] = [];
    let defaultRevisionId: string | undefined;
    for (const starter of starterTemplates) {
      const canonicalDigest = createHash("sha256")
        .update(JSON.stringify(starter.specification))
        .digest("hex");
      const digest = createHash("sha256")
        .update(`${shop}\0${starter.id}\0${canonicalDigest}`)
        .digest("hex");
      const template = await client.printPackets.templates.create(
        {
          name: `Shopify ${starter.name}`,
          specification: starter.specification,
        },
        `shopify-managed-template-${digest}`,
      );
      const revision = await client.printPackets.templates.publish(
        template.id,
        starter.specification,
        `shopify-managed-publish-${digest}`,
      );
      const local =
        stored.find(
          (candidate) => candidate.id === systemTemplateId(starter.id),
        ) ?? findSystemTemplate(stored, starter.id);
      if (!local) throw new Error("PIQAE_DEFAULT_TEMPLATE_MISSING");
      // Provisioning/relinking publishes the current canonical starter. A
      // routine seed must never move an already-published revision implicitly.
      const envelope = parseTemplateEnvelope(starter.source);
      envelope.published = {
        piqaeAccountId: accountId,
        piqaeEnvironmentId: environmentId,
        piqaeTemplateId: template.id,
        piqaeRevisionId: revision.id,
        canonicalDigest,
      };
      updates.push({
        ...local,
        source: serializeTemplateEnvelope(envelope),
        expectedDraftRevision: local.draftRevision,
      });
      if (starter.id === "invoice") defaultRevisionId = revision.id;
    }
    if (!defaultRevisionId) throw new Error("PIQAE_DEFAULT_TEMPLATE_MISSING");
    await this.workflows.saveTemplatesAtomically(shop, updates);
    return defaultRevisionId;
  }
}
