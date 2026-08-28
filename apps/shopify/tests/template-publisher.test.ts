import { describe, expect, it, vi } from "vitest";

import { CredentialVault } from "../app/core/credentials.server";
import { MemoryShopRepository } from "../app/core/model";
import { PiqaeAccountLinker } from "../app/core/piqae-account-link.server";
import { publishCanonicalTemplate } from "../app/core/template-publisher.server";
import {
  parseTemplateEnvelope,
  serializeTemplateEnvelope,
  removeSystemOwnership,
} from "../app/core/template-model";
import { starterTemplates } from "../app/core/starter-templates";
import { MemoryWorkflowRepository } from "../app/core/workflows.server";

function linkerFor(
  shops: MemoryShopRepository,
  workflows: MemoryWorkflowRepository,
  vault: CredentialVault,
  workspaceCurrent: () => Promise<{ id: string; status: "active" }>,
  suffix: string,
) {
  return new PiqaeAccountLinker(
    shops,
    workflows,
    vault,
    () =>
      ({
        workspaces: { current: workspaceCurrent },
        printPackets: {
          templates: {
            create: async () => ({ id: `template_${suffix}` }),
            publish: async () => ({ id: `revision_${suffix}` }),
          },
        },
      }) as never,
  );
}

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

  it("finishes custom publication before a waiting relink generation", async () => {
    const shop = "publish-first.myshopify.com";
    const shops = new MemoryShopRepository();
    const workflows = new MemoryWorkflowRepository();
    const vault = new CredentialVault(Buffer.alloc(32, 14));
    await linkerFor(
      shops,
      workflows,
      vault,
      async () => ({ id: "account_first", status: "active" }),
      "first",
    ).linkExisting(shop, "piqae-credential-first");
    let notifyPublishPaused!: () => void;
    let releasePublish!: () => void;
    const publishPaused = new Promise<void>((resolve) => {
      notifyPublishPaused = resolve;
    });
    const publishReleased = new Promise<void>((resolve) => {
      releasePublish = resolve;
    });
    let relinkEntered = false;
    const custom = removeSystemOwnership(
      parseTemplateEnvelope(starterTemplates[0]!.source),
    );
    const publication = publishCanonicalTemplate({
      shop,
      name: "Custom invoice",
      source: serializeTemplateEnvelope(custom),
      shops,
      vault,
      baseUrl: "https://unused.example.invalid",
      clientFactory: () =>
        ({
          printPackets: {
            resources: { putJpeg: vi.fn() },
            templates: {
              create: async () => {
                notifyPublishPaused();
                await publishReleased;
                return { id: "custom_template" };
              },
              publish: async () => ({ id: "custom_revision" }),
            },
          },
        }) as never,
    });
    await publishPaused;
    const relink = linkerFor(
      shops,
      workflows,
      vault,
      async () => {
        relinkEntered = true;
        return { id: "account_second", status: "active" };
      },
      "second",
    ).linkExisting(shop, "piqae-credential-second");
    await Promise.resolve();
    expect(relinkEntered).toBe(false);
    releasePublish();
    await publication;
    await relink;
    const active = (await shops.get(shop))!;
    expect(active.piqaeAccountId).toBe("account_second");
    expect(
      (await workflows.listTemplates(shop)).every(
        (template) =>
          parseTemplateEnvelope(template.published!.source).published
            ?.piqaeAccountId === active.piqaeAccountId,
      ),
    ).toBe(true);
  });

  it("waits to publish a custom document until a relink generation commits", async () => {
    const shop = "relink-first.myshopify.com";
    const shops = new MemoryShopRepository();
    const workflows = new MemoryWorkflowRepository();
    const vault = new CredentialVault(Buffer.alloc(32, 15));
    await linkerFor(
      shops,
      workflows,
      vault,
      async () => ({ id: "account_first", status: "active" }),
      "first",
    ).linkExisting(shop, "piqae-credential-first");
    let notifyRelinkPaused!: () => void;
    let releaseRelink!: () => void;
    const relinkPaused = new Promise<void>((resolve) => {
      notifyRelinkPaused = resolve;
    });
    const relinkReleased = new Promise<void>((resolve) => {
      releaseRelink = resolve;
    });
    const relink = linkerFor(
      shops,
      workflows,
      vault,
      async () => {
        notifyRelinkPaused();
        await relinkReleased;
        return { id: "account_second", status: "active" };
      },
      "second",
    ).linkExisting(shop, "piqae-credential-second");
    await relinkPaused;
    const create = vi.fn(async () => ({ id: "custom_template" }));
    const custom = removeSystemOwnership(
      parseTemplateEnvelope(starterTemplates[0]!.source),
    );
    const publication = publishCanonicalTemplate({
      shop,
      name: "Custom invoice",
      source: serializeTemplateEnvelope(custom),
      shops,
      vault,
      baseUrl: "https://unused.example.invalid",
      clientFactory: () =>
        ({
          printPackets: {
            resources: { putJpeg: vi.fn() },
            templates: {
              create,
              publish: async () => ({ id: "custom_revision" }),
            },
          },
        }) as never,
    });
    await Promise.resolve();
    expect(create).not.toHaveBeenCalled();
    releaseRelink();
    await relink;
    const source = await publication;
    expect(parseTemplateEnvelope(source).published).toMatchObject({
      piqaeAccountId: "account_second",
      piqaeRevisionId: "custom_revision",
    });
    expect((await shops.get(shop))?.piqaeAccountId).toBe("account_second");
    expect(
      (await workflows.listTemplates(shop)).every(
        (template) =>
          parseTemplateEnvelope(template.published!.source).published
            ?.piqaeAccountId === "account_second",
      ),
    ).toBe(true);
  });
});
