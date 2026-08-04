import { beforeEach, describe, expect, it, vi } from 'vitest';

const { privateEnvironment } = vi.hoisted(() => ({
  privateEnvironment: {} as Record<string, string | undefined>
}));
vi.mock('$env/dynamic/private', () => ({ env: privateEnvironment }));

import { GET } from './+server';

describe('Apple application-site association', () => {
  beforeEach(() => {
    delete privateEnvironment.APPLE_TEAM_ID;
  });

  it('fails closed when the signing team is not configured', async () => {
    const response = GET({} as never);
    expect(response.status).toBe(503);
    expect(response.headers.get('cache-control')).toBe('no-store');
    expect(await response.text()).not.toContain('TEAM_IDENTIFIER');
  });

  it('publishes only the exact connect route for a validated team identifier', async () => {
    privateEnvironment.APPLE_TEAM_ID = 'A1B2C3D4E5';
    const response = GET({} as never);
    expect(response.status).toBe(200);
    expect(await response.json()).toEqual({
      applinks: {
        details: [
          {
            appIDs: ['A1B2C3D4E5.com.piqae.node.menu'],
            components: [{ '/': '/connect', comment: 'Piqae node connector consent handoff' }]
          }
        ]
      }
    });
  });
});
