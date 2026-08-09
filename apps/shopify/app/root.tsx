import { Links, Meta, Outlet, Scripts, ScrollRestoration } from "react-router";
import type { LinksFunction } from "react-router";
import styles from "./shopify-ui.css?url";

export const links: LinksFunction = () => [{ rel: "stylesheet", href: styles }];
export const headers = () => ({
  "content-security-policy":
    "default-src 'self'; script-src 'self' 'unsafe-inline' https://cdn.shopify.com; style-src 'self' 'unsafe-inline'; img-src 'self' data: https:; connect-src 'self' https://api.piqae.com https://partners.shopify.com; frame-ancestors https://admin.shopify.com https://*.myshopify.com; base-uri 'self'; object-src 'none'",
  "referrer-policy": "strict-origin-when-cross-origin",
  "x-content-type-options": "nosniff",
});

export default function Root() {
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
        <Outlet />
        <ScrollRestoration />
        <Scripts />
      </body>
    </html>
  );
}
