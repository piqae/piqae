import type {
  MerchantSettings,
  MerchantTemplate,
  WorkflowRepository,
} from "./workflows.server";
import { WorkflowConflictError } from "./workflows.server";
import { templateDigest } from "./template-digest.server";
import { parseTemplateEnvelope } from "./template-model";

const ACTIVE_SYSTEM_TEMPLATE_KEYS = new Set([
  "invoice",
  "packing-slip",
  "receipt",
  "credit-note",
]);
const SYSTEM_TEMPLATE_POSITIONS = new Map([
  ["invoice", 1],
  ["packing-slip", 2],
  ["receipt", 3],
  ["product-label", 4],
]);

export function systemTemplateId(key: string): string | null {
  const position = SYSTEM_TEMPLATE_POSITIONS.get(key);
  return position
    ? `00000000-0000-4000-8000-${String(position).padStart(12, "0")}`
    : null;
}

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
  const { starterTemplates } = await import("./starter-templates");
  for (const [position, starter] of starterTemplates.entries()) {
    const deterministicId = systemTemplateId(starter.id);
    if (
      !deterministicId ||
      position + 1 !== SYSTEM_TEMPLATE_POSITIONS.get(starter.id)
    )
      throw new Error(`Shopify starter position is invalid for ${starter.id}`);
    let saved = false;
    for (let attempt = 0; attempt < 8; attempt += 1) {
      const current =
        (await repository.getTemplate(shop, deterministicId)) ??
        findSystemTemplate(await repository.listTemplates(shop), starter.id);
      if (
        current?.published &&
        hasValidPublishedRevision(current.published.source, starter.id)
      ) {
        saved = true;
        break;
      }
      if (
        current?.source === starter.source &&
        current.published?.source === starter.source &&
        current.name === starter.name &&
        current.kind === starter.kind &&
        current.pageSize === starter.pageSize &&
        current.state === "published"
      ) {
        saved = true;
        break;
      }
      try {
        await repository.saveTemplate(shop, {
          id: current?.id ?? deterministicId,
          name: starter.name,
          kind: starter.kind,
          pageSize: starter.pageSize,
          state: "published",
          source: starter.source,
          revision: 1,
          designTargetId: null,
          designSpecificationRevision: null,
          expectedDraftRevision: current?.draftRevision ?? null,
        });
        saved = true;
        break;
      } catch (error) {
        if (!(error instanceof WorkflowConflictError)) throw error;
      }
    }
    if (!saved)
      throw new Error(`Could not seed Shopify starter template ${starter.id}`);
  }
}

export function findSystemTemplate(
  templates: MerchantTemplate[],
  key: string,
): MerchantTemplate | undefined {
  return templates.find((value) => {
    for (const source of [value.published?.source, value.source]) {
      if (!source) continue;
      try {
        if (
          (JSON.parse(source) as { system?: { key?: unknown } }).system?.key ===
          key
        )
          return true;
      } catch {
        // Try the other immutable/draft source before falling back to the ID.
      }
    }
    return false;
  });
}

function hasValidPublishedRevision(source: string, systemKey: string): boolean {
  try {
    const envelope = parseTemplateEnvelope(source);
    const published = envelope.published;
    return Boolean(
      envelope.system?.key === systemKey &&
      envelope.system.immutable === true &&
      published &&
      published.canonicalDigest ===
        templateDigest(JSON.stringify(envelope.document)),
    );
  } catch {
    return hasRecoverablePreContextPublication(source, systemKey);
  }
}

function hasRecoverablePreContextPublication(
  source: string,
  systemKey: string,
): boolean {
  try {
    const raw = JSON.parse(source) as Record<string, unknown> & {
      published?: Record<string, unknown>;
    };
    const published = raw.published;
    if (
      !published ||
      "piqaeAccountId" in published ||
      "piqaeEnvironmentId" in published ||
      !validPublicationId(published.piqaeTemplateId) ||
      !validPublicationId(published.piqaeRevisionId) ||
      typeof published.canonicalDigest !== "string" ||
      !/^[a-f0-9]{64}$/.test(published.canonicalDigest)
    )
      return false;
    const unowned = structuredClone(raw);
    delete unowned.published;
    const envelope = parseTemplateEnvelope(JSON.stringify(unowned));
    return (
      envelope.system?.key === systemKey &&
      envelope.system.immutable === true &&
      published.canonicalDigest ===
        templateDigest(JSON.stringify(envelope.document))
    );
  } catch {
    return false;
  }
}

function validPublicationId(value: unknown): value is string {
  return (
    typeof value === "string" &&
    value.length > 0 &&
    value.length <= 200 &&
    !/[\s\u0000-\u001f\u007f]/.test(value)
  );
}
