import { randomBytes } from "node:crypto";
import { readFile } from "node:fs/promises";
import pg from "pg";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

import { CredentialVault } from "../app/core/credentials.server";
import { PiqaeAccountLinker } from "../app/core/piqae-account-link.server";
import { PostgresShopRepository } from "../app/core/postgres-shop-repository.server";
import { starterTemplates } from "../app/core/starter-templates";
import {
  seedStarterTemplates,
  systemTemplateId,
} from "../app/core/template-index.server";
import {
  parseTemplateEnvelope,
  serializeTemplateEnvelope,
} from "../app/core/template-model";
import { templateDigest } from "../app/core/template-digest.server";
import { PostgresWorkflowRepository } from "../app/core/workflows.server";

const databaseUrl = process.env.PIQAE_TEST_DATABASE_URL;
const postgresDescribe = databaseUrl ? describe : describe.skip;
const shop = "postgres-starters.myshopify.com";
const schema = `piqae_shopify_workflow_${randomBytes(8).toString("hex")}`;
const identifier = `"${schema}"`;
let pool: pg.Pool;
let peerPool: pg.Pool;
let lockPool: pg.Pool;
let peerLockPool: pg.Pool;

async function prepareLinkedShop(
  repository: PostgresShopRepository,
  fixtureShop: string,
  suffix: string,
) {
  await pool.query(
    `INSERT INTO shopify_installations(shop,state)
     VALUES($1,'installed')
     ON CONFLICT(shop) DO UPDATE SET state='installed',uninstalled_at=NULL`,
    [fixtureShop],
  );
  await repository.put({
    shop: fixtureShop,
    piqaeAccountId: `account_${suffix}`,
    encryptedCredential: `encrypted_${suffix}`,
    templateRevisionId: `revision_${suffix}`,
    createdAt: "2026-08-30T00:00:00.000Z",
  });
}

