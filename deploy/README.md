# Spool deployment foundations

Spool supports two self-hosting shapes:

- `self-host/`: one-node Docker Compose for evaluation and small installations;
- `helm/spool/`: production Kubernetes control plane using external PostgreSQL
  and S3-compatible storage.

The Helm chart creates three independently scalable compute pools:

```text
Ingress / Gateway ──> api Service  ─┐
Agent routing ──────> sync Service ─┼─> external PostgreSQL
No public Service ──> worker Pods  ─┘   external S3
Optional public route ─> dashboard ───> api Service
```

Today each pool runs the same combined `spool-server` process. PostgreSQL
leases and transactional outboxes make that topology safe, while separate
Deployments provide disruption and scaling boundaries. Do not use NetworkPolicy
or cost estimates as if unused roles were disabled inside a pool.

The server supports explicit `api`, `sync`, `worker`, and local `all` roles.
Helm and Cloud Run assign roles to independent pools, disable replica startup
DDL, and provide bounded migration Jobs. The global load balancer routes device
sync and lease/content paths to the sync pool; worker services have no public
invoker binding.

Production operators must provide:

- PostgreSQL HA, PITR, tested restores, connection pooling, and migration DDL
  permissions;
- versioned S3 storage, retention/replication, lifecycle policy, and credentials;
- immutable server, migration, and dashboard image digests;
- the runtime Secret or a working External Secrets Operator store;
- DNS, TLS, WAF/rate limiting, alerting, and an incident-tested failover plan.

Run all deployment checks with:

```sh
./deploy/validate.sh
```

The check lints and renders Helm, validates built-in Kubernetes resources with
kubeconform, formats Terraform, initializes providers without a backend, and
validates configuration. Docker is used when local Helm/Terraform binaries are
not installed. It never plans, applies, opens credentials, or accesses a
cluster/cloud account.

Run the fail-closed production preflight with:

```sh
SPOOL_PRODUCTION_VERCEL_ENV_FILE=/protected/vercel-production.env \
SPOOL_PRODUCTION_TFVARS_FILE=/protected/production.tfvars \
SPOOL_PRODUCTION_EVIDENCE_DIR=/protected/release-evidence \
  ./deploy/production-check.sh
```

The command deliberately fails until code gates, populated configuration, and
external evidence all exist. `./deploy/production-check.sh structural` checks
only repository-owned templates and policies and is safe for normal CI. It is
not production approval.

## Hosted GCP option

`terraform/` continues to default to one Cloud Run region with external
PostgreSQL/S3. Production flags add a warm Melbourne service, global HTTPS load
balancing, a regional-HA Cloud SQL primary with a cross-region DR replica, and
dual-region GCS. When the managed data plane is enabled, Terraform creates the
database identity, stores its generated URL in Secret Manager, mounts the Cloud
SQL connector into API/sync/worker services and the migration Job, and grants
the runtime service account only the required client and object permissions.
Database DR promotion and application traffic cutover still require the
explicit, fenced operator procedure in the production runbook.

Use separate projects and Terraform states for staging and production. Review
every plan, provider release, quota, and deletion-protection change before
apply.
