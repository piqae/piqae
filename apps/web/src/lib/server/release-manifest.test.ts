import { describe, expect, it } from 'vitest';
import {
  combineReleaseManifests,
  detectClient,
  loadReleaseManifest,
  parseReleaseManifest,
  recommendedArtifact
} from './release-manifest';

const supportedManifest = {
  schemaVersion: 1,
  channel: 'stable',
  currentVersion: '1.2.3',
  updatedAt: '2026-07-29T10:00:00.000Z',
  releasesUrl: 'https://releases.piqae.test/',
  repositoryUrl: 'https://github.com/example/piqae',
  artifacts: [
    {
      id: 'macos-universal',
      platform: 'macos',
      title: 'macOS node',
      version: '1.2.3',
      fileName: 'Piqae-1.2.3.zip',
      architectures: ['arm64', 'x86_64'],
      minimumOs: 'macOS 13+',
      status: 'supported',
      statusReason: 'Release gates passed.',
      downloadUrl: 'https://releases.piqae.test/Piqae-1.2.3.zip',
      releaseUrl: 'https://releases.piqae.test/1.2.3',
      sha256: 'a'.repeat(64),
      checksumUrl: 'https://releases.piqae.test/Piqae-1.2.3.zip.sha256',
      signing: { status: 'verified', label: 'Developer ID and notarised' },
      notes: ['Universal app']
    }
  ],
  olderReleases: [
    {
      version: '1.2.2',
      publishedAt: '2026-07-01T10:00:00.000Z',
      status: 'supported',
      releaseUrl: 'https://releases.piqae.test/1.2.2',
      notes: ['Previous stable']
    }
  ]
};

