import { describe, expect, it } from "vitest";

import {
  ACCOUNT_DEFAULT_DOCUMENT_ID,
  adminPreviewPlaceholderUrl,
  loadAdminPrintOptions,
} from "../app/core/admin-print-options.server";
import { CredentialVault } from "../app/core/credentials.server";
import { MemoryShopRepository } from "../app/core/model";
import { MemoryWorkflowRepository } from "../app/core/workflows.server";
import { starterTemplates } from "../app/core/starter-templates";

const APP_URL = "https://shopify.example.com";

function a4Target(
  specificationRevision: string,
  readinessStatus: "ready" | "not_ready" = "ready",
  mediaStatus: "ready" | "not_reported" = "ready",
) {
  const now = Date.now();
  return {
    target: {
      id: "tgt_a4",
      name: "Packing station",
      enabled: true,
    },
    stock: {
      id: "stock_a4",
      name: "A4",
      attributes: {
        kind: "sheet",
        width_mm: 210,
        height_mm: 297,
        orientation: "either",
      },
    },
    readiness: {
      status: readinessStatus,
      selected_binding_id: "bind_a4",
      bindings: [
        {
          binding: { id: "bind_a4" },
          status: readinessStatus === "ready" ? "ready" : "node_offline",
          reasons: readinessStatus === "ready" ? [] : ["node_offline"],
        },
      ],
    },
    destinations: [
      {
        binding: {
          id: "bind_a4",
          destination_id: "pdst_a4",
          route_id: "route_a4",
          role: "primary",
          enabled: true,
        },
        printer: { id: "printer_a4", name: "Office printer" },
        profile: { name: "A4 profile" },
        media_compatibility: {
          status: mediaStatus,
          reasons: mediaStatus === "ready" ? [] : ["stock_not_loaded"],
          profile_dimensions_mm: { width_mm: 210, height_mm: 297 },
          loaded_media: {
            source: "node",
            confidence: "operator_confirmed",
            observed_at: new Date(now - 60_000).toISOString(),
            fresh_until: new Date(now + 15 * 60_000).toISOString(),
            stock: { id: "stock_a4", revision: 1 },
          },
        },
      },
    ],
    specification_revision: specificationRevision,
  };
}

