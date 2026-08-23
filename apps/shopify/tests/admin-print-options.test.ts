import { describe, expect, it } from "vitest";

import {
  ACCOUNT_DEFAULT_DOCUMENT_ID,
  loadAdminPrintOptions,
} from "../app/core/admin-print-options.server";
import { CredentialVault } from "../app/core/credentials.server";
import { MemoryShopRepository } from "../app/core/model";
import { MemoryWorkflowRepository } from "../app/core/workflows.server";

describe("admin print options", () => {
  it("returns published documents and a setup state before Piqae is linked", async () => {
    const shops = new MemoryShopRepository();
    const workflowRepository = new MemoryWorkflowRepository();
    await workflowRepository.saveTemplate("fixtures.myshopify.com", {
      id: "published-invoice",
      name: "Invoice",
      kind: "invoice",
      pageSize: "A4",
      state: "published",
      source: "{}",
      revision: 1,
    });
    await workflowRepository.saveTemplate("fixtures.myshopify.com", {
      id: "draft-slip",
      name: "Draft packing slip",
      kind: "packing_slip",
      pageSize: "A4",
      state: "draft",
      source: "{}",
      revision: 1,
    });

    const result = await loadAdminPrintOptions({
      shop: "fixtures.myshopify.com",
      shops,
      workflows: workflowRepository,
      vault: CredentialVault.fromBase64(Buffer.alloc(32, 4).toString("base64")),
      baseUrl: "https://unused.example.invalid",
    });

    expect(result.linked).toBe(false);
    expect(result.documents).toEqual([
      expect.objectContaining({ id: "published-invoice", name: "Invoice" }),
    ]);
    expect(result.destinations).toEqual([]);
    expect(result.setupDestinationUrl).toBe("/app/settings");
    expect(result.renderExecutionPolicy).toBe("automatic");
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
      managedClientFactory: () =>
        ({
          printers: { list: async () => ({ data: [] }) },
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
