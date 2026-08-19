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

## Piqae test runtime

`PIQAE_SHOPIFY_RUNTIME` is explicit: `fake` uses only the loopback virtual
printer environment, `local` targets a loopback or HTTPS self-hosted control
plane, and `live` requires an HTTPS Piqae endpoint. Live mode exercises the real
network and enrolled nodes, but it does not bypass Piqae's separate explicit
physical-printer authorization and destination confirmation controls. CI and
ordinary development use `fake`; staging may deliberately select `live`.
`PIQAE_SHOPIFY_STORAGE` defaults to durable PostgreSQL outside tests; the
in-memory repository is allowed only for explicit development fixtures and unit
tests, and is rejected in production.

## Production Shopify configuration

`shopify.app.toml` deliberately contains non-deployable placeholder values.
Extension UIDs are source-defined and checked in so the same extension maps
across development, staging, and production app instances. App client IDs and
origins live as GitHub environment variables; API secrets and app-scoped
automation tokens live as protected secrets. `scripts/render-release-config.mjs`
creates a temporary mode-`0600` configuration during deployment. Its
`application_url` must exactly match `SHOPIFY_APP_URL`, and its redirect,
webhook, app-proxy, and customer extension origins use the same HTTPS origin.

The authoritative daily development, Railway, pilot, production, rollback,
privacy, and App Store process is in
[`docs/operations/shopify-release.md`](../../docs/operations/shopify-release.md).

## PostgreSQL migration gate

Run the fresh-database and N−1 upgrade assertions against a disposable PostgreSQL database:

```console
PIQAE_TEST_DATABASE_URL=postgres://... pnpm --filter @piqae/shopify-app test:postgres
```

The test creates two randomly named `piqae_shopify_*` schemas and drops only those exact schemas. It never drops the database. Without `PIQAE_TEST_DATABASE_URL` the local command reports `SKIP` and succeeds. Release and CI jobs must also set `PIQAE_REQUIRE_POSTGRES_TESTS=1`, which turns a missing URL into a failure.
