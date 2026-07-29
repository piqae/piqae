# Self-hosted Kubernetes

**Status:** Helm infrastructure foundation implemented and manifest-validated;
no cluster conformance or production release certification has run.

The chart at [`deploy/helm/spool`](../../deploy/helm/spool/README.md) requires
external PostgreSQL and S3-compatible storage. It renders API, sync, and worker
Deployments, migration hooks, disruption budgets, autoscaling, topology
spreading, NetworkPolicies, restricted security contexts, optional dashboard,
External Secrets, and Ingress or Gateway API routing.

```sh
./deploy/validate.sh
helm upgrade --install spool deploy/helm/spool \
  --namespace spool --create-namespace \
  --values production-values.yaml \
  --set image.digest=sha256:... \
  --set migration.image.digest=sha256:...
```

The current server binary is combined-role. The three pools are independent
scaling/failure boundaries, but every pod still exposes HTTP and runs leased
background work. Do not treat the worker pool as code-level role isolation.

Production prerequisites:

- PostgreSQL HA, pooling, PITR, migration DDL rights, and a tested restore;
- versioned/replicated S3 storage and tested checksum recovery;
- pre-created secrets or a working External Secrets controller;
- TLS, DNS, WAF/rate limits, logs/traces, alerts, and Pod Security admission;
- stable private CIDRs before enabling restricted egress;
- rehearsed application and database failover.

See [configuration](../operations/configuration.md),
[monitoring](../operations/monitoring.md), and
[high availability](../operations/high-availability.md).
