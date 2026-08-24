import * as Sentry from "@sentry/react-router";
import {
  resolveServerSentryConfiguration,
  sanitizeSentryBreadcrumb,
  sanitizeSentryEvent,
} from "./sentry";

/**
 * Server-side Sentry initialization. Mirrors `apps/web/src/hooks.server.ts`:
 * everything is gated on a configured DSN, so with no `SENTRY_DSN` the SDK is
 * never initialized, no transport is created, and no event can leave the host.
 */
const configuration = resolveServerSentryConfiguration(process.env);

export const serverSentryEnabled = configuration !== null;

if (configuration) {
  Sentry.init({
    dsn: configuration.dsn,
    environment: configuration.environment,
    release: configuration.release,
    sendDefaultPii: false,
    tracesSampleRate: configuration.tracesSampleRate,
    maxValueLength: 1_000,
    beforeSend: sanitizeSentryEvent,
    beforeSendTransaction: sanitizeSentryEvent,
    beforeBreadcrumb: sanitizeSentryBreadcrumb,
  });
}

/** Report a handled failure without leaking merchant or buyer data. */
export function captureServerException(
  error: unknown,
  component: string,
): void {
  if (!serverSentryEnabled) return;
  Sentry.captureException(error, { tags: { component } });
}