postgresDescribe("PostgreSQL starter publication transactions", () => {
  beforeAll(async () => {
    if (!databaseUrl) return;
    if (!/^piqae_shopify_workflow_[a-f0-9]{16}$/.test(schema))
      throw new Error("unsafe PostgreSQL test schema");
    const bootstrap = new pg.Pool({ connectionString: databaseUrl, max: 1 });
    await bootstrap.query(`CREATE SCHEMA ${identifier}`);
    await bootstrap.end();
    pool = new pg.Pool({
      connectionString: databaseUrl,
      max: 3,
      options: `-c search_path=${schema},public`,
    });
    peerPool = new pg.Pool({
      connectionString: databaseUrl,
      max: 3,
      options: `-c search_path=${schema},public`,
    });
    lockPool = new pg.Pool({
      connectionString: databaseUrl,
      max: 2,
      connectionTimeoutMillis: 1_000,
    });
    peerLockPool = new pg.Pool({
      connectionString: databaseUrl,
      max: 2,
      connectionTimeoutMillis: 1_000,
    });
    for (const migration of [
      "0001_shopify_core.sql",
      "0002_merchant_workflows.sql",
      "0003_render_execution_policy.sql",
      "0004_managed_piqae_accounts.sql",
      "0005_template_targets_and_media.sql",
      "0006_template_draft_published_pointers.sql",
      "0007_print_order_settings.sql",
      "0008_document_usage_events.sql",
    ])
      await pool.query(
        await readFile(
          new URL(`../migrations/${migration}`, import.meta.url),
          "utf8",
        ),
      );
    await pool.query(
      "INSERT INTO shopify_installations(shop,state) VALUES($1,'installed')",
      [shop],
    );
  });

  afterAll(async () => {
    if (!databaseUrl || !pool) return;
    await pool.end();
    await peerPool.end();
    await lockPool.end();
    await peerLockPool.end();
    const cleanup = new pg.Pool({ connectionString: databaseUrl, max: 1 });
    await cleanup.query(`DROP SCHEMA ${identifier} CASCADE`);
    await cleanup.end();
  });

  it("updates independent settings without overwriting a concurrent preference", async () => {
    const first = new PostgresWorkflowRepository(pool);
    const second = new PostgresWorkflowRepository(peerPool);
    await Promise.all([
      first.updateSettings(shop, { defaultTemplateId: "template_quick" }),
      second.updateSettings(shop, {
        printOrder: {
          hierarchy: ["taxonomy", "customer"],
          taxonomyDepth: "specific",
          mixedOrderMode: "contains",
        },
      }),
    ]);
    await first.updateSettings(shop, { defaultPrinterId: "printer_default" });
    expect(await first.getSettings(shop)).toMatchObject({
      defaultTemplateId: "template_quick",
      defaultPrinterId: "printer_default",
      printOrder: {
        hierarchy: ["taxonomy", "customer"],
        taxonomyDepth: "specific",
        mixedOrderMode: "contains",
      },
    });
  });

  it("keeps the deterministic owner and rolls a stale publication batch back", async () => {
    const repository = new PostgresWorkflowRepository(pool);
    await seedStarterTemplates(repository, shop);
    const deterministic = (await repository.listTemplates(shop)).find(
      ({ id }) => id === "00000000-0000-4000-8000-000000000001",
    )!;
    const ownerEnvelope = parseTemplateEnvelope(deterministic.source);
    ownerEnvelope.published = {
      piqaeAccountId: "account_owner",
      piqaeEnvironmentId: null,
      piqaeTemplateId: "template_owner",
      piqaeRevisionId: "revision_owner",
      canonicalDigest: templateDigest(JSON.stringify(ownerEnvelope.document)),
    };
    const owner = await repository.saveTemplate(shop, {
      ...deterministic,
      source: serializeTemplateEnvelope(ownerEnvelope),
      expectedDraftRevision: deterministic.draftRevision,
    });
    await repository.saveTemplate(shop, {
      ...starterTemplates[0]!,
      id: "00000000-0000-4000-8000-000000009999",
      state: "published",
      revision: 1,
    });
    await seedStarterTemplates(repository, shop);
    expect(await repository.getTemplate(shop, owner.id)).toEqual(owner);

    const before = await repository.listTemplates(shop);
    const first = before.find(({ id }) => id.endsWith("0001"))!;
    const second = before.find(({ id }) => id.endsWith("0002"))!;
    await expect(
      repository.saveTemplatesAtomically(shop, [
        {
          ...first,
          name: "Must roll back",
          expectedDraftRevision: first.draftRevision,
        },
        {
          ...second,
          name: "Stale",
          expectedDraftRevision: second.draftRevision - 1,
        },
      ]),
    ).rejects.toThrow("changed in another session");
    expect(await repository.getTemplate(shop, first.id)).toEqual(first);
  });

  it("serializes the inverse relink interleaving across repository instances", async () => {
    const secondShops = new PostgresShopRepository(peerPool, peerLockPool);
    const firstShops = new PostgresShopRepository(pool, lockPool);
    const workflows = new PostgresWorkflowRepository(pool);
    let notifySecondPaused!: () => void;
    let releaseSecond!: () => void;
    let notifyFirstWorkspace!: () => void;
    const secondPaused = new Promise<void>((resolve) => {
      notifySecondPaused = resolve;
    });
    const secondReleased = new Promise<void>((resolve) => {
      releaseSecond = resolve;
    });
    const firstWorkspace = new Promise<void>((resolve) => {
      notifyFirstWorkspace = resolve;
    });
    const makeLinker = (shops: PostgresShopRepository) =>
      new PiqaeAccountLinker(
        shops,
        workflows,
        new CredentialVault(Buffer.alloc(32, 3)),
        (credential) => {
          const suffix = credential.endsWith("second") ? "second" : "first";
          return {
            workspaces: {
              current: async () => {
                if (suffix === "second") {
                  notifySecondPaused();
                  await secondReleased;
                } else {
                  notifyFirstWorkspace();
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
    const secondLinker = makeLinker(secondShops);
    const firstLinker = makeLinker(firstShops);

    const second = secondLinker.linkExisting(shop, "piqae-credential-second");
    await secondPaused;
    const first = firstLinker.linkExisting(shop, "piqae-credential-first");
    expect(
      await Promise.race([
        firstWorkspace.then(() => "entered"),
        new Promise<string>((resolve) =>
          setTimeout(() => resolve("blocked"), 20),
        ),
      ]),
    ).toBe("blocked");
    releaseSecond();
    await expect(second).resolves.toMatchObject({
      piqaeAccountId: "ws_second",
    });
    await expect(first).resolves.toMatchObject({ piqaeAccountId: "ws_first" });

    const active = (await firstShops.get(shop))!;
    expect(active.piqaeAccountId).toBe("ws_first");
    const starterIds = new Set(
      starterTemplates.map((starter) => systemTemplateId(starter.id)),
    );
    expect(
      (await workflows.listTemplates(shop))
        .filter((template) => starterIds.has(template.id))
        .every(
          (template) =>
            parseTemplateEnvelope(template.published!.source).published
              ?.piqaeAccountId === active.piqaeAccountId,
        ),
    ).toBe(true);

    const next = { ...active, planHandle: "cas-updated" };
    expect(await firstShops.putIfCurrentMatches(next, active)).toBe(true);
    expect(await firstShops.putIfCurrentMatches(active, active)).toBe(false);
  });

  it("keeps data capacity when distinct-shop locks exceed the lock pool", async () => {
    const repository = new PostgresShopRepository(pool, lockPool);
    const releases: Array<() => void> = [];
    let entered = 0;
    let notifyTwoEntered!: () => void;
    let notifyThreeEntered!: () => void;
    const twoEntered = new Promise<void>((resolve) => {
      notifyTwoEntered = resolve;
    });
    const threeEntered = new Promise<void>((resolve) => {
      notifyThreeEntered = resolve;
    });
    const runs = ["one", "two", "three"].map((name) =>
      repository.withShopLock(`capacity-${name}.myshopify.com`, async () => {
        await pool.query("SELECT 1");
        entered += 1;
        if (entered === 2) notifyTwoEntered();
        if (entered === 3) notifyThreeEntered();
        await new Promise<void>((resolve) => releases.push(resolve));
      }),
    );

    await twoEntered;
    expect(entered).toBe(2);
    releases.shift()?.();
    await threeEntered;
    expect(entered).toBe(3);
    for (const release of releases) release();
    await expect(Promise.all(runs)).resolves.toEqual([
      undefined,
      undefined,
      undefined,
    ]);
  });

  it("does not resurrect a first link after cross-instance uninstall", async () => {
    const activationShops = new PostgresShopRepository(peerPool, peerLockPool);
    const uninstallShops = new PostgresShopRepository(pool, lockPool);
    await uninstallShops.deleteShop(shop);
    let notifyPaused!: () => void;
    let releaseActivation!: () => void;
    const paused = new Promise<void>((resolve) => {
      notifyPaused = resolve;
    });
    const released = new Promise<void>((resolve) => {
      releaseActivation = resolve;
    });
    const linker = new PiqaeAccountLinker(
      activationShops,
      new PostgresWorkflowRepository(pool),
      new CredentialVault(Buffer.alloc(32, 7)),
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
    const uninstall = uninstallShops.deleteShop(shop).then(() => {
      uninstallFinished = true;
    });
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(uninstallFinished).toBe(false);
    releaseActivation();
    await activation;
    await uninstall;
    expect(await activationShops.get(shop)).toBeNull();
  });

  it("requires an installed linked shop and redacts stored customer ownership", async () => {
    const fixtureShop = "postgres-render-redaction.myshopify.com";
    const repository = new PostgresShopRepository(pool, lockPool);
    await expect(
      repository.recordRender(
        fixtureShop,
        "render_without_link",
        "ownership-key-without-link",
      ),
    ).rejects.toThrow("SHOPIFY_RENDER_OWNERSHIP_UNAVAILABLE");

    await prepareLinkedShop(repository, fixtureShop, "redaction");
    await repository.recordRender(
      fixtureShop,
      "render_redacted",
      "ownership-key-redacted",
      {
        orderGid: "gid://shopify/Order/51",
        customerGid: "gid://shopify/Customer/61",
      },
    );
    await repository.recordRender(
      fixtureShop,
      "render_retained",
      "ownership-key-retained",
      {
        orderGid: "gid://shopify/Order/52",
        customerGid: "gid://shopify/Customer/62",
      },
    );
    await expect(
      repository.recordRender(
        fixtureShop,
        "render_redacted",
        "ownership-key-redacted",
        {
          orderGid: "gid://shopify/Order/51",
          customerGid: "gid://shopify/Customer/61",
        },
      ),
    ).resolves.toBeUndefined();
    await expect(
      repository.recordRender(
        fixtureShop,
        "render_conflicting",
        "ownership-key-redacted",
        {
          orderGid: "gid://shopify/Order/99",
          customerGid: "gid://shopify/Customer/99",
        },
      ),
    ).rejects.toThrow("SHOPIFY_RENDER_OWNERSHIP_CONFLICT");

    await repository.redactCustomer(fixtureShop, "61");
    expect(await repository.ownsRender(fixtureShop, "render_redacted")).toBe(
      false,
    );
    expect(await repository.ownsRender(fixtureShop, "render_retained")).toBe(
      true,
    );
    await repository.redactCustomer(fixtureShop, "gid://shopify/Customer/62");
    expect(await repository.ownsRender(fixtureShop, "render_retained")).toBe(
      false,
    );

    await pool.query(
      "UPDATE shopify_installations SET state='uninstalled' WHERE shop=$1",
      [fixtureShop],
    );
    await expect(
      repository.recordRender(
        fixtureShop,
        "render_uninstalled",
        "ownership-key-uninstalled",
      ),
    ).rejects.toThrow("SHOPIFY_RENDER_OWNERSHIP_UNAVAILABLE");
  });

  it("serializes an in-flight ownership registration with uninstall and does not restore it on reinstall", async () => {
    const fixtureShop = "postgres-render-uninstall.myshopify.com";
    const gateRepository = new PostgresShopRepository(peerPool, peerLockPool);
    const uninstallRepository = new PostgresShopRepository(pool, lockPool);
    await prepareLinkedShop(gateRepository, fixtureShop, "uninstall_first");
    await gateRepository.recordRender(
      fixtureShop,
      "render_before_uninstall",
      "ownership-key-before-uninstall",
    );

    let notifyEntered!: () => void;
    let releaseGate!: () => void;
    const entered = new Promise<void>((resolve) => {
      notifyEntered = resolve;
    });
    const released = new Promise<void>((resolve) => {
      releaseGate = resolve;
    });
    const gate = gateRepository.withShopLock(fixtureShop, async () => {
      notifyEntered();
      await released;
    });
    await entered;
    const uninstall = uninstallRepository.deleteShop(fixtureShop);
    const lateRecord = gateRepository
      .recordRender(
        fixtureShop,
        "render_racing_uninstall",
        "ownership-key-racing-uninstall",
      )
      .then(
        () => "recorded" as const,
        () => "rejected" as const,
      );
    releaseGate();
    await gate;
    await uninstall;
    await lateRecord;

    expect(
      await uninstallRepository.ownsRender(
        fixtureShop,
        "render_before_uninstall",
      ),
    ).toBe(false);
    expect(
      await uninstallRepository.ownsRender(
        fixtureShop,
        "render_racing_uninstall",
      ),
    ).toBe(false);

    await pool.query(
      "UPDATE shopify_installations SET state='uninstalled' WHERE shop=$1",
      [fixtureShop],
    );
    await pool.query(
      "UPDATE shopify_installations SET state='installed',uninstalled_at=NULL WHERE shop=$1",
      [fixtureShop],
    );
    await gateRepository.put({
      shop: fixtureShop,
      piqaeAccountId: "account_uninstall_reinstalled",
      encryptedCredential: "encrypted_uninstall_reinstalled",
      templateRevisionId: "revision_uninstall_reinstalled",
      createdAt: "2026-08-30T00:00:00.000Z",
    });
    expect(
      await gateRepository.ownsRender(fixtureShop, "render_before_uninstall"),
    ).toBe(false);
    await gateRepository.recordRender(
      fixtureShop,
      "render_after_reinstall",
      "ownership-key-after-reinstall",
    );
    expect(
      await gateRepository.ownsRender(fixtureShop, "render_after_reinstall"),
    ).toBe(true);
  });

  it("rolls ownership deletion back when link deletion fails", async () => {
    const fixtureShop = "postgres-render-rollback.myshopify.com";
    const repository = new PostgresShopRepository(pool, lockPool);
    await prepareLinkedShop(repository, fixtureShop, "rollback");
    await repository.recordRender(
      fixtureShop,
      "render_rollback",
      "ownership-key-rollback",
    );
    await pool.query(`
      CREATE FUNCTION reject_render_lifecycle_link_delete() RETURNS trigger
      LANGUAGE plpgsql AS $$
      BEGIN
        IF OLD.shop = '${fixtureShop}' THEN
          RAISE EXCEPTION 'fixture link deletion failure';
        END IF;
        RETURN OLD;
      END $$;
      CREATE TRIGGER reject_render_lifecycle_link_delete
      BEFORE DELETE ON shopify_shop_links
      FOR EACH ROW EXECUTE FUNCTION reject_render_lifecycle_link_delete();
    `);
    try {
      await expect(repository.deleteShop(fixtureShop)).rejects.toThrow(
        "fixture link deletion failure",
      );
      expect(await repository.get(fixtureShop)).not.toBeNull();
      expect(await repository.ownsRender(fixtureShop, "render_rollback")).toBe(
        true,
      );
    } finally {
      await pool.query(
        "DROP TRIGGER IF EXISTS reject_render_lifecycle_link_delete ON shopify_shop_links; DROP FUNCTION IF EXISTS reject_render_lifecycle_link_delete()",
      );
    }
  });
});
