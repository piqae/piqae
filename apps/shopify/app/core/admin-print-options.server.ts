import { PiqaeClient } from "@piqae/sdk";

import type { CredentialVault } from "./credentials.server";
import type { ShopRepository } from "./model";
import { normalizeShopDomain } from "./model";
import type { WorkflowRepository } from "./workflows.server";

export type AdminPrintOptions = {
  linked: boolean;
  documents: Array<{
    id: string;
    name: string;
    kind: string;
    isDefault: boolean;
  }>;
  destinations: Array<{
    id: string;
    name: string;
    state: string;
    eligible: boolean;
    isDefault: boolean;
  }>;
  manageDocumentsUrl: string;
  setupDestinationUrl: string;
  destinationError?: string;
};

export async function loadAdminPrintOptions(input: {
  shop: string;
  shops: ShopRepository;
  workflows: WorkflowRepository;
  vault: CredentialVault;
  baseUrl: string;
}): Promise<AdminPrintOptions> {
  const shop = normalizeShopDomain(input.shop);
  const [link, settings, templates] = await Promise.all([
    input.shops.get(shop),
    input.workflows.getSettings(shop),
    input.workflows.listTemplates(shop),
  ]);
  const published = templates.filter(
    (template) => template.state === "published",
  );
  const documents = published.map((template) => ({
    id: template.id,
    name: template.name,
    kind: template.kind,
    isDefault: template.id === settings.defaultTemplateId,
  }));

  if (!link) {
    return {
      linked: false,
      documents,
      destinations: [],
      manageDocumentsUrl: "/app/templates",
      setupDestinationUrl: "/app/settings",
    };
  }

  const token = input.vault.open(link.encryptedCredential, shop);
  const client = new PiqaeClient({
    baseUrl: input.baseUrl,
    accessToken: () => token,
  });
  let destinations: AdminPrintOptions["destinations"] = [];
  let destinationError: string | undefined;
  try {
    const page = await client.printers.list({ limit: 100 });
    destinations = page.data.map((printer) => ({
      id: printer.id,
      name: printer.name,
      state: printer.state,
      eligible: printer.state === "online",
      isDefault: printer.id === settings.defaultPrinterId,
    }));
  } catch {
    // A printer-list outage must not hide document preview or PDF download.
    destinationError = "Printer status is temporarily unavailable";
  }
  return {
    linked: true,
    documents:
      documents.length > 0
        ? documents
        : [
            {
              id: link.templateRevisionId,
              name: "Default document",
              kind: "document",
              isDefault: true,
            },
          ],
    destinations,
    manageDocumentsUrl: "/app/templates",
    setupDestinationUrl: "/app/settings",
    destinationError,
  };
}
