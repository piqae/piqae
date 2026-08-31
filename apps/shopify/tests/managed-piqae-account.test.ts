import { createHash } from "node:crypto";
import { describe, expect, it, vi } from "vitest";

import { ManagedPiqaeAccountService } from "../app/core/managed-piqae-account.server";
import { MemoryShopRepository } from "../app/core/model";
import { CredentialVault } from "../app/core/credentials.server";
import { ShopifyPrintingService } from "../app/core/printing.server";
import { PiqaeAccountLinker } from "../app/core/piqae-account-link.server";
import { starterTemplates } from "../app/core/starter-templates";
import { MemoryWorkflowRepository } from "../app/core/workflows.server";
import {
  parseTemplateEnvelope,
  removeSystemOwnership,
  serializeTemplateEnvelope,
} from "../app/core/template-model";
import { seedStarterTemplates } from "../app/core/template-index.server";
import { publishCanonicalTemplate } from "../app/core/template-publisher.server";

const shop = "managed-shop.myshopify.com";
const fixturePlatformKey = ["piq", "platform", "fixture"].join("_");

class DelayedPlanShopRepository extends MemoryShopRepository {
  private notifyPlanPending!: () => void;
  private releasePlanWrite!: () => void;
  readonly planPending = new Promise<void>((resolve) => {
    this.notifyPlanPending = resolve;
  });
  private readonly planReleased = new Promise<void>((resolve) => {
    this.releasePlanWrite = resolve;
  });

  override async putIfCurrentMatches(
    ...args: Parameters<MemoryShopRepository["putIfCurrentMatches"]>
  ) {
    if (args[0].planHandle === "growth") {
      this.notifyPlanPending();
      await this.planReleased;
    }
    return super.putIfCurrentMatches(...args);
  }

  releasePlan() {
    this.releasePlanWrite();
  }
}

