---
name: add-postgres-migration
description: Add and validate append-only Piqae PostgreSQL migrations with mandatory tenant isolation and upgrade compatibility. Use whenever database schema or persistent projections change.
---

# Add a PostgreSQL migration

1. Read `AGENTS.md` and `crates/control-plane/AGENTS.md`.
2. Allocate the next migration number; never edit an applied migration.
3. Scope tenant resources by `workspace_id` and `environment_id` where
   applicable, and include tenant keys in every lookup and mutation.
4. Use `.piqae-test-fixtures/postgres-migration` with Compose PostgreSQL.
5. Test empty-database migration, N-1 upgrade, application startup, and
   cross-workspace ID probing.
6. Record schema version, migration timings, query evidence, and test results.

Do not log database URLs, passwords, identity payloads, API hashes, device code
hashes, or customer rows. Drop only the disposable database/volume created for
this skill.
