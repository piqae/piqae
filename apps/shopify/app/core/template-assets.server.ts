import { createHash } from "node:crypto";
import { lookup } from "node:dns/promises";
import { isIP } from "node:net";
import sharp from "sharp";
import {
  ASSET_LIMITS,
  validateAssets,
  type ExternalAsset,
  type TemplateAssetSourceMediaType,
} from "./template-model";

const cache = new Map<string, { expiresAt: number; bytes: Uint8Array }>();
const MAX_CACHE_ENTRIES = 256;
const SOURCE_IMAGE_LIMIT = 12 * 1024 * 1024;
const SOURCE_MEDIA_TYPES = new Set<TemplateAssetSourceMediaType>([
  "image/jpeg",
  "image/png",
  "image/webp",
  "image/gif",
  "image/svg+xml",
]);
const SOURCE_IMAGE_ACCEPT =
  "image/jpeg,image/png,image/webp,image/gif,image/svg+xml";

export async function fetchTemplateAsset(
  asset: ExternalAsset,
): Promise<Uint8Array> {
  validateAssets([asset]);
  const cached = cache.get(asset.digest);
  if (cached && cached.expiresAt > Date.now()) return cached.bytes.slice();
  if (!asset.sourceUrl)
    throw new Error(
      "Asset has no ingestion source; use the Piqae content-addressed asset store",
    );
  const source = new URL(asset.sourceUrl);
  if (source.origin !== "https://cdn.shopify.com")
    throw new Error("Asset ingestion requires the exact Shopify CDN origin");
  await assertPublicHost(source.hostname);
  const response = await fetch(asset.sourceUrl, {
    redirect: "error",
    signal: AbortSignal.timeout(5_000),
    headers: { accept: SOURCE_IMAGE_ACCEPT },
  });
  const responseMediaType = sourceMediaType(response);
  if (!response.ok || !responseMediaType)
    throw new Error(
      "Published template asset is unavailable or has changed type",
    );
  if (
    asset.sourceMediaType !== undefined &&
    responseMediaType !== asset.sourceMediaType
  )
    throw new Error("Published template asset source type changed");
  const sourceBytes = await readBoundedResponse(response, SOURCE_IMAGE_LIMIT);
  const bytes = asset.sourceTransform
    ? await convertTemplateImageToJpeg(sourceBytes, responseMediaType)
    : sourceBytes;
  if (bytes.byteLength !== asset.bytes)
    throw new Error("Published template asset length does not match its pin");
  if (createHash("sha256").update(bytes).digest("hex") !== asset.digest)
    throw new Error("Published template asset digest does not match its pin");
  pruneCache();
  cache.set(asset.digest, {
    expiresAt: Date.now() + ASSET_LIMITS.cacheSeconds * 1_000,
    bytes,
  });
  return bytes.slice();
}

export async function pinShopifyTemplateJpeg(
  sourceUrl: string,
  id: string,
  declaredSourceMediaType?: string,
): Promise<ExternalAsset> {
  const source = new URL(sourceUrl);
  if (source.origin !== "https://cdn.shopify.com")
    throw new Error("Template assets require the exact Shopify CDN origin");
  await assertPublicHost(source.hostname);
  const response = await fetch(source, {
    redirect: "error",
    signal: AbortSignal.timeout(5_000),
    headers: { accept: SOURCE_IMAGE_ACCEPT },
  });
  const responseMediaType = sourceMediaType(response);
  if (!response.ok || !responseMediaType)
    throw new Error("The selected Shopify file is not a supported image");
  if (
    declaredSourceMediaType &&
    normalizeSourceMediaType(declaredSourceMediaType) !== responseMediaType
  )
    throw new Error("The selected Shopify image type changed while loading");
  const sourceBytes = await readBoundedResponse(response, SOURCE_IMAGE_LIMIT);
  const bytes = await convertTemplateImageToJpeg(
    sourceBytes,
    responseMediaType,
  );
  const length = bytes.byteLength;
  const digest = createHash("sha256").update(bytes).digest("hex");
  pruneCache();
  cache.set(digest, {
    expiresAt: Date.now() + ASSET_LIMITS.cacheSeconds * 1_000,
    bytes,
  });
  return {
    id,
    digest,
    mediaType: "image/jpeg",
    bytes: length,
    sourceUrl,
    sourceMediaType: responseMediaType,
    sourceTransform: "piqae-jpeg-v1",
  };
}

