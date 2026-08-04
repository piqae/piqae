import { describe, expect, it } from 'vitest';
import {
  publishedReleaseManifest,
  releaseObjectKey,
  releaseOriginConfig
} from './release-origin';

const validEnvironment = {
  PIQAE_RELEASES_S3_ENDPOINT: 'https://storage.railway.app',
  PIQAE_RELEASES_S3_ACCESS_KEY_ID: 'release-access',
  PIQAE_RELEASES_S3_SECRET_ACCESS_KEY: 'release-secret',
  PIQAE_RELEASES_S3_BUCKET: 'piqae-releases-1234',
  PIQAE_RELEASES_S3_REGION: 'sin',
  PIQAE_RELEASES_S3_VIRTUAL_HOSTED_STYLE: 'true'
};

describe('native release origin', () => {
  it('accepts a complete isolated release-bucket configuration', () => {
    expect(releaseOriginConfig(validEnvironment)).toEqual({
      endpoint: 'https://storage.railway.app',
      accessKeyId: 'release-access',
      secretAccessKey: 'release-secret',
      bucket: 'piqae-releases-1234',
      region: 'sin',
      forcePathStyle: false
    });
  });

  it('fails closed when credentials or a safe HTTPS endpoint are unavailable', () => {
    expect(releaseOriginConfig({ ...validEnvironment, PIQAE_RELEASES_S3_SECRET_ACCESS_KEY: '' })).toBeNull();
    expect(
      releaseOriginConfig({
        ...validEnvironment,
        PIQAE_RELEASES_S3_ENDPOINT: 'http://storage.example.test'
      })
    ).toBeNull();
  });

  it('maps only allowlisted release assets into the dedicated native prefix', () => {
    expect(releaseObjectKey('stable', 'piqae-macos-universal.dmg')).toBe(
      'native/stable/piqae-macos-universal.dmg'
    );
    expect(releaseObjectKey('stable', 'piqae-macos-universal.pkg')).toBe(
      'native/stable/piqae-macos-universal.pkg'
    );
    expect(releaseObjectKey('preview', 'appcast-windows.xml')).toBe(
      'native/preview/appcast-windows.xml'
    );
    expect(releaseObjectKey('nightly', 'piqae-macos-universal.dmg')).toBeNull();
    expect(releaseObjectKey('stable', '../customer-document.pdf')).toBeNull();
    expect(releaseObjectKey('stable', 'unlisted.pdf')).toBeNull();
  });

  it('fails closed when the release origin is not configured', async () => {
    await expect(publishedReleaseManifest('stable', {})).resolves.toBeNull();
  });
});
