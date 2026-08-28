import { PiqaeClient } from "@piqae/sdk";

import type { CredentialVault } from "./credentials.server";
import type { ShopRepository } from "./model";
import type { ShopLink } from "./model";
import { normalizeShopDomain } from "./model";
import type { WorkflowRepository } from "./workflows.server";
import type { RenderExecutionPolicy } from "./workflows.server";
import { parseTemplateEnvelope } from "./template-model";
import { loadShopifyPrintTargets } from "./shopify-print-targets.server";
import {
  targetSupportsDocument,
  type ShopifyPrintTarget,
} from "./shopify-print-targets";

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
    designTargetId: string | null;
    compatibilityKnown: boolean;
    compatibleTargetIds: string[];
  }>;
  targets: Array<
    ShopifyPrintTarget & {
      eligible: boolean;
      isDefault: boolean;
      nodeRendering: {
        supported: boolean;
        ready: boolean;
        cacheState: "ready" | "warming" | "missing" | "unknown";
        reason: string;
      };
    }
  >;
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
  const parsedDocuments = published.map((template) => ({
    template,
    document: parseTemplateEnvelope(template.source).document,
  }));

  if (!link) {
    return {
      linked: false,
      documents: parsedDocuments.map(({ template }) => ({
        id: template.id,
        name: template.name,
        kind: template.kind,
        isDefault: template.id === settings.defaultTemplateId,
        designTargetId: template.designTargetId ?? null,
        compatibilityKnown: true,
        compatibleTargetIds: [],
      })),
      targets: [],
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
  let targets: AdminPrintOptions["targets"] = [];
  let destinationError: string | undefined;
  try {
    const loaded = await loadShopifyPrintTargets(client);
    targets = loaded.map((target) => ({
      ...target,
      eligible: target.ready,
      isDefault: parsedDocuments.some(
        ({ template }) => template.designTargetId === target.id,
      ),
      // Readiness is deliberately fail-closed until the platform's signed
      // per-destination renderer decision is available. Printer online state
      // alone does not prove renderer ABI or resource-cache compatibility.
      nodeRendering: {
        supported: false,
        ready: false,
        cacheState: "unknown",
        reason:
          "Node renderer capability is checked against each rendered document",
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
          designTargetId: null,
          compatibilityKnown: false,
          compatibleTargetIds: targets.map(({ id }) => id),
        },
      ]
    : [];
  return {
    linked: true,
    documents:
      parsedDocuments.length > 0
        ? parsedDocuments.map(({ template, document }) => ({
            id: template.id,
            name: template.name,
            kind: template.kind,
            isDefault: template.id === settings.defaultTemplateId,
            designTargetId: template.designTargetId ?? null,
            compatibilityKnown: true,
            compatibleTargetIds: targets
              .filter((target) => targetSupportsDocument(target, document))
              .map(({ id }) => id),
          }))
        : accountDefault,
    targets,
    manageDocumentsUrl: "/app/templates",
    setupDestinationUrl: "/app/printers",
    destinationError,
    renderExecutionPolicy: settings.renderExecutionPolicy,
  };
}
