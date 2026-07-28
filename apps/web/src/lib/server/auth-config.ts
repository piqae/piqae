import { env } from '$env/dynamic/private';

export type ServerAuthMode = 'workos' | 'local' | 'demo';

const requiredWorkOs = [
  env.WORKOS_CLIENT_ID,
  env.WORKOS_API_KEY,
  env.WORKOS_REDIRECT_URI,
  env.WORKOS_COOKIE_PASSWORD
];

export const workosConfigured = requiredWorkOs.every((value) => Boolean(value));

export const authMode: ServerAuthMode =
  env.SPOOL_AUTH_MODE === 'workos' || env.SPOOL_AUTH_MODE === 'local' || env.SPOOL_AUTH_MODE === 'demo'
    ? env.SPOOL_AUTH_MODE
    : workosConfigured
      ? 'workos'
      : 'local';

if (authMode === 'workos' && !workosConfigured) {
  throw new Error(
    'SPOOL_AUTH_MODE=workos requires WORKOS_CLIENT_ID, WORKOS_API_KEY, ' +
      'WORKOS_REDIRECT_URI, and WORKOS_COOKIE_PASSWORD'
  );
}

export const workosConfig = workosConfigured
  ? {
      clientId: env.WORKOS_CLIENT_ID as string,
      apiKey: env.WORKOS_API_KEY as string,
      redirectUri: env.WORKOS_REDIRECT_URI as string,
      cookiePassword: env.WORKOS_COOKIE_PASSWORD as string
    }
  : null;
