import { authKitHandle, configureAuthKit } from '@workos/authkit-sveltekit';
import * as Sentry from '@sentry/sveltekit';
import { env } from '$env/dynamic/private';
import { env as publicEnv } from '$env/dynamic/public';
import type { Handle, HandleServerError } from '@sveltejs/kit';
import { sequence } from '@sveltejs/kit/hooks';
import {
  publicErrorMessage,
  safeErrorKind,
  sanitizeSentryBreadcrumb,
  sanitizeSentryEvent,
  sentrySampleRate
} from '$lib/observability/sentry';
import { authMode, workosConfig } from '$lib/server/auth-config';
import { localSessionToken } from '$lib/server/local-owner-auth';

const serverDsn = env.SENTRY_DSN?.trim();

if (serverDsn) {
  Sentry.init({
    dsn: serverDsn,
    environment: env.SENTRY_ENVIRONMENT?.trim() || undefined,
    release: env.SENTRY_RELEASE?.trim() || undefined,
    sendDefaultPii: false,
    tracesSampleRate: sentrySampleRate(env.SENTRY_TRACES_SAMPLE_RATE),
    maxValueLength: 1_000,
    beforeSend: sanitizeSentryEvent,
    beforeSendTransaction: sanitizeSentryEvent,
    beforeBreadcrumb: sanitizeSentryBreadcrumb
  });
}

const hostedHandle: Handle | null = workosConfig
  ? (() => {
      configureAuthKit(workosConfig);
      return authKitHandle({
        debug: false,
        onError: (error) => {
          if (serverDsn) {
            Sentry.captureException(error, {
              tags: { component: 'hosted-authentication' }
            });
          }
          console.error('Hosted authentication failed', { kind: safeErrorKind(error) });
        }
      });
    })()
  : null;

const applicationHandle: Handle = async ({ event, resolve }) => {
  event.locals.authMode = authMode;
  if (authMode === 'local') {
    event.locals.localSessionToken = localSessionToken(event.cookies) ?? undefined;
  }
  const response =
    authMode === 'workos' && hostedHandle
      ? await hostedHandle({ event, resolve })
      : await resolve(event);
  const privatePath =
    event.url.pathname.startsWith('/api/') ||
    event.url.pathname.startsWith('/auth/') ||
    event.url.pathname.startsWith('/dashboard') ||
    event.url.pathname.startsWith('/login') ||
    event.url.pathname.startsWith('/onboarding') ||
    event.url.pathname.startsWith('/pair') ||
    event.url.pathname.startsWith('/preview/') ||
    event.url.pathname === '/compare/qz-tray' ||
    event.url.pathname === '/compare/ezeep';
  if (privatePath || publicEnv.PUBLIC_MARKETING_INDEXABLE !== 'true') {
    response.headers.set('x-robots-tag', 'noindex, nofollow');
  }
  response.headers.set('x-content-type-options', 'nosniff');
  response.headers.set('referrer-policy', 'strict-origin-when-cross-origin');
  response.headers.set('permissions-policy', 'camera=(), microphone=(), geolocation=()');
  if (event.url.protocol === 'https:') {
    response.headers.set('strict-transport-security', 'max-age=31536000; includeSubDomains');
  }
  return response;
};

const sentryRequestHandle: Handle = serverDsn
  ? Sentry.sentryHandle()
  : async ({ event, resolve }) => resolve(event);

export const handle: Handle = sequence(sentryRequestHandle, applicationHandle);

const safeServerErrorHandler: HandleServerError = ({ error, event, status, message }) => {
  console.error('Unhandled server request error', {
    kind: safeErrorKind(error),
    route: event.route.id ?? 'unknown',
    status
  });
  return { message: publicErrorMessage(status, message) };
};

export const handleError: HandleServerError = serverDsn
  ? Sentry.handleErrorWithSentry(safeServerErrorHandler)
  : safeServerErrorHandler;
