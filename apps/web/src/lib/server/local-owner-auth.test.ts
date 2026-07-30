import { beforeEach, describe, expect, it, vi } from 'vitest';

const { publicEnvironment, privateEnvironment } = vi.hoisted(() => ({
  publicEnvironment: {} as Record<string, string>,
  privateEnvironment: {} as Record<string, string>
}));

vi.mock('$env/dynamic/public', () => ({ env: publicEnvironment }));
vi.mock('$env/dynamic/private', () => ({ env: privateEnvironment }));

import {
  currentLocalIdentity,
  ensureLocalSession,
  exchangeLocalOwnerCredential,
  localSessionToken
} from './local-owner-auth';

function cookieJar(seed: Record<string, string> = {}) {
  const values = new Map(Object.entries(seed));
  return {
    get: (name: string) => values.get(name),
    set: (name: string, value: string) => values.set(name, value),
    delete: (name: string) => values.delete(name),
    values
  };
}

describe('local owner browser adapter', () => {
  beforeEach(() => {
    for (const key of Object.keys(publicEnvironment)) delete publicEnvironment[key];
    for (const key of Object.keys(privateEnvironment)) delete privateEnvironment[key];
    publicEnvironment.PUBLIC_PIQAE_API_URL = 'https://api.piqae.test';
  });

  it('exchanges the credential without storing it and sets an HttpOnly session', async () => {
    const credential = 'piq_owner_00000000-0000-7000-8000-000000000000.private';
    const fetcher = vi.fn<typeof fetch>().mockResolvedValueOnce(
      Response.json({
        token: 'piq_session_00000000-0000-7000-8000-000000000001.private',
        expires_at: '2030-01-01T00:00:00Z'
      })
    );
    const cookies = cookieJar();
    await exchangeLocalOwnerCredential({
      fetch: fetcher,
      url: new URL('https://dashboard.piqae.test/login'),
      cookies: cookies as never
    }, credential);

    const [, request] = fetcher.mock.calls[0] ?? [];
    expect(String(request?.body)).toContain(credential);
    expect([...cookies.values.values()]).not.toContain(credential);
    expect(localSessionToken(cookies as never)).toMatch(/^piq_session_/);
  });

  it('rotates an expired session and returns only the replacement', async () => {
    const oldToken = 'piq_session_00000000-0000-7000-8000-000000000001.old';
    const newToken = 'piq_session_00000000-0000-7000-8000-000000000002.new';
    const cookies = cookieJar({
      piqae_local_session: oldToken,
      piqae_local_session_expiry: '2020-01-01T00:00:00Z'
    });
    const fetcher = vi.fn<typeof fetch>().mockResolvedValueOnce(
      Response.json({ token: newToken, expires_at: '2030-01-01T00:00:00Z' })
    );

    await expect(
      ensureLocalSession({
        fetch: fetcher,
        url: new URL('https://dashboard.piqae.test/dashboard'),
        cookies: cookies as never
      })
    ).resolves.toBe(newToken);
    expect(new Headers(fetcher.mock.calls[0]?.[1]?.headers).get('authorization')).toBe(
      `Bearer ${oldToken}`
    );
    expect(cookies.values.get('piqae_local_session')).toBe(newToken);
  });

  it('migrates a valid pre-rename session without forcing reauthentication', async () => {
    const token = 'spl_session_00000000-0000-7000-8000-000000000001.legacy';
    const cookies = cookieJar({
      spool_local_session: token,
      spool_local_session_expiry: '2030-01-01T00:00:00Z'
    });
    const fetcher = vi.fn<typeof fetch>();

    await expect(
      ensureLocalSession({
        fetch: fetcher,
        url: new URL('https://dashboard.piqae.test/dashboard'),
        cookies: cookies as never
      })
    ).resolves.toBe(token);
    expect(fetcher).not.toHaveBeenCalled();
    expect(cookies.values.get('piqae_local_session')).toBe(token);
    expect(cookies.values.has('spool_local_session')).toBe(false);
    expect(cookies.values.has('spool_local_session_expiry')).toBe(false);
  });

  it('maps tenant identity and clears a rejected session', async () => {
    const cookies = cookieJar({ piqae_local_session: 'piq_session_rejected' });
    const fetcher = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(new Response(null, { status: 401 }));
    await expect(
      currentLocalIdentity({
        fetch: fetcher,
        url: new URL('https://dashboard.piqae.test/dashboard'),
        cookies: cookies as never
      })
    ).resolves.toBeNull();
    expect(cookies.values.has('piqae_local_session')).toBe(false);
  });
});
