# Railway low-cost private preview

**Status:** operational private preview. This shape is suitable for Piqae's own
early use and controlled design partners. It is not approved as a production,
high-availability, or 99.95% deployment.

Railway is the low-cost launch path between local Compose and the managed
multi-region design. It runs the same server image and preserves a move to
Cloud Run or Helm without introducing a second application architecture.

## Current shape

```text
public Railway web service
             |
             v
public Railway API (one replica, PIQAE_SERVICE_ROLE=api)
             |                         |
             v                         v
      Railway PostgreSQL       piqae-documents bucket
             ^
             |
private Railway worker (one replica, PIQAE_SERVICE_ROLE=worker)

private Cloudflare R2 piqae-releases bucket -> web-owned short-lived download redirects
```

- The web and API services have public Railway domains behind the Piqae custom
  domains.
- The worker has no public domain. It runs webhook and, when enabled, billing
  outbox work against the same PostgreSQL and object store.
- Node sync currently uses the public API. A separate `sync` Railway service
  would run the same routes and add cost without creating path-level isolation,
  so it is intentionally omitted from this preview.
- PostgreSQL is the durable source for jobs, events, leases, idempotency,
  memberships, webhooks, and object references. The bucket holds print
  documents. The node's SQLite queue remains a separate local durability
  boundary.
- The first private deployment keeps API, worker, and PostgreSQL in Railway's
  US West region. Its Railway bucket is in Singapore. This is operational but
  adds document latency and transfer distance; measure it and migrate to a
  colocated object store before opening the service broadly.
- Native installers and update metadata use the separate private Cloudflare R2
  `piqae-releases` bucket. Never grant the web or a release publisher access to customer print
  objects merely because both stores expose an S3-compatible API.

Keep the API and worker at one always-running replica. Autosleep is a poor fit
for a print control plane because node polling and job pickup are
latency-sensitive. One small warm replica is the useful low-cost floor; it
avoids paying for idle multi-region capacity without introducing a cold start
into every reconnect.

## Source-controlled deployment

The root [`railway.toml`](../../railway.toml) selects
[`deploy/docker/Dockerfile.server`](../../deploy/docker/Dockerfile.server) and
gates the public service on `/v1/ready`.

The web service selects `/railway.web.toml` and gates on `/healthz`. The API and
worker use the same server image and release. Configure their service-specific
values separately:

| Variable | API | Worker |
| --- | --- | --- |
| `PIQAE_SERVICE_ROLE` | `api` | `worker` |
| `PIQAE_RUN_MIGRATIONS_ON_STARTUP` | `false` | `false` |
| Public domain | enabled | disabled |
| Replica floor | 1 | 1 |

The server still binds an HTTP listener in worker mode, but that listener must
remain private.

### Staging before production

Railway is the canonical hosted web and control-plane deployment target for
the private preview. Keep `staging` and `production` as isolated Railway
environments:

- pull requests run repository CI and may create disposable Railway PR
  environments;
- a reviewed merge to `main` deploys the web, API, and worker to `staging`;
- staging uses its own PostgreSQL instance, object bucket, release
  configuration, domains, and device identities;
- staging stays `noindex`, uses local-owner identity while hosted identity is
  unverified, and keeps billing disabled;
- production is promoted from the exact commit tested in staging through a
  manual release action; it must not rebuild from a different source state;
- production migration, API, worker, and web promotion follow the order below.

Do not connect a PR environment to production PostgreSQL, either production
bucket, production WorkOS/Stripe/Sentry credentials, or real customer nodes.
A preview environment is disposable and is not a suitable update-feed origin.

GitHub remains the source and required check surface. Railway supplies build
and deployment automation after those checks pass. Replacing the GitHub-hosted
runner with Blacksmith changes where Actions jobs execute; it does not remove
GitHub or its status checks from the deployment chain.

## Configuration contract

Set these non-secret values on both services unless the table above overrides
them:

```text
PIQAE_BIND=0.0.0.0:8080
PIQAE_DATABASE_URL=${{Postgres.DATABASE_URL}}
PIQAE_OBJECT_STORE=s3
PIQAE_S3_ENDPOINT=<Railway bucket endpoint>
PIQAE_S3_BUCKET=<Railway bucket name>
PIQAE_S3_REGION=<Railway bucket region>
PIQAE_S3_ALLOW_HTTP=false
PIQAE_S3_VIRTUAL_HOSTED_STYLE=false
PIQAE_DEPLOYMENT=self_hosted
PIQAE_IDENTITY_PROVIDER=local_owner
PIQAE_AUTH_MODE=bootstrap
PIQAE_BILLING_ENABLED=false
PIQAE_LOCAL_OWNER_SESSION_SECONDS=86400
```

