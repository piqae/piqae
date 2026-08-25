# Incident response

**Status:** operational runbook; automation is partial.

## First actions

1. Declare severity, commander, timeline, and affected environments.
2. Preserve logs, job events, node SQLite, and native spooler evidence.
3. Stop expansion: pause routing or enrolment, not arbitrary database writes.
4. Identify whether jobs are waiting, safely failed, or delivery-uncertain.
5. Protect customer content and rotate exposed credentials immediately.

## Printing safety

Do not bulk retry ambiguous jobs. Reconcile Piqae ID, native spooler ID,
physical output, and stock. One uncertain job outside an incident is
[`uncertain-delivery-response.md`](uncertain-delivery-response.md). For labels, check the next serial/order before
releasing work. Disable a bad profile/target rather than editing history.

## Dependency failures

- PostgreSQL: fence writers before promotion; record replication position.
- Object store: stop jobs whose digest/content cannot be verified.
- Node outage: let new jobs wait or expire; preserve already accepted local work.
- Webhooks: consumers must deduplicate; replay only after durable receiver repair.
- Driver regression: retire the affected revision and restore the last tested
  driver/profile combination.

Share only a [redacted diagnostic bundle](../nodes/diagnostics.md). After
recovery, reconcile uncertain deliveries, rotate temporary access, document
customer impact, and convert lessons into tests/runbooks.