describe("admin print options", () => {
  it("normalizes the absolute preview placeholder URL", () => {
    expect(adminPreviewPlaceholderUrl(`${APP_URL}/`)).toBe(
      `${APP_URL}/api/public/print-placeholder`,
    );
  });

  it("returns published documents and a setup state before Piqae is linked", async () => {
    const shops = new MemoryShopRepository();
    const workflowRepository = new MemoryWorkflowRepository();
    await workflowRepository.saveTemplate("fixtures.myshopify.com", {
      id: "published-invoice",
      name: "Invoice",
      kind: "invoice",
      pageSize: "A4",
      state: "published",
      source: starterTemplates[0]!.source,
      revision: 1,
    });
    await workflowRepository.saveTemplate("fixtures.myshopify.com", {
      id: "draft-slip",
      name: "Draft packing slip",
      kind: "packing_slip",
      pageSize: "A4",
      state: "draft",
      source: starterTemplates[0]!.source,
      revision: 1,
    });

    const result = await loadAdminPrintOptions({
      shop: "fixtures.myshopify.com",
      shops,
      workflows: workflowRepository,
      vault: CredentialVault.fromBase64(Buffer.alloc(32, 4).toString("base64")),
      baseUrl: "https://unused.example.invalid",
      appUrl: APP_URL,
    });

    expect(result.linked).toBe(false);
    expect(result.documents).toEqual([
      expect.objectContaining({
        id: "published-invoice",
        name: "Invoice",
        targetBindingStatus: "unbound",
      }),
    ]);
    expect(result.targets).toEqual([]);
    expect(result.printers).toEqual([]);
    expect(result.setupDestinationUrl).toBe("/app/printers");
    expect(result.previewPlaceholderUrl).toBe(
      "https://shopify.example.com/api/public/print-placeholder",
    );
    expect(result.renderExecutionPolicy).toBe("automatic");
  });

  it("keeps a malformed publication isolated from the document list", async () => {
    const shops = new MemoryShopRepository();
    const workflows = {
      getSettings: async () => ({
        defaultPrinterId: "",
        defaultTemplateId: "",
        preferDirect: true,
        offerPdf: true,
        metafieldAllowlist: [],
        retentionDays: 30,
        renderExecutionPolicy: "automatic" as const,
      }),
      listTemplates: async () => [
        {
          id: "damaged",
          state: "published",
          published: {
            name: "Damaged document",
            kind: "invoice",
            designTargetId: "tgt_a4",
            designSpecificationRevision: "spec_a4_1",
            source: "not-json",
          },
        },
      ],
    };
    const result = await loadAdminPrintOptions({
      shop: "fixtures.myshopify.com",
      shops,
      workflows: workflows as never,
      vault: CredentialVault.fromBase64(Buffer.alloc(32, 4).toString("base64")),
      baseUrl: "https://unused.example.invalid",
      appUrl: APP_URL,
    });
    expect(result.documents).toEqual([
      expect.objectContaining({
        id: "damaged",
        targetBindingStatus: "document_invalid",
        compatibilityKnown: false,
        compatibleTargetIds: [],
      }),
    ]);
  });

  it("uses the immutable published target revision and surfaces target drift", async () => {
    const shops = new MemoryShopRepository();
    const workflowRepository = new MemoryWorkflowRepository();
    await shops.put({
      shop: "fixtures.myshopify.com",
      piqaeAccountId: "wsp_fixture",
      piqaeLiveEnvironmentId: "env_live",
      piqaeTestEnvironmentId: "env_test",
      encryptedCredential: "",
      templateRevisionId: "rev_starter_invoice",
      entitlementMode: "shopify_child",
      planHandle: "development",
      createdAt: new Date(0).toISOString(),
    });
    await workflowRepository.saveTemplate("fixtures.myshopify.com", {
      ...starterTemplates[0]!,
      id: "published-invoice",
      state: "published",
      revision: 1,
      designTargetId: "tgt_a4",
      designSpecificationRevision: "spec_a4_1",
    });
    let currentRevision = "spec_a4_1";
    const managedClientFactory = () =>
      ({
        targets: {
          list: async () => [{ id: "tgt_a4", enabled: true }],
          designSpecification: async () => a4Target(currentRevision),
        },
        printers: {
          list: async () => ({
            data: [
              {
                id: "printer_a4",
                name: "Office printer",
                state: "online",
              },
              {
                id: "printer_label",
                name: "Label printer",
                state: "offline",
              },
            ],
          }),
        },
      }) as never;
    const input = {
      shop: "fixtures.myshopify.com",
      shops,
      workflows: workflowRepository,
      vault: CredentialVault.fromBase64(Buffer.alloc(32, 4).toString("base64")),
      baseUrl: "https://unused.example.invalid",
      appUrl: APP_URL,
      managedClientFactory,
    };

    const ready = await loadAdminPrintOptions(input);
    expect(ready.documents[0]).toMatchObject({
      designTargetId: "tgt_a4",
      designSpecificationRevision: "spec_a4_1",
      targetBindingStatus: "ready",
      compatibleTargetIds: ["tgt_a4"],
      advisoryDestination: {
        printerName: "Office printer",
        profileName: "A4 profile",
        mediaStatus: "ready",
      },
    });
    expect(ready.printers).toEqual([
      expect.objectContaining({
        id: "printer_a4",
        targetIds: ["tgt_a4"],
      }),
      expect.objectContaining({
        id: "printer_label",
        targetIds: [],
      }),
    ]);

    const offlineButConfigured = await loadAdminPrintOptions({
      ...input,
      managedClientFactory: () =>
        ({
          targets: {
            list: async () => [{ id: "tgt_a4", enabled: true }],
            designSpecification: async () =>
              a4Target(currentRevision, "not_ready", "ready"),
          },
        }) as never,
    });
    expect(offlineButConfigured.documents[0]).toMatchObject({
      targetBindingStatus: "ready",
      compatibleTargetIds: ["tgt_a4"],
      advisoryDestination: { readinessStatus: "node_offline" },
    });

    const stalePrimary = a4Target(currentRevision);
    stalePrimary.destinations[0]!.media_compatibility.status = "not_reported";
    stalePrimary.destinations[0]!.media_compatibility.reasons = [
      "loaded_media_stale",
    ];
    stalePrimary.destinations.push({
      ...structuredClone(stalePrimary.destinations[0]!),
      binding: {
        id: "bind_a4_standby",
        destination_id: "pdst_a4_standby",
        route_id: "route_a4_standby",
        role: "standby",
        enabled: true,
      },
      printer: { id: "printer_a4_standby", name: "Standby office printer" },
      media_compatibility: {
        ...structuredClone(stalePrimary.destinations[0]!.media_compatibility),
        status: "ready",
        reasons: [],
      },
    });
    stalePrimary.readiness.bindings.push({
      binding: { id: "bind_a4_standby" },
      status: "ready",
      reasons: [],
    });
    const standby = await loadAdminPrintOptions({
      ...input,
      managedClientFactory: () =>
        ({
          targets: {
            list: async () => [{ id: "tgt_a4", enabled: true }],
            designSpecification: async () => stalePrimary,
          },
        }) as never,
    });
    expect(standby.documents[0]).toMatchObject({
      targetBindingStatus: "ready",
      compatibleTargetIds: ["tgt_a4"],
      advisoryDestination: {
        printerName: "Standby office printer",
        mediaStatus: "ready",
      },
    });

    const partial = await loadAdminPrintOptions({
      ...input,
      managedClientFactory: () =>
        ({
          targets: {
            list: async () => [
              { id: "tgt_a4", enabled: true },
              { id: "tgt_unknown", enabled: true },
            ],
            designSpecification: async (id: string) => {
              if (id === "tgt_unknown") throw new Error("temporary outage");
              return a4Target(currentRevision);
            },
          },
        }) as never,
    });
    expect(partial.destinationError).toBe(
      "Some printer status is temporarily unavailable",
    );
    expect(partial.documents[0]).toMatchObject({
      targetBindingStatus: "ready",
      compatibleTargetIds: ["tgt_a4"],
    });

    currentRevision = "spec_a4_2";
    const drifted = await loadAdminPrintOptions(input);
    expect(drifted.documents[0]).toMatchObject({
      designSpecificationRevision: "spec_a4_1",
      targetBindingStatus: "revision_changed",
      compatibleTargetIds: [],
    });

    currentRevision = "spec_a4_1";
    const notReady = await loadAdminPrintOptions({
      ...input,
      managedClientFactory: () =>
        ({
          targets: {
            list: async () => [{ id: "tgt_a4", enabled: true }],
            designSpecification: async () =>
              a4Target(currentRevision, "not_ready", "not_reported"),
          },
        }) as never,
    });
    expect(notReady.documents[0]).toMatchObject({
      targetBindingStatus: "media_unverified",
      compatibleTargetIds: [],
    });

    const unavailable = await loadAdminPrintOptions({
      ...input,
      managedClientFactory: () =>
        ({
          targets: {
            list: async () => {
              throw new Error("temporary outage");
            },
          },
        }) as never,
    });
    expect(unavailable).toMatchObject({
      destinationError: "Printer status is temporarily unavailable",
      documents: [
        {
          id: "published-invoice",
          targetBindingStatus: "unknown",
          compatibleTargetIds: [],
        },
      ],
    });
  });

  it("offers a printable account default when nothing is published yet", async () => {
    const shops = new MemoryShopRepository();
    await shops.put({
      shop: "fixtures.myshopify.com",
      piqaeAccountId: "wsp_fixture",
      piqaeLiveEnvironmentId: "env_live",
      piqaeTestEnvironmentId: "env_test",
      encryptedCredential: "",
      templateRevisionId: "rev_starter_invoice",
      entitlementMode: "shopify_child",
      planHandle: "development",
      createdAt: new Date(0).toISOString(),
    });

    const result = await loadAdminPrintOptions({
      shop: "fixtures.myshopify.com",
      shops,
      workflows: new MemoryWorkflowRepository(),
      vault: CredentialVault.fromBase64(Buffer.alloc(32, 4).toString("base64")),
      baseUrl: "https://unused.example.invalid",
      appUrl: APP_URL,
      managedClientFactory: () =>
        ({
          targets: { list: async () => [] },
        }) as never,
    });

    // The id must be resolvable by the print path. Exposing the revision id
    // here produced a blank picker and a failed preview.
    expect(result.documents).toEqual([
      expect.objectContaining({
        id: ACCOUNT_DEFAULT_DOCUMENT_ID,
        isDefault: true,
      }),
    ]);
  });
});
