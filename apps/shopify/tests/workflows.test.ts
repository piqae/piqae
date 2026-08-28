import { describe, expect, it } from "vitest";
import {
  MemoryWorkflowRepository,
  newWorkflowId,
  parseSettings,
  validateDocumentSource,
  WorkflowConflictError,
  type MerchantTemplate,
} from "../app/core/workflows.server";
import {
  ASSET_LIMITS,
  parseTemplateEnvelope,
  serializeTemplateEnvelope,
} from "../app/core/template-model";
import { starterTemplates } from "../app/core/starter-templates";
import { templateDigest } from "../app/core/template-digest.server";
import {
  buildTemplateIndex,
  isActiveTemplate,
  seedStarterTemplates,
} from "../app/core/template-index.server";

const alpha = "alpha.myshopify.com";
const beta = "beta.myshopify.com";

class CorruptedTemplateReadRepository extends MemoryWorkflowRepository {
  private corruption?: {
    id: string;
    draftRevision: number;
    source: string;
  };

  corruptTemplateRead(template: MerchantTemplate, source: string) {
    this.corruption = {
      id: template.id,
      draftRevision: template.draftRevision,
      source,
    };
  }

  private corrupted(value: MerchantTemplate): MerchantTemplate {
    if (
      value.id !== this.corruption?.id ||
      value.draftRevision !== this.corruption.draftRevision
    )
      return value;
    return {
      ...value,
      source: this.corruption.source,
      published: value.published
        ? { ...value.published, source: this.corruption.source }
        : null,
    };
  }

  override async listTemplates(shop: string) {
    return (await super.listTemplates(shop)).map((value) =>
      this.corrupted(value),
    );
  }

  override async getTemplate(shop: string, id: string) {
    const value = await super.getTemplate(shop, id);
    return value ? this.corrupted(value) : null;
  }
}

describe("merchant workflow persistence", () => {
  it("keeps every resource tenant scoped", async () => {
    const repository = new MemoryWorkflowRepository();
    const id = newWorkflowId();
    await repository.saveTemplate(alpha, {
      id,
      name: "Invoice",
      kind: "invoice",
      pageSize: "A4",
      state: "draft",
      source: starterTemplates[0]!.source,
      revision: 1,
      designTargetId: "tgt_invoice",
      designSpecificationRevision: "spec_invoice_3",
    });
    expect(await repository.getTemplate(alpha, id)).toMatchObject({
      designTargetId: "tgt_invoice",
      designSpecificationRevision: "spec_invoice_3",
    });
    expect(await repository.getTemplate(beta, id)).toBeNull();
    expect(await repository.deleteTemplate(beta, id)).toBe(false);
    expect(await repository.getTemplate(alpha, id)).not.toBeNull();
  });

  it("publishes revisions without making published templates deletable", async () => {
    const repository = new MemoryWorkflowRepository();
    const id = newWorkflowId();
    await repository.saveTemplate(alpha, {
      id,
      name: "Packing slip",
      kind: "packing_slip",
      pageSize: "A4",
      state: "published",
      source: starterTemplates[1]!.source,
      revision: 1,
    });
    expect((await repository.getTemplate(alpha, id))?.state).toBe("published");
    expect(await repository.deleteTemplate(alpha, id)).toBe(false);
  });

  it("keeps publication immutable and rejects stale draft saves", async () => {
    const repository = new MemoryWorkflowRepository();
    const id = newWorkflowId();
    const published = await repository.saveTemplate(alpha, {
      id,
      name: "Receipt v1",
      kind: "receipt",
      pageSize: "80mm",
      state: "published",
      source: starterTemplates[2]!.source,
      revision: 1,
      designTargetId: "target_receipt",
      designSpecificationRevision: "spec_1",
    });
    const draft = await repository.saveTemplate(alpha, {
      id,
      name: "Receipt draft v2",
      kind: "receipt",
      pageSize: "80mm",
      state: "draft",
      source: starterTemplates[2]!.source,
      revision: published.revision,
      designTargetId: "target_other",
      designSpecificationRevision: "spec_2",
      expectedDraftRevision: published.draftRevision,
    });
    expect(draft.published).toMatchObject({
      revision: 1,
      name: "Receipt v1",
      designTargetId: "target_receipt",
      designSpecificationRevision: "spec_1",
      media: { kind: "continuous", width_mm: 80 },
    });
    await expect(
      repository.saveTemplate(alpha, {
        id,
        name: "Stale edit",
        kind: "receipt",
        pageSize: "80mm",
        state: "draft",
        source: starterTemplates[2]!.source,
        revision: 1,
        expectedDraftRevision: published.draftRevision,
      }),
    ).rejects.toBeInstanceOf(WorkflowConflictError);
  });

  it("filters bounded activity without crossing shops", async () => {
    const repository = new MemoryWorkflowRepository();
    await repository.recordActivity(alpha, {
      id: newWorkflowId(),
      orderName: "#1042",
      documentName: "Invoice",
      destination: "Warehouse",
      state: "uncertain",
    });
    expect(
      await repository.listActivity(alpha, "1042", "uncertain"),
    ).toHaveLength(1);
    expect(await repository.listActivity(beta)).toHaveLength(0);
  });
});

