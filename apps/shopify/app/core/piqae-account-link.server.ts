import { createHash } from "node:crypto";
import type {
  CreateBusinessDocumentTemplate,
  BusinessDocumentTemplate,
  BusinessDocumentTemplateRevision,
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
import { seedStarterTemplates } from "./template-index.server";
import type { WorkflowRepository } from "./workflows.server";

interface LinkClient {
  workspaces: { current(): Promise<Workspace> };
  businessDocuments: {
    templates: {
      create(
        input: CreateBusinessDocumentTemplate,
        idempotencyKey: string,
      ): Promise<BusinessDocumentTemplate>;
      publish(
        id: string,
        specification: CreateBusinessDocumentTemplate["specification"],
        idempotencyKey: string,
      ): Promise<BusinessDocumentTemplateRevision>;
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
    const client = this.clientFactory(credential);
    const workspace = await client.workspaces.current();
    if (workspace.status !== "active")
      throw new Error("PIQAE_ACCOUNT_INACTIVE");

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
        `shopify-link-template-${digest}`,
      );
      const revision = await client.businessDocuments.templates.publish(
        template.id,
        starter.specification,
        `shopify-link-publish-${digest}`,
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
    const link: ShopLink = {
      shop,
      piqaeAccountId: workspace.id,
      encryptedCredential: this.vault.seal(credential, shop),
      templateRevisionId: defaultRevisionId,
      entitlementMode: "existing_piqae",
      planHandle: null,
      createdAt: new Date().toISOString(),
    };
    await this.shops.put(link);
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