export async function convertTemplateImageToJpeg(
  input: Uint8Array,
  mediaType: TemplateAssetSourceMediaType,
): Promise<Uint8Array> {
  if (!input.byteLength) throw new Error("The selected Shopify image is empty");
  if (!SOURCE_MEDIA_TYPES.has(mediaType))
    throw new Error("The selected Shopify file is not a supported image");
  if (mediaType === "image/svg+xml") validateSafeSvg(input);
  try {
    const pipeline = sharp(input, {
      animated: false,
      density: 144,
      failOn: "warning",
      limitInputPixels: 40_000_000,
    })
      .rotate()
      .resize({
        width: 4096,
        height: 4096,
        fit: "inside",
        withoutEnlargement: true,
      })
      .flatten({ background: "#ffffff" });
    let output = await pipeline
      .clone()
      .jpeg({ quality: 90, chromaSubsampling: "4:4:4" })
      .toBuffer();
    if (output.byteLength > ASSET_LIMITS.maxBytes)
      output = await pipeline
        .clone()
        .resize({ width: 3000, height: 3000, fit: "inside" })
        .jpeg({ quality: 82, chromaSubsampling: "4:2:0" })
        .toBuffer();
    if (output.byteLength > ASSET_LIMITS.maxBytes)
      throw new Error("The converted image exceeds 2 MiB");
    return new Uint8Array(output);
  } catch (error) {
    if (error instanceof Error && error.message.includes("exceeds 2 MiB"))
      throw error;
    throw new Error(
      "The selected Shopify image could not be prepared for print",
    );
  }
}

function sourceMediaType(
  response: Response,
): TemplateAssetSourceMediaType | null {
  return normalizeSourceMediaType(
    response.headers.get("content-type")?.split(";", 1)[0] ?? "",
  );
}

function normalizeSourceMediaType(
  value: string,
): TemplateAssetSourceMediaType | null {
  const normalized = value.trim().toLowerCase() as TemplateAssetSourceMediaType;
  return SOURCE_MEDIA_TYPES.has(normalized) ? normalized : null;
}

async function readBoundedResponse(
  response: Response,
  limit: number,
): Promise<Uint8Array> {
  const announced = Number(response.headers.get("content-length") ?? 0);
  if (announced > limit) throw new Error("The selected image exceeds 12 MiB");
  const reader = response.body?.getReader();
  if (!reader) throw new Error("The selected Shopify image has no body");
  const chunks: Uint8Array[] = [];
  let length = 0;
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    length += value.byteLength;
    if (length > limit) {
      await reader.cancel();
      throw new Error("The selected image exceeds 12 MiB");
    }
    chunks.push(value);
  }
  const bytes = new Uint8Array(length);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return bytes;
}

function validateSafeSvg(input: Uint8Array): void {
  const svg = new TextDecoder().decode(input);
  if (
    !/<svg(?:\s|>)/i.test(svg) ||
    /<!doctype|<!entity|<(?:script|foreignObject|iframe|object|embed)\b/i.test(
      svg,
    ) ||
    /(?:href|xlink:href)\s*=\s*["']\s*(?:https?:|file:|ftp:|\/\/)/i.test(svg) ||
    /url\(\s*["']?\s*(?:https?:|file:|ftp:|\/\/)/i.test(svg)
  )
    throw new Error("The selected SVG contains unsupported external content");
}

function pruneCache(): void {
  const now = Date.now();
  for (const [key, value] of cache)
    if (value.expiresAt <= now) cache.delete(key);
  while (cache.size >= MAX_CACHE_ENTRIES) {
    const oldest = cache.keys().next().value as string | undefined;
    if (!oldest) break;
    cache.delete(oldest);
  }
}

async function assertPublicHost(hostname: string): Promise<void> {
  const addresses = isIP(hostname)
    ? [{ address: hostname }]
    : await lookup(hostname, { all: true });
  if (
    !addresses.length ||
    addresses.some(({ address }) => isPrivateAddress(address))
  )
    throw new Error("Template assets cannot resolve to a private network");
}

function isPrivateAddress(address: string): boolean {
  const normalized = address.toLowerCase();
  return (
    normalized === "::1" ||
    normalized.startsWith("fe80:") ||
    normalized.startsWith("fc") ||
    normalized.startsWith("fd") ||
    /^127\./.test(normalized) ||
    /^10\./.test(normalized) ||
    /^192\.168\./.test(normalized) ||
    /^172\.(1[6-9]|2\d|3[01])\./.test(normalized) ||
    /^169\.254\./.test(normalized) ||
    normalized === "0.0.0.0"
  );
}