describe("settings validation", () => {
  it("deduplicates valid allowlisted metafields", () => {
    const form = new FormData();
    form.set(
      "metafields",
      "custom.vat\ncustom.vat\nlogistics.route\nproduct:custom.origin.country",
    );
    form.set("retentionDays", "45");
    form.set("preferDirect", "on");
    form.set("renderExecutionPolicy", "prefer_node");
    expect(parseSettings(form)).toMatchObject({
      retentionDays: 45,
      preferDirect: true,
      metafieldAllowlist: [
        "custom.vat",
        "logistics.route",
        "product:custom.origin.country",
      ],
      renderExecutionPolicy: "prefer_node",
    });
  });

  it("rejects broad metafield access and invalid retention", () => {
    const wildcard = new FormData();
    wildcard.set("metafields", "*");
    expect(() => parseSettings(wildcard)).toThrow("namespace.key");
    const retention = new FormData();
    retention.set("retentionDays", "366");
    expect(() => parseSettings(retention)).toThrow("between 1 and 365");
    const policy = new FormData();
    policy.set("renderExecutionPolicy", "fastest_at_any_cost");
    expect(() => parseSettings(policy)).toThrow("Render location");
  });
});

describe("template source validation", () => {
  it("accepts only the bounded PrintPacket envelope", () => {
    expect(validateDocumentSource(starterTemplates[0]!.source)).toContain(
      "printpacket/v1",
    );
    expect(() => validateDocumentSource('{"schema":"unsupported"}')).toThrow(
      "piqae.shopify-printpacket-template/v1",
    );
    expect(() => validateDocumentSource("not json")).toThrow("valid JSON");
  });
});

