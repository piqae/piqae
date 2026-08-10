import { PiqaeClient } from "@piqae/sdk";
import type { ShopRepository } from "./model";
import type { CredentialVault } from "./credentials.server";
import {
  parseTemplateEnvelope,
  serializeTemplateEnvelope,
} from "./template-model";
import { templateDigest } from "./template-digest.server";

export async function publishCanonicalTemplate(input: {
  shop: string;
  name: string;
  source: string;
  shops: ShopRepository;
  vault: CredentialVault;
  baseUrl: string;
}): Promise<string> {
  const link = await input.shops.get(input.shop);
  if (!link)
    throw new Error("Connect a Piqae account before publishing a document");
  const envelope = parseTemplateEnvelope(input.source);
  const canonicalDigest = templateDigest(JSON.stringify(envelope.canonical));
  const client = new PiqaeClient({
    baseUrl: input.baseUrl,
    accessToken: () => input.vault.open(link.encryptedCredential, input.shop),
  });
  const template = await client.documents.templates.create(
    { name: input.name, specification: envelope.canonical },
    `shopify-template-${canonicalDigest}`,
  );
  const revision = await client.documents.templates.publish(
    template.id,
    envelope.canonical,
    `shopify-template-publish-${canonicalDigest}`,
  );
  envelope.published = {
    piqaeTemplateId: template.id,
    piqaeRevisionId: revision.id,
    canonicalDigest,
  };
  await input.shops.put({ ...link, templateRevisionId: revision.id });
  return serializeTemplateEnvelope(envelope);
}
