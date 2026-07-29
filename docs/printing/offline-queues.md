# Offline and layered queues

**Status:** durable control-plane registration, agent-local SQLite queue, and
native spooler handoff are implemented.

Spool has three different layers:

1. **Control plane:** durable job identity, content reference, target/profile
   pin, expiry, audit events, and delivery state.
2. **Node queue:** locally durable accepted work and recovery state, allowing a
   node to finish already-downloaded jobs through a network interruption.
3. **OS spooler:** CUPS or Windows owns the native job after handoff.

The hosted layer is therefore a durable delivery queue, but it does not replace
the node or OS queue. New remote work waits while a node is offline. Work that
the node already durably accepted can continue locally. The node reports
recovery when connectivity returns.

Never automatically retry after an ambiguous native handoff. Persist intent
before submission, correlate the native job, and use `delivery_uncertain` when
the result cannot be proven. Operators should check physical output before
releasing or cloning the job.

Expiry prevents obsolete labels from printing after a long outage. Size local
disk for the expected offline window, monitor queue depth/oldest age, and keep
content encrypted and access-controlled.

See [`04-protocol-queues-and-state.md`](../04-protocol-queues-and-state.md) and
[`08-testing-and-reliability.md`](../08-testing-and-reliability.md).
