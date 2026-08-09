import { describe, expect, it } from "vitest";
import {
  MemoryWorkflowRepository,
  newWorkflowId,
  parseSettings,
  validateDocumentSource,
} from "../app/core/workflows.server";

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
      "piqae.document/v1",
    );
    expect(() => validateDocumentSource("not json")).toThrow("valid JSON");
  });
});
