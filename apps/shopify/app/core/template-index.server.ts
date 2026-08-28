import type {
  MerchantSettings,
  MerchantTemplate,
  WorkflowRepository,
} from "./workflows.server";
import { templateDigest } from "./template-digest.server";

const ACTIVE_SYSTEM_TEMPLATE_KEYS = new Set([
  "invoice",
  "packing-slip",
  "receipt",
  "credit-note",
]);

export function isActiveTemplate(template: MerchantTemplate): boolean {
  try {
    const key = (
      JSON.parse(template.published?.source ?? template.source) as {
        system?: { key?: unknown };
      }
    ).system?.key;
    return typeof key !== "string" || ACTIVE_SYSTEM_TEMPLATE_KEYS.has(key);
  } catch {
    return true;
  }
}

type AdminGraphql = (
  query: string,
  options?: { variables?: Record<string, unknown> },
) => Promise<Response>;
export type TemplateIndex = {
  schema: "piqae.shopify-template-index/v1";
  version: number;
  defaultDocumentId: string | null;
  defaultDestinationId: string | null;
  digest: string;
  documents: Array<{
    id: string;
    name: string;
    kind: string;
    pageSize: string;
    revision: number;
    designTargetId: string | null;
    designSpecificationRevision: string | null;
    digest: string;
  }>;
};

export function buildTemplateIndex(
  templates: MerchantTemplate[],
  settings: MerchantSettings,
): TemplateIndex {
  const documents = templates
    .filter((value) => value.published && isActiveTemplate(value))
    .slice(0, 50)
    .map((value) => {
      const published = value.published!;
      return {
        id: value.id,
        name: published.name,
        kind: published.kind,
        pageSize: published.pageSize,
        revision: published.revision,
        designTargetId: published.designTargetId,
        designSpecificationRevision: published.designSpecificationRevision,
        digest: templateDigest(published.source),
      };
    });
  const digest = templateDigest(JSON.stringify(documents));
  return {
    schema: "piqae.shopify-template-index/v1",
    version: Math.max(0, ...documents.map((value) => value.revision)),
    defaultDocumentId: settings.defaultTemplateId || null,
    defaultDestinationId: settings.defaultPrinterId || null,
    digest,
    documents,
  };
}

export async function syncTemplateIndex(
  admin: { graphql: AdminGraphql },
  repository: WorkflowRepository,
  shop: string,
): Promise<TemplateIndex> {
  const index = buildTemplateIndex(
    await repository.listTemplates(shop),
    await repository.getSettings(shop),
  );
  const ownerResponse = await admin.graphql(
    `#graphql\nquery PiqaeAppInstallation { currentAppInstallation { id } }`,
  );
  const ownerPayload = (await ownerResponse.json()) as {
    data?: { currentAppInstallation?: { id?: string } };
    errors?: unknown;
  };
  const ownerId = ownerPayload.data?.currentAppInstallation?.id;
  if (!ownerResponse.ok || !ownerId || ownerPayload.errors)
    throw new Error(
      "Could not resolve the Shopify app installation for template index sync",
    );
  const response = await admin.graphql(
    `#graphql
    mutation PiqaeTemplateIndex($metafields: [MetafieldsSetInput!]!) {
      metafieldsSet(metafields: $metafields) { userErrors { field message code } }
    }`,
    {
      variables: {
        metafields: [
          {
            ownerId,
            namespace: "piqae",
            key: "template_index",
            type: "json",
            value: JSON.stringify(index),
          },
        ],
      },
    },
  );
  const payload = (await response.json()) as {
    data?: { metafieldsSet?: { userErrors?: Array<{ message: string }> } };
    errors?: unknown;
  };
  const errors = payload.data?.metafieldsSet?.userErrors ?? [];
  if (!response.ok || payload.errors || errors.length)
    throw new Error(errors[0]?.message ?? "Shopify template index sync failed");
  return index;
}

export async function seedStarterTemplates(
  repository: WorkflowRepository,
  shop: string,
): Promise<void> {
  const existing = await repository.listTemplates(shop);
  const existingSystemTemplates = new Map(
    existing.flatMap((value) => {
      try {
        const parsed = JSON.parse(value.source) as {
          system?: { key?: unknown };
        };
        return typeof parsed.system?.key === "string"
          ? [[parsed.system.key, value] as const]
          : [];
      } catch {
        return [];
      }
    }),
  );
  const { starterTemplates } = await import("./starter-templates");
  for (const [position, starter] of starterTemplates.entries()) {
    const current = existingSystemTemplates.get(starter.id);
    if (current?.source === starter.source) continue;
    await repository.saveTemplate(shop, {
      id:
        current?.id ??
        `00000000-0000-4000-8000-${String(position + 1).padStart(12, "0")}`,
      name: starter.name,
      kind: starter.kind,
      pageSize: starter.pageSize,
      state: "published",
      source: starter.source,
      revision: 1,
      expectedDraftRevision: current?.draftRevision ?? null,
    });
  }
}
