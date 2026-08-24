import type { ClientOnErrorFunction } from "react-router";
import {
  resolveBrowserSentryConfiguration,
  sanitizeSentryBreadcrumb,
  sanitizeSentryEvent,
} from "./sentry";

declare global {
  interface Window {
    /** Bootstrap payload written by the root document, see `app/root.tsx`. */
    __piqaeSentry?: Record<string, string | undefined>;
  }
}

/** Browser reporting settings published by the root loader, if any. */
function browserSentryConfiguration() {
  return resolveBrowserSentryConfiguration(
    (typeof window === "undefined" ? undefined : window.__piqaeSentry) ?? {},
  );
}

/**
 * Browser-side Sentry initialization. Mirrors `apps/web/src/hooks.client.ts`:
 * without `PUBLIC_SENTRY_DSN` the SDK is never initialized and nothing is sent.
 * The SDK is a dynamic import so an unconfigured deployment does not ship it to
 * Shopify Admin at all.
 *
 * @returns the router `onError` reporter, or `undefined` when reporting is off.
 */
export async function initializeBrowserSentry(): Promise<
  ClientOnErrorFunction | undefined
> {
  const configuration = browserSentryConfiguration();
  if (!configuration) return undefined;

  const Sentry = await import("@sentry/react-router");
  Sentry.init({
    dsn: configuration.dsn,
    environment: configuration.environment,
    release: configuration.release,
    sendDefaultPii: false,
    tracesSampleRate: configuration.tracesSampleRate,
    maxValueLength: 1_000,
    integrations: [Sentry.reactRouterTracingIntegration()],
    beforeSend: sanitizeSentryEvent,
    beforeSendTransaction: sanitizeSentryEvent,
    beforeBreadcrumb: sanitizeSentryBreadcrumb,
  });
  return Sentry.sentryOnError;
}
