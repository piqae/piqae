# Hosted infrastructure

This module deploys the Piqae control plane to Cloud Run in Sydney. By default,
PostgreSQL and S3 remain separately managed services and their credentials are
passed to Secret Manager.

`webhook_master_key_secret` must be a base64-encoded 32-byte random key. The
module stores it, the Neon connection URL, and both R2 credentials in Secret
Manager; none are emitted as Terraform outputs.

Production uses three always-allocated instances so API, agent long-polling,
outbox workers, and webhook delivery continue without a cold start. PostgreSQL
leases and transactional outboxes make every replica safe to run the combined
`all` role.

Hosted authentication defaults to OIDC. For WorkOS AuthKit, set
`oidc_jwks_url` to the application's HTTPS signing JWKS endpoint and
`oidc_binding_value` to its client ID; leave `oidc_audience` empty. Providers
that issue a standard audience can set `oidc_audience` and leave
`oidc_binding_value` empty. The module refuses an OIDC deployment without
exactly one application-binding mechanism. OIDC permissions are mapped from
the verified `permissions` array and unrestricted OIDC access is always
disabled in this hosted module.

The control plane currently verifies 50 MiB object digests from bounded
in-memory buffers, and Base64 compatibility requests can briefly hold both
encoded and decoded forms. Cloud Run concurrency is therefore capped at four and
instances use 1 GiB of memory. Increase neither limit independently; migrate
the object-store boundary to streaming before raising transfer concurrency.

Apply staging and production from different GCP projects and separate Terraform
state. Always provide the server image by digest, never by a mutable tag.

## Optional Australian HA foundation

`enable_multi_region` creates a warm service in Melbourne
(`australia-southeast2`). `enable_global_load_balancer` adds global HTTPS
routing across Sydney and Melbourne serverless NEGs. DNS is deliberately not
managed here: point every configured certificate name at
`global_load_balancer_ip` only after the certificate and both revisions are
healthy. Cloud Run serverless NEGs do not expose conventional load-balancer
health checks. The backend enables bounded 5xx outlier detection, which can
reduce traffic to a failing region but can still leak errors while proxies
converge. PostgreSQL-backed `/v1/ready` remains the deployment gate.

The optional `enable_managed_data_plane` resources create:

- a regional-HA PostgreSQL 16 primary in Sydney with PITR and retained backups;
- a cross-region read replica in Melbourne (promotion remains an explicit,
  rehearsed operator action);
- a versioned, public-access-prevented GCS bucket placed across Sydney and
  Melbourne.

These managed resources are intentionally not switched into the runtime
secret automatically. Creating database roles, rotating credentials, choosing
private connectivity, promoting the replica, and updating
`database_url_secret` are controlled cutover steps. The external
PostgreSQL/S3 path remains the safe default.

For a global load balancer, `allow_public_cloud_run_invocation=true` grants the
transport-level Cloud Run invoker role to `allUsers`; Piqae's OIDC/API-key
authentication still protects every application route. Enforce Cloud Armor,
rate limits, and organization policy outside this small module.

Provider-sensitive behavior is based on the current Google documentation for
[multi-region serverless NEGs](https://cloud.google.com/load-balancing/docs/negs/serverless-neg-concepts),
[cross-region PostgreSQL replicas](https://cloud.google.com/sql/docs/postgres/replication/cross-region-replicas),
and [configurable dual-region buckets](https://cloud.google.com/storage/docs/bucket-locations).
Recheck these constraints and regional product availability before every
production plan.