Set these as protected Railway variables, never in Git, build arguments,
domains, or logs:

```text
PIQAE_S3_ACCESS_KEY_ID
PIQAE_S3_SECRET_ACCESS_KEY
PIQAE_WEBHOOK_MASTER_KEY
PIQAE_LOCAL_OWNER_BOOTSTRAP_TOKEN  # one-time only; remove after bootstrap
```

`PIQAE_WEBHOOK_MASTER_KEY` must be a base64-encoded 32-byte key. Use the
database service reference for `PIQAE_DATABASE_URL` instead of copying its
resolved credential into another configuration surface.

### Railway bucket addressing

For the current `t3.storageapi.dev` Railway bucket endpoint, Piqae must use
path-style S3 requests:

```text
PIQAE_S3_VIRTUAL_HOSTED_STYLE=false
```

Do not copy a credential response's `urlStyle` field directly into this Piqae
flag. With this endpoint and Piqae's current S3 client, virtual-hosted mode
misaddresses the readiness key and returns an authorization error. Verify the
integration with a non-customer object:

```console
cargo run -p piqae-object-store --example seed_readiness
```

Run that command only in a protected environment containing the bucket
variables. It writes `health/readiness-probe` and prints no credentials.
`/v1/ready` subsequently exercises PostgreSQL plus an object-store existence
request. `/v1/health` checks only the server process.

## Identity bootstrap

This preview deliberately uses local-owner identity. WorkOS and Stripe stay
disabled until their live tenant, session, webhook, and billing evidence gates
pass.

1. Generate a high-entropy `PIQAE_LOCAL_OWNER_BOOTSTRAP_TOKEN` in a protected
   secret store.
2. Use it once with `/v1/identity/local/bootstrap`.
3. Put the returned owner credential in the operator's password manager or OS
   secret store.
4. Delete `PIQAE_LOCAL_OWNER_BOOTSTRAP_TOKEN` from both Railway services and
   redeploy.
5. Create scoped API keys through the authenticated application; do not retain
   a legacy bootstrap API key as the normal integration credential.

The dashboard runs as a normal Node service on Railway. Its service selects
`/railway.web.toml`, builds
[`deploy/docker/Dockerfile.web`](../../deploy/docker/Dockerfile.web), and uses
`/healthz` for the deployment gate. Browser identity and control-plane service
credentials stay server-side.

Set:

```text
ORIGIN=https://<web-service-domain>
PIQAE_AUTH_MODE=local
PUBLIC_PIQAE_DASHBOARD_MODE=live
PUBLIC_PIQAE_API_URL=https://<api-service-domain>
PUBLIC_SITE_URL=https://<web-service-domain>
PUBLIC_MARKETING_INDEXABLE=false
PIQAE_COOKIE_SECURE=true
STRIPE_CHECKOUT_ENABLED=false
```

`ORIGIN` must exactly match the public HTTPS dashboard origin so adapter-node
can reject forged cross-origin form submissions. Do not enable WorkOS or
Stripe merely to move the web runtime. Add their protected server variables
only after the corresponding identity or billing evidence gate passes.

The web service is stateless and may later scale independently. Its `/healthz`
route proves only that the SvelteKit process can serve requests; the dashboard
continues to display control-plane failures rather than hiding them behind its
own liveness response.

### Native release origin

Configure the web service with dedicated origin credentials for the
`piqae-releases` bucket:

```text
PIQAE_RELEASES_S3_ENDPOINT
PIQAE_RELEASES_S3_ACCESS_KEY_ID
PIQAE_RELEASES_S3_SECRET_ACCESS_KEY
PIQAE_RELEASES_S3_BUCKET=piqae-releases
PIQAE_RELEASES_S3_REGION
PIQAE_RELEASES_S3_VIRTUAL_HOSTED_STYLE
```

The public routes map constrained filenames to
`native/<stable|preview>/<artifact>` and issue short-lived redirects. Missing
configuration or an absent object returns not found. Release publishers use
different write-capable credentials. The web path performs reads only through
an R2 `Object Read only` token scoped to this bucket. Never use the web
credential to publish or replace an appcast.

