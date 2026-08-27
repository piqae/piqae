import { describe, expect, it, vi } from "vitest";

import { CredentialVault } from "../app/core/credentials.server";
import { MemoryShopRepository } from "../app/core/model";
import { publishCanonicalTemplate } from "../app/core/template-publisher.server";
import {
  parseTemplateEnvelope,
  serializeTemplateEnvelope,
} from "../app/core/template-model";
import { starterTemplates } from "../app/core/starter-templates";

describe("template resource publication", () => {
  it("uploads every verified content-addressed asset before publishing", async () => {
    const shop = "assets.myshopify.com";
    const shops = new MemoryShopRepository();
    const vault = new CredentialVault(Buffer.alloc(32, 12));
    await shops.put({
      shop,
      piqaeAccountId: "acct_assets",
      encryptedCredential: vault.seal("token", shop),
      templateRevisionId: "rev_old",
      createdAt: new Date().toISOString(),
    });
    const envelope = parseTemplateEnvelope(starterTemplates[0]!.source);
    envelope.assets = [
      {
        id: "logo",
        digest: "a".repeat(64),
        mediaType: "image/jpeg",
        bytes: 3,
        sourceUrl: "https://cdn.shopify.com/logo.jpg",
      },
    ];
    const calls: string[] = [];
    const putJpeg = vi.fn(async (digest: string) => {
      calls.push(`resource:${digest}`);
    });
    const create = vi.fn(async () => {
      calls.push("create");
      return { id: "template_new" };
    });
    const publish = vi.fn(async () => {
      calls.push("publish");
      return { id: "revision_new" };
    });
    const published = await publishCanonicalTemplate({
      shop,
      name: "Invoice",
      source: serializeTemplateEnvelope(envelope),
      shops,
      vault,
      baseUrl: "https://unused.example.invalid",
      assetFetcher: vi.fn(async () => new Uint8Array([1, 2, 3])),
      clientFactory: () =>
        ({
          printPackets: {
            resources: { putJpeg },
            templates: { create, publish },
          },
        }) as never,
    });

    expect(calls).toEqual([`resource:${"a".repeat(64)}`, "create", "publish"]);
    expect(parseTemplateEnvelope(published).published).toMatchObject({
      piqaeTemplateId: "template_new",
      piqaeRevisionId: "revision_new",
    });
  });

  it("does not publish when a resource upload fails", async () => {
    const shop = "failure.myshopify.com";
    const shops = new MemoryShopRepository();
    const vault = new CredentialVault(Buffer.alloc(32, 13));
    await shops.put({
      shop,
      piqaeAccountId: "acct_failure",
      encryptedCredential: vault.seal("token", shop),
      templateRevisionId: "rev_old",
      createdAt: new Date().toISOString(),
    });
    const envelope = parseTemplateEnvelope(starterTemplates[0]!.source);
    envelope.assets = [
      {
        id: "logo",
        digest: "b".repeat(64),
        mediaType: "image/jpeg",
        bytes: 1,
      },
    ];
    const create = vi.fn();
    await expect(
      publishCanonicalTemplate({
        shop,
        name: "Invoice",
        source: serializeTemplateEnvelope(envelope),
        shops,
        vault,
        baseUrl: "https://unused.example.invalid",
        assetFetcher: vi.fn(async () => new Uint8Array([1])),
        clientFactory: () =>
          ({
            printPackets: {
              resources: {
                putJpeg: vi.fn(async () => {
                  throw new Error("resource unavailable");
                }),
              },
              templates: { create },
            },
          }) as never,
      }),
    ).rejects.toThrow("resource unavailable");
    expect(create).not.toHaveBeenCalled();
  });
});