describe('server-owned release manifest', () => {
  it('defaults to truthful preview and development claims without inventing downloads', () => {
    const manifest = loadReleaseManifest({});

    expect(manifest.channel).toBe('preview');
    expect(manifest.artifacts.map((artifact) => artifact.status)).toEqual([
      'development',
      'preview',
      'preview'
    ]);
    expect(manifest.artifacts.every((artifact) => artifact.downloadUrl === null)).toBe(true);
    expect(manifest.artifacts.every((artifact) => artifact.sha256 === null)).toBe(true);
    expect(manifest.artifacts.every((artifact) => artifact.signing.status === 'unsigned')).toBe(true);
  });

  it('accepts a complete signed supported artifact', () => {
    const parsed = parseReleaseManifest(supportedManifest);
    expect(parsed.artifacts[0]).toMatchObject({
      status: 'supported',
      sha256: 'a'.repeat(64),
      downloadUrl: 'https://releases.piqae.test/Piqae-1.2.3.zip'
    });
    expect(parsed.olderReleases).toHaveLength(1);
  });

  it('refuses supported claims without all release evidence', () => {
    const missingChecksum = structuredClone(supportedManifest);
    missingChecksum.artifacts.at(0)!.sha256 = null as never;
    expect(() => parseReleaseManifest(missingChecksum)).toThrow(/cannot be supported/);

    const unsigned = structuredClone(supportedManifest);
    unsigned.artifacts.at(0)!.signing.status = 'unsigned';
    expect(() => parseReleaseManifest(unsigned)).toThrow(/cannot be supported/);
  });

  it('allows checksummed unsigned downloads only on the preview channel', () => {
    const preview = structuredClone(supportedManifest);
    preview.channel = 'preview';
    preview.artifacts[0]!.status = 'preview';
    preview.artifacts[0]!.signing = { status: 'unsigned', label: 'Unsigned prerelease' };
    expect(parseReleaseManifest(preview).artifacts[0]!.downloadUrl).not.toBeNull();

    preview.channel = 'stable';
    expect(() => parseReleaseManifest(preview)).toThrow(/unsigned download outside/);
    preview.channel = 'preview';
    preview.artifacts[0]!.sha256 = null as never;
    expect(() => parseReleaseManifest(preview)).toThrow(/unsigned download outside/);
  });

  it('fills only platforms without stable downloads from preview', () => {
    const stable = parseReleaseManifest(supportedManifest);
    const previewInput = structuredClone(supportedManifest);
    previewInput.channel = 'preview';
    previewInput.currentVersion = '1.3.0';
    previewInput.artifacts = [
      {
        ...previewInput.artifacts[0]!,
        version: '1.3.0',
        status: 'preview',
        downloadUrl: 'https://releases.piqae.test/Piqae-1.3.0.zip'
      },
      {
        ...previewInput.artifacts[0]!,
        id: 'windows-x86_64',
        platform: 'windows',
        title: 'Windows node',
        version: '1.3.0',
        status: 'preview',
        downloadUrl: 'https://releases.piqae.test/Piqae-1.3.0.exe',
        signing: { status: 'unsigned', label: 'Unsigned prerelease' }
      }
    ];
    const combined = combineReleaseManifests(stable, parseReleaseManifest(previewInput));

    expect(combined?.artifacts.map(({ platform, version }) => [platform, version])).toEqual([
      ['macos', '1.2.3'],
      ['windows', '1.3.0']
    ]);
    expect(combined?.channel).toBe('stable');
  });

  it('replaces a non-downloadable stable placeholder with a preview artifact', () => {
    const stableInput = structuredClone(supportedManifest);
    stableInput.artifacts[0]!.downloadUrl = null as never;
    stableInput.artifacts[0]!.sha256 = null as never;
    stableInput.artifacts[0]!.fileName = null as never;
    stableInput.artifacts[0]!.status = 'unavailable';
    const stable = parseReleaseManifest(stableInput);
    const previewInput = structuredClone(supportedManifest);
    previewInput.channel = 'preview';
    previewInput.artifacts[0]!.status = 'preview';

    expect(combineReleaseManifests(stable, parseReleaseManifest(previewInput))?.artifacts)
      .toHaveLength(1);
    expect(combineReleaseManifests(stable, parseReleaseManifest(previewInput))?.artifacts[0]!.status)
      .toBe('preview');
  });

  it('rejects insecure, credentialed, and fragmented release URLs', () => {
    for (const releaseUrl of [
      'http://releases.piqae.test/1.2.3',
      'https://user:secret@releases.piqae.test/1.2.3',
      'https://releases.piqae.test/1.2.3#mutable'
    ]) {
      const manifest = structuredClone(supportedManifest);
      manifest.artifacts.at(0)!.releaseUrl = releaseUrl;
      expect(() => parseReleaseManifest(manifest)).toThrow(/HTTPS URL/);
    }
  });

  it('detects common desktop platforms without treating mobile devices as printer nodes', () => {
    const mac = detectClient(
      new Headers({
        'sec-ch-ua-platform': '"macOS"',
        'user-agent': 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)'
      })
    );
    expect(mac).toMatchObject({ platform: 'macos', architecture: null, label: 'this Mac' });

    expect(
      detectClient(new Headers({ 'user-agent': 'Mozilla/5.0 (Windows NT 10.0; Win64; x64)' }))
    ).toMatchObject({ platform: 'windows', architecture: 'x86_64' });
    expect(
      detectClient(new Headers({ 'user-agent': 'Mozilla/5.0 (iPhone; CPU iPhone OS 18_0)' }))
    ).toMatchObject({ platform: 'unknown', architecture: null });
  });

  it('recommends the matching platform and architecture', () => {
    const manifest = loadReleaseManifest({});
    expect(
      recommendedArtifact(manifest, {
        platform: 'windows',
        architecture: 'x86_64',
        label: 'this Windows computer'
      })
    ).toBe('windows-x86_64');
    expect(
      recommendedArtifact(manifest, {
        platform: 'unknown',
        architecture: null,
        label: 'your printer computer'
      })
    ).toBeNull();
  });
});
