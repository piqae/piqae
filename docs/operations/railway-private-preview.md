# Railway low-cost private preview

**Status:** operational private preview. This shape is suitable for Spool's own
early use and controlled design partners. It is not approved as a production,
high-availability, or 99.95% deployment.

Railway is the low-cost launch path between local Compose and the managed
multi-region design. It runs the same server image and preserves a move to
Cloud Run or Helm without introducing a second application architecture.

## Current shape

```text
Vercel dashboard
      |
      v
public Railway API (one replica, SPOOL_SERVICE_ROLE=api)
      |                         |
      v                         v
Railway PostgreSQL       Railway S3-compatible bucket
      ^
      |
private Railway worker (one replica, SPOOL_SERVICE_ROLE=worker)
```

- Only the API has a public Railway domain.
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

Keep the API and worker at one always-running replica. Autosleep is a poor fit
for a print control plane because node polling and job pickup are
latency-sensitive. One small warm replica is the useful low-cost floor; it
avoids paying for idle multi-region capacity without introducing a cold start
into every reconnect.

## Source-controlled deployment

The root [`railway.toml`](../../railway.toml) selects
[`deploy/docker/Dockerfile.server`](../../deploy/docker/Dockerfile.server) and
gates the public service on `/v1/ready`.

The API and worker use the same image and release. Configure their
service-specific values separately:

| Variable | API | Worker |
| --- | --- | --- |
| `SPOOL_SERVICE_ROLE` | `api` | `worker` |
| `SPOOL_RUN_MIGRATIONS_ON_STARTUP` | `false` | `false` |
| Public domain | enabled | disabled |
| Replica floor | 1 | 1 |

The server still binds an HTTP listener in worker mode, but that listener must
remain private.

## Configuration contract

Set these non-secret values on both services unless the table above overrides
them:

```text
SPOOL_BIND=0.0.0.0:8080
SPOOL_DATABASE_URL=${{Postgres.DATABASE_URL}}
SPOOL_OBJECT_STORE=s3
SPOOL_S3_ENDPOINT=<Railway bucket endpoint>
SPOOL_S3_BUCKET=<Railway bucket name>
SPOOL_S3_REGION=<Railway bucket region>
SPOOL_S3_ALLOW_HTTP=false
SPOOL_S3_VIRTUAL_HOSTED_STYLE=false
SPOOL_DEPLOYMENT=self_hosted
SPOOL_IDENTITY_PROVIDER=local_owner
SPOOL_AUTH_MODE=bootstrap
SPOOL_BILLING_ENABLED=false
SPOOL_LOCAL_OWNER_SESSION_SECONDS=86400
```

Set these as protected Railway variables, never in Git, build arguments,
domains, or logs:

```text
SPOOL_S3_ACCESS_KEY_ID
SPOOL_S3_SECRET_ACCESS_KEY
SPOOL_WEBHOOK_MASTER_KEY
SPOOL_LOCAL_OWNER_BOOTSTRAP_TOKEN  # one-time only; remove after bootstrap
```

`SPOOL_WEBHOOK_MASTER_KEY` must be a base64-encoded 32-byte key. Use the
database service reference for `SPOOL_DATABASE_URL` instead of copying its
resolved credential into another configuration surface.

### Railway bucket addressing

For the current `t3.storageapi.dev` Railway bucket endpoint, Spool must use
path-style S3 requests:

```text
SPOOL_S3_VIRTUAL_HOSTED_STYLE=false
```

Do not copy a credential response's `urlStyle` field directly into this Spool
flag. With this endpoint and Spool's current S3 client, virtual-hosted mode
misaddresses the readiness key and returns an authorization error. Verify the
integration with a non-customer object:

```console
cargo run -p spool-object-store --example seed_readiness
```

Run that command only in a protected environment containing the bucket
variables. It writes `health/readiness-probe` and prints no credentials.
`/v1/ready` subsequently exercises PostgreSQL plus an object-store existence
request. `/v1/health` checks only the server process.

## Identity bootstrap

This preview deliberately uses local-owner identity. WorkOS and Stripe stay
disabled until their live tenant, session, webhook, and billing evidence gates
pass.

1. Generate a high-entropy `SPOOL_LOCAL_OWNER_BOOTSTRAP_TOKEN` in a protected
   secret store.
2. Use it once with `/v1/identity/local/bootstrap`.
3. Put the returned owner credential in the operator's password manager or OS
   secret store.
4. Delete `SPOOL_LOCAL_OWNER_BOOTSTRAP_TOKEN` from both Railway services and
   redeploy.
5. Create scoped API keys through the authenticated application; do not retain
   a legacy bootstrap API key as the normal integration credential.

The Vercel dashboard uses `SPOOL_AUTH_MODE=local` and points
`PUBLIC_SPOOL_API_URL` at the public Railway API. Browser identity and
control-plane service credentials stay server-side.

## Migrations and releases

Ordinary API and worker replicas must not run schema changes. For the initial
empty database only, one API replica may start once with
`SPOOL_RUN_MIGRATIONS_ON_STARTUP=true`; turn it off immediately after that
deployment succeeds.

For every later release:

1. Back up PostgreSQL and referenced objects.
2. Run the candidate image once with `spool-server migrate`, using the private
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
