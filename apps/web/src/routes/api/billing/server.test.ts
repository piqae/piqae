import { beforeEach, describe, expect, it, vi } from 'vitest';

const { getUser, privateEnvironment } = vi.hoisted(() => ({
  getUser: vi.fn(),
  privateEnvironment: {} as Record<string, string>
}));

vi.mock('@workos/authkit-sveltekit', () => ({
  authKit: { getUser }
}));
vi.mock('$env/dynamic/private', () => ({ env: privateEnvironment }));
vi.mock('$env/dynamic/public', () => ({ env: {} }));
vi.mock('$lib/server/auth-config', () => ({ authMode: 'workos' }));

import { POST as checkout } from './checkout/+server';
import { POST as portal } from './portal/+server';

function event(path: string, role: string) {
  const url = new URL(`https://cloud.piqae.test${path}`);
  return {
    url,
    request: new Request(url, {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
        origin: url.origin
      },
      body: JSON.stringify({ plan: 'pro', interval: 'monthly' })
    }),
    locals: {
      authMode: 'workos',
      auth: {
        organizationId: 'org_workos',
        role,
        accessToken: 'verified-session-token'
      }
    },
    fetch: vi.fn()
  };
}

describe('hosted billing route authorization', () => {
  beforeEach(() => {
    getUser.mockReset();
    getUser.mockResolvedValue({
      id: 'user_123',
      email: 'developer@piqae.test'
    });
  });

  for (const role of ['admin', 'developer', 'operator', 'viewer']) {
    it(`denies Checkout to the ${role} role before Stripe is called`, async () => {
      await expect(checkout(event('/api/billing/checkout', role) as never)).rejects.toMatchObject({
        status: 403,
        body: { message: 'Billing access denied' }
      });
    });

    it(`denies Customer Portal to the ${role} role before Stripe is called`, async () => {
      await expect(portal(event('/api/billing/portal', role) as never)).rejects.toMatchObject({
        status: 403,
        body: { message: 'Billing access denied' }
      });
    });
  }
});
