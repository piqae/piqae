# Piqae Shopify app

Embedded Shopify React Router app for direct order-document printing through
Piqae, with an authenticated PDF fallback. It targets Shopify Admin GraphQL and
webhook API `2026-07`; REST Admin API is not used.

The checked-in configuration contains placeholders only. Production startup is
fail-closed until a durable, tenant-fenced `ShopRepository` and Shopify durable
session storage are supplied. Piqae child-account credentials are encrypted at
rest with AES-256-GCM and shop-bound associated data; the wrapping key belongs in
the deployment secret manager.

Printable endpoints authenticate Shopify session tokens and derive the shop
from that context. Callers cannot provide a shop domain. Order IDs are validated
as Shopify Order GIDs and re-fetched with the authenticated GraphQL client.

The configuration requests both `read_orders` and Shopify-approved
`read_all_orders` because historical invoice reprints and exports are core app
features. Existing installations receive the expanded history only after the
updated Shopify app version is released and the merchant approves the added
required scope. The runtime must continue to inspect granted scopes and explain
the standard 60-day window until that approval is reflected on the installation.

No real Shopify or Piqae credentials are needed for unit tests.

## Render location policy

Each shop has a tenant-scoped document-rendering policy. `automatic` is the
default and lets Piqae choose the faster compatible path from bounded measured
costs. `cloud_only` always prints the exact approved preview PDF.
`prefer_node` uses node rendering only when the destination's renderer ABI and
content-addressed resources are ready, otherwise it safely falls back to that
PDF. `require_node` is an advanced fail-closed mode: printing is unavailable
until the selected node proves compatibility and resource readiness. An online
printer alone is never treated as node-render ready.

The setting affects execution location, not delivery truth. Native spooler
acceptance still does not prove that paper was produced.

## Document and print-target authority

Each merchant document keeps an editable draft separately from its immutable
published revision. Saving a draft uses compare-and-swap revision checks and
never changes the document currently used by Admin, POS, automations, or print
handoff. Publishing snapshots the exact PrintPacket source, document media,
target id, and target design-specification revision. A stale editor receives a
conflict response and must reload instead of overwriting newer work.

When a document is pinned, the target is the authority for its immutable
printer/profile binding and business stock. The editor sizes its canvas from
`document.media`, then offers only compatible targets. Loaded-media evidence is
operational truth reported by Piqae: absent, stale, untrusted, or mismatched
evidence is shown distinctly and cannot be fabricated from profile dimensions.
Printing sends the saved target id and exact design-specification revision so
the control plane can revalidate the binding and stock immediately before
handoff. Unpinned documents instead use a directly selected printer and its
current operating-system/driver defaults; saved profiles are optional and are
never selected implicitly. A pinned document never silently falls back to this
automatic path.

## Editor preview data

Opening an editor never fetches an order or serializes buyer data into its HTML.
Preview is an explicit authenticated action: it validates the bounded current
draft, selects and hydrates the latest accessible order on the server, and asks
Piqae's canonical renderer for a short-lived PDF. The browser receives only an
opaque same-origin artifact URL; customer, address, line-item, and metafield
data remain server-side. Leaving Preview removes the PDF from the page and stale
responses are ignored.

Preview therefore uses the same PrintPacket/PDF semantics as printing. It does
not mount the visual editor canvas or maintain a second partial evaluator, and
it never saves, publishes, or otherwise mutates the merchant's current draft.

## Piqae test runtime

`PIQAE_SHOPIFY_RUNTIME` is explicit: `fake` uses only the loopback virtual
printer environment, `local` targets a loopback or HTTPS self-hosted control
plane, and `live` requires an HTTPS Piqae endpoint. Live mode exercises the real
network and enrolled nodes, but it does not bypass Piqae's separate explicit
physical-printer authorization and destination confirmation controls. CI and
ordinary development use `fake`; only an explicitly approved hosted pilot uses
`live`.
`PIQAE_SHOPIFY_STORAGE` defaults to durable PostgreSQL outside tests; the
in-memory repository is allowed only for explicit development fixtures and unit
tests, and is rejected in production.

## Production Shopify configuration

`shopify.app.toml` deliberately contains non-deployable placeholder values.
Extension UIDs are source-defined and checked in so CLI Dev Store previews and
released versions update the same permanent public app. Its client ID and
origin live as GitHub environment variables; API secrets and app-scoped
automation tokens live as protected secrets. `scripts/render-release-config.mjs`
creates a temporary mode-`0600` configuration during deployment. Its
`application_url` must exactly match `SHOPIFY_APP_URL`, and its redirect,
webhook, app-proxy, and customer extension origins use the same HTTPS origin.

The authoritative daily development, Railway, pilot, production, rollback,
privacy, and App Store process is in
[`docs/operations/shopify-release.md`](../../docs/operations/shopify-release.md).

### Shopify app icon

The Shopify-ready 1200 × 1200 PNG is
[`public/piqae-shopify-app-icon-1200.png`](public/piqae-shopify-app-icon-1200.png).
Shopify does not read an app icon from `shopify.app.toml`: upload this asset
manually in the app's **Dev Dashboard → Settings → App icon**. Releasing the
application or its extensions does not update that dashboard asset.

## Error reporting

Sentry is gated on a configured DSN, the same way `apps/web` gates it. With
`SENTRY_DSN` unset the SDK is never initialized on the server, and with
`PUBLIC_SENTRY_DSN` unset nothing is initialized in the browser and no DSN is
serialized into the embedded Admin document. Environment, release, and trace
sample rate use the `SENTRY_*` and `PUBLIC_SENTRY_*` names shared with the web
service.

`app/observability/sentry.ts` is a port of
`apps/web/src/lib/observability/sentry.ts`. The two apps are separate workspace
packages on different frameworks and Sentry SDKs, so the rules are duplicated
rather than imported across the app boundary. On top of the shared redaction it
scrubs Shopify-specific identity before an event leaves the process:

- the `*.myshopify.com` shop domain, which names one merchant, becomes `[shop]`;
- Shopify GIDs keep their resource kind and lose the record id;
- Shopify access tokens (`shpat_`/`shpca_`/`shppa_`/`shpss_`/`shpua_`) and App
  Bridge session tokens (JWTs) are removed, along with `hmac` and `id_token`
  pairs;
- buyer fields — customer, names, email, phone, addresses, city, postcode,
  province, and order notes — are dropped by key;
- legacy numeric Shopify identifiers in URL paths become `:id`.

`user`, `server_name`, request headers, request bodies, query strings, stack
frame locals, console breadcrumbs, and UI interaction breadcrumbs are never
sent. `tests/sentry.test.ts` asserts on the real redaction output.

## PostgreSQL migration gate

Run the fresh-database and N−1 upgrade assertions against a disposable PostgreSQL database:

```console
PIQAE_TEST_DATABASE_URL=postgres://... pnpm --filter @piqae/shopify-app test:postgres
```

The test creates two randomly named `piqae_shopify_*` schemas and drops only those exact schemas. It never drops the database. Without `PIQAE_TEST_DATABASE_URL` the local command reports `SKIP` and succeeds. Release and CI jobs must also set `PIQAE_REQUIRE_POSTGRES_TESTS=1`, which turns a missing URL into a failure.
