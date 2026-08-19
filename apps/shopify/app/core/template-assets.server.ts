import { createHash } from "node:crypto";
import { lookup } from "node:dns/promises";
import { isIP } from "node:net";
import {
  ASSET_LIMITS,
  validateAssets,
  type ExternalAsset,
} from "./template-model";

const cache = new Map<string, { expiresAt: number; bytes: Uint8Array }>();
const MAX_CACHE_ENTRIES = 256;

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
    headers: { accept: asset.mediaType },
  });
  if (
    !response.ok ||
    response.headers.get("content-type")?.split(";", 1)[0] !== asset.mediaType
  )
    throw new Error(
      "Published template asset is unavailable or has changed type",
    );
  const announced = Number(response.headers.get("content-length") ?? 0);
  if (
    announced &&
    (announced !== asset.bytes || announced > ASSET_LIMITS.maxBytes)
  )
    throw new Error("Published template asset length does not match its pin");
  const reader = response.body?.getReader();
  if (!reader) throw new Error("Published template asset has no body");
  const chunks: Uint8Array[] = [];
  let length = 0;
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    length += value.byteLength;
    if (length > ASSET_LIMITS.maxBytes || length > asset.bytes) {
      await reader.cancel();
      throw new Error("Published template asset exceeded its pinned size");
    }
    chunks.push(value);
  }
  if (length !== asset.bytes)
    throw new Error("Published template asset length does not match its pin");
  const bytes = new Uint8Array(length);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }
  if (createHash("sha256").update(bytes).digest("hex") !== asset.digest)
    throw new Error("Published template asset digest does not match its pin");
  pruneCache();
  cache.set(asset.digest, {
    expiresAt: Date.now() + ASSET_LIMITS.cacheSeconds * 1_000,
    bytes,
  });
  return bytes.slice();
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
