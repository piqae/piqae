import { describe, expect, it, vi } from 'vitest';
import { createAuthBoundary } from './auth';

describe('auth boundary', () => {
  it('keeps provider details behind same-origin routes', () => {
    const auth = createAuthBoundary('hosted');
    expect(auth.signInUrl('/dashboard')).toBe('/auth/login?return_to=%2Fdashboard');
    expect(auth.signOutUrl()).toBe('/auth/logout');
  });

  it('returns no token when the server-side exchange is unavailable', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response(null, { status: 401 })));
    await expect(createAuthBoundary().accessToken()).resolves.toBeUndefined();
    vi.unstubAllGlobals();
  });
});
