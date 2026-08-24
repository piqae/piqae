import type { ReactNode } from "react";
import {
  isRouteErrorResponse,
  Links,
  Meta,
  Outlet,
  Scripts,
  ScrollRestoration,
  useRouteError,
  useRouteLoaderData,
} from "react-router";
import type { LinksFunction } from "react-router";
import styles from "./shopify-ui.css?url";
import {
  browserSentryBootstrapScript,
  browserSentryEnvironment,
  publicErrorMessage,
  sentryIngestOrigin,
} from "./observability/sentry";

export const links: LinksFunction = () => [{ rel: "stylesheet", href: styles }];

/**
 * Only `PUBLIC_`-prefixed browser reporting settings are serialized into the
 * embedded Admin document. The server `SENTRY_DSN` is never exposed.
 */
export function loader() {
  return { sentry: browserSentryEnvironment(process.env) };
}

export const headers = () => {
  const ingest = sentryIngestOrigin(process.env.PUBLIC_SENTRY_DSN);
  const connectSources = [
    "'self'",
    "https://api.piqae.com",
    "https://partners.shopify.com",
    ...(ingest ? [ingest] : []),
  ].join(" ");
  return {
    "content-security-policy":
      `default-src 'self'; script-src 'self' 'unsafe-inline' https://cdn.shopify.com; style-src 'self' 'unsafe-inline' https://cdn.shopify.com; font-src 'self' https://cdn.shopify.com; img-src 'self' data: https:; connect-src ${connectSources}; ` +
      "frame-ancestors https://admin.shopify.com https://*.myshopify.com; base-uri 'self'; object-src 'none'",
    "referrer-policy": "strict-origin-when-cross-origin",
    "x-content-type-options": "nosniff",
  };
};

export function Layout({ children }: { children: ReactNode }) {
  const data = useRouteLoaderData<typeof loader>("root");
  const bootstrap = browserSentryBootstrapScript(data?.sentry ?? null);
  return (
    <html lang="en">
      <head>
        <meta charSet="utf-8" />
        <meta name="viewport" content="width=device-width,initial-scale=1" />
        <Meta />
        <Links />
        <script src="https://cdn.shopify.com/shopifycloud/app-bridge.js" />
        <script src="https://cdn.shopify.com/shopifycloud/polaris.js" />
      </head>
      <body>
        {bootstrap ? (
          <script dangerouslySetInnerHTML={{ __html: bootstrap }} />
        ) : null}
        {children}
        <ScrollRestoration />
        <Scripts />
      </body>
    </html>
  );
}

export default function Root() {
  return <Outlet />;
}

/**
 * Errors are reported by `entry.server.tsx` (server) and by the Sentry
 * `onError` hook in `entry.client.tsx` (browser). This boundary only renders a
 * message that is safe to show inside Shopify Admin.
 */
export function ErrorBoundary() {
  const error = useRouteError();
  const routeError = isRouteErrorResponse(error) ? error : null;
  const status = routeError?.status ?? 500;
  const message = publicErrorMessage(
    status,
    routeError?.statusText || "The request could not be completed.",
  );
  return (
    <main>
      <h1>Something went wrong</h1>
      <p>{message}</p>
    </main>
  );
}
