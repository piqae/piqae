# Upgrades

**Status:** v0.1.22 establishes a fresh PostgreSQL baseline; later rolling
deployment foundations are implemented, while fully automated signed node
updates are not Supported.

## v0.1.22 fresh PostgreSQL baseline

v0.1.22 is an intentional pre-release reset of the PostgreSQL migration
history. The server and migration command refuse every applied migration row
that is not an exact prefix of the v0.1.22 history, before SQLx can report a
checksum error. In particular, a database created by the released v0.1.21
build cannot be upgraded in place. There is no compatibility view, hidden data
rewrite, fallback identifier, or automatic reset.

For an existing evaluation installation:

1. Stop new job registration, drain work that has not crossed the native
   handoff, and record every `delivery_uncertain` job without resubmitting it.
2. Take a restorable PostgreSQL dump and object-store snapshot. Keep them
   read-only for audit; do not load their tables into the new schema.
3. Export the operator-owned inventory needed to rebuild the installation:
   workspace/environment names, integrator configuration, node/printer names,
   route intent, stocks, profile references, and canonical PrintPacket source
   held by the originating integration. Never export device keys or lease
   capabilities.
4. Provision a new empty PostgreSQL database and a new empty object-store
   prefix, configure v0.1.22 to use them, and run its migrations once.
5. Recreate tenant configuration through current APIs, re-enrol standalone
   nodes and connector-owned embedded nodes, reselect printers, then republish
   canonical PrintPacket templates/resources from their owning applications.
6. Verify tenant isolation, inventory, route health, profiles/stocks, and a
   deterministic virtual-printer job before admitting live work.
7. Retain the old snapshot for the required audit period. Retire it only after
   the new installation and its backup/restore path are accepted.

Do not copy SQL rows, encryption-key references, queue/lease state, device
identity, or document ciphertext between the two baselines. Piqae has no
supported importer for those pre-release internals.

The sequence below applies to releases after a deployment is already on the
v0.1.22 baseline.

Server-first sequence:

1. Read release notes, support matrix, migration SQL, and protocol changes.
2. Back up PostgreSQL and object storage; verify the restore path.
3. Deploy to staging and exercise enrolment, PDF/RAW, webhooks, and offline
   recovery.
4. Run migrations with a dedicated bounded job.
5. Roll server replicas with readiness gates and no unavailable API capacity.
6. Observe error rate, queue age, database locks, webhook backlog, and node
   reconnects.
7. For a route-fencing release, confirm older nodes are visible but new work is
   held with `node_upgrade_required` rather than offered without a reservation.
8. Roll a small node canary set. Require current route projection health and
   fresh route telemetry before widening.

After the v0.1.22 fresh baseline, migrations must be backward-compatible with
the previous server during a rolling window. Never roll back application
binaries across a destructive schema change without a tested recovery plan.

N/N-1 describes the declared server-binary and schema rollout window. It does
not promise that an N-1 node can accept an N job whose handoff requires route
reservation and fencing fields it cannot understand. The fail-closed sequence
is migration, server, then node canary: old nodes may continue presence and
inventory synchronization, while new work waits until the upgraded node has
projected its routes. Already accepted work recovers from the same durable
local queue. Do not remove the hold, forge a projection, or fail over after an
ambiguous native handoff; those shortcuts can duplicate paper output.

Preserve node data directories, device keys, profiles, and delivery evidence.
Do not reinstall drivers during the same change window as Piqae unless the
profile invalidation is intentional and tested.

Keep prior immutable images/binaries until the observation period ends. See
[node updates](../nodes/updates.md) and
[`contributing/releases.md`](../contributing/releases.md).
