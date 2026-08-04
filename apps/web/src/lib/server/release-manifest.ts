import { env as privateEnv } from '$env/dynamic/private';
import { productEnvironmentValue } from './product-env';

export type ReleasePlatform = 'windows' | 'macos' | 'linux';
export type ReleaseStatus = 'supported' | 'preview' | 'development' | 'unavailable';
export type SigningStatus = 'verified' | 'unsigned' | 'not_applicable';

export interface ReleaseArtifact {
  id: string;
  platform: ReleasePlatform;
  title: string;
  version: string;
  fileName: string | null;
  architectures: string[];
  minimumOs: string;
  status: ReleaseStatus;
  statusReason: string;
  downloadUrl: string | null;
  releaseUrl: string;
  sha256: string | null;
  checksumUrl: string | null;
  signing: {
    status: SigningStatus;
    label: string;
  };
  notes: string[];
}

export interface OlderRelease {
  version: string;
  publishedAt: string;
  status: ReleaseStatus;
  releaseUrl: string;
  notes: string[];
}

export interface ReleaseManifest {
  schemaVersion: 1;
  channel: 'stable' | 'preview';
  currentVersion: string;
  updatedAt: string | null;
  artifacts: ReleaseArtifact[];
  olderReleases: OlderRelease[];
  releasesUrl: string;
  repositoryUrl: string;
}

export interface DetectedClient {
  platform: ReleasePlatform | 'unknown';
  architecture: 'x86_64' | 'arm64' | null;
  label: string;
}

const maximumManifestBytes = 128 * 1024;
const identifierPattern = /^[a-z0-9][a-z0-9_-]{0,63}$/;
const versionPattern = /^[0-9]+(?:\.[0-9]+){2}(?:[-+][0-9A-Za-z.-]+)?$/;
const sha256Pattern = /^[a-f0-9]{64}$/;
const releaseStatuses = ['supported', 'preview', 'development', 'unavailable'] as const;
const signingStatuses = ['verified', 'unsigned', 'not_applicable'] as const;
const platforms = ['windows', 'macos', 'linux'] as const;

const builtInManifest: ReleaseManifest = {
  schemaVersion: 1,
  channel: 'preview',
  currentVersion: '0.1.0',
  updatedAt: null,
  releasesUrl: 'https://github.com/piqae/piqae/releases',
  repositoryUrl: 'https://github.com/piqae/piqae',
  artifacts: [
    {
      id: 'windows-x86_64',
      platform: 'windows',
      title: 'Windows node',
      version: '0.1.0',
      fileName: null,
      architectures: ['x86_64'],
      minimumOs: 'Windows 10 or 11, 64-bit',
      status: 'development',
      statusReason: 'Physical PDF, RAW, driver, and clean-install release gates have not run.',
      downloadUrl: null,
      releaseUrl: 'https://github.com/piqae/piqae/releases',
      sha256: null,
      checksumUrl: null,
      signing: { status: 'unsigned', label: 'Unsigned development package' },
      notes: [
        'Runs per user in the interactive printer-driver session.',
        'Advanced driver profiles use the manufacturer’s native DocumentProperties interface.'
      ]
    },
    {
      id: 'macos-universal',
      platform: 'macos',
      title: 'macOS node',
      version: '0.1.0',
      fileName: null,
      architectures: ['arm64', 'x86_64'],
      minimumOs: 'macOS 13 or newer',
      status: 'preview',
      statusReason: 'Packaging, notarisation, and the physical printer matrix are not certified.',
      downloadUrl: null,
      releaseUrl: 'https://github.com/piqae/piqae/releases',
      sha256: null,
      checksumUrl: null,
      signing: { status: 'unsigned', label: 'Unsigned Preview build' },
      notes: [
        'Native menu app with PrintCore profile capture and headless replay.',
        'Apple silicon and Intel are built from the same source package.'
      ]
    },
    {
      id: 'linux-x86_64',
      platform: 'linux',
      title: 'Linux node',
      version: '0.1.0',
      fileName: null,
      architectures: ['x86_64'],
      minimumOs: 'Ubuntu 22.04/24.04 or Debian 12',
      status: 'preview',
      statusReason: 'Distribution packaging and physical printer gates have not run.',
      downloadUrl: null,
      releaseUrl: 'https://github.com/piqae/piqae/releases',
      sha256: null,
      checksumUrl: null,
      signing: { status: 'unsigned', label: 'Unsigned Preview archive' },
      notes: [
        'Headless CUPS node for self-hosted and low-resource installations.',
        'ARM Linux remains a source build until low-power hardware gates run.'
      ]
    }
  ],
  olderReleases: []
};

