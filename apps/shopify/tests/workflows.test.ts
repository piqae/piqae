import { describe, expect, it } from "vitest";
import {
  MemoryWorkflowRepository,
  newWorkflowId,
  parseSettings,
  validateDocumentSource,
} from "../app/core/workflows.server";
import {
  ASSET_LIMITS,
  parseTemplateEnvelope,
  serializeTemplateEnvelope,
  visualCompatibility,
} from "../app/core/template-model";
import { templateDigest } from "../app/core/template-digest.server";
import {
  buildTemplateIndex,
  seedStarterTemplates,
} from "../app/core/template-index.server";

const alpha = "alpha.myshopify.com";
const beta = "beta.myshopify.com";

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
      source: '{"schema":"piqae.document/v1"}',
      revision: 1,
    });
    expect(await repository.getTemplate(alpha, id)).not.toBeNull();
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
      source: '{"schema":"piqae.document/v1"}',
      revision: 1,
    });
    expect((await repository.getTemplate(alpha, id))?.state).toBe("published");
    expect(await repository.deleteTemplate(alpha, id)).toBe(false);
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
    form.set("metafields", "custom.vat\ncustom.vat\nlogistics.route");
    form.set("retentionDays", "45");
    form.set("preferDirect", "on");
    expect(parseSettings(form)).toMatchObject({
      retentionDays: 45,
      preferDirect: true,
      metafieldAllowlist: ["custom.vat", "logistics.route"],
    });
  });

  it("rejects broad metafield access and invalid retention", () => {
    const wildcard = new FormData();
    wildcard.set("metafields", "*");
    expect(() => parseSettings(wildcard)).toThrow("namespace.key");
    const retention = new FormData();
    retention.set("retentionDays", "366");
    expect(() => parseSettings(retention)).toThrow("between 1 and 365");
  });
});

describe("template source validation", () => {
  it("accepts only the bounded native schema", () => {
    expect(
      validateDocumentSource('{"schema":"piqae.document/v1","nodes":[]}'),
    ).toContain("piqae.document/v1");
    expect(() => validateDocumentSource('{"schema":"pdfme"}')).toThrow(
      "piqae.shopify-template/v1",
    );
    expect(() => validateDocumentSource("not json")).toThrow("valid JSON");
  });
});

describe("hybrid template authority", () => {
  it("seeds eight immutable published defaults once", async () => {
    const repository = new MemoryWorkflowRepository();
    await seedStarterTemplates(repository, alpha);
    await seedStarterTemplates(repository, alpha);
    const templates = await repository.listTemplates(alpha);
    expect(templates).toHaveLength(8);
    expect(
      templates.every(
        (value) => parseTemplateEnvelope(value.source).system?.immutable,
      ),
    ).toBe(true);
  });

  it("builds a compact non-sensitive Shopify cache", async () => {
    const repository = new MemoryWorkflowRepository();
    await seedStarterTemplates(repository, alpha);
    const index = buildTemplateIndex(
      await repository.listTemplates(alpha),
      await repository.getSettings(alpha),
    );
    expect(index.documents).toHaveLength(8);
    expect(JSON.stringify(index)).not.toContain("canonical");
    expect(index.digest).toMatch(/^[a-f0-9]{64}$/);
  });

  it("accepts native documents through a compatibility envelope", () => {
    const source =
      '{"spec_version":"piqae.document/v1","page":{"size":"a4","margin_mm":10},"body":[]}';
    const envelope = parseTemplateEnvelope(source);
    expect(envelope.editor.mode).toBe("native");
    expect(templateDigest(serializeTemplateEnvelope(envelope))).toMatch(
      /^[a-f0-9]{64}$/,
    );
  });

  it("reports exact supported visual mappings and rejects unpinned assets", () => {
    expect(
      visualCompatibility({
        schema: "pdfme-compatible/v1",
        page: "A4",
        fields: [
          {
            id: "qr",
            type: "qrcode",
            x: 0,
            y: 0,
            width: 20,
            height: 20,
            binding: "/order/id",
          },
        ],
      }).roundTrip,
    ).toBe("lossless");
    expect(() =>
      serializeTemplateEnvelope({
        schema: "piqae.shopify-template/v1",
        canonical: {
          spec_version: "piqae.document/v1",
          page: { size: "a4", margin_mm: 10 },
          body: [],
        },
        editor: { mode: "native", roundTrip: "lossless", warnings: [] },
        assets: [
          {
            url: "http://cdn.shopify.com/font.woff2",
            sha256: "0".repeat(64),
            contentType: "font/woff2",
            bytes: ASSET_LIMITS.maxBytes,
          },
        ],
      }),
    ).toThrow("allowlisted HTTPS CDN");
  });
});
