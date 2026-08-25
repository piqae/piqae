# Jobs and statuses

**Status:** durable job state machine, local execution transitions, and event
reporting implemented.

Normal high-level flow:

```text
registered → content_pending → waiting_for_agent → agent_downloading
→ agent_accepted → queued_local → preparing → rendering
→ spool_intent → accepted_by_spooler → spooling → printing
→ completed_reported
```

Not every executor reports every intermediate native state. Important terminal
or intervention states include `cancelled`, `expired`, `failed_terminal`,
`failed_retryable`, and `delivery_uncertain`.

`accepted_by_spooler` means the operating-system spooler accepted the job, not
that paper exited the printer. `completed_reported` is the strongest available
reported completion, not independent physical proof. `delivery_uncertain`
means retrying could duplicate output; an operator must reconcile the printer,
native queue, and stock before deciding. The procedure for doing that is
[`operations/uncertain-delivery-response.md`](../operations/uncertain-delivery-response.md).

Store and search by Piqae job ID. Native spooler IDs are node-local correlation
data and may be reused. State transitions and audit events are append-oriented;
do not rewrite history to make a retry look like the original attempt.

The authoritative transition table is
[`04-protocol-queues-and-state.md`](../04-protocol-queues-and-state.md).
