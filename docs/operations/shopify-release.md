# Shopify development, pilot, and release operations

**Status:** the source application and virtual/PostgreSQL gates are implemented.
The real-store pilot remains Preview until the external gates in
[`release/shopify-pilot-gates.yaml`](../../release/shopify-pilot-gates.yaml)
are recorded. App Store publication is a later Shopify-reviewed promotion.

This runbook is the source of truth for the hosted Shopify app. Shopify app
configuration/extensions and the Railway web runtime are separate release
surfaces and must identify the same reviewed commit.

## Environment model

| Target | Shopify app | Railway environment/service | Store and data |
| --- | --- | --- | --- |
| Local | developer-selected app | local Shopify CLI tunnel | development store, synthetic data, fake printer |
| Staging | separate development/staging app | `staging` / `piqae-shopify` | development store, isolated PostgreSQL, fake printer |
| Pilot/production | production app registration | `production` / `piqae-shopify` | explicitly approved real store, production PostgreSQL, live Piqae account |

Do not copy a production API secret, session database, encryption key, Piqae
credential, App Automation Token, or customer document into staging or a pull
request environment. Extension UIDs are source-defined and intentionally shared
between Shopify app instances; app client IDs and secrets are not.

The temporary single-store pilot may use Custom distribution. The permanent
App Store registration must use Public distribution because Shopify does not
allow changing the selected distribution method later. Custom distribution is
not an App Store publication path and cannot use Shopify billing. Set
`SHOPIFY_DISTRIBUTION=single_merchant` only on the custom pilot service; staging
and the permanent public app use `SHOPIFY_DISTRIBUTION=app_store`.

## One-time Shopify setup

1. In Shopify Dev Dashboard, create or select the staging and production app
   registrations. Use a dedicated development store for staging.
2. For a one-store pre-review pilot, choose Custom distribution on the pilot
   registration and restrict its installation link to the exact
   `*.myshopify.com` store. Keep the future public registration separate.
3. Set the application, callback, webhook, app-proxy, and customer extension
   origins to the matching HTTPS Railway origin.
4. Select only the protected customer fields required to render the supported
   documents. `read_all_orders` remains absent until Shopify separately approves
   it and the product documents the expanded history.
5. In each app's **Settings → App Automation Token**, create an app-scoped
   token. Store it once in the matching GitHub environment as
   `SHOPIFY_APP_AUTOMATION_TOKEN`. Tokens expire after at most six months;
   create the replacement, update GitHub, prove a staging deploy, then revoke
   the old token.
6. Link the app once with Shopify CLI to verify that the checked-in extension
   handles/UIDs map as updates. Stop if a deploy proposes removing and
   re-creating an extension.
7. Generate the install link for the pilot app. The store owner must review the
   scopes and approve installation interactively.

The repository deliberately keeps the client ID out of the checked-in default
configuration. GitHub environments contain non-secret `SHOPIFY_CLIENT_ID` and
`SHOPIFY_APP_URL` variables. The workflow renders a mode-`0600` temporary
`shopify.app.release.toml`, validates it, and never uploads it as evidence.

## One-time GitHub setup

Create `shopify-staging` and `shopify-production` environments.

| Setting | Staging | Production |
| --- | --- | --- |
| `SHOPIFY_CLIENT_ID` variable | staging app client ID | production/pilot app client ID |
| `SHOPIFY_APP_URL` variable | staging HTTPS origin | `https://shopify.piqae.com` |
| `SHOPIFY_APP_AUTOMATION_TOKEN` secret | staging token | production/pilot token |
| Required reviewers | none | release owner |
| Deployment branches | `main` | `main` and `shopify-v*` tags only |

`main` remains protected by `CI result` and `Supply-chain result`. Do not add
the path-scoped `Shopify` job as an independently required check because it is
correctly skipped for unrelated changes.

Keep repository variable `SHOPIFY_STAGING_ENABLED=false` until the staging
Shopify app, Railway service, domain, database, variables, and automation token
are all healthy. Set it to `true` only after a manual staging deployment passes;
the post-CI workflow otherwise skips cleanly.

## One-time Railway setup

Both environments use `/railway.shopify.toml`,
`deploy/docker/Dockerfile.shopify`, `/healthz`, one always-running replica, and
the repository source. Enable GitHub check-suite waiting and these watch paths:

```text
/apps/shopify/**
/sdk/typescript/**
/deploy/docker/Dockerfile.shopify
/railway.shopify.toml
/package.json
/pnpm-lock.yaml
/pnpm-workspace.yaml
```