describe("managed Piqae Shopify accounts", () => {
  it("provisions an isolated account and publishes the starter documents", async () => {
    let template = 0;
    let revision = 0;
    const fetcher = vi.fn(
      async (input: string | URL | Request, init?: RequestInit) => {
        const url = new URL(String(input));
        const headers = new Headers(init?.headers);
        expect(headers.get("authorization")).toBe(
          `Bearer ${fixturePlatformKey}`,
        );
        if (url.pathname === `/v1/platform/accounts/${shop}`) {
          expect(init?.method).toBe("PUT");
          return Response.json({
            id: "ws_managed",
            external_id: shop,
            name: "managed-shop",
            status: "active",
            metadata: { source: "shopify", shop },
            environments: {
              live: { id: "env_live", kind: "live" },
              test: { id: "env_test", kind: "test" },
            },
            created_at: "2026-08-19T00:00:00Z",
            updated_at: "2026-08-19T00:00:00Z",
          });
        }
        expect(headers.get("x-piqae-workspace-id")).toBe("ws_managed");
        expect(headers.get("x-piqae-environment-id")).toBe("env_live");
        if (url.pathname === "/v1/printpacket/templates") {
          template += 1;
          return Response.json({ id: `tpl_${template}` });
        }
        if (
          /^\/v1\/printpacket\/templates\/tpl_\d+\/publish$/.test(url.pathname)
        ) {
          revision += 1;
          return Response.json({ id: `rev_${revision}` });
        }
        return Response.json(
          { error: { code: "unexpected", message: url.pathname } },
          { status: 500 },
        );
      },
    );
    const shops = new MemoryShopRepository();
    const workflows = new MemoryWorkflowRepository();
    const service = new ManagedPiqaeAccountService(
      shops,
      workflows,
      fixturePlatformKey,
      "https://api.piqae.test",
      fetcher as typeof fetch,
    );

    const link = await service.ensure(shop);
    expect(link).toMatchObject({
      shop,
      piqaeAccountId: "ws_managed",
      piqaeLiveEnvironmentId: "env_live",
      piqaeTestEnvironmentId: "env_test",
      encryptedCredential: "",
      entitlementMode: "shopify_child",
      planHandle: "development",
    });
    expect(template).toBe(starterTemplates.length);
    expect(revision).toBe(starterTemplates.length);
    expect(
      (await workflows.listTemplates(shop)).every(
        (value) => value.state === "published",
      ),
    ).toBe(true);

    await seedStarterTemplates(workflows, shop);
    const restartedService = new ManagedPiqaeAccountService(
      shops,
      workflows,
      fixturePlatformKey,
      "https://api.piqae.test",
      fetcher as typeof fetch,
    );
    await restartedService.ensure(shop);
    expect(fetcher).toHaveBeenCalledTimes(1 + starterTemplates.length * 2);
    expect(
      (await workflows.listTemplates(shop)).every((value) =>
        Boolean(
          parseTemplateEnvelope(value.published!.source).published
            ?.piqaeRevisionId,
        ),
      ),
    ).toBe(true);

    const packingSlip = (await workflows.listTemplates(shop)).find(
      (value) =>
        parseTemplateEnvelope(value.published!.source).system?.key ===
        "packing-slip",
    )!;
    const mismatched = parseTemplateEnvelope(packingSlip.published!.source);
    mismatched.published!.piqaeAccountId = "other_account";
    await workflows.saveTemplate(shop, {
      ...packingSlip,
      source: serializeTemplateEnvelope(mismatched),
      expectedDraftRevision: packingSlip.draftRevision,
    });
    const beforeRepair = fetcher.mock.calls.length;
    await restartedService.ensure(shop);
    expect(fetcher).toHaveBeenCalledTimes(
      beforeRepair + starterTemplates.length * 2,
    );
    const afterRepair = fetcher.mock.calls.length;
    await restartedService.ensure(shop);
    expect(fetcher).toHaveBeenCalledTimes(afterRepair);
    expect(
      (await workflows.listTemplates(shop)).every(
        (value) =>
          parseTemplateEnvelope(value.published!.source).published
            ?.piqaeAccountId === "ws_managed",
      ),
    ).toBe(true);

    const label = (await workflows.listTemplates(shop)).find(
      (value) =>
        parseTemplateEnvelope(value.published!.source).system?.key ===
        "product-label",
    )!;
    const outdatedLabel = parseTemplateEnvelope(label.published!.source);
    outdatedLabel.document.theme = {
      ...outdatedLabel.document.theme,
      font_size_pt: 8,
    };
    outdatedLabel.published!.canonicalDigest = createHash("sha256")
      .update(JSON.stringify(outdatedLabel.document))
      .digest("hex");
    await workflows.saveTemplate(shop, {
      ...label,
      source: serializeTemplateEnvelope(outdatedLabel),
      expectedDraftRevision: label.draftRevision,
    });
    const beforeStarterUpgrade = fetcher.mock.calls.length;
    await restartedService.ensure(shop);
    expect(fetcher).toHaveBeenCalledTimes(
      beforeStarterUpgrade + starterTemplates.length * 2,
    );
    const upgradedLabel = (await workflows.listTemplates(shop)).find(
      (value) =>
        parseTemplateEnvelope(value.published!.source).system?.key ===
        "product-label",
    )!;
    expect(
      parseTemplateEnvelope(upgradedLabel.published!.source).document,
    ).toEqual(
      starterTemplates.find(({ id }) => id === "product-label")!.specification,
    );

    const beforeCustomStarters = new Map(
      (await workflows.listTemplates(shop)).map((value) => [
        value.id,
        value.published!.source,
      ]),
    );
    const defaultBeforeCustom = (await shops.get(shop))!.templateRevisionId;
    const custom = removeSystemOwnership(
      parseTemplateEnvelope(starterTemplates[0]!.source),
    );
    let customActivated = false;
    await publishCanonicalTemplate({
      shop,
      name: "Custom invoice",
      source: serializeTemplateEnvelope(custom),
      shops,
      vault: new CredentialVault(Buffer.alloc(32, 8)),
      baseUrl: "https://api.piqae.test",
      managedClientFactory: (link) => restartedService.client(link),
      activate: async () => {
        customActivated = true;
      },
    });
    expect(customActivated).toBe(true);
    const callsAfterCustom = fetcher.mock.calls.length;
    await restartedService.ensure(shop);
    expect(fetcher).toHaveBeenCalledTimes(callsAfterCustom);
    expect((await shops.get(shop))?.templateRevisionId).toBe(
      defaultBeforeCustom,
    );
    expect(
      new Map(
        (await workflows.listTemplates(shop)).map((value) => [
          value.id,
          value.published!.source,
        ]),
      ),
    ).toEqual(beforeCustomStarters);
  });

  it("reprovisions N-1 child pins once, survives restart, and prints the repaired receipt", async () => {
    const shops = new MemoryShopRepository();
    const workflows = new MemoryWorkflowRepository();
    await seedStarterTemplates(workflows, shop);
    const legacy = (await workflows.listTemplates(shop)).map((template) => {
      const envelope = JSON.parse(template.source) as Record<string, unknown>;
      const key = (envelope as { system: { key: string } }).system.key;
      envelope.published = {
        piqaeTemplateId: `legacy_template_${key}`,
        piqaeRevisionId: `legacy_revision_${key}`,
        canonicalDigest: createHash("sha256")
          .update(JSON.stringify(envelope.document))
          .digest("hex"),
      };
      const source = JSON.stringify(envelope);
      return {
        ...template,
        source,
        published: { ...template.published!, source },
      };
    });
    const storage = workflows as unknown as {
      templates: Map<string, typeof legacy>;
    };
    storage.templates.set(shop, legacy);
    await shops.put({
      shop,
      piqaeAccountId: "ws_managed",
      piqaeLiveEnvironmentId: "env_live",
      piqaeTestEnvironmentId: "env_test",
      encryptedCredential: "",
      templateRevisionId: "legacy_revision_invoice",
      entitlementMode: "shopify_child",
      planHandle: "development",
      createdAt: new Date(0).toISOString(),
    });

    let template = 0;
    let revision = 0;
    let renders = 0;
    const fetcher = vi.fn(
      async (input: string | URL | Request, init?: RequestInit) => {
        const url = new URL(String(input));
        const headers = new Headers(init?.headers);
        expect(headers.get("x-piqae-workspace-id")).toBe("ws_managed");
        expect(headers.get("x-piqae-environment-id")).toBe("env_live");
        if (url.pathname === "/v1/printpacket/templates") {
          template += 1;
          return Response.json({ id: `upgrade_template_${template}` });
        }
        if (
          /^\/v1\/printpacket\/templates\/[^/]+\/publish$/.test(url.pathname)
        ) {
          revision += 1;
          return Response.json({ id: `upgrade_revision_${revision}` });
        }
        if (url.pathname === "/v1/printpacket/renders") {
          renders += 1;
          return Response.json({
            id: "render_n1",
            state: "completed",
            failure_code: null,
          });
        }
        return Response.json(
          { error: { code: "unexpected", message: url.pathname } },
          { status: 500 },
        );
      },
    );
    const service = new ManagedPiqaeAccountService(
      shops,
      workflows,
      fixturePlatformKey,
      "https://api.piqae.test",
      fetcher as typeof fetch,
    );

    const repaired = await service.ensure(shop);
    expect(repaired.templateRevisionId).toBe("upgrade_revision_1");
    expect(template).toBe(starterTemplates.length);
    expect(revision).toBe(starterTemplates.length);
    for (const stored of await workflows.listTemplates(shop)) {
      const pin = parseTemplateEnvelope(stored.published!.source).published;
      expect(pin).toMatchObject({
        piqaeAccountId: "ws_managed",
        piqaeEnvironmentId: "env_live",
      });
    }

    const callsAfterRepair = fetcher.mock.calls.length;
    const restarted = new ManagedPiqaeAccountService(
      shops,
      workflows,
      fixturePlatformKey,
      "https://api.piqae.test",
      fetcher as typeof fetch,
    );
    await restarted.ensure(shop);
    expect(fetcher).toHaveBeenCalledTimes(callsAfterRepair);

    const printing = new ShopifyPrintingService(
      shops,
      new CredentialVault(Buffer.alloc(32, 1)),
      () => {
        throw new Error("existing-token client must not be used");
      },
      "https://shopify.example.test",
      undefined,
      workflows,
      (link) => restarted.client(link),
    );
    const result = await printing.printOrders({
      shop,
      admin: {
        graphql: vi.fn(async () =>
          Response.json({
            data: {
              order: {
                id: "gid://shopify/Order/42",
                name: "#42",
                createdAt: "2026-08-28T00:00:00Z",
                currencyCode: "NZD",
                customer: null,
                shippingAddress: null,
                billingAddress: null,
                note: "",
                shippingLine: null,
                statusPageUrl: "",
                metafieldsByIdentifiers: [],
                lineItems: {
                  nodes: [],
                  pageInfo: { hasNextPage: false, endCursor: null },
                },
                subtotalPriceSet: { shopMoney: { amount: "0" } },
                totalTaxSet: { shopMoney: { amount: "0" } },
                totalPriceSet: { shopMoney: { amount: "0" } },
              },
            },
          }),
        ),
      },
      orderIds: ["42"],
      systemTemplateKey: "receipt",
      requestKey: "n1-repaired",
    });
    expect(result).toMatchObject({ mode: "download", renderId: "render_n1" });
    expect(renders).toBe(1);
    const renderBody = JSON.parse(
      String(fetcher.mock.calls.at(-1)?.[1]?.body),
    ) as { template_revision_id: string };
    expect(renderBody.template_revision_id).toBe("upgrade_revision_3");
  });

  it("does not let a delayed billing link write overwrite a waiting relink", async () => {
    const shops = new DelayedPlanShopRepository();
    const workflows = new MemoryWorkflowRepository();
    let template = 0;
    const managed = new ManagedPiqaeAccountService(
      shops,
      workflows,
      fixturePlatformKey,
      "https://api.piqae.test",
      (async (input: string | URL | Request) => {
        const path = new URL(String(input)).pathname;
        if (path === `/v1/platform/accounts/${shop}`)
          return Response.json({
            id: "ws_managed",
            external_id: shop,
            name: "managed-shop",
            status: "active",
            metadata: {},
            environments: {
              live: { id: "env_live", kind: "live" },
              test: { id: "env_test", kind: "test" },
            },
            created_at: "2026-08-19T00:00:00Z",
            updated_at: "2026-08-19T00:00:00Z",
          });
        if (path === "/v1/printpacket/templates")
          return Response.json({ id: `managed_template_${++template}` });
        if (/\/publish$/.test(path))
          return Response.json({ id: `managed_revision_${template}` });
        return Response.json({}, { status: 500 });
      }) as typeof fetch,
    );
    let relinkEntered = false;
    const relink = new PiqaeAccountLinker(
      shops,
      workflows,
      new CredentialVault(Buffer.alloc(32, 5)),
      () =>
        ({
          workspaces: {
            current: async () => {
              relinkEntered = true;
              return { id: "ws_existing", status: "active" };
            },
          },
          printPackets: {
            templates: {
              create: async () => ({ id: "existing_template" }),
              publish: async () => ({ id: "existing_revision" }),
            },
          },
        }) as never,
    );

    const billing = managed.activatePlan(shop, "growth", 5_000);
    await shops.planPending;
    const relinking = relink.linkExisting(shop, "piqae-credential-existing");
    await Promise.resolve();
    expect(relinkEntered).toBe(false);
    shops.releasePlan();
    await billing;
    await relinking;

    const active = (await shops.get(shop))!;
    expect(active.piqaeAccountId).toBe("ws_existing");
    expect(
      (await workflows.listTemplates(shop)).every(
        (stored) =>
          parseTemplateEnvelope(stored.published!.source).published
            ?.piqaeAccountId === active.piqaeAccountId,
      ),
    ).toBe(true);
    expect(await workflows.getBilling(shop)).toMatchObject({
      plan: "growth",
      status: "active",
    });
  });
});