export function loadReleaseManifest(
  environment: Record<string, string | undefined> = privateEnv
): ReleaseManifest {
  const raw = productEnvironmentValue(environment, 'PIQAE_RELEASE_MANIFEST_JSON');
  if (raw === undefined || raw.trim() === '') return structuredClone(builtInManifest);
  if (new TextEncoder().encode(raw).byteLength > maximumManifestBytes) {
    throw new Error('PIQAE_RELEASE_MANIFEST_JSON exceeds 128 KiB');
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    throw new Error('PIQAE_RELEASE_MANIFEST_JSON is not valid JSON');
  }
  return parseReleaseManifest(parsed);
}

export function parseReleaseManifest(value: unknown): ReleaseManifest {
  const root = record(value, 'manifest');
  if (root.schemaVersion !== 1) throw new Error('manifest.schemaVersion must be 1');
  const channel = oneOf(root.channel, ['stable', 'preview'] as const, 'manifest.channel');
  const artifacts = array(root.artifacts, 'manifest.artifacts').map((entry, index) =>
    parseArtifact(entry, index)
  );
  if (artifacts.length === 0 || artifacts.length > 24) {
    throw new Error('manifest.artifacts must contain between 1 and 24 entries');
  }
  const ids = new Set(artifacts.map((artifact) => artifact.id));
  if (ids.size !== artifacts.length) throw new Error('manifest artifact ids must be unique');

  return {
    schemaVersion: 1,
    channel,
    currentVersion: version(root.currentVersion, 'manifest.currentVersion'),
    updatedAt: nullableIsoDate(root.updatedAt, 'manifest.updatedAt'),
    artifacts,
    olderReleases: array(root.olderReleases, 'manifest.olderReleases')
      .slice(0, 20)
      .map((entry, index) => parseOlderRelease(entry, index)),
    releasesUrl: httpsUrl(root.releasesUrl, 'manifest.releasesUrl'),
    repositoryUrl: httpsUrl(root.repositoryUrl, 'manifest.repositoryUrl')
  };
}

export function detectClient(headers: Pick<Headers, 'get'>): DetectedClient {
  const hint = (headers.get('sec-ch-ua-platform') ?? '').replaceAll('"', '').toLowerCase();
  const userAgent = (headers.get('user-agent') ?? '').toLowerCase();
  const source = `${hint} ${userAgent}`;

  let platform: DetectedClient['platform'] = 'unknown';
  if (source.includes('windows')) platform = 'windows';
  else if (!source.includes('iphone') && !source.includes('ipad') && source.includes('mac')) {
    platform = 'macos';
  } else if (!source.includes('android') && source.includes('linux')) {
    platform = 'linux';
  }

  let architecture: DetectedClient['architecture'] = null;
  if (/(arm64|aarch64)/.test(source)) architecture = 'arm64';
  else if (/(x86_64|x64|win64|amd64)/.test(source)) architecture = 'x86_64';

  const label =
    platform === 'windows'
      ? 'this Windows computer'
      : platform === 'macos'
        ? 'this Mac'
        : platform === 'linux'
          ? 'this Linux computer'
          : 'your printer computer';
  return { platform, architecture, label };
}

export function recommendedArtifact(
  manifest: ReleaseManifest,
  detected: DetectedClient
): string | null {
  if (detected.platform === 'unknown') return null;
  const candidates = manifest.artifacts.filter((artifact) => artifact.platform === detected.platform);
  const architectureMatch = detected.architecture
    ? candidates.find((artifact) => artifact.architectures.includes(detected.architecture!))
    : null;
  return (architectureMatch ?? candidates[0])?.id ?? null;
}

