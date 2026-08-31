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
  selectTargetDestination,
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
    designSpecificationRevision: string | null;
    targetBindingStatus:
      | "ready"
      | "unbound"
      | "target_missing"
      | "unknown"
      | "document_invalid"
      | "revision_changed"
      | "media_incompatible";
    compatibilityKnown: boolean;
    compatibleTargetIds: string[];
    advisoryDestination: null | {
      printerName: string;
      profileName: string;
      mediaStatus: string;
      readinessStatus: string;
    };
  }>;
  targets: Array<
    ShopifyPrintTarget & {
      eligible: boolean;
      isDefault: boolean;
    }
  >;
  printers: Array<{
    id: string;
    name: string;
    state: string;
    targetIds: string[];
    isDefault: boolean;
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
  const published = templates.filter((template) => template.published);
  const parsedDocuments = published.map((template) => {
    let document: ReturnType<typeof parseTemplateEnvelope>["document"] | null =
      null;
    try {
      document = parseTemplateEnvelope(template.published!.source).document;
    } catch {
      // One damaged publication must not hide every printable document.
    }
    return { template, published: template.published!, document };
  });

  if (!link) {
    return {
      linked: false,
      documents: parsedDocuments.map(({ template, published, document }) => ({
        id: template.id,
        name: published.name,
        kind: published.kind,
        isDefault: template.id === settings.defaultTemplateId,
        designTargetId: published.designTargetId,
        designSpecificationRevision: published.designSpecificationRevision,
        targetBindingStatus: !document
          ? "document_invalid"
          : published.designTargetId
            ? "target_missing"
            : "unbound",
        compatibilityKnown: document !== null,
        compatibleTargetIds: [],
        advisoryDestination: null,
      })),
      targets: [],
      printers: [],
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
  let printers: AdminPrintOptions["printers"] = [];
  let destinationError: string | undefined;
  try {
    const loaded = await loadShopifyPrintTargets(client);
    if (loaded.partial)
      destinationError = "Some printer status is temporarily unavailable";
    targets = loaded.targets.map((target) => ({
      ...target,
      eligible: target.hasMediaCandidate,
      isDefault: parsedDocuments.some(
        ({ published }) => published.designTargetId === target.id,
      ),
    }));
  } catch {
    // A printer-list outage must not hide document preview or PDF download.
    destinationError = "Printer status is temporarily unavailable";
  }
  try {
    const inventory = await client.printers.list();
    printers = inventory.data.map((printer) => ({
      id: printer.id,
      name: printer.name,
      state: printer.state,
      targetIds: targets
        .filter((target) =>
          target.destinations.some(
            (destination) => destination.printerId === printer.id,
          ),
        )
        .map((target) => target.id),
      isDefault: printer.id === settings.defaultPrinterId,
    }));
  } catch {
    destinationError ??= "Printer status is temporarily unavailable";
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
          designSpecificationRevision: null,
          targetBindingStatus: "unbound" as const,
          compatibilityKnown: false,
          compatibleTargetIds: targets.map(({ id }) => id),
          advisoryDestination: null,
        },
      ]
    : [];
  return {
    linked: true,
    documents:
      parsedDocuments.length > 0
        ? parsedDocuments.map(({ template, published, document }) => {
            const target = published.designTargetId
              ? targets.find(({ id }) => id === published.designTargetId)
              : undefined;
            const revisionCurrent = Boolean(
              target &&
              published.designSpecificationRevision ===
                target.specificationRevision,
            );
            const advisoryDestination =
              target && document
                ? selectTargetDestination(target, document)
                : null;
            const mediaCompatible = advisoryDestination !== null;
            const targetBindingStatus = !document
              ? "document_invalid"
              : !published.designTargetId
                ? "unbound"
                : !target
                  ? destinationError
                    ? "unknown"
                    : "target_missing"
                  : !revisionCurrent
                    ? "revision_changed"
                    : !mediaCompatible
                      ? "media_incompatible"
                      : "ready";
            return {
              id: template.id,
              name: published.name,
              kind: published.kind,
              isDefault: template.id === settings.defaultTemplateId,
              designTargetId: published.designTargetId,
              designSpecificationRevision:
                published.designSpecificationRevision,
              targetBindingStatus,
              compatibilityKnown: document !== null,
              compatibleTargetIds:
                targetBindingStatus === "ready" && target ? [target.id] : [],
              advisoryDestination: advisoryDestination
                ? {
                    printerName: advisoryDestination.printerName,
                    profileName: advisoryDestination.profileName,
                    mediaStatus: advisoryDestination.mediaCompatibility.status,
                    readinessStatus: advisoryDestination.readinessStatus,
                  }
                : null,
            };
          })
        : accountDefault,
    targets,
    printers,
    manageDocumentsUrl: "/app/templates",
    setupDestinationUrl: "/app/printers",
    destinationError,
    renderExecutionPolicy: settings.renderExecutionPolicy,
  };
}
