import { env as privateEnv } from '$env/dynamic/private';
import { env as publicEnv } from '$env/dynamic/public';
import { error, type Cookies, type RequestEvent } from '@sveltejs/kit';

const SESSION_COOKIE = 'spool_local_session';
const EXPIRY_COOKIE = 'spool_local_session_expiry';
const ROTATE_BEFORE_MS = 60 * 60 * 1000;

export interface LocalIdentity {
  id: string;
  email: string;
  name: string | null;
  workspaceId: string;
  environmentId: string;
  roles: string[];
}

interface SessionExchange {
  token: string;
  expires_at: string;
}

interface IdentityResponse {
  id: string;
  email: string;
  name: string | null;
  workspace_id: string;
  environment_id: string;
  roles: string[];
}

function apiUrl(event: Pick<RequestEvent, 'url'>, path: string): string {
  const base = publicEnv.PUBLIC_SPOOL_API_URL || event.url.origin;
  return `${base.replace(/\/$/, '')}${path}`;
}

function cookieOptions(event: Pick<RequestEvent, 'url'>, expires?: Date) {
  return {
    path: '/',
    httpOnly: true,
    sameSite: 'strict' as const,
    secure: privateEnv.SPOOL_COOKIE_SECURE === 'true' || event.url.protocol === 'https:',
    ...(expires ? { expires } : {})
  };
}

export function localSessionToken(cookies: Cookies): string | null {
  return cookies.get(SESSION_COOKIE) ?? null;
}

export function clearLocalSession(event: Pick<RequestEvent, 'cookies' | 'url'>): void {
  const options = cookieOptions(event);
  event.cookies.delete(SESSION_COOKIE, options);
  event.cookies.delete(EXPIRY_COOKIE, options);
}

function setLocalSession(
  event: Pick<RequestEvent, 'cookies' | 'url'>,
  session: SessionExchange
): void {
  const expires = new Date(session.expires_at);
  if (!Number.isFinite(expires.getTime())) throw new Error('The identity service returned an invalid expiry.');
  const options = cookieOptions(event, expires);
  event.cookies.set(SESSION_COOKIE, session.token, options);
  event.cookies.set(EXPIRY_COOKIE, session.expires_at, options);
}

async function sessionRequest(
  event: Pick<RequestEvent, 'fetch' | 'url' | 'cookies'>,
  path: string,
  authorization: string,
  body?: unknown
): Promise<Response> {
  return event.fetch(apiUrl(event, path), {
    method: 'POST',
    headers: {
      accept: 'application/json',
      authorization: `Bearer ${authorization}`,
      ...(body ? { 'content-type': 'application/json' } : {})
    },
    body: body ? JSON.stringify(body) : undefined
  });
}

export async function exchangeLocalOwnerCredential(
  event: Pick<RequestEvent, 'fetch' | 'url' | 'cookies'>,
  credential: string
): Promise<void> {
  const response = await event.fetch(apiUrl(event, '/v1/identity/local/exchange'), {
    method: 'POST',
    headers: { accept: 'application/json', 'content-type': 'application/json' },
    body: JSON.stringify({ credential })
  });
  if (!response.ok) {
    throw error(response.status === 401 ? 401 : 503, 'The owner credential could not be exchanged.');
  }
  setLocalSession(event, (await response.json()) as SessionExchange);
}

export async function revokeLocalSession(
  event: Pick<RequestEvent, 'fetch' | 'url' | 'cookies'>
): Promise<void> {
  const token = localSessionToken(event.cookies);
  if (token) {
    try {
      await sessionRequest(event, '/v1/identity/local/sessions/revoke', token);
    } finally {
      clearLocalSession(event);
    }
  } else {
    clearLocalSession(event);
  }
}

export async function ensureLocalSession(
  event: Pick<RequestEvent, 'fetch' | 'url' | 'cookies'>
): Promise<string | null> {
  const token = localSessionToken(event.cookies);
  if (!token) return null;
  const expiry = Date.parse(event.cookies.get(EXPIRY_COOKIE) ?? '');
  if (Number.isFinite(expiry) && expiry - Date.now() > ROTATE_BEFORE_MS) return token;
  const response = await sessionRequest(event, '/v1/identity/local/sessions/rotate', token);
  if (!response.ok) {
    clearLocalSession(event);
    return null;
  }
  const session = (await response.json()) as SessionExchange;
  setLocalSession(event, session);
  return session.token;
}

export async function currentLocalIdentity(
  event: Pick<RequestEvent, 'fetch' | 'url' | 'cookies'>,
  token = localSessionToken(event.cookies)
): Promise<LocalIdentity | null> {
  if (!token) return null;
  const response = await event.fetch(apiUrl(event, '/v1/identity/me'), {
    headers: { accept: 'application/json', authorization: `Bearer ${token}` }
  });
  if (response.status === 401) {
    clearLocalSession(event);
    return null;
  }
  if (!response.ok) throw error(503, 'The local identity service is unavailable.');
  const identity = (await response.json()) as IdentityResponse;
  return {
    id: identity.id,
    email: identity.email,
    name: identity.name,
    workspaceId: identity.workspace_id,
    environmentId: identity.environment_id,
    roles: identity.roles
  };
}
