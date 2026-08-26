import { PiqaeClient } from "@piqae/sdk";

import type { CredentialVault } from "./credentials.server";
import type { ShopRepository } from "./model";
import type { ShopLink } from "./model";
import { normalizeShopDomain } from "./model";
import type { WorkflowRepository } from "./workflows.server";
import type { RenderExecutionPolicy } from "./workflows.server";

/**
 * Selects the account's provisioned starter revision instead of a merchant
 * template. Printing must still work before the merchant has published a
 * document of their own, so this id is resolved against the shop link rather
 * than the local template store.
 */
export const ACCOUNT_DEFAULT_DOCUMENT_ID = "account-default";

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
    nodeRendering: {
      supported: boolean;
      ready: boolean;
      cacheState: "ready" | "warming" | "missing" | "unknown";
      reason: string;
    };
  }>;
  manageDocumentsUrl: string;
  setupDestinationUrl: string;
  destinationError?: string;
  renderExecutionPolicy: RenderExecutionPolicy;
};

export async function loadAdminPrintOptions(input: {
  shop: string;
  shops: ShopRepository;
  workflows: WorkflowRepository;
  vault: CredentialVault;
  baseUrl: string;
  managedClientFactory?: (link: ShopLink) => PiqaeClient;
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
      setupDestinationUrl: "/app/printers",
      renderExecutionPolicy: settings.renderExecutionPolicy,
    };
  }

  const client =
    link.entitlementMode === "shopify_child"
      ? input.managedClientFactory?.(link)
      : new PiqaeClient({
          baseUrl: input.baseUrl,
          accessToken: () => input.vault.open(link.encryptedCredential, shop),
        });
  if (!client) throw new Error("PIQAE_MANAGED_ACCOUNT_NOT_READY");
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
      // Readiness is deliberately fail-closed until the platform's signed
      // per-destination renderer decision is available. Printer online state
      // alone does not prove renderer ABI or resource-cache compatibility.
      nodeRendering: {
        supported: false,
        ready: false,
        cacheState: "unknown",
        reason: "Node renderer capability has not been verified",
      },
    }));
  } catch {
    // A printer-list outage must not hide document preview or PDF download.
    destinationError = "Printer status is temporarily unavailable";
  }
  // A shop with no published document of its own still prints, using the
  // starter revision provisioned when the managed account was created. The
  // previous fallback exposed the revision id as if it were a merchant
  // template id, which no lookup could ever resolve.
  const accountDefault = link.templateRevisionId
    ? [
        {
          id: ACCOUNT_DEFAULT_DOCUMENT_ID,
          name: "Default document",
          kind: "document",
          isDefault: true,
        },
      ]
    : [];
  return {
    linked: true,
    documents: documents.length > 0 ? documents : accountDefault,
    destinations,
    manageDocumentsUrl: "/app/templates",
    setupDestinationUrl: "/app/printers",
    destinationError,
    renderExecutionPolicy: settings.renderExecutionPolicy,
  };
}
