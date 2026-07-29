# Self-hosted Docker Compose

**Status:** implemented evaluation topology; production operations remain
Preview.

Compose runs `spool-server`, PostgreSQL 16, and MinIO. The dashboard is an
optional profile.

```sh
cd deploy/self-host
cp .env.example .env
# Replace every placeholder, then:
docker compose --env-file .env up -d
```

Add `--profile dashboard` after configuring the dashboard origin and WorkOS
values. Never expose the bootstrap API key to browser JavaScript or a node.

Before printing:

1. Confirm the server health endpoint returns success.
2. Create or retain the bootstrap tenant identifiers.
3. Enrol a node using [pairing](../nodes/pairing.md).
4. Discover a printer and create a profile.
5. Submit a small PDF using the [API quickstart](../api/quickstart.md).

This topology is not highly available. Back up PostgreSQL and object storage as
one consistency set, pin images by digest, and read
[`operations/self-hosting.md`](../operations/self-hosting.md),
[`backups-and-restore.md`](../operations/backups-and-restore.md), and
[`upgrades.md`](../operations/upgrades.md) before production use.
