# Piqae Helm chart

This chart deploys Piqae against externally managed PostgreSQL and
S3-compatible storage. It does not install stateful databases or object stores.
Supply immutable image digests and either an existing runtime Secret or an
External Secrets Operator store.

That runtime Secret must contain `PIQAE_DESTINATION_IDENTITY_KEY` as the
canonical Base64 encoding of exactly 32 random bytes. Keep it distinct from the
webhook and document-encryption keys and stable across service URL, workspace
name, and ordinary credential changes. Rotation requires a versioned identity
evidence migration and route reprojection; changing it in place can leave jobs
safely held because existing physical-destination evidence no longer matches.

The current server binary is a safe combined process: every instance exposes
HTTP/agent-sync routes and runs PostgreSQL-leased background work. The chart
separates `api`, `sync`, and `worker` pools for independent scaling, network
entry points, disruption budgets, and future role isolation; it does not claim
that the current binary disables unused roles inside those pools.

```sh
helm upgrade --install piqae deploy/helm/piqae \
  --namespace piqae --create-namespace \
  --values production-values.yaml \
  --set image.digest=sha256:... \
  --set migration.image.digest=sha256:...
```

The migration hook uses the image built by
`deploy/docker/Dockerfile.migrate`, which runs the checked-in SQLx migrations
and exits before application pods roll on upgrades. On first installation it
runs as a post-install hook so an ExternalSecret can reconcile; the current
server also runs the same idempotent SQLx migration set during startup.
PostgreSQL must permit all migration DDL from both identities. Pre-provision
the target Secret when an operator requires a strict migrate-before-first-pod
installation gate.

Restricted egress is opt-in because Kubernetes NetworkPolicy accepts CIDRs,
while managed PostgreSQL and S3 endpoints often rotate IPs. Enable it only
after supplying stable private CIDRs (or use a CNI policy with FQDN support).
Ingress is default-deny except from the configured namespace/pod selectors.
OIDC/JWKS, telemetry, webhook, and other required HTTPS destinations must be
listed in `networkPolicy.egress.additionalRules`; Kubernetes NetworkPolicy
cannot express DNS names.

Gateway API and Kubernetes Ingress are alternative public entry points. TLS
certificate issuance, DNS, WAF, backups, PostgreSQL failover, and object-store
replication remain operator responsibilities.
