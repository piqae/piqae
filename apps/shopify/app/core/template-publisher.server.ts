import { PiqaeClient } from "@piqae/sdk";
import type { ShopRepository } from "./model";
import type { CredentialVault } from "./credentials.server";
import {
  parseTemplateEnvelope,
  serializeTemplateEnvelope,
} from "./template-model";
import { templateDigest } from "./template-digest.server";
import { fetchTemplateAsset } from "./template-assets.server";

export async function publishCanonicalTemplate(input: {
  shop: string;
  name: string;
  source: string;
  shops: ShopRepository;
  vault: CredentialVault;
  baseUrl: string;
  clientFactory?: (token: string) => PiqaeClient;
  assetFetcher?: typeof fetchTemplateAsset;
}): Promise<string> {
  const link = await input.shops.get(input.shop);
  if (!link)
    throw new Error("Connect a Piqae account before publishing a document");
  const envelope = parseTemplateEnvelope(input.source);
  const canonicalDigest = templateDigest(JSON.stringify(envelope.document));
  const token = input.vault.open(link.encryptedCredential, input.shop);
  const client = input.clientFactory
    ? input.clientFactory(token)
    : new PiqaeClient({
        baseUrl: input.baseUrl,
        accessToken: () => token,
      });
  const fetchAsset = input.assetFetcher ?? fetchTemplateAsset;
  await mapWithConcurrency(envelope.assets, 4, async (asset) => {
    const bytes = await fetchAsset(asset);
    const body = bytes.buffer.slice(
      bytes.byteOffset,
      bytes.byteOffset + bytes.byteLength,
    ) as ArrayBuffer;
    await client.businessDocuments.resources.putJpeg(asset.digest, body);
  });
  const template = await client.businessDocuments.templates.create(
    { name: input.name, specification: envelope.document },
    `shopify-template-${canonicalDigest}`,
  );
  const revision = await client.businessDocuments.templates.publish(
    template.id,
    envelope.document,
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

async function mapWithConcurrency<T>(
  values: T[],
  concurrency: number,
  visit: (value: T) => Promise<void>,
): Promise<void> {
  let cursor = 0;
  await Promise.all(
    Array.from({ length: Math.min(concurrency, values.length) }, async () => {
      while (cursor < values.length) {
        const value = values[cursor++];
        if (value !== undefined) await visit(value);
      }
    }),
  );
}
