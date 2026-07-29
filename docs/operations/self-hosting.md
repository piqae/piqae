# Self-hosting Spool

The supported initial self-host topology is one or more `spool-server`
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

For WorkOS, change the control-plane `SPOOL_AUTH_MODE` to `hybrid`, configure
the exact issuer and application JWKS URL, and bind the token with
`SPOOL_OIDC_CLIENT_ID`. The SvelteKit server forwards the verified WorkOS
access token directly to the control plane; it never exposes that token to
browser JavaScript or falls back to the bootstrap key. Add the Spool
permissions used by each role to the WorkOS `permissions` claim. A private
self-host can instead make the explicit
`SPOOL_OIDC_ALLOW_UNRESTRICTED=true` choice, but hosted deployments must leave
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

## Backups

Back up PostgreSQL and object storage together. PostgreSQL is authoritative for
object references. A valid restore test must verify migrations, job event
sequences, webhook outboxes, object checksums, and the ability to enrol a new
agent.

## Upgrades

Run migrations as an explicit pre-deployment operation. Keep the previous
container digest until health, compatibility, and queue recovery checks pass.
Agent protocol N and N-1 are supported, so upgrade the server before agents.
