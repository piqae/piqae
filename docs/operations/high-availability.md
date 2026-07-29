# High availability

**Status:** stateless replica/lease/outbox architecture and deployment
foundations implemented; end-to-end regional failover is operator-controlled
and not certified.

Multiple server replicas can share PostgreSQL and S3 because claims, leases,
idempotency, and outboxes are durable. Readiness checks PostgreSQL and a probe
object. Keep at least two replicas across nodes/zones with disruption budgets.

The optional GCP foundation adds Sydney and Melbourne Cloud Run services behind
multi-region serverless NEGs with bounded 5xx outlier detection. This can
reduce traffic to a failing compute region; it cannot make the database
writable or guarantee zero leaked errors.

The optional Cloud SQL primary is regional HA within Sydney. Its Melbourne
cross-region replica requires deliberate promotion and connection-secret
cutover. Promotion is not the same as automatic zonal HA and can involve data
loss up to replication lag. Dual-region objects also have replication delay.

Rehearse:

- loss of one application replica and one zone;
- PostgreSQL connection saturation and primary failover;
- object-store errors and checksum mismatch;
- full regional loss, replica promotion, secret rotation, and DNS/LB behavior;
- recovery/failback without duplicating jobs or webhooks.

Use explicit fencing during database promotion so Sydney and Melbourne cannot
both accept writes to divergent primaries. Record RPO/RTO, decision authority,
and reconciliation steps in the incident runbook.
