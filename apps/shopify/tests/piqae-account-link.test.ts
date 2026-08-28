import { createHash } from "node:crypto";
import { describe, expect, it, vi } from "vitest";
import { CredentialVault } from "../app/core/credentials.server";
import { MemoryShopRepository } from "../app/core/model";
import { PiqaeAccountLinker } from "../app/core/piqae-account-link.server";
import { starterTemplates } from "../app/core/starter-templates";
import { parseTemplateEnvelope } from "../app/core/template-model";
import { MemoryWorkflowRepository } from "../app/core/workflows.server";
import { seedStarterTemplates } from "../app/core/template-index.server";

const shop = "fixture-shop.myshopify.com";

class RejectingBatchWorkflowRepository extends MemoryWorkflowRepository {
  override async saveTemplatesAtomically(): Promise<never> {
    throw new Error("LOCAL_PUBLICATION_BATCH_FAILED");
  }
}

describe("existing Piqae account linking", () => {
  it("verifies the account and idempotently publishes a usable default", async () => {
    const repository = new MemoryShopRepository();
    const workflows = new MemoryWorkflowRepository();
    const vault = new CredentialVault(Buffer.alloc(32, 9));
    const create = vi.fn(async (_input: unknown, _key: string) => ({
      id: "dtpl_default",
    }));
    const publish = vi.fn(
      async (_id: string, _specification: unknown, _key: string) => ({
        id: "dtrv_default",
      }),
    );
    const linker = new PiqaeAccountLinker(
      repository,
      workflows,
      vault,
      () =>
        ({
          workspaces: {
            current: async () => ({ id: "ws_fixture", status: "active" }),
          },
          printPackets: { templates: { create, publish } },
        }) as never,
    );

    const linked = await linker.linkExisting(shop, "piqae-fixture-credential");
    expect(linked.piqaeAccountId).toBe("ws_fixture");
    expect(linked.templateRevisionId).toBe("dtrv_default");
    expect(linked.encryptedCredential).not.toContain(
      "piqae-fixture-credential",
    );
    expect(vault.open(linked.encryptedCredential, shop)).toBe(
      "piqae-fixture-credential",
    );
    expect(create).toHaveBeenCalledTimes(starterTemplates.length);
    expect(publish).toHaveBeenCalledTimes(starterTemplates.length);
    const canonicalDigest = createHash("sha256")
      .update(JSON.stringify(starterTemplates[0]!.specification))
      .digest("hex");
    const identityDigest = createHash("sha256")
      .update(`${shop}\0${starterTemplates[0]!.id}\0${canonicalDigest}`)
      .digest("hex");
    expect(create.mock.calls[0]?.[1]).toBe(
      `shopify-link-template-${identityDigest}`,
    );
    expect(publish.mock.calls[0]?.[2]).toBe(
      `shopify-link-publish-${identityDigest}`,
    );
    const packingSlip = (await workflows.listTemplates(shop)).find((value) =>
      value.name.toLowerCase().includes("packing slip"),
    );
    expect(
      parseTemplateEnvelope(packingSlip?.published?.source ?? "").published,
    ).toMatchObject({
      piqaeTemplateId: "dtpl_default",
      piqaeRevisionId: "dtrv_default",
    });
    expect(await repository.get(shop)).toMatchObject({
      templateRevisionId: "dtrv_default",
    });
  });

  it("rejects inactive accounts without storing credentials", async () => {
    const repository = new MemoryShopRepository();
    const workflows = new MemoryWorkflowRepository();
    const linker = new PiqaeAccountLinker(
      repository,
      workflows,
      new CredentialVault(Buffer.alloc(32, 8)),
      () =>
        ({
          workspaces: {
            current: async () => ({ id: "ws_fixture", status: "suspended" }),
          },
          printPackets: {
            templates: { create: vi.fn(), publish: vi.fn() },
          },
        }) as never,
    );
    await expect(
      linker.linkExisting(shop, "piqae-fixture-credential"),
    ).rejects.toThrow("PIQAE_ACCOUNT_INACTIVE");
    expect(await repository.get(shop)).toBeNull();
  });

  it("only advances starter content and pins during an explicit account relink", async () => {
    const repository = new MemoryShopRepository();
    const workflows = new MemoryWorkflowRepository();
    const vault = new CredentialVault(Buffer.alloc(32, 6));
    const linker = new PiqaeAccountLinker(
      repository,
      workflows,
      vault,
      (credential) => {
        const suffix = credential.endsWith("second") ? "second" : "first";
        return {
          workspaces: {
            current: async () => ({ id: `ws_${suffix}`, status: "active" }),
          },
          printPackets: {
            templates: {
              create: async () => ({ id: `template_${suffix}` }),
              publish: async () => ({ id: `revision_${suffix}` }),
            },
          },
        } as never;
      },
    );

    await linker.linkExisting(shop, "piqae-credential-first");
    await Promise.all(
      Array.from({ length: 6 }, () => seedStarterTemplates(workflows, shop)),
    );
    let invoice = (await workflows.listTemplates(shop)).find(
      (candidate) =>
        parseTemplateEnvelope(candidate.source).system?.key === "invoice",
    )!;
    expect(
      parseTemplateEnvelope(invoice.published!.source).published,
    ).toMatchObject({
      piqaeTemplateId: "template_first",
      piqaeRevisionId: "revision_first",
    });

    const relinked = await linker.linkExisting(shop, "piqae-credential-second");
    invoice = (await workflows.listTemplates(shop)).find(
      (candidate) =>
        parseTemplateEnvelope(candidate.source).system?.key === "invoice",
    )!;
    const relinkedEnvelope = parseTemplateEnvelope(invoice.published!.source);
    expect(relinked).toMatchObject({
      piqaeAccountId: "ws_second",
      templateRevisionId: "revision_second",
    });
    expect(relinkedEnvelope.published).toMatchObject({
      piqaeTemplateId: "template_second",
      piqaeRevisionId: "revision_second",
    });
    delete relinkedEnvelope.published;
    expect(relinkedEnvelope).toEqual(
      parseTemplateEnvelope(starterTemplates[0]!.source),
    );
  });

  it("collects every remote publication before an atomic local activation", async () => {
    const repository = new MemoryShopRepository();
    const workflows = new RejectingBatchWorkflowRepository();
    const create = vi.fn(async () => ({ id: "template_remote" }));
    const publish = vi.fn(async () => ({ id: "revision_remote" }));
    const linker = new PiqaeAccountLinker(
      repository,
      workflows,
      new CredentialVault(Buffer.alloc(32, 5)),
      () =>
        ({
          workspaces: {
            current: async () => ({ id: "ws_remote", status: "active" }),
          },
          printPackets: { templates: { create, publish } },
        }) as never,
    );

    await expect(
      linker.linkExisting(shop, "piqae-credential-remote"),
    ).rejects.toThrow("LOCAL_PUBLICATION_BATCH_FAILED");
    expect(create).toHaveBeenCalledTimes(starterTemplates.length);
    expect(publish).toHaveBeenCalledTimes(starterTemplates.length);
    expect(await repository.get(shop)).toBeNull();
    expect(
      (await workflows.listTemplates(shop)).every(
        (template) =>
          !parseTemplateEnvelope(template.published!.source).published,
      ),
    ).toBe(true);
  });

  it("serializes a relink paused after its snapshot before another generation", async () => {
    const repository = new MemoryShopRepository();
    const workflows = new MemoryWorkflowRepository();
    const vault = new CredentialVault(Buffer.alloc(32, 4));
    let notifySecondPaused!: () => void;
    let releaseSecond!: () => void;
    const secondPaused = new Promise<void>((resolve) => {
      notifySecondPaused = resolve;
    });
    const secondReleased = new Promise<void>((resolve) => {
      releaseSecond = resolve;
    });
    const workspaceCalls: string[] = [];
    const linker = new PiqaeAccountLinker(
      repository,
      workflows,
      vault,
      (credential) => {
        const suffix = credential.endsWith("second") ? "second" : "first";
        return {
          workspaces: {
            current: async () => {
              workspaceCalls.push(suffix);
              if (suffix === "second") {
                notifySecondPaused();
                await secondReleased;
              }
              return { id: `ws_${suffix}`, status: "active" };
            },
          },
          printPackets: {
            templates: {
              create: async () => ({ id: `template_${suffix}` }),
              publish: async () => ({ id: `revision_${suffix}` }),
            },
          },
        } as never;
      },
    );

    const second = linker.linkExisting(shop, "piqae-credential-second");
    await secondPaused;
    const first = linker.linkExisting(shop, "piqae-credential-first");
    await Promise.resolve();
    expect(workspaceCalls).toEqual(["second"]);
    releaseSecond();
    await expect(second).resolves.toMatchObject({
      piqaeAccountId: "ws_second",
    });
    await expect(first).resolves.toMatchObject({
      piqaeAccountId: "ws_first",
    });
    const active = (await repository.get(shop))!;
    const publications = (await workflows.listTemplates(shop)).map(
      (template) =>
        parseTemplateEnvelope(template.published!.source).published!,
    );
    expect(
      publications.every(
        ({ piqaeAccountId }) => piqaeAccountId === active.piqaeAccountId,
      ),
    ).toBe(true);
    expect(
      new Set(publications.map(({ piqaeRevisionId }) => piqaeRevisionId)),
    ).toEqual(new Set([`revision_${active.piqaeAccountId.slice(3)}`]));
    expect(active.piqaeAccountId).toBe("ws_first");
  });

  it("does not resurrect a first link after uninstall waits for activation", async () => {
    const repository = new MemoryShopRepository();
    const workflows = new MemoryWorkflowRepository();
    let notifyPaused!: () => void;
    let releaseActivation!: () => void;
    const paused = new Promise<void>((resolve) => {
      notifyPaused = resolve;
    });
    const released = new Promise<void>((resolve) => {
      releaseActivation = resolve;
    });
    const linker = new PiqaeAccountLinker(
      repository,
      workflows,
      new CredentialVault(Buffer.alloc(32, 2)),
      () =>
        ({
          workspaces: {
            current: async () => {
              notifyPaused();
              await released;
              return { id: "ws_activation", status: "active" };
            },
          },
          printPackets: {
            templates: {
              create: async () => ({ id: "template_activation" }),
              publish: async () => ({ id: "revision_activation" }),
            },
          },
        }) as never,
    );

    const activation = linker.linkExisting(shop, "piqae-credential-new");
    await paused;
    let uninstallFinished = false;
    const uninstall = repository.deleteShop(shop).then(() => {
      uninstallFinished = true;
    });
    await Promise.resolve();
    expect(uninstallFinished).toBe(false);
    releaseActivation();
    await activation;
    await uninstall;
    expect(await repository.get(shop)).toBeNull();
  });
});
