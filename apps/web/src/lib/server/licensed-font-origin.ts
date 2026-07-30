import { env as privateEnv } from '$env/dynamic/private';
import { GetObjectCommand, S3Client } from '@aws-sdk/client-s3';
import { releaseOriginConfig, type ReleaseOriginConfig } from './release-origin';

const licensedFontAssets = new Set([
  'exact-black.woff2',
  'exact-bold.woff2',
  'exact-italic.woff2',
  'exact-light.woff2',
  'exact-medium.woff2',
  'exact-regular.woff2',
  'exact-xlight.woff2'
]);
const maximumFontBytes = 64 * 1024;
const fontReadTimeoutMilliseconds = 2_000;

export interface LicensedFont {
  bytes: Uint8Array;
}

export function licensedFontObjectKey(asset: string): string | null {
  return licensedFontAssets.has(asset) ? `webfonts/exact/${asset}` : null;
}

export async function publishedLicensedFont(
  asset: string,
  environment: Record<string, string | undefined> = privateEnv
): Promise<LicensedFont | null> {
  const config = releaseOriginConfig(environment);
  const key = licensedFontObjectKey(asset);
  if (!config || !key) return null;

  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), fontReadTimeoutMilliseconds);
  try {
    const response = await fontS3Client(config).send(
      new GetObjectCommand({
        Bucket: config.bucket,
        Key: key
      }),
      { abortSignal: controller.signal }
    );
    if (
      response.ContentLength !== undefined &&
      response.ContentLength > maximumFontBytes
    ) {
      controller.abort();
      await response.Body?.transformToWebStream().cancel().catch(() => undefined);
      return null;
    }
    if (!response.Body) return null;
    const bytes = await readBoundedFontStream(response.Body.transformToWebStream());
    if (!bytes) return null;
    return { bytes };
  } catch {
    return null;
  } finally {
    clearTimeout(timeout);
  }
}

export async function readBoundedFontStream(
  stream: ReadableStream<Uint8Array>,
  maximumBytes = maximumFontBytes
): Promise<Uint8Array | null> {
  const reader = stream.getReader();
  const chunks: Uint8Array[] = [];
  let total = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      total += value.byteLength;
      if (total > maximumBytes) {
        await reader.cancel('licensed font exceeds the configured limit').catch(() => undefined);
        return null;
      }
      chunks.push(value);
    }
  } finally {
    reader.releaseLock();
  }
  if (total === 0) return null;
  const bytes = new Uint8Array(total);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return bytes;
}

function fontS3Client(config: ReleaseOriginConfig): S3Client {
  return new S3Client({
    endpoint: config.endpoint,
    region: config.region,
    forcePathStyle: config.forcePathStyle,
    credentials: {
      accessKeyId: config.accessKeyId,
      secretAccessKey: config.secretAccessKey
    }
  });
}
