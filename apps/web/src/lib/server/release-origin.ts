import { env as privateEnv } from '$env/dynamic/private';
import { GetObjectCommand, HeadObjectCommand, S3Client } from '@aws-sdk/client-s3';
import { getSignedUrl } from '@aws-sdk/s3-request-presigner';
import { productEnvironmentValue } from './product-env';
import {
  parseReleaseManifest,
  type ReleaseManifest
} from '$lib/server/release-manifest';

export type ReleaseChannel = 'stable' | 'preview';

export interface ReleaseOriginConfig {
  endpoint: string;
  accessKeyId: string;
  secretAccessKey: string;
  bucket: string;
  region: string;
  forcePathStyle: boolean;
}

const releaseAssetPattern =
  /^(?:appcast-(?:macos|windows)\.xml|piqae-[A-Za-z0-9][A-Za-z0-9._-]{0,160}\.(?:dmg|pkg|exe|zip|json|txt|sha256))$/;
const maximumManifestBytes = 128 * 1024;
const manifestReadTimeoutMilliseconds = 2_000;
const assetLookupTimeoutMilliseconds = 2_000;

export function releaseOriginConfig(
  environment: Record<string, string | undefined> = privateEnv
): ReleaseOriginConfig | null {
  const endpoint = safeHttpsUrl(
    productEnvironmentValue(environment, 'PIQAE_RELEASES_S3_ENDPOINT')
  );
  const accessKeyId = present(
    productEnvironmentValue(environment, 'PIQAE_RELEASES_S3_ACCESS_KEY_ID')
  );
  const secretAccessKey = present(
    productEnvironmentValue(environment, 'PIQAE_RELEASES_S3_SECRET_ACCESS_KEY')
  );
  const bucket = safeBucket(
    productEnvironmentValue(environment, 'PIQAE_RELEASES_S3_BUCKET')
  );
  const region = safeRegion(
    productEnvironmentValue(environment, 'PIQAE_RELEASES_S3_REGION')
  );
  if (!endpoint || !accessKeyId || !secretAccessKey || !bucket || !region) return null;
  return {
    endpoint,
    accessKeyId,
    secretAccessKey,
    bucket,
    region,
    forcePathStyle:
      productEnvironmentValue(environment, 'PIQAE_RELEASES_S3_VIRTUAL_HOSTED_STYLE') !==
      'true'
  };
}

export function releaseObjectKey(channel: string, asset: string): string | null {
  if (channel !== 'stable' && channel !== 'preview') return null;
  if (!releaseAssetPattern.test(asset)) return null;
  return `native/${channel}/${asset}`;
}

export async function signedReleaseAssetUrl(
  channel: ReleaseChannel,
  asset: string,
  environment: Record<string, string | undefined> = privateEnv
): Promise<string | null> {
  const config = releaseOriginConfig(environment);
  const key = releaseObjectKey(channel, asset);
  if (!config || !key) return null;

  const client = releaseS3Client(config);
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), assetLookupTimeoutMilliseconds);
  try {
    await client.send(
      new HeadObjectCommand({
        Bucket: config.bucket,
        Key: key
      }),
      { abortSignal: controller.signal }
    );
  } catch {
    return null;
  } finally {
    clearTimeout(timeout);
  }

  const inline = asset.endsWith('.xml') || asset.endsWith('.json') || asset.endsWith('.txt');
  const command = new GetObjectCommand({
    Bucket: config.bucket,
    Key: key,
    ResponseContentDisposition: `${inline ? 'inline' : 'attachment'}; filename="${asset}"`,
    ResponseContentType: contentType(asset)
  });
  return getSignedUrl(client, command, { expiresIn: 300 });
}

export async function publishedReleaseManifest(
  channel: ReleaseChannel = 'stable',
  environment: Record<string, string | undefined> = privateEnv
): Promise<ReleaseManifest | null> {
  const config = releaseOriginConfig(environment);
  if (!config) return null;

  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), manifestReadTimeoutMilliseconds);
  try {
    const response = await releaseS3Client(config).send(
      new GetObjectCommand({
        Bucket: config.bucket,
        Key: `native/${channel}/manifest.json`
      }),
      { abortSignal: controller.signal }
    );
    if (
      response.ContentLength !== undefined &&
      response.ContentLength > maximumManifestBytes
    ) {
      return null;
    }
    const raw = await response.Body?.transformToString();
    if (!raw || new TextEncoder().encode(raw).byteLength > maximumManifestBytes) return null;
    return parseReleaseManifest(JSON.parse(raw) as unknown);
  } catch {
    return null;
  } finally {
    clearTimeout(timeout);
  }
}

function releaseS3Client(config: ReleaseOriginConfig): S3Client {
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

function contentType(asset: string): string {
  if (asset.endsWith('.xml')) return 'application/xml';
  if (asset.endsWith('.json')) return 'application/json';
  if (asset.endsWith('.txt') || asset.endsWith('.sha256')) return 'text/plain; charset=utf-8';
  if (asset.endsWith('.zip')) return 'application/zip';
  if (asset.endsWith('.pkg')) return 'application/vnd.apple.installer+xml';
  return 'application/octet-stream';
}

function present(value: string | undefined): string | null {
  const result = value?.trim();
  return result ? result : null;
}

function safeHttpsUrl(value: string | undefined): string | null {
  const candidate = present(value);
  if (!candidate) return null;
  try {
    const parsed = new URL(candidate);
    if (
      parsed.protocol !== 'https:' ||
      !parsed.hostname ||
      parsed.username ||
      parsed.password ||
      parsed.search ||
      parsed.hash
    ) {
      return null;
    }
    return parsed.toString().replace(/\/$/, '');
  } catch {
    return null;
  }
}

function safeBucket(value: string | undefined): string | null {
  const candidate = present(value);
  return candidate && /^[a-z0-9][a-z0-9.-]{1,126}[a-z0-9]$/.test(candidate) ? candidate : null;
}

function safeRegion(value: string | undefined): string | null {
  const candidate = present(value);
  return candidate && /^[a-z0-9][a-z0-9-]{0,31}$/.test(candidate) ? candidate : null;
}
