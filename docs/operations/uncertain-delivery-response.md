# Uncertain delivery response

**Status:** the `delivery_uncertain` state, its node-side producers, signed
webhook delivery with retries and replay, and the tenant event stream are
implemented. The dedicated `job.delivery_uncertain` event, the unresolved
sweep, `PIQAE_DELIVERY_UNCERTAIN_ALERT_SECONDS`, and the
`delivery_uncertain_since` / `delivery_uncertain_alerted_at` columns land with
`feat/uncertain-delivery-event` (PR #141) and are not on `main` yet. Routing,
triage, and physical reconciliation are operator-owned and manual. Nothing here
produces proof that ink reached paper.

This is the runbook
[`reliability-and-job-lifecycle.md`](reliability-and-job-lifecycle.md) names as
required evidence before a paid beta: the procedure for one uncertain delivery,
end to end. For fleet-wide impact use
[`incident-response.md`](incident-response.md) instead.

## What `delivery_uncertain` means

A job reached the point where the node handed it to the operating-system
spooler, and then the node could not establish what happened next. It is not a
failure and not a success. The distinction between accepted, printing, reported
complete, and uncertain is a product invariant, not an implementation detail;
see [`jobs-and-statuses.md`](../printing/jobs-and-statuses.md) and
[ADR-0001](../architecture/adr-0001-rust-postgres-durable-edges.md).

The state is terminal. `JobState::is_terminal` includes it and the transition
table in `crates/domain/src/job.rs` has no edge out of it, so nothing in Piqae
will move the job again. Only a human decides what happens next.

### How a job gets there

Every path is in `crates/agent-core/src/lib.rs`, and every one records the
failure reason `ambiguous_handoff`. The reason code does not discriminate
between them; the event `message` does.

| Message on the job event | What the node knows |
| --- | --- |
| The executor's own error text | `submit` returned an error whose `handoff_may_have_succeeded` flag was set. The call may or may not have reached the spooler |
| `Native reconciliation schedule has no spooler identifier` | The handoff completed without yielding a native job ID to reconcile against |
| `Native job could not be observed before the uncertainty deadline` | The executor could not be queried at all before the deadline |
| `Native spooler could not prove the final job outcome` | The native job was reported missing or unknown at the deadline |
| `Cancellation outcome could not be proved` | A cancel was requested after handoff and its result could not be established |

The uncertainty deadline is `AgentEngine::DEFAULT_UNCERTAINTY_AFTER_MS`, a
fixed ten minutes from native acceptance. It is a compile-time constant today,
not a configurable timer.

The node never automatically resubmits after an ambiguous handoff. A blind
retry can produce a duplicate physical label, invoice, cheque, or dispatch
document, and Piqae cannot tell the difference afterwards.

## Why this is not an error, and not a Sentry issue

Nothing threw. The API returned 200, the job was durably registered, the node
accepted it, the spool intent was persisted before the native call, and the
state machine recorded the only honest outcome available. Every component
behaved exactly as designed. The uncertainty is a property of the world —
printer firmware, driver, USB cable, power — not of the code.

Routing it into exception monitoring is therefore wrong in both directions:

- as an alert, it would fire on an outcome nobody can fix by changing code, and
  an alert that cannot be acted on by its recipient trains that recipient to
  dismiss the whole channel, including the exceptions that do matter;
- as a metric, it would inflate the error rate with events that are not errors,
  so the error rate stops meaning "something is broken" and stops being usable
  as a release gate.

The implementation matches that reasoning. The sweep logs a stuck job with
`tracing::warn!`, and the Sentry layer in
`crates/control-plane/src/observability/error_reporting.rs` maps `WARN` to a
breadcrumb, not an event; only `ERROR` becomes an issue. The sweep reserves
`tracing::error!` for its own plumbing failing — the claim query erroring, the
payload failing to serialize, the event failing to enqueue. Those are real
bugs, and those are the only things Sentry should see here. See
[`observability.md`](observability.md) for the full reporting and redaction
contract, which is optional and off unless `SENTRY_DSN` is set.

Uncertain delivery belongs in a work queue, next to the other things a person
has to look at and close. The rest of this document is how to get it there.