describe("hybrid template authority", () => {
  it("seeds the focused immutable published defaults once", async () => {
    const repository = new MemoryWorkflowRepository();
    await seedStarterTemplates(repository, alpha);
    await seedStarterTemplates(repository, alpha);
    const templates = await repository.listTemplates(alpha);
    expect(templates).toHaveLength(starterTemplates.length);
    expect(
      templates.every(
        (value) => parseTemplateEnvelope(value.source).system?.immutable,
      ),
    ).toBe(true);
  });

  it("seeds concurrently without creating replacement revisions", async () => {
    const repository = new MemoryWorkflowRepository();
    await Promise.all(
      Array.from({ length: 12 }, () => seedStarterTemplates(repository, alpha)),
    );
    const templates = await repository.listTemplates(alpha);
    expect(templates).toHaveLength(starterTemplates.length);
    expect(templates.every((value) => value.revision === 1)).toBe(true);
  });

  it("retains exact immutable publication pins across restart reseeding", async () => {
    const repository = new MemoryWorkflowRepository();
    await seedStarterTemplates(repository, alpha);
    const invoice = (await repository.listTemplates(alpha)).find(
      (value) => parseTemplateEnvelope(value.source).system?.key === "invoice",
    )!;
    const envelope = parseTemplateEnvelope(invoice.source);
    envelope.published = {
      piqaeAccountId: "account_invoice",
      piqaeEnvironmentId: null,
      piqaeTemplateId: "template_invoice",
      piqaeRevisionId: "revision_invoice",
      canonicalDigest: templateDigest(JSON.stringify(envelope.document)),
    };
    const pinned = await repository.saveTemplate(alpha, {
      ...invoice,
      source: serializeTemplateEnvelope(envelope),
      expectedDraftRevision: invoice.draftRevision,
    });

    await Promise.all(
      Array.from({ length: 8 }, () => seedStarterTemplates(repository, alpha)),
    );
    const restarted = await repository.getTemplate(alpha, invoice.id);
    expect(restarted).toEqual(pinned);
    expect(
      parseTemplateEnvelope(restarted!.published!.source).published,
    ).toEqual(envelope.published);
  });

  it("never lets a duplicate system-key clone replace the deterministic owner", async () => {
    const repository = new MemoryWorkflowRepository();
    await seedStarterTemplates(repository, alpha);
    const deterministic = (await repository.listTemplates(alpha)).find(
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
    const owner = await repository.saveTemplate(alpha, {
      ...deterministic,
      source: serializeTemplateEnvelope(ownerEnvelope),
      expectedDraftRevision: deterministic.draftRevision,
    });
    const cloneEnvelope = parseTemplateEnvelope(starterTemplates[0]!.source);
    cloneEnvelope.published = {
      piqaeAccountId: "account_clone",
      piqaeEnvironmentId: null,
      piqaeTemplateId: "template_clone",
      piqaeRevisionId: "revision_clone",
      canonicalDigest: templateDigest(JSON.stringify(cloneEnvelope.document)),
    };
    await repository.saveTemplate(alpha, {
      ...starterTemplates[0]!,
      id: newWorkflowId(),
      source: serializeTemplateEnvelope(cloneEnvelope),
      state: "published",
      revision: 1,
    });

    await seedStarterTemplates(repository, alpha);
    expect(await repository.getTemplate(alpha, owner.id)).toEqual(owner);
  });

  it.each([
    {
      piqaeTemplateId: "template_invoice",
      piqaeRevisionId: "",
      canonicalDigest: "a".repeat(64),
    },
    {
      piqaeTemplateId: "template_invoice",
      piqaeRevisionId: "revision_invoice",
      canonicalDigest: "a".repeat(64),
    },
    {
      piqaeTemplateId: "template invoice",
      piqaeRevisionId: "revision_invoice",
      canonicalDigest: "current",
    },
  ])(
    "fails closed and repairs a malformed publication pin",
    async (published) => {
      const repository = new CorruptedTemplateReadRepository();
      await seedStarterTemplates(repository, alpha);
      const invoice = (await repository.listTemplates(alpha)).find(
        (value) =>
          parseTemplateEnvelope(value.source).system?.key === "invoice",
      )!;
      const envelope = parseTemplateEnvelope(invoice.source);
      envelope.published = {
        piqaeAccountId: "account_invoice",
        piqaeEnvironmentId: null,
        ...published,
        canonicalDigest:
          published.canonicalDigest === "current"
            ? templateDigest(JSON.stringify(envelope.document))
            : published.canonicalDigest,
      };
      repository.corruptTemplateRead(invoice, JSON.stringify(envelope));

      await seedStarterTemplates(repository, alpha);
      const repaired = await repository.getTemplate(alpha, invoice.id);
      expect(repaired?.source).toBe(starterTemplates[0]!.source);
      expect(repaired?.published?.source).toBe(starterTemplates[0]!.source);
      expect(
        parseTemplateEnvelope(repaired!.published!.source).published,
      ).toBeUndefined();
    },
  );

  it("builds a compact non-sensitive Shopify cache", async () => {
    const repository = new MemoryWorkflowRepository();
    await seedStarterTemplates(repository, alpha);
    const index = buildTemplateIndex(
      await repository.listTemplates(alpha),
      await repository.getSettings(alpha),
    );
    expect(index.documents).toHaveLength(
      (await repository.listTemplates(alpha)).filter(isActiveTemplate).length,
    );
    expect(JSON.stringify(index)).not.toContain("canonical");
    expect(index.digest).toMatch(/^[a-f0-9]{64}$/);
    expect(index.documents[0]).toMatchObject({
      designTargetId: null,
      designSpecificationRevision: null,
    });
  });

  it("indexes the immutable publication while newer draft edits stay private", async () => {
    const repository = new MemoryWorkflowRepository();
    await seedStarterTemplates(repository, alpha);
    const invoice = (await repository.listTemplates(alpha)).find(
      ({ id }) => id === "00000000-0000-4000-8000-000000000001",
    )!;
    await repository.saveTemplate(alpha, {
      id: invoice.id,
      name: "Unpublished invoice edit",
      kind: invoice.kind,
      pageSize: invoice.pageSize,
      state: "draft",
      source: invoice.source,
      revision: invoice.revision,
      designTargetId: "target_draft_only",
      designSpecificationRevision: "spec_draft_only",
      expectedDraftRevision: invoice.draftRevision,
    });
    const index = buildTemplateIndex(
      await repository.listTemplates(alpha),
      await repository.getSettings(alpha),
    );
    expect(index.documents.find(({ id }) => id === invoice.id)).toMatchObject({
      name: invoice.published!.name,
      designTargetId: null,
    });
  });

  it("accepts the new PrintPacket envelope", () => {
    const envelope = parseTemplateEnvelope(starterTemplates[0]!.source);
    expect(envelope.editor.mode).toBe("visual");
    expect(templateDigest(serializeTemplateEnvelope(envelope))).toMatch(
      /^[a-f0-9]{64}$/,
    );
  });

  it("rejects unpinned and non-Shopify asset ingestion", () => {
    const envelope = parseTemplateEnvelope(starterTemplates[0]!.source);
    expect(() =>
      serializeTemplateEnvelope({
        ...envelope,
        assets: [
          {
            id: "logo",
            sourceUrl: "http://cdn.shopify.com/logo.jpg",
            digest: "0".repeat(64),
            mediaType: "image/jpeg",
            bytes: ASSET_LIMITS.maxBytes,
          },
        ],
      }),
    ).toThrow("Shopify CDN HTTPS");
    for (const sourceUrl of [
      "https://cdn.shopify.com.attacker.example/logo.jpg",
      "https://cdn.shopify.com:8443/logo.jpg",
      "https://127.0.0.1/logo.jpg",
      "https://shopify.com/logo.jpg",
    ])
      expect(() =>
        serializeTemplateEnvelope({
          ...envelope,
          assets: [
            {
              id: "logo",
              sourceUrl,
              digest: "0".repeat(64),
              mediaType: "image/jpeg",
              bytes: 100,
            },
          ],
        }),
      ).toThrow("Shopify CDN HTTPS");
    expect(() =>
      serializeTemplateEnvelope({
        ...envelope,
        assets: [
          {
            id: "logo",
            sourceUrl: "https://cdn.shopify.com/logo.jpg",
            digest: "A".repeat(64),
            mediaType: "image/jpeg",
            bytes: 100,
          },
        ],
      }),
    ).toThrow("SHA-256 digest");
  });
});
