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
| Development/pilot | the public app registration in CLI dev-preview mode | local Shopify CLI tunnel | Shopify Dev Store, synthetic data, fake printer |
| Production | the same public app registration, released app version | `production` / `piqae-shopify` | approved merchant stores, production PostgreSQL, live Piqae account |

Do not copy a production API secret, session database, encryption key, Piqae
credential, App Automation Token, or customer document into a local or pull
request environment. The hosted runtime uses `SHOPIFY_DISTRIBUTION=app_store`.

This is deliberately a one-app lifecycle. A draft public app can be installed
on a Dev Store owned by the same developer organization, but not on an ordinary
paid merchant store. Testing on a paid store before App Store approval would
require a separate Custom-distribution app, and Shopify does not permit changing
that app to Public distribution later. For a solo developer, use one public app
and one Dev Store, submit it for review, then use Limited visibility for the
first approved live-store pilot.

## One-time Shopify setup

1. In Shopify Dev Dashboard, create or select the single permanent app and
   choose Public distribution. This choice cannot be changed later.
2. Create a Dev Store in the Dev Dashboard and install the draft app there.
   Do not use a client-transfer store; draft public apps are not installable on
   that store type.
3. Set the application, callback, webhook, app-proxy, and customer extension
   origins to the matching HTTPS Railway origin.
4. Select only the protected customer fields required to render the supported
   documents. Piqae's `read_all_orders` access is approved and declared beside
   `read_orders`; retain the approval evidence with the Shopify release record.
   Existing stores are prompted for the added required scope when they next open
   the app after that configuration version is released.
5. In the app's **Settings → App Automation Token**, create an app-scoped
   token. Store it once in the GitHub production environment as
   `SHOPIFY_APP_AUTOMATION_TOKEN`. Tokens expire after at most six months;
   create the replacement, update GitHub, validate a no-release version, then revoke
   the old token.
6. Link the app once with Shopify CLI to verify that the checked-in extension
   handles/UIDs map as updates. Stop if a deploy proposes removing and
   re-creating an extension.
7. Before review, install only through the app's **Installs** section or
   `shopify app dev` onto the Dev Store. After approval, set the listing to
   Limited visibility and share its App Store URL with the first pilot merchant.

The repository deliberately keeps the client ID out of the checked-in default
configuration. GitHub environments contain non-secret `SHOPIFY_CLIENT_ID` and
`SHOPIFY_APP_URL` variables. The workflow renders a mode-`0600` temporary
`shopify.app.release.toml`, validates it, and never uploads it as evidence.

## One-time GitHub setup

Create only the `shopify-production` environment.

| Setting | Value |
| --- | --- |
| `SHOPIFY_CLIENT_ID` variable | permanent public app client ID |
| `SHOPIFY_APP_URL` variable | `https://shopify.piqae.com` |
| `SHOPIFY_APP_AUTOMATION_TOKEN` secret | permanent public app token |
| Required reviewers | release owner |
| Deployment branches | `main` and `shopify-v*` tags only |

`main` remains protected by `CI result` and `Supply-chain result`. Do not add
the path-scoped `Shopify` job as an independently required check because it is
correctly skipped for unrelated changes.

## One-time Railway setup

The production service uses `/railway.shopify.toml`,
`deploy/docker/Dockerfile.shopify`, `/healthz`, one always-running replica, and
the repository source. Enable GitHub check-suite waiting and these watch paths:

```text
/apps/shopify/**
/sdk/typescript/**
/sdk/printpacket/**
/deploy/docker/Dockerfile.shopify
/railway.shopify.toml
/package.json
/pnpm-lock.yaml
/pnpm-workspace.yaml
```

Required protected runtime variables are documented in
[`apps/shopify/.env.example`](../../apps/shopify/.env.example). Production uses
`PIQAE_SHOPIFY_RUNTIME=live`, but that setting is not authorization for a
physical print. Do not enable autosleep: OAuth callbacks, webhooks, and customer
document links require an available service.

For a repository-linked deployment, Railway injects
`RAILWAY_GIT_COMMIT_SHA`. A local-source `railway up` uploads the checkout but
does **not** refresh that repo-linked value. Before or with every local-source
production deploy, set the Shopify service variable `PIQAE_RELEASE_SHA` to the
exact full 40-character reviewed commit SHA, deploy from a clean checkout of
that commit, wait for terminal `SUCCESS`, and verify that `/healthz` reports the
same revision. Never use a branch, tag, `latest`, or a locally modified tree as
release identity. `PIQAE_RELEASE_SHA` takes precedence over Railway's value; if
the service returns to repo-linked deploys, remove it or update it on every
deploy so a stale override cannot pass as current evidence. The GitHub deploy
workflow refuses to update Shopify until `/healthz` equals the reviewed source
commit.

## One-time Piqae platform setup

The production operator role account is `admin@piqae.com`. This is an identity
reference only; passwords, recovery material, sessions and platform keys never
belong in this repository or an operations ticket.

1. Sign in to `https://app.piqae.com/dashboard/settings` as that role account.
2. Enable the Advanced platform integration and capture its one-time
   `piq_platform_…` service-account key.
3. Store it only as the protected Railway Shopify-service variable
   `PIQAE_SHOPIFY_PLATFORM_KEY`.
4. Verify an installed Dev Store receives an isolated child workspace, Live and
   Test environments, starter templates, and a short-lived node connection
   session without asking the merchant for a Piqae credential.

The role account owns the integration; individual Shopify merchants do not
create Piqae accounts. Use `/auth/logout?return_to=/login` to change dashboard
operator identity.

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

`app dev` applies development configuration only to the selected Dev Store and
does not create a released production version. Use synthetic orders and
fake/virtual printers. Commit with DCO sign-off, open a pull request, and merge
only after the aggregate checks pass.

## Production/pilot release

1. Confirm the intended commit is merged to `main` and was tested through
   `shopify app dev` on the Dev Store.
2. Confirm Railway production has deployed that same commit and `/healthz`
   reports its exact full SHA. For a local-source `railway up`, set
   `PIQAE_RELEASE_SHA` to that reviewed SHA before or with the deploy; do not
   trust `RAILWAY_GIT_COMMIT_SHA` for a local upload. Source changes are
   backward compatible with the currently released Shopify extensions.
3. Open **Actions → Deploy Shopify app → Run workflow**.
4. Enter the full 40-character reviewed commit SHA, enter a SemVer such as
   `0.2.0-beta.1`, and type `RELEASE-SHOPIFY`.
5. Approve the protected `shopify-production` environment.
6. The workflow re-runs bounded tests, verifies runtime identity, renders and
   validates config, creates an unreleased app version, releases that exact
   version, independently verifies that Shopify reports it active, uploads
   non-secret evidence, then creates `shopify-v<version>` and a GitHub release.
   Shopify CLI can render a release error while exiting successfully, so its
   process exit status alone is never accepted as release evidence. A retry
   resumes an exact-name inactive candidate and still requires that candidate
   to become active before publishing evidence.
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

Before App Store submission, prove all four flows on the Dev Store, publish
privacy/terms/support URLs, complete protected-customer-data
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
