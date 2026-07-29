import { describe, expect, it } from 'vitest';
import {
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
  releasesUrl: 'https://releases.spool.test/',
  repositoryUrl: 'https://github.com/example/spool',
  artifacts: [
    {
      id: 'macos-universal',
      platform: 'macos',
      title: 'macOS node',
      version: '1.2.3',
      fileName: 'Spool-1.2.3.zip',
      architectures: ['arm64', 'x86_64'],
      minimumOs: 'macOS 13+',
      status: 'supported',
      statusReason: 'Release gates passed.',
      downloadUrl: 'https://releases.spool.test/Spool-1.2.3.zip',
      releaseUrl: 'https://releases.spool.test/1.2.3',
      sha256: 'a'.repeat(64),
      checksumUrl: 'https://releases.spool.test/Spool-1.2.3.zip.sha256',
      signing: { status: 'verified', label: 'Developer ID and notarised' },
      notes: ['Universal app']
    }
  ],
  olderReleases: [
    {
      version: '1.2.2',
      publishedAt: '2026-07-01T10:00:00.000Z',
      status: 'supported',
      releaseUrl: 'https://releases.spool.test/1.2.2',
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
      downloadUrl: 'https://releases.spool.test/Spool-1.2.3.zip'
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

  it('rejects insecure, credentialed, and fragmented release URLs', () => {
    for (const releaseUrl of [
      'http://releases.spool.test/1.2.3',
      'https://user:secret@releases.spool.test/1.2.3',
      'https://releases.spool.test/1.2.3#mutable'
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