Required protected runtime variables are documented in
[`apps/shopify/.env.example`](../../apps/shopify/.env.example). Staging uses an
isolated database and `PIQAE_SHOPIFY_RUNTIME=fake`. Production uses
`PIQAE_SHOPIFY_RUNTIME=live`, but that setting is not authorization for a
physical print. Do not enable autosleep: OAuth callbacks, webhooks, and customer
document links require an available service.

Railway injects `RAILWAY_GIT_COMMIT_SHA`. `/healthz` returns that revision, and
the GitHub deploy workflow refuses to update Shopify until it equals the
reviewed source commit.

## Daily development

```console
git switch main
git pull --ff-only
git switch -c feat/shopify-short-description
pnpm install --frozen-lockfile
pnpm --filter @piqae/sdk build
pnpm --filter @piqae/shopify-app check
pnpm --filter @piqae/shopify-app test
pnpm --filter @piqae/shopify-app build
cd apps/shopify
pnpm --package=@shopify/cli@4.6.1 dlx shopify app dev --config development
```

Use synthetic orders and fake/virtual printers. Commit with DCO sign-off, open a
pull request, and merge only after the aggregate checks pass. A successful
post-merge `CI` run invokes **Deploy Shopify staging** for that exact SHA.

## Production/pilot release

1. Confirm the intended commit is merged to `main` and its staging deployment
   passed.
2. Confirm Railway production has deployed that same commit and `/healthz`
   reports it. Source changes are backward compatible with the currently
   released Shopify extensions.
3. Open **Actions → Deploy Shopify app → Run workflow**.
4. Select `production`, enter the full 40-character reviewed commit SHA, enter a SemVer such as
   `0.2.0-beta.1`, and type `RELEASE-SHOPIFY`.
5. Approve the protected `shopify-production` environment.
6. The workflow re-runs bounded tests, verifies runtime identity, renders and
   validates config, creates an unreleased app version, releases that exact
   version, uploads non-secret evidence, then creates `shopify-v<version>` and a
   GitHub release.
7. Verify install/reopen, one synthetic order, duplicate submission behavior,
   webhook receipt, and authenticated PDF fallback. Record only opaque IDs and
   outcomes—never documents, credentials, customer data, or signed URLs.

Automatic CI uses `--allow-updates`. Extension/config deletion is intentionally
unsupported. A required deletion is a separate reviewed maintenance operation
after impact and recovery have been documented.

## Database changes

Shopify migrations are append-only and must remain compatible with the running
and candidate application versions. Every schema change requires fresh-database,
N−1 upgrade, and tenant-isolation tests:

```console
PIQAE_REQUIRE_POSTGRES_TESTS=1 \
PIQAE_TEST_DATABASE_URL=postgres://... \
  pnpm --filter @piqae/shopify-app test:postgres
```

The Railway pre-deploy command runs the migration before the app starts.
Application rollback never runs a down-migration.

## Rollback

1. Select the previous successful Railway deployment and redeploy it. Verify
   terminal `SUCCESS` and `/healthz`; do not infer success from a queued build.
2. In Shopify Dev Dashboard, release the previous known-good app version, or
   use `shopify app release --config release --version <exact-version>` with a
   valid app-scoped token.
3. Do not reverse migrations automatically. If the previous runtime is not
   compatible with the expanded schema, roll forward with a fix.
4. Reconcile accepted/printing/reported-complete/uncertain jobs by durable event
   evidence. Never bulk-resubmit uncertain jobs.
5. Record cause, affected release identities, actions, and verification without
   customer data.

## Store lifecycle and privacy

Webhook HMAC verification is provided by Shopify's server library. Operational
webhooks are durably deduplicated by webhook ID. `app/uninstalled` removes the
shop repository and sessions; `shop/redact` removes installation/session data;
`customers/redact` removes retained customer render inputs; and
`customers/data_request` is handled according to the documented no-profile
storage model.

Before App Store submission, prove all four flows on a development store and
the pilot, publish privacy/terms/support URLs, complete protected-customer-data
review, configure Shopify App Pricing, supply reviewer credentials and an
English demo, run Shopify's automated checks, and keep the initial listing at
Limited visibility during the soft launch.

## Evidence and support claims

`release/shopify-pilot-gates.yaml` is the checked-in checklist. Code presence,
a healthy HTTP response, a Shopify version, or a virtual spooler acceptance is
not proof of physical delivery or public support. Keep
`release/support-matrix.yaml` at Disabled until every enabled claim has the
required external evidence; then promote only to Preview until the documented
soak and security gates are complete.
