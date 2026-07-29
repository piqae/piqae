import * as Sentry from '@sentry/sveltekit';
import { env } from '$env/dynamic/public';
import type { HandleClientError } from '@sveltejs/kit';
import {
  publicErrorMessage,
  safeErrorKind,
  sanitizeSentryBreadcrumb,
  sanitizeSentryEvent,
  sentrySampleRate
} from '$lib/observability/sentry';

const clientDsn = env.PUBLIC_SENTRY_DSN?.trim();

if (clientDsn) {
  Sentry.init({
    dsn: clientDsn,
    environment: env.PUBLIC_SENTRY_ENVIRONMENT?.trim() || undefined,
    sendDefaultPii: false,
    tracesSampleRate: sentrySampleRate(env.PUBLIC_SENTRY_TRACES_SAMPLE_RATE),
    maxValueLength: 1_000,
    beforeSend: sanitizeSentryEvent,
    beforeSendTransaction: sanitizeSentryEvent,
    beforeBreadcrumb: sanitizeSentryBreadcrumb
  });
}

const safeClientErrorHandler: HandleClientError = ({ error, status, message }) => {
  console.error('Unhandled browser error', {
    kind: safeErrorKind(error),
    status
  });
  return { message: publicErrorMessage(status, message) };
};

export const handleError: HandleClientError = clientDsn
  ? Sentry.handleErrorWithSentry(safeClientErrorHandler)
  : safeClientErrorHandler;
