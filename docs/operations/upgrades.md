# Upgrades

**Status:** database migrations and rolling deployment foundations implemented;
fully automated signed node updates are not Supported.

Server-first sequence:

1. Read release notes, support matrix, migration SQL, and protocol changes.
2. Back up PostgreSQL and object storage; verify the restore path.
3. Deploy to staging and exercise enrolment, PDF/RAW, webhooks, and offline
   recovery.
4. Run migrations with a dedicated bounded job.
5. Roll server replicas with readiness gates and no unavailable API capacity.
6. Observe error rate, queue age, database locks, webhook backlog, and node
   reconnects.
7. Roll a small node canary set, then widen.

Migrations must be backward-compatible with the previous server during a
rolling window. Never roll back application binaries across a destructive
schema change without a tested recovery plan.

Preserve node data directories, device keys, profiles, and delivery evidence.
Do not reinstall drivers during the same change window as Piqae unless the
profile invalidation is intentional and tested.

Keep prior immutable images/binaries until the observation period ends. See
[node updates](../nodes/updates.md) and
[`contributing/releases.md`](../contributing/releases.md).
