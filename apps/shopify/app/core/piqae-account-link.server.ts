import { createHash } from "node:crypto";
import type {
  CreatePrintPacketTemplate,
  PrintPacketTemplate,
  PrintPacketTemplateRevision,
  Workspace,
} from "@piqae/sdk";
import type { CredentialVault } from "./credentials.server";
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
  SaveMerchantTemplate,
  WorkflowRepository,
} from "./workflows.server";

interface LinkClient {
  workspaces: { current(): Promise<Workspace> };
  printPackets: {
    templates: {
      create(
        input: CreatePrintPacketTemplate,
        idempotencyKey: string,
      ): Promise<PrintPacketTemplate>;
      publish(
        id: string,
        specification: CreatePrintPacketTemplate["specification"],
        idempotencyKey: string,
      ): Promise<PrintPacketTemplateRevision>;
    };
  };
}

export class PiqaeAccountLinker {
  constructor(
    private readonly shops: ShopRepository,
    private readonly workflows: WorkflowRepository,
    private readonly vault: CredentialVault,
    private readonly clientFactory: (credential: string) => LinkClient,
  ) {}

  async linkExisting(
    shopInput: string,
    credentialInput: string,
  ): Promise<ShopLink> {
    const shop = normalizeShopDomain(shopInput);
    const credential = validateCredential(credentialInput);
    return this.shops.withShopLock(shop, () =>
      this.linkExistingLocked(shop, credential),
    );
  }

  private async linkExistingLocked(
    shop: string,
    credential: string,
  ): Promise<ShopLink> {
    const expectedLink = await this.shops.get(shop);
    const client = this.clientFactory(credential);
    const workspace = await client.workspaces.current();
    if (workspace.status !== "active")
      throw new Error("PIQAE_ACCOUNT_INACTIVE");

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
        `shopify-link-template-${digest}`,
      );
      const revision = await client.printPackets.templates.publish(
        template.id,
        starter.specification,
        `shopify-link-publish-${digest}`,
      );
      const local =
        stored.find(
          (candidate) => candidate.id === systemTemplateId(starter.id),
        ) ?? findSystemTemplate(stored, starter.id);
      if (!local) throw new Error("PIQAE_DEFAULT_TEMPLATE_MISSING");
      // Relinking is the explicit point where a system starter advances to the
      // current canonical specification. Routine seeding preserves the prior
      // immutable publication and its exact revision pin.
      const envelope = parseTemplateEnvelope(starter.source);
      envelope.published = {
        piqaeAccountId: workspace.id,
        // Existing-token workspace discovery does not expose an environment;
        // the control plane's tenant-scoped revision lookup remains the final
        // authorization boundary for this mode.
        piqaeEnvironmentId: null,
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
    const link: ShopLink = {
      shop,
      piqaeAccountId: workspace.id,
      encryptedCredential: this.vault.seal(credential, shop),
      templateRevisionId: defaultRevisionId,
      entitlementMode: "existing_piqae",
      planHandle: null,
      createdAt: new Date().toISOString(),
    };
    if (!(await this.shops.putIfCurrentMatches(link, expectedLink)))
      throw new Error("PIQAE_ACCOUNT_LINK_CHANGED");
    return link;
  }
}

function validateCredential(input: string): string {
  const value = input.trim();
  if (
    value.length < 16 ||
    value.length > 4096 ||
    /[\s\u0000-\u001f\u007f]/.test(value)
  )
    throw new Error("PIQAE_CREDENTIAL_INVALID");
  return value;
}
