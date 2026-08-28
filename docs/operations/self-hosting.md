# Self-hosting Piqae

The supported initial self-host topology is one or more `piqae-server`
containers, PostgreSQL 16 or newer, and an S3-compatible object store.

## Start

1. Change to `deploy/self-host`.
2. Copy `.env.example` to `.env`.
3. Replace every placeholder with randomly generated values.
4. Run `docker compose --env-file .env up -d --build`.
5. Use the bootstrap API key only from a trusted server-side integration.

The API-only topology has no identity-provider dependency. To include the
optional dashboard, configure the `WORKOS_*` values and run:

```sh
docker compose --env-file .env --profile dashboard up -d
```

For WorkOS, change the control-plane `PIQAE_AUTH_MODE` to `hybrid`, configure
the exact issuer and application JWKS URL, and bind the token with
`PIQAE_OIDC_CLIENT_ID`. The SvelteKit server forwards the verified WorkOS
access token directly to the control plane; it never exposes that token to
browser JavaScript or falls back to the bootstrap key. Add the Piqae
permissions used by each role to the WorkOS `permissions` claim. A private
self-host can instead make the explicit
`PIQAE_OIDC_ALLOW_UNRESTRICTED=true` choice, but hosted deployments must leave
that disabled. Dashboard readers normally need `agents_read`,
`printers_read`, `jobs_read`, `webhooks_read`, and `api_keys_read`; grant write
permissions only to roles that expose the matching mutation.

The Compose file includes source builds as a fallback, so a checkout can start
without a previously published registry tag. Pin released images by digest for
repeatable production upgrades.

The server writes structured JSON logs by default. Optional feature-gated OTLP
traces, W3C trace propagation, resource labels, safe error events, exporter
configuration, and failure limits are documented in
[control-plane observability](observability.md).

The current V1 bootstrap API key is a deployment credential, not a one-time
owner token. Rotate or remove it after creating durable API keys. Do not expose
it to browsers or native agents.

Set `PIQAE_DESTINATION_IDENTITY_KEY` to a stable, canonical Base64 encoding of
exactly 32 random bytes. It is a deployment trust-domain key used to
pseudonymize node-reported physical-printer identity evidence per tenant. It
must be distinct from webhook, document-encryption, API, and session keys and
must remain unchanged across server URL, workspace-name, and vanity-domain
changes. Treat rotation as a versioned identity migration: changing it without
an overlap and reprojection plan prevents existing evidence from matching and
can leave jobs safely held for route projection.

## Backups

Back up PostgreSQL and object storage together. PostgreSQL is authoritative for
object references. A valid restore test must verify migrations, job event
sequences, webhook outboxes, object checksums, and the ability to enrol a new
agent.

## Upgrades

v0.1.22 requires the fresh PostgreSQL and object-store procedure in
[`upgrades.md`](upgrades.md#v0122-fresh-postgresql-baseline); its migration
command deterministically refuses an in-place upgrade from v0.1.21. For later
releases, run migrations as an explicit pre-deployment operation and keep the
previous container digest until health, compatibility, and queue recovery
checks pass. Those later server schema changes remain compatible across the
declared N/N-1 application rollout window, but that is not a blanket
node-handoff compatibility promise.

For releases that introduce destination-route fencing, deploy the migration and
server (with the stable destination identity key) before nodes. Older nodes may
remain visible and synchronize, but the server deliberately holds new work with
`node_upgrade_required` until each route projection is current. Upgrade a node
canary, wait for current projection health and fresh route telemetry, then
widen. Already accepted local work continues recovery on the same node. Never
bypass the hold or route an ambiguous post-handoff attempt through another
node.
