import { beforeEach, describe, expect, it, vi } from 'vitest';

const { beginMagicAuth, currentLocalIdentity } = vi.hoisted(() => ({
  beginMagicAuth: vi.fn(),
  currentLocalIdentity: vi.fn()
}));

vi.mock('$lib/server/auth-config', () => ({ authMode: 'workos' }));
vi.mock('$lib/server/local-owner-auth', () => ({
  currentLocalIdentity,
  exchangeLocalOwnerCredential: vi.fn()
}));
vi.mock('$lib/server/workos-first-party-auth', () => ({
  authenticatePassword: vi.fn(),
  beginMagicAuth,
  beginTotpChallenge: vi.fn(),
  completeEmailVerification: vi.fn(),
  completeMagicAuth: vi.fn(),
  completeTotp: vi.fn(),
  isAdvancedChallenge: vi.fn(() => false),
  registerPassword: vi.fn(),
  requestPasswordReset: vi.fn(),
  resetPassword: vi.fn(),
  saveEmailVerificationChallenge: vi.fn(() => false),
  saveWorkosSession: vi.fn()
}));

import { actions, load } from './+page.server';

function event(body?: FormData) {
  const url = new URL('https://app.piqae.test/login');
  const setHeaders = vi.fn();
  const encodedBody = body
    ? new URLSearchParams(
        [...body.entries()].map(([name, value]) => [name, String(value)])
      )
    : undefined;
  const request = new Request(url, {
    method: body ? 'POST' : 'GET',
    body: encodedBody,
    headers: body
      ? { 'content-type': 'application/x-www-form-urlencoded;charset=UTF-8' }
      : undefined
  });
  if (body) request.headers.set('origin', url.origin);
  return {
    url,
    request,
    route: { id: '/login' },
    setHeaders,
    getClientAddress: vi.fn(() => '192.0.2.55'),
    cookies: { get: vi.fn(), set: vi.fn(), delete: vi.fn() }
  };
}

describe('login server security response', () => {
  beforeEach(() => vi.clearAllMocks());

  it('marks login page data private and non-cacheable', async () => {
    const request = event();
    await load(request as never);
    expect(request.setHeaders).toHaveBeenCalledWith({ 'cache-control': 'private, no-store' });
  });

  it('returns the same public Magic Auth response when WorkOS accepts or rejects the address', async () => {
    const makeBody = () => {
      const body = new FormData();
      body.set('email', 'person@example.test');
      body.set('return_to', '/dashboard');
      return body;
    };
    beginMagicAuth.mockResolvedValueOnce(undefined);
    const acceptedEvent = event(makeBody());
    const accepted = await actions.magicStart!(acceptedEvent as never);

    beginMagicAuth.mockRejectedValueOnce(new Error('unknown account'));
    const rejectedEvent = event(makeBody());
    const rejected = await actions.magicStart!(rejectedEvent as never);

    expect(rejected).toEqual(accepted);
    expect(acceptedEvent.setHeaders).toHaveBeenCalledWith({
      'cache-control': 'private, no-store'
    });
    expect(JSON.stringify(rejected)).not.toMatch(/unknown|account|email/i);
  });

  it('uses the trusted platform client address rather than a submitted forwarding header', async () => {
    const body = new FormData();
    body.set('email', 'person@example.test');
    body.set('return_to', '/dashboard');
    const request = event(body);
    request.request.headers.set('x-forwarded-for', '203.0.113.99');
    await actions.magicStart!(request as never);
    expect(request.getClientAddress).toHaveBeenCalledTimes(1);
  });
});
