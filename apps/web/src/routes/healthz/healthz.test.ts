import { describe, expect, it, vi } from 'vitest';

const { privateEnvironment } = vi.hoisted(() => ({
  privateEnvironment: {} as Record<string, string | undefined>
}));

vi.mock('$env/dynamic/public', () => ({
  env: { PUBLIC_PIQAE_VERSION: '0.1.0-test' }
}));
vi.mock('$env/dynamic/private', () => ({ env: privateEnvironment }));

import { GET } from './+server';

const REVISION = '0123456789abcdef0123456789abcdef01234567';

describe('web health endpoint', () => {
  it('returns a small non-cacheable response without probing tenant services', async () => {
    for (const key of Object.keys(privateEnvironment)) {
      delete privateEnvironment[key];
    }
    const response = GET();

    expect(response.status).toBe(200);
    expect(response.headers.get('cache-control')).toBe('no-store');
    expect(await response.json()).toEqual({
      status: 'ok',
      service: 'piqae-web',
      version: '0.1.0-test',
      revision: 'unknown'
    });
  });

  it('reports the deployed commit so a post-deploy gate can pin the revision', async () => {
    privateEnvironment.PIQAE_RELEASE_SHA = REVISION.toUpperCase();

    expect((await GET().json()).revision).toBe(REVISION);
  });

  it('falls back to the platform commit variable', async () => {
    delete privateEnvironment.PIQAE_RELEASE_SHA;
    privateEnvironment.RAILWAY_GIT_COMMIT_SHA = REVISION;

    expect((await GET().json()).revision).toBe(REVISION);
  });

  it('never echoes a value that is not a commit hash', async () => {
    privateEnvironment.PIQAE_RELEASE_SHA = 'latest';
    delete privateEnvironment.RAILWAY_GIT_COMMIT_SHA;

    expect((await GET().json()).revision).toBe('unknown');
  });
});
