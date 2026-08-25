# ADR-0005: Physical destinations, routes, and fenced delivery attempts

Status: accepted

## Context

An installed operating-system queue is not necessarily a unique physical
printer. The same printer can be reachable from several nodes, and one node can
project its queues into several isolated hosted, managed-customer, or
self-hosted connections. Names, addresses, and drivers are insufficient proof
that two queues reach the same device.

Automatic rerouting also has a hard safety boundary. Reassignment is safe
before native spooler acceptance when a stale attempt is fenced. It is not safe
after an ambiguous handoff because the first route may still print.

## Decision

PostgreSQL stores these separate tenant-scoped resources:

- `physical_destinations`: the tenant's inferred real printer;
- `printer_routes`: one native queue on one node that can reach a destination;
- `destination_identity_evidence`: digested, bounded evidence reported for a
  route;
- `destination_identity_decisions`: append-only merge, split, confirmation,
  rejection, and reversal audit records;
- `route_observations`: privacy-safe printer and OS-spooler counts with explicit
  observation/freshness times;
- `projection_acknowledgements`: independent inventory delivery state for each
  connector and route;
- `delivery_attempts` and `route_reservations`: monotonically generated,
  token-fenced execution ownership;
- `scheduling_authorities` and `site_coordinator_memberships`: the explicit
  authority boundary for hosted, self-hosted, and site-coordinated scheduling.

Every primary key, lookup, mutation, and foreign key in this topology includes
`workspace_id` and `environment_id`. Raw serial numbers, MAC addresses,
certificate keys, or external spool-job details are not stored in identity
evidence. Callers submit one-way evidence digests and privacy-safe queue counts.

Strong or verified evidence can support a later grouping decision. The storage
layer never merges routes automatically. Conflicting evidence changes the
destination confidence to `conflict`; an operator or identity policy must make
an audited decision. Decisions are reversible through a new append-only audit
row rather than mutation or deletion.

A delivery attempt owns one route and one job generation. The repository
returns the plaintext fencing token once and stores only its SHA-256 digest.
State changes require the exact tenant, attempt, generation, and token. One
active attempt per job and one active reservation per route are enforced in
PostgreSQL. Terminal states release the reservation. A superseded generation
cannot resume.

`accepted_by_spooler`, `printing_reported`, and `completed_reported` remain
observations. They are not proof that paper emerged. A loss of certainty during
handoff becomes `delivery_uncertain`, not an automatic failover.

## Upgrade and compatibility

Migration 42 upgrades the tenant-scoped printer identity introduced by
migration 41. It creates one `unknown`-confidence destination and one primary
route for each existing printer. This is deliberately conservative: it does
not group similar names, driver fingerprints, or addresses.

Existing logical target bindings are backfilled with authoritative
`destination_id` and `route_id` references. Existing jobs receive compatible
destination/route references. Legacy printer/agent fields remain readable only
for the supported rolling-upgrade window; new scheduling uses destination,
route, attempt, and reservation records.

## Consequences

- One local printer ID can safely appear in several tenant projections.
- Multiple node routes can represent one destination without sharing tenant
  credentials or job metadata.
- Live status always has an observation time and freshness bound.
- Hosted and self-hosted schedulers cannot pretend to offer cross-server
  exactly-once failover without an explicit shared scheduling authority.
- Automatic failover is safe only before handoff or after an acknowledged,
  fenced cancellation. Ambiguous native handoff remains a human decision.
