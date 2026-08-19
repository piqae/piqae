import { beforeEach, describe, expect, it, vi } from 'vitest';

const { authenticateWithPassword, authenticateWithTotp, createMagicAuth, createUserAuthFactor, challengeFactor, sealData, unsealData } = vi.hoisted(() => ({
  authenticateWithPassword: vi.fn(),
  authenticateWithTotp: vi.fn(),
  createMagicAuth: vi.fn(),
  createUserAuthFactor: vi.fn(),
  challengeFactor: vi.fn(),
  sealData: vi.fn(),
  unsealData: vi.fn()
}));

vi.mock('@workos-inc/node', () => ({
  WorkOS: class {
    userManagement = { authenticateWithPassword, authenticateWithTotp, createMagicAuth };
    multiFactorAuth = { createUserAuthFactor, challengeFactor };
  }
}));
vi.mock('@workos/authkit-session', () => ({
  sessionEncryption: { sealData, unsealData }
}));
vi.mock('./auth-config', () => ({
  workosConfig: {
    clientId: 'client_test',
    cookiePassword: 'not-a-production-cookie-password-123456',
    apiKey: 'secret-never-returned',
    redirectUri: 'https://app.piqae.test/auth/callback'
  }
}));

import {
  authenticatePassword,
  beginMagicAuth,
  beginTotpChallenge,
  completeTotp,
  isAdvancedChallenge,
  saveWorkosSession
} from './workos-first-party-auth';

function event(protocol = 'https:') {
  const values = new Map<string, { value: string; options: Record<string, unknown> }>();
  return {
    url: new URL(`${protocol}//app.piqae.test/login`),
    request: new Request(`${protocol}//app.piqae.test/login`, {
      headers: { 'user-agent': 'test-agent', 'x-forwarded-for': '192.0.2.2' }
    }),
    getClientAddress: () => '192.0.2.3',
    cookies: {
      set: (name: string, value: string, options: Record<string, unknown>) =>
        values.set(name, { value, options }),
      get: (name: string) => values.get(name)?.value,
      delete: (name: string) => values.delete(name)
    },
    values
  };
}

describe('first-party WorkOS authentication boundary', () => {
  beforeEach(() => vi.clearAllMocks());

  it('requests a WorkOS-sealed session without exposing provider credentials', async () => {
    authenticateWithPassword.mockResolvedValue({ sealedSession: 'sealed-session' });
    const request = event();
    const result = await authenticatePassword(request as never, 'person@example.test', 'private');

    expect(authenticateWithPassword).toHaveBeenCalledWith(
      expect.objectContaining({
        email: 'person@example.test',
        password: 'private',
        ipAddress: '192.0.2.3',
        clientId: 'client_test',
        session: expect.objectContaining({ sealSession: true })
      })
    );
    expect(JSON.stringify(result)).not.toContain('secret-never-returned');
  });

  it('stores only the sealed session in a secure HttpOnly same-site cookie', async () => {
    const request = event();
    await saveWorkosSession(request as never, { sealedSession: 'sealed-session' } as never);
    expect(request.values.get('wos-session')).toEqual({
      value: 'sealed-session',
      options: expect.objectContaining({ httpOnly: true, secure: true, sameSite: 'lax', path: '/' })
    });
  });

  it('seals Magic Auth state server-side and never puts an email in the cookie', async () => {
    createMagicAuth.mockResolvedValue({});
    sealData.mockResolvedValue('opaque-flow');
    const request = event();
    await beginMagicAuth(request as never, 'person@example.test');
    expect(request.values.get('piqae-magic-auth')?.value).toBe('opaque-flow');
    expect(request.values.get('piqae-magic-auth')?.value).not.toContain('person@example.test');
  });

  it('enrolls TOTP and seals pending authentication identifiers in an HttpOnly cookie', async () => {
    createUserAuthFactor.mockResolvedValue({
      authenticationFactor: {
        totp: { qrCode: 'data:image/png;base64,safe', secret: 'SETUPSECRET' }
      },
      authenticationChallenge: { id: 'challenge-enrollment' }
    });
    sealData.mockResolvedValue('opaque-mfa-state');
    const request = event();
    const error = Object.assign(new Error('do not render'), {
      code: 'mfa_enrollment',
      pendingAuthenticationToken: 'pending-secret',
      rawData: { user: { id: 'user_1', email: 'person@example.test' } }
    });

    await expect(beginTotpChallenge(request as never, error)).resolves.toEqual({
      enrollment: { qrCode: 'data:image/png;base64,safe', secret: 'SETUPSECRET' }
    });
    expect(request.values.get('piqae-auth-challenge')?.value).toBe('opaque-mfa-state');
    expect(request.values.get('piqae-auth-challenge')?.options).toEqual(
      expect.objectContaining({ httpOnly: true, secure: true, sameSite: 'lax' })
    );
    expect(request.values.get('piqae-auth-challenge')?.value).not.toContain('pending-secret');
  });

  it('challenges an existing TOTP factor and authenticates from sealed state', async () => {
    challengeFactor.mockResolvedValue({ id: 'challenge-existing' });
    sealData.mockResolvedValue('opaque-mfa-state');
    const request = event();
    const error = Object.assign(new Error('challenge'), {
      code: 'mfa_challenge',
      pendingAuthenticationToken: 'pending-secret',
      rawData: { authentication_factors: [{ id: 'factor_1', type: 'totp' }] }
    });
    await expect(beginTotpChallenge(request as never, error)).resolves.toEqual({ enrollment: null });

    unsealData.mockResolvedValue({
      kind: 'totp',
      pendingAuthenticationToken: 'pending-secret',
      authenticationChallengeId: 'challenge-existing'
    });
    request.values.set('piqae-auth-challenge', { value: 'opaque-mfa-state', options: {} });
    authenticateWithTotp.mockResolvedValue({ sealedSession: 'sealed-session' });
    await completeTotp(request as never, '123456');
    expect(authenticateWithTotp).toHaveBeenCalledWith(
      expect.objectContaining({
        code: '123456',
        pendingAuthenticationToken: 'pending-secret',
        authenticationChallengeId: 'challenge-existing',
        session: expect.objectContaining({ sealSession: true })
      })
    );
  });

  it.each([
    'sso_required',
    'organization_selection_required',
    'radar_email_challenge',
    'radar_sms_challenge',
    'mfa_verification'
  ])('recognizes unsupported continuation %s for safe hosted completion', (code) => {
    expect(isAdvancedChallenge(Object.assign(new Error('private'), { code }))).toBe(true);
  });

  it('does not send ordinary credential failures to the hosted continuation', () => {
    expect(isAdvancedChallenge(Object.assign(new Error('private'), { code: 'invalid_credentials' }))).toBe(false);
  });
});
