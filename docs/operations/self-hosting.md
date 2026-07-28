# Self-hosting Spool

The supported initial self-host topology is one or more `spool-server`
containers, PostgreSQL 16 or newer, and an S3-compatible object store.

## Start

1. Copy `deploy/self-host/.env.example` to `.env`.
2. Replace every placeholder with randomly generated values.
3. Set `SPOOL_PUBLIC_API_ORIGIN` to the HTTPS origin users and agents reach.
4. Run `docker compose --env-file .env up -d`.
5. Use the bootstrap API key only from a trusted server-side integration.

The API-only topology has no identity-provider dependency. To include the
optional dashboard, configure the `WORKOS_*` values and run:

```sh
docker compose --env-file .env --profile dashboard up -d
```

The dashboard's temporary API credential remains server-side. Until the
generic OIDC/session exchange gate is complete, the dashboard profile requires
WorkOS and is not a multi-tenant SaaS identity boundary.

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
