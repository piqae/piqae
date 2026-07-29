import { describe, expect, it, vi } from 'vitest';

vi.mock('$env/dynamic/public', () => ({
  env: { PUBLIC_SPOOL_VERSION: '0.1.0-test' }
}));

import { GET } from './+server';

describe('web health endpoint', () => {
  it('returns a small non-cacheable response without probing tenant services', async () => {
    const response = GET();

    expect(response.status).toBe(200);
    expect(response.headers.get('cache-control')).toBe('no-store');
    expect(await response.json()).toEqual({
      status: 'ok',
      service: 'piqae-web',
      version: '0.1.0-test'
    });
  });
});
