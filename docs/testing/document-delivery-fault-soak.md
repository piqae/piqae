# Accelerated document delivery fault soak

The repository includes a fake-only accelerated restart test:

```console
cargo test -p piqae-agent-storage --test offline_recovery \
  accelerated_disconnect_retry_soak_has_no_loss_or_duplicate_activation
```

It creates 250 cloud-managed jobs on a temporary SQLite database. For every
job it closes the process boundary after persisting the acceptance intent,
reopens the database, replays an ambiguous acceptance response twice, closes
again, and finally verifies:

- 500 simulated disconnect boundaries;
- 250 replayed acceptance responses;
- 250 unique durable jobs with no loss;
- exactly 250 unique `queued_local` outbox events;
- one runnable head for the shared printer, preserving queue order;
- acknowledgement replay produces no duplicate;
- all intents and acknowledged events drain;
- SQLite integrity remains valid.

This is deterministic fault coverage, not a duration or capacity claim. The
production gate still requires a long-running environment-equivalent soak with
PostgreSQL, object storage, real network faults, process termination, queue
leases and telemetry. It uses no physical printers.
