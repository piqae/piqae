# Backups and restore

**Status:** operator responsibility; application data model supports consistent
recovery, but no managed backup service is bundled.

Back up PostgreSQL and object storage as one logical set. PostgreSQL is
authoritative for object references, tenant state, jobs/events, profile
metadata, webhooks, and outboxes. Object storage contains job content.

Minimum policy:

- PostgreSQL PITR plus retained full backups;
- object versioning/replication and deletion protection;
- encrypted backups in a separate failure domain;
- documented RPO/RTO and restore credentials;
- recurring automated integrity checks and human restore exercises.

Restore drill:

1. Isolate a new environment and block outbound webhooks/printing.
2. Restore PostgreSQL to the chosen point.
3. Restore matching object versions.
4. Run compatible migrations.
5. verify object digests, tenant boundaries, job event ordering, idempotency,
   webhook outbox state, and profile metadata.
6. Enrol a disposable node and print controlled PDF/RAW canaries.
7. Reconcile jobs accepted near the recovery point before enabling agents.

Agent-local SQLite and native spooler queues are separate evidence. Preserve
them during a control-plane restore; never blindly resubmit delivery-uncertain
jobs.
