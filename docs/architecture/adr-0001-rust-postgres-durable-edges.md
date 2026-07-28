# ADR-0001: Rust, PostgreSQL, and durable responsibility edges

Status: accepted

## Context

Remote printing crosses an API, object store, network, local machine, renderer,
driver, OS spooler, and physical printer. No distributed transaction spans
those systems. Treating job delivery as a simple request/response operation
causes silent loss or duplicate output.

## Decision

- Use Rust for the shared domain, control plane, agent, and executor protocol.
- Use PostgreSQL as the hosted job, lease, event, and outbox authority.
- Use SQLite WAL with full synchronous writes as the local responsibility
  boundary.
- Persist `spool_intent` before invoking a non-transactional OS spooler.
- If OS acceptance cannot be distinguished from a process failure, record
  `delivery_uncertain` and require an explicit human retry decision.
- Keep OS integrations in killable helper processes.
- Use long-polling for agent commands and an outbox for agent events.

## Consequences

The model may expose uncertainty instead of pretending every print can be
proven exactly once. This is intentional. It gives operators enough evidence
to make a safe decision and prevents automatic duplicate labels, receipts, or
shipping documents.

PostgreSQL is sufficient for the initial queue envelope through row locking,
leases, and transactional outboxes. A broker is only introduced after measured
contention or throughput requires it.
