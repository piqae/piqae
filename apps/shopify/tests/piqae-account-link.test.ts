import { describe, expect, it, vi } from "vitest";
import { CredentialVault } from "../app/core/credentials.server";
import { MemoryShopRepository } from "../app/core/model";
import { PiqaeAccountLinker } from "../app/core/piqae-account-link.server";
import { starterTemplates } from "../app/core/starter-templates";
import { parseTemplateEnvelope } from "../app/core/template-model";
import { MemoryWorkflowRepository } from "../app/core/workflows.server";

const shop = "fixture-shop.myshopify.com";

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
          documents: { templates: { create, publish } },
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
    expect(create.mock.calls[0]?.[1]).toMatch(/^shopify-link-template-/);
    expect(publish.mock.calls[0]?.[2]).toMatch(/^shopify-link-publish-/);
    const packingSlip = (await workflows.listTemplates(shop)).find((value) =>
      value.name.includes("Packing slip"),
    );
    expect(
      parseTemplateEnvelope(packingSlip?.source ?? "").published,
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
          documents: { templates: { create: vi.fn(), publish: vi.fn() } },
        }) as never,
    );
    await expect(
      linker.linkExisting(shop, "piqae-fixture-credential"),
    ).rejects.toThrow("PIQAE_ACCOUNT_INACTIVE");
    expect(await repository.get(shop)).toBeNull();
  });
});
