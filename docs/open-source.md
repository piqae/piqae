# Open source and self-hosting

Piqae is Apache-2.0 printing infrastructure. Self-hosting includes the complete
control plane, dashboard, platform accounts, API keys, webhooks, native
profiles, durable queues, diagnostics, update policy, and local-owner or OIDC
authentication. It requires no WorkOS account, Stripe account, licence server,
or phone-home.

## Choose one mode

- **Docker Compose:** development and normal small installations.
- **Helm/Kubernetes:** highly available installations using external
  PostgreSQL and S3-compatible storage.
- **Local-only:** one node, its SQLite queue, and a loopback API with no control
  plane.

Start Compose:

```console
cd deploy/self-host
cp .env.example .env
# Replace every placeholder.
docker compose --env-file .env up -d
```

Use the official signed node or build it from source, point it at the
self-hosted HTTPS URL, and pair through local-owner or OIDC approval. The same
platform-account SDK works when `baseUrl` points to the self-hosted API.

Compose is not highly available. Before production use, pin images by digest,
run migrations as an explicit job, back up PostgreSQL and object storage as one
consistency set, monitor queue age, and prove restore and upgrade procedures.

Detailed operator material:

- [Compose](getting-started/self-hosted-compose.md)
- [Kubernetes](getting-started/self-hosted-kubernetes.md)
- [Local-only](getting-started/local-only.md)
- [Configuration](operations/configuration.md)
- [Backups and restore](operations/backups-and-restore.md)
- [High availability](operations/high-availability.md)
- [Upgrades](operations/upgrades.md)
