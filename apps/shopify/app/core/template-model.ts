/** Shopify-owned authoring envelope. Piqae only receives `document`. */
import type {
  BusinessDocumentExpression,
  BusinessDocumentInline,
  BusinessDocumentNode,
  BusinessDocumentV1,
} from "@piqae/sdk";
export type Expression = BusinessDocumentExpression;
export type TextStyle = {
  bold?: boolean;
  italic?: boolean;
  underline?: boolean;
  font_size_pt?: number;
  align?: "left" | "center" | "right";
};
export type Inline = BusinessDocumentInline;
export type Block = BusinessDocumentNode;
export type BusinessDocument = BusinessDocumentV1;
export type TemplateEditorMode = "visual" | "liquid" | "source";
export type ExternalAsset = {
  id: string;
  digest: string;
  mediaType: "image/jpeg";
  bytes: number;
  sourceUrl?: string;
};
export const ASSET_LIMITS = {
  maxBytes: 2 * 1024 * 1024,
  maxAssets: 20,
  cacheSeconds: 3600,
} as const;
export type TemplateEnvelope = {
  schema: "piqae.shopify-business-template/v1";
  document: BusinessDocument;
  editor: {
    mode: TemplateEditorMode;
    liquid: string;
    roundTrip: "lossless";
    warnings: string[];
  };
  assets: ExternalAsset[];
  system?: { key: string; immutable: true };
  published?: {
    piqaeTemplateId: string;
    piqaeRevisionId: string;
    canonicalDigest: string;
  };
};

export function parseTemplateEnvelope(source: string): TemplateEnvelope {
  if (!source || new TextEncoder().encode(source).byteLength > 262_144)
    throw new Error("Document source is invalid");
  let value: unknown;
  try {
    value = JSON.parse(source);
  } catch {
    throw new Error("Document source must be valid JSON");
  }
  const envelope = value as TemplateEnvelope;
  if (envelope?.schema !== "piqae.shopify-business-template/v1")
    throw new Error(
      "Legacy templates are not supported; create a new business document",
    );
  validateBusinessDocument(envelope.document);
  if (
    !envelope.editor ||
    !["visual", "liquid", "source"].includes(envelope.editor.mode)
  )
    throw new Error("Document editor metadata is invalid");
  return envelope;
}
export function serializeTemplateEnvelope(value: TemplateEnvelope): string {
  validateAssets(value.assets);
  validateBusinessDocument(value.document);
  const source = JSON.stringify(value);
  if (new TextEncoder().encode(source).byteLength > 262_144)
    throw new Error("Document source exceeds 256 KiB");
  return source;
}
export function validateAssets(assets: ExternalAsset[]) {
  if (assets.length > ASSET_LIMITS.maxAssets)
    throw new Error("Too many assets");
  for (const asset of assets) {
    if (!asset.id || !/^[a-f0-9]{64}$/.test(asset.digest))
      throw new Error("Assets require an ID and SHA-256 digest");
    if (
      !Number.isInteger(asset.bytes) ||
      asset.bytes < 1 ||
      asset.bytes > ASSET_LIMITS.maxBytes
    )
      throw new Error("Asset size is invalid");
    if (asset.mediaType !== "image/jpeg")
      throw new Error("Asset type is unsupported");
    if (asset.sourceUrl) {
      const url = new URL(asset.sourceUrl);
      if (url.origin !== "https://cdn.shopify.com")
        throw new Error("Asset ingestion requires Shopify CDN HTTPS");
    }
  }
}
export function validateBusinessDocument(document: BusinessDocument): void {
  if (
    document?.format !== "piqae.business-document/v1" ||
    !Array.isArray(document.body)
  )
    throw new Error("Document must use piqae.business-document/v1");
  let count = 0;
  const walk = (blocks: Block[], depth: number) => {
    if (depth > 12) throw new Error("Document nesting exceeds 12 levels");
    for (const block of blocks) {
      if (++count > 2_000) throw new Error("Document exceeds 2,000 blocks");
      if ("children" in block) walk(block.children, depth + 1);
      if (block.type === "conditional") {
        walk(block.then, depth + 1);
        walk(block.else ?? [], depth + 1);
      }
    }
  };
  walk(
    [
      ...(document.header?.first ?? []),
      ...(document.header?.default ?? []),
      ...document.body,
      ...(document.footer?.default ?? []),
      ...(document.footer?.last ?? []),
    ],
    0,
  );
}
export function removeSystemOwnership(envelope: TemplateEnvelope) {
  delete envelope.system;
  return envelope;
}
