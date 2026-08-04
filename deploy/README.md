# Piqae deployment foundations

Piqae supports two self-hosting shapes:

- `self-host/`: one-node Docker Compose for evaluation and small installations;
- `helm/piqae/`: production Kubernetes control plane using external PostgreSQL
  and S3-compatible storage.

The current low-cost hosted launch shape uses public web and API services plus
one private worker on Railway, with Railway PostgreSQL, a print-document bucket,
and a separate native-release bucket. Staging and production are isolated
Railway environments. A reviewed commit deploys to staging before an operator
promotes that exact commit to production.

This is an operational private preview, not a high-availability or 99.95%
claim. See
[`Railway low-cost private preview`](../docs/operations/railway-private-preview.md)
for the exact configuration, migration, scaling, backup, and release limits.

The Helm chart creates three independently scalable compute pools:

```text
Ingress / Gateway ──> api Service  ─┐
Agent routing ──────> sync Service ─┼─> external PostgreSQL
No public Service ──> worker Pods  ─┘   external S3
Optional public route ─> dashboard ───> api Service
```

Today each pool runs the same combined `piqae-server` process. PostgreSQL
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

The control plane applies its own fixed-window limits to the unauthenticated
node-onboarding endpoints (pairing creation, pairing polling, and enrolment) so
its tables stay bounded without an edge WAF. Those limits bound table growth,
not bandwidth: an operator-provided WAF is still required for volumetric
defence. Per-client limiting keys on `x-forwarded-for`, so terminate that header
at a trusted proxy.

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
PIQAE_PRODUCTION_RAILWAY_ENV_FILE=/protected/railway-production-web.env \
PIQAE_PRODUCTION_EVIDENCE_DIR=/protected/release-evidence \
  ./deploy/production-check.sh
```

The command deliberately fails until code gates, populated configuration, and
external evidence all exist. `./deploy/production-check.sh structural` checks
only repository-owned templates and policies and is safe for normal CI. It is
not production approval. The protected environment starts from
[`hosted/railway.env.example`](hosted/railway.env.example) and must never be
committed.

The `railway-production-runtime.json` evidence record contains no secrets. In
addition to the common gate, status, commit, timestamp, and evidence URL fields,
it records the exact successful deployment IDs and separation boundaries:

```json
{
  "railway": {
    "project_id": "<project-id>",
    "environment_id": "<production-environment-id>",
    "services": {
      "web": {
        "deployment_id": "<id>",
        "status": "SUCCESS",
        "public_domain": true
      },
      "api": {
        "deployment_id": "<id>",
        "status": "SUCCESS",
        "public_domain": true
      },
      "worker": {
        "deployment_id": "<id>",
        "status": "SUCCESS",
        "public_domain": false
      }
    },
    "document_bucket": "piqae-documents",
    "release_bucket": "piqae-releases"
  }
}
```

The preflight rejects non-successful services, a public worker, or one bucket
being reused for documents and releases.

## Optional managed-HA scale-up

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

Cloud Run, Cloud SQL, GCS, the global load balancer, and regional DR are not
required to launch the Railway private beta. When promoting that optional
managed-HA target, run the stricter profile:

```sh
PIQAE_PRODUCTION_RAILWAY_ENV_FILE=/protected/railway-production-web.env \
PIQAE_PRODUCTION_TFVARS_FILE=/protected/production.tfvars \
PIQAE_PRODUCTION_EVIDENCE_DIR=/protected/release-evidence \
  ./deploy/production-check.sh managed-ha
```

That profile additionally requires digest-pinned managed-HA configuration and
regional disaster-recovery evidence.
