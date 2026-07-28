# Self-hosting Spool

The supported initial self-host topology is one or more `spool-server`
containers, PostgreSQL 16 or newer, and an S3-compatible object store.

## Start

1. Copy `deploy/self-host/.env.example` to `.env`.
2. Replace every placeholder with randomly generated values.
3. Set `SPOOL_PUBLIC_API_ORIGIN` to the HTTPS origin users and agents reach.
4. Run `docker compose --env-file .env up -d`.
5. Open the configured origin and consume the one-time owner bootstrap.
6. Disable bootstrap mode by configuring OIDC before inviting other users.

The bootstrap secret is single-use and is never printed again after an owner
is created.

## Backups

Back up PostgreSQL and object storage together. PostgreSQL is authoritative for
object references. A valid restore test must verify migrations, job event
sequences, webhook outboxes, object checksums, and the ability to enrol a new
agent.

## Upgrades

Run migrations as an explicit pre-deployment operation. Keep the previous
container digest until health, compatibility, and queue recovery checks pass.
Agent protocol N and N-1 are supported, so upgrade the server before agents.
