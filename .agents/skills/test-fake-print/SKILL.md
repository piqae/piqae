---
name: test-fake-print
description: Exercise Spool job registration, offline queueing, leasing, status propagation, and idempotency with deterministic virtual nodes and printers. Use for printing-flow tests that must not reach physical hardware.
---

# Test fake printing

1. Read `AGENTS.md` and printing safety rules.
2. Create `.spool-test-fixtures/fake-print` as the state directory.
3. Run `cargo test -p spool-control-plane -p spool-agent -p spool-fake-executor`.
4. Start `cargo xtask dev` without `--real-printers`.
5. Submit the repository fixture to the virtual printer twice with one
   idempotency key; expect one job and one spooler-acceptance usage event.
6. Disconnect and reconnect the virtual node; expect the durable job to be
   leased after reconnect and every transition to remain queryable.
7. Record job/event IDs, final observer state, and assertions as evidence.

Never use customer documents or log API keys, lease tokens, content URLs,
device codes, or payload bytes. Stop virtual services and remove only the skill
state directory.
