import { describe, expect, it } from 'vitest';
import { createAuthBoundary } from './auth';

describe('auth boundary', () => {
  it('keeps provider details behind same-origin routes', () => {
    const auth = createAuthBoundary('hosted');
    expect(auth.signInUrl('/dashboard')).toBe('/auth/login?return_to=%2Fdashboard');
    expect(auth.signOutUrl()).toBe('/auth/logout');
  });
});
