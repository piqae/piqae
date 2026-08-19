import { WorkOS, type AuthenticationResponse } from '@workos-inc/node';
import { sessionEncryption } from '@workos/authkit-session';
import type { Cookies, RequestEvent } from '@sveltejs/kit';
import { workosConfig } from './auth-config';

const SESSION_COOKIE = 'wos-session';
const CHALLENGE_COOKIE = 'piqae-auth-challenge';
const MAGIC_COOKIE = 'piqae-magic-auth';
const FLOW_TTL_SECONDS = 10 * 60;

type Challenge = {
  kind: 'email-verification' | 'totp';
  pendingAuthenticationToken: string;
  authenticationChallengeId?: string;
};

type WorkosAuthenticationError = Error & {
  code?: string;
  pendingAuthenticationToken?: string;
  rawData?: {
    user?: { id?: string; email?: string };
    authentication_factors?: Array<{ id?: string; type?: string }>;
  };
};

function authenticationError(error: unknown): WorkosAuthenticationError | null {
  if (!(error instanceof Error)) return null;
  const candidate = error as WorkosAuthenticationError;
  return typeof candidate.code === 'string' ? candidate : null;
}

function cookieOptions(event: Pick<RequestEvent, 'url'>, maxAge: number) {
  return {
    path: '/',
    httpOnly: true,
    secure: event.url.protocol === 'https:',
    sameSite: 'lax' as const,
    maxAge
  };
}

function requireConfig() {
  if (!workosConfig) throw new Error('WorkOS authentication is not configured');
  return workosConfig;
}

let client: WorkOS | null = null;
function workos() {
  const config = requireConfig();
  return (client ??= new WorkOS(config.apiKey, { clientId: config.clientId }));
}

function requestContext(event: RequestEvent) {
  let direct: string | undefined;
  try {
    direct = event.getClientAddress();
  } catch {
    direct = undefined;
  }
  return {
    // SvelteKit's adapter supplies the address from its trusted deployment
    // boundary. Do not accept a caller-controlled forwarding header here.
    ipAddress: direct,
    userAgent: event.request.headers.get('user-agent')?.slice(0, 512)
  };
}

export async function saveWorkosSession(event: RequestEvent, result: AuthenticationResponse) {
  if (!result.sealedSession) throw new Error('WorkOS did not return a sealed session');
  event.cookies.set(SESSION_COOKIE, result.sealedSession, cookieOptions(event, 60 * 60 * 24 * 30));
  clearAuthFlow(event.cookies);
}

export async function authenticatePassword(event: RequestEvent, email: string, password: string) {
  const config = requireConfig();
  return workos().userManagement.authenticateWithPassword({
    email,
    password,
    clientId: config.clientId,
    ...requestContext(event),
    session: { sealSession: true, cookiePassword: config.cookiePassword }
  });
}

export async function registerPassword(event: RequestEvent, email: string, password: string) {
  await workos().userManagement.createUser({
    email,
    password,
    ...requestContext(event)
  });
  return authenticatePassword(event, email, password);
}

export async function beginMagicAuth(event: RequestEvent, email: string) {
  await workos().userManagement.createMagicAuth({ email, ...requestContext(event) });
  const config = requireConfig();
  const sealed = await sessionEncryption.sealData(
    { email: email.toLowerCase(), issuedAt: Date.now() },
    { password: config.cookiePassword, ttl: FLOW_TTL_SECONDS }
  );
  event.cookies.set(MAGIC_COOKIE, sealed, cookieOptions(event, FLOW_TTL_SECONDS));
}

export async function completeMagicAuth(event: RequestEvent, code: string) {
  const config = requireConfig();
  const sealed = event.cookies.get(MAGIC_COOKIE);
  if (!sealed) throw new Error('Magic authentication has expired');
  const flow = await sessionEncryption.unsealData<{ email?: string }>(sealed, {
    password: config.cookiePassword,
    ttl: FLOW_TTL_SECONDS
  });
  if (!flow.email) throw new Error('Magic authentication is invalid');
  return workos().userManagement.authenticateWithMagicAuth({
    email: flow.email,
    code,
    clientId: config.clientId,
    ...requestContext(event),
    session: { sealSession: true, cookiePassword: config.cookiePassword }
  });
}

export async function saveEmailVerificationChallenge(event: RequestEvent, error: unknown) {
  const authError = authenticationError(error);
  if (authError?.code !== 'email_verification_required') {
    return false;
  }
  if (!authError.pendingAuthenticationToken) return false;
  const config = requireConfig();
  const challenge: Challenge = {
    kind: 'email-verification',
    pendingAuthenticationToken: authError.pendingAuthenticationToken
  };
  const sealed = await sessionEncryption.sealData(challenge, {
    password: config.cookiePassword,
    ttl: FLOW_TTL_SECONDS
  });
  event.cookies.set(CHALLENGE_COOKIE, sealed, cookieOptions(event, FLOW_TTL_SECONDS));
  return true;
}

