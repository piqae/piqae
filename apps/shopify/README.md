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

The default development configuration requests `read_orders`, which covers
Shopify's standard order-history window. Historical invoice reprints and exports
beyond that window require the protected `read_all_orders` scope. Add that scope
to the production configuration only after Shopify approves it. Until approval
is present, the product must explicitly describe and enforce the shorter history;
it must not imply that older orders are accessible.

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

The target is the authority for its immutable printer/profile binding and
business stock. The editor sizes its canvas from `document.media`, then offers
only compatible targets. Loaded-media evidence is operational truth reported by
Piqae: absent, stale, untrusted, or mismatched evidence is shown distinctly and
cannot be fabricated from profile dimensions. Printing sends the saved target
id and exact design-specification revision so the control plane can revalidate
the binding and stock immediately before handoff.

## Editor preview data

The in-editor Preview is a source and layout preview. It renders one
representative pass through collection-driven content and keeps Shopify data
bindings visible; it never fetches customer records merely because a merchant
opens an editor. The authenticated print flow remains the authority for a
data-resolved PDF.

A recent-order preview of an unsaved draft requires a server-render endpoint
that accepts that bounded draft plus authenticated order input. The current
preview endpoint intentionally accepts only immutable published revisions, and
the browser does not carry a second, partial PrintPacket evaluator. Until that
contract exists, the editor must not imitate a rendered order with a divergent
client-side implementation or serialize buyer data into the page by default.

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