See [Native release publishing](native-release-publishing.md). No stable native
download exists merely because these routes and credentials are configured.

## Migrations and releases

Ordinary API and worker replicas must not run schema changes. For the initial
empty database only, one API replica may start once with
`PIQAE_RUN_MIGRATIONS_ON_STARTUP=true`; turn it off immediately after that
deployment succeeds.

For every later release:

1. Back up PostgreSQL and referenced objects.
2. Run the candidate image once with `piqae-server migrate`, using the private
   production database variables.
3. Require the migration to remain compatible with the current and candidate
   server versions.
4. Deploy the API and observe `/v1/ready`, registration errors, queue age,
   leases, and object failures.
5. Deploy the worker. Never percentage-split two worker revisions.
6. Verify the newest Railway deployment reaches terminal `SUCCESS`; a queued
   build is not a successful release.
7. Keep the prior deployment selectable until the observation window closes.

Application rollback selects the previous image/deployment. It does not run a
down-migration. Jobs already durably registered remain in PostgreSQL, jobs
already accepted locally remain in node SQLite, and ambiguous native handoffs
must remain `delivery_uncertain` rather than being printed again.

Non-physical release smoke checks:

```console
curl -fsS https://<api-host>/v1/health
curl -fsS https://<api-host>/v1/ready
curl -fsS https://<api-host>/v1/meta
cargo xtask test changed
```

Use the deterministic fake-print flow for lifecycle validation. A healthy HTTP
response is not permission to send a physical print.

## Scaling path

Scale only from observed saturation:

1. Increase API CPU or memory while keeping one replica.
2. Add a second same-region API replica when request concurrency requires it.
   Every server process currently opens a PostgreSQL pool of up to 20
   connections, so confirm database headroom before adding replicas.
3. Keep one worker until webhook or billing backlog requires another. Exercise
   duplicate-claim and worker-termination tests before increasing worker
   concurrency.
4. Move object storage closer to compute and enable independently recoverable
   object retention before adding customers with large documents.
5. Add a stable custom API domain and edge/WAF controls. A global proxy in
   front of one region improves routing and policy, not database or object
   availability.
6. When availability evidence or customer load justifies the fixed cost, move
   the same image to the existing two-region Cloud Run foundation or the Helm
   chart. Add a regional-HA database, cross-region DR replica, replicated
   objects, fenced promotion, and rehearsed rollback together.

Do not turn on Kubernetes or a global load balancer merely to claim scale.
They become useful after there are multiple healthy replicas and a recoverable
data plane behind them.

## Backups and recovery

The Railway PostgreSQL volume and Railway bucket are live application storage,
not sufficient restore evidence.

- Take retained PostgreSQL backups and an encrypted logical export.
- Copy referenced objects to a separate failure domain while preserving keys,
  digests, and retention metadata.
- Bind the database and object checkpoints into one restore record.
- Restore into an isolated environment with outbound webhooks and node
  delivery blocked.
- Verify migrations, tenant boundaries, job/event ordering, idempotency,
  object digests, leases, and outbox state before reconnecting nodes.
- Preserve node SQLite queues and native spooler evidence during server
  recovery. Never bulk-resubmit uncertain jobs.

The private preview has no checked-in proof of PostgreSQL PITR, bucket
versioning/replication, a complete isolated restore, or regional failover.
Record those as evidence rather than assuming platform-managed storage closes
the release gate.

## Known preview limits

- One public API replica and one private worker in one compute region are not
  highly available.
- The database has no repository-proven cross-region promotion path in this
  deployment.
- The initial bucket is geographically separate and has no checked-in
  replication or restore evidence.
- Documents are limited to 50 MiB. Uploads and node downloads currently proxy
  through the API; JSON/base64 compatibility requests have a 72 MiB body
  ceiling. Direct presigned transfer, byte-range resume, and orphan cleanup are
  not yet implemented.
- Long-lived dashboard event streams and node reconnect behavior still require
  deployed acceptance evidence.
- WorkOS, Stripe billing, production Sentry evidence, signed native releases,
  Windows/OKI physical certification, regional disaster recovery, and the
  no-loss soak remain open release gates.
- `/v1/ready` proves database and object-store reachability. It does not prove
  that any node, printer, driver, or stock is ready, or that ink reached paper.

See [Reliability and job lifecycle](reliability-and-job-lifecycle.md),
[Backups and restore](backups-and-restore.md), and
[Production release](production-release.md) for the authoritative acceptance
boundaries.