export async function completeEmailVerification(event: RequestEvent, code: string) {
  const config = requireConfig();
  const sealed = event.cookies.get(CHALLENGE_COOKIE);
  if (!sealed) throw new Error('Email verification has expired');
  const challenge = await sessionEncryption.unsealData<Challenge>(sealed, {
    password: config.cookiePassword,
    ttl: FLOW_TTL_SECONDS
  });
  if (challenge.kind !== 'email-verification' || !challenge.pendingAuthenticationToken) {
    throw new Error('Email verification is invalid');
  }
  return workos().userManagement.authenticateWithEmailVerification({
    code,
    pendingAuthenticationToken: challenge.pendingAuthenticationToken,
    clientId: config.clientId,
    ...requestContext(event),
    session: { sealSession: true, cookiePassword: config.cookiePassword }
  });
}

export async function beginTotpChallenge(event: RequestEvent, error: unknown) {
  const authError = authenticationError(error);
  if (!authError || !['mfa_enrollment', 'mfa_challenge'].includes(authError.code ?? '')) {
    return null;
  }
  if (!authError.pendingAuthenticationToken) return null;

  let authenticationChallengeId: string;
  let enrollment: { qrCode: string; secret: string } | null = null;
  if (authError.code === 'mfa_enrollment') {
    const user = authError.rawData?.user;
    if (!user?.id || !user.email) return null;
    const created = await workos().multiFactorAuth.createUserAuthFactor({
      userId: user.id,
      type: 'totp',
      totpIssuer: 'Piqae',
      totpUser: user.email
    });
    authenticationChallengeId = created.authenticationChallenge.id;
    enrollment = {
      qrCode: created.authenticationFactor.totp.qrCode,
      secret: created.authenticationFactor.totp.secret
    };
  } else {
    const factor = authError.rawData?.authentication_factors?.find(
      (candidate) => candidate.type === 'totp' && candidate.id
    );
    if (!factor?.id) return null;
    const challenge = await workos().multiFactorAuth.challengeFactor({
      authenticationFactorId: factor.id
    });
    authenticationChallengeId = challenge.id;
  }

  const config = requireConfig();
  const sealed = await sessionEncryption.sealData(
    {
      kind: 'totp',
      pendingAuthenticationToken: authError.pendingAuthenticationToken,
      authenticationChallengeId
    } satisfies Challenge,
    { password: config.cookiePassword, ttl: FLOW_TTL_SECONDS }
  );
  event.cookies.set(CHALLENGE_COOKIE, sealed, cookieOptions(event, FLOW_TTL_SECONDS));
  return { enrollment };
}

export async function completeTotp(event: RequestEvent, code: string) {
  const config = requireConfig();
  const sealed = event.cookies.get(CHALLENGE_COOKIE);
  if (!sealed) throw new Error('MFA verification has expired');
  const challenge = await sessionEncryption.unsealData<Challenge>(sealed, {
    password: config.cookiePassword,
    ttl: FLOW_TTL_SECONDS
  });
  if (
    challenge.kind !== 'totp' ||
    !challenge.pendingAuthenticationToken ||
    !challenge.authenticationChallengeId
  ) {
    throw new Error('MFA verification is invalid');
  }
  return workos().userManagement.authenticateWithTotp({
    code,
    pendingAuthenticationToken: challenge.pendingAuthenticationToken,
    authenticationChallengeId: challenge.authenticationChallengeId,
    clientId: config.clientId,
    ...requestContext(event),
    session: { sealSession: true, cookiePassword: config.cookiePassword }
  });
}

export async function requestPasswordReset(email: string) {
  await workos().userManagement.createPasswordReset({ email });
}

export async function resetPassword(token: string, newPassword: string) {
  await workos().userManagement.resetPassword({ token, newPassword });
}

export function isAdvancedChallenge(error: unknown) {
  const authError = authenticationError(error);
  return Boolean(
    authError &&
      [
        'sso_required',
        'organization_selection_required',
        'radar_email_challenge',
        'radar_sms_challenge',
        'mfa_verification'
      ].includes(authError.code ?? '')
  );
}

export function clearAuthFlow(cookies: Cookies) {
  cookies.delete(CHALLENGE_COOKIE, { path: '/' });
  cookies.delete(MAGIC_COOKIE, { path: '/' });
}