function parseArtifact(value: unknown, index: number): ReleaseArtifact {
  const path = `manifest.artifacts[${index}]`;
  const item = record(value, path);
  const status = oneOf(item.status, releaseStatuses, `${path}.status`);
  const signingStatus = oneOf(
    record(item.signing, `${path}.signing`).status,
    signingStatuses,
    `${path}.signing.status`
  );
  const downloadUrl = nullableHttpsUrl(item.downloadUrl, `${path}.downloadUrl`);
  const sha256 = nullableSha256(item.sha256, `${path}.sha256`);
  if (
    status === 'supported' &&
    (!downloadUrl || !sha256 || signingStatus !== 'verified')
  ) {
    throw new Error(`${path} cannot be supported without a signed download and SHA-256`);
  }
  return {
    id: matchingText(item.id, identifierPattern, `${path}.id`),
    platform: oneOf(item.platform, platforms, `${path}.platform`),
    title: text(item.title, `${path}.title`, 80),
    version: version(item.version, `${path}.version`),
    fileName: nullableText(item.fileName, `${path}.fileName`, 180),
    architectures: stringArray(item.architectures, `${path}.architectures`, 6),
    minimumOs: text(item.minimumOs, `${path}.minimumOs`, 160),
    status,
    statusReason: text(item.statusReason, `${path}.statusReason`, 280),
    downloadUrl,
    releaseUrl: httpsUrl(item.releaseUrl, `${path}.releaseUrl`),
    sha256,
    checksumUrl: nullableHttpsUrl(item.checksumUrl, `${path}.checksumUrl`),
    signing: {
      status: signingStatus,
      label: text(record(item.signing, `${path}.signing`).label, `${path}.signing.label`, 120)
    },
    notes: stringArray(item.notes, `${path}.notes`, 8)
  };
}

function parseOlderRelease(value: unknown, index: number): OlderRelease {
  const path = `manifest.olderReleases[${index}]`;
  const item = record(value, path);
  return {
    version: version(item.version, `${path}.version`),
    publishedAt: isoDate(item.publishedAt, `${path}.publishedAt`),
    status: oneOf(item.status, releaseStatuses, `${path}.status`),
    releaseUrl: httpsUrl(item.releaseUrl, `${path}.releaseUrl`),
    notes: stringArray(item.notes, `${path}.notes`, 4)
  };
}

function record(value: unknown, path: string): Record<string, unknown> {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) {
    throw new Error(`${path} must be an object`);
  }
  return value as Record<string, unknown>;
}

function array(value: unknown, path: string): unknown[] {
  if (!Array.isArray(value)) throw new Error(`${path} must be an array`);
  return value;
}

function text(value: unknown, path: string, maximum = 200): string {
  if (typeof value !== 'string' || value.trim() === '' || value.length > maximum) {
    throw new Error(`${path} must be non-empty text no longer than ${maximum} characters`);
  }
  return value.trim();
}

function nullableText(value: unknown, path: string, maximum: number): string | null {
  return value === null ? null : text(value, path, maximum);
}

function matchingText(value: unknown, pattern: RegExp, path: string): string {
  const result = text(value, path);
  if (!pattern.test(result)) throw new Error(`${path} has an invalid format`);
  return result;
}

function version(value: unknown, path: string): string {
  return matchingText(value, versionPattern, path);
}

function oneOf<const T extends readonly string[]>(
  value: unknown,
  allowed: T,
  path: string
): T[number] {
  if (typeof value !== 'string' || !allowed.includes(value)) {
    throw new Error(`${path} must be one of ${allowed.join(', ')}`);
  }
  return value as T[number];
}

function httpsUrl(value: unknown, path: string): string {
  const result = text(value, path, 2048);
  let parsed: URL;
  try {
    parsed = new URL(result);
  } catch {
    throw new Error(`${path} must be an HTTPS URL`);
  }
  if (
    parsed.protocol !== 'https:' ||
    !parsed.hostname ||
    parsed.username ||
    parsed.password ||
    parsed.hash
  ) {
    throw new Error(`${path} must be an HTTPS URL without credentials or a fragment`);
  }
  return parsed.toString();
}

function nullableHttpsUrl(value: unknown, path: string): string | null {
  return value === null ? null : httpsUrl(value, path);
}

function nullableSha256(value: unknown, path: string): string | null {
  if (value === null) return null;
  return matchingText(value, sha256Pattern, path);
}

function stringArray(value: unknown, path: string, maximum: number): string[] {
  const items = array(value, path);
  if (items.length === 0 || items.length > maximum) {
    throw new Error(`${path} must contain between 1 and ${maximum} items`);
  }
  return items.map((item, index) => text(item, `${path}[${index}]`, 280));
}

function isoDate(value: unknown, path: string): string {
  const result = text(value, path, 40);
  if (Number.isNaN(Date.parse(result))) throw new Error(`${path} must be an ISO date`);
  return result;
}

function nullableIsoDate(value: unknown, path: string): string | null {
  return value === null ? null : isoDate(value, path);
}
