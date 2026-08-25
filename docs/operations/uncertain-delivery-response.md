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

## Subscribing to the signal

### The two events

| Event | Published when | `data` |
| --- | --- | --- |
| `job.delivery_uncertain` | The control plane applies a node event whose state is `delivery_uncertain`, once, on the way into the state | The stored job record |
| `job.delivery_uncertain.unresolved` | A worker sweep finds a job that has been uncertain for longer than the alert threshold | `job_id`, `workspace_id`, `environment_id`, `uncertain_since` |

`job.updated` is published for the same transition and continues to be. The
dedicated event exists so a consumer does not have to subscribe to every job
update and filter.

Subscribe to both, and treat them differently. `job.delivery_uncertain` is
timely and noisy: entering the state is unremarkable, and many of these
questions are answered the moment somebody glances at the printer. It is the
right feed for a dashboard or a log. `job.delivery_uncertain.unresolved` is the one that has
earned a human's attention, because the job stayed uncertain past the
threshold and was never surfaced before. It is the right feed for a ticket.

The unresolved payload is deliberately small — it carries no title, printer, or
node — so a consumer that wants a useful ticket must call
`GET /v1/jobs/{job_id}` and `GET /v1/jobs/{job_id}/events` for the rest.

The `job.delivery_uncertain` payload is the stored job record, which is not the
same shape as the `JobResponse` returned by `GET /v1/jobs/{job_id}`: it also
carries `workspace_id`, `environment_id`, `options`, and a `content` reference.
Document bytes are never in it — base64 content is written to object storage
before the job is persisted, and URI credentials are rejected at registration —
but `title` and `metadata` can carry customer data. Apply the same handling
rules as [`diagnostics.md`](../nodes/diagnostics.md) before forwarding it into a
chat channel or a ticket body.

### Creating the subscription

```console
curl -X POST https://api.example.com/v1/webhooks \
  -H 'authorization: Bearer <api key with webhooks_write>' \
  -H 'content-type: application/json' \
  -d '{"url":"https://ops.example.com/piqae/uncertain",
       "events":["job.delivery_uncertain","job.delivery_uncertain.unresolved"]}'
```

The response returns the endpoint plus a `whsec_...` signing secret **once**.
Store it in your secret manager before closing the terminal; it cannot be read
back.

Three details decide whether this works:

- **Event names are matched exactly.** The outbox selects endpoints with
  `$3 = ANY(subscribed_events)`. There is no wildcard expansion anywhere in the
  path, so `job.*` matches nothing. The dashboard's "event families" checkboxes
  currently submit the literal values `job.*`, `agent.*`, and `printer.*`, so an
  endpoint created through the settings dialog receives no deliveries. Create
  these subscriptions through the API or the SDK with full event names.
- **Endpoints are scoped to one workspace and one environment.** An endpoint
  created with a Test key never sees Live events. Subscribe in both, or accept
  that you are only watching one.
- **The destination must be publicly resolvable.** `localhost`, `*.localhost`,
  loopback, unspecified, private, link-local, and unique-local addresses are
  rejected at creation and again at delivery, with no configuration override.

Confirm with `GET /v1/webhooks` (scope `webhooks_read`) that the stored
`events` array contains the two full names.

### Verifying the signature before processing

Piqae sends, per attempt:

```text
content-type: application/json
user-agent: Piqae-Webhook/1.0
piqae-event-id: <event id, stable across retries>
piqae-timestamp: <unix seconds>
piqae-signature: v1=<base64 HMAC-SHA256>
piqae-attempt: <1-based attempt number>
```

The signed value is the decimal timestamp, a single `.`, then the exact raw
request body:

```text
HMAC-SHA256(key = utf8("whsec_..."), message = "<piqae-timestamp>.<raw body>")
```

The key is the UTF-8 bytes of the whole secret including the `whsec_` prefix,
not a decoded form of it. The digest is standard base64. Verify against the
bytes you received: re-serializing parsed JSON changes the body and the
signature will not match.

The TypeScript SDK ships this as `verifyWebhookSignature` from `@piqae/sdk`,
which is Web Crypto based and defaults to a 300-second timestamp tolerance:

```ts
const raw = await request.text();
if (!(await verifyWebhookSignature(secret, raw, request.headers))) {
  return new Response('invalid signature', { status: 400 });
}
const event = JSON.parse(raw);
```

Implementing it yourself is the same four steps: reject missing or duplicate
headers, reject a timestamp outside your tolerance, compare in constant time,
and only then parse. [`api/webhooks.md`](../api/webhooks.md) is the reference.

The envelope is:

```json
{ "id": "evt_...", "type": "job.delivery_uncertain.unresolved",
  "created_at": "...", "data": { } }
```

`id` is also the `piqae-event-id` header. Deduplicate on it durably: delivery is
at least once, and a manual replay of the same delivery re-sends the same
event ID by design.

### Delivery, retries, and giving up

The control-plane worker claims due deliveries every 500 ms, up to 25 per
batch and 8 concurrently. Each attempt gets a 10-second HTTP timeout, follows
no redirects, and is pinned to the first address the destination host resolved
to. A non-2xx response, a timeout, or a blocked destination is a failure.

Failures retry on a fixed schedule with about 10% jitter: 5 s, 30 s, 2 min,
10 min, 1 h, 6 h, 24 h. That is eight attempts spanning roughly 31 hours, after
which the delivery is dead-lettered and never retried automatically.

An endpoint that is down for two days therefore loses the signal permanently
unless somebody replays it. Check `GET /v1/webhooks/{webhook_id}/deliveries`
for rows with `dead_lettered_at` set, and replay with
`POST /v1/webhook-deliveries/{delivery_id}/replay` once the receiver is
durably repaired. Monitor webhook backlog and attempt counts as described in
[`monitoring.md`](monitoring.md); a silent alerting path is worse than none.

### Pulling instead of receiving

If the receiver cannot be a public HTTPS endpoint — a common, first-class case
for a self-hosted deployment on a private network — do not try to defeat the
destination policy. Consume `GET /v1/events/stream` instead. It is a
server-sent event stream over the same authenticated API, scoped to one
workspace and environment, requiring scope `jobs_read`. Each event carries the
Piqae event ID as the SSE `id` and the event type as the SSE event name, so
`last-event-id` on reconnect resumes from your cursor. Both uncertain-delivery
events appear on it, because the stream reads the same durable event table the
webhook outbox does.

The stream has no signature: it is authenticated by the API key on a TLS
connection you opened, so there is nothing to verify. It also has no retry
schedule and no dead letters — the cursor is yours to persist.

## Routing it somewhere a human notices

Piqae deliberately has no opinion about the destination, and self-hosted
operation is not a degraded path here: the receiver is an HTTPS endpoint you
control, or the event stream you poll. Whatever you choose has to satisfy four
properties.

- Somebody is accountable for it during the hours the printers run.
- It can hold an item open until a person closes it. A chat message scrolls
  away; uncertain delivery needs a thing that stays open.
- It is searchable by Piqae job ID months later, because that is the only
  identifier that ties the ticket to the event history.
- It does not page. This is not an outage, and paging on it recreates exactly
  the fatigue that keeping it out of Sentry was meant to avoid.

An issue tracker satisfies all four. Linear is the tool the Piqae operator
already triages in, so it is a reasonable default: a small receiver verifies
the signature, then creates one issue per uncertain job. Nothing in Piqae knows
or cares that it is Linear — a ticket queue, an internal ops app, a shared
mailbox watched by the shift lead, or a row in your own database all work the
same way. No vendor is required and none is integrated in the product.

The receiver itself is the same shape whichever destination you pick:

1. read the raw body, verify `piqae-signature`, reject on failure;
2. look up `piqae-event-id`; if already processed, return 2xx and stop;
3. key the work item on the Piqae job ID, not the event ID, and upsert. The
   timely event and the later unresolved event then collapse into one item
   instead of two, and a replayed delivery updates rather than duplicates;
4. persist durably, then return 2xx. A 2xx before the write means a lost
   signal that will never be retried;
5. put the job ID, the printer, the last event message, and `uncertain_since`
   in the item, and a link to the job. Keep the document title out of any
   channel wider than the people already entitled to see the document.

If you route the timely `job.delivery_uncertain` anywhere at all, route it
somewhere ambient — a queue view, a log, a counter on a dashboard. The item
that a human is expected to act on should come from
`job.delivery_uncertain.unresolved`.

## What to do when it fires

This is a judgement call, and the runbook cannot make it for you. Reprinting
risks a duplicate physical document. Doing nothing risks a document that never
existed. Piqae has no way to distinguish the two after the fact, which is the
entire reason the state exists. What follows narrows the judgement, not
removes it.

### 1. Gather the evidence Piqae has

- `GET /v1/jobs/{job_id}` — title, printer, and your own `metadata`, which is
  usually what ties the job back to an order or consignment.
- `GET /v1/jobs/{job_id}/events` — the append-only history in sequence order.
  The last event is the one that matters. Two fields carry most of the signal:
  - `message` identifies which of the five paths above produced the state;
  - `native_job_id` is `null` when the spooler never returned an identifier,
    and set when the job did exist in the native queue and was then lost track
    of. A job that reached the native queue is materially more likely to have
    printed than one whose handoff is itself in doubt.
- `agent_id` on that event names the node to go and look at.

Do not rewrite this history to make a later attempt look like the original.

### 2. Gather the evidence Piqae does not have

Physical evidence beats every inference available from the API.

- The printer's output. For a label or receipt printer, the last item in the
  tray or on the roll answers the question outright.
- The printer's own counter or job log, where the hardware keeps one.
- The operating system's queue view on that node, searched for the
  `native_job_id` if there is one.
- The node's `agent.log` (plus four rotated generations, per
  [`diagnostics.md`](../nodes/diagnostics.md)) around `occurred_at`.
- The downstream system. If a picker scanned the label or the order moved on,
  the document reached the world.

### 3. Decide

Two questions decide it.

**What does a duplicate cost?** Sort the document into one of two classes
before anything else:

| Class | Examples | Default |
| --- | --- | --- |
| Consumes something scarce or externally visible | Carrier label with a live tracking number, cheque, ticket, serial-numbered or controlled form, anything already handed to a third party | A duplicate is an incident in its own right. Do not reprint without positive evidence that nothing printed |
| Safely re-printable | Picking list, packing slip copy, internal worksheet, shelf label | A duplicate is waste paper. Reprint and move on |

**Can anyone still look?** Time and attendance decide whether physical evidence
is retrievable at all. Within minutes, on an attended printer, go and look —
that is the cheapest and most conclusive step in this entire document. Hours
later, on an unattended printer in a warehouse, the paper has moved and the
evidence is gone.

Combining them:

- evidence says it printed → do not reprint; close the item with the evidence;
- evidence says it did not print → reprint as a new attempt;
- no evidence, safely re-printable → reprint;
- no evidence, duplicate is harmful → do not decide alone. Escalate to whoever
  owns the downstream process — the person who can void a duplicate label or
  reconcile a numbered form — and let them choose. Record that you asked.

Never bulk-retry a set of uncertain jobs, and never let an automated retry loop
near this state. The same rule appears in
[`incident-response.md`](incident-response.md),
[`production-release.md`](production-release.md), and
[`backups-and-restore.md`](backups-and-restore.md) because it is the one
mistake that turns a recoverable ambiguity into a customer-visible one.

### 4. Reprint, if that is the decision

There is no server-side reprint of an uncertain job. Two paths exist.

- **Re-issue from the system that created the job.** Submit a new job through
  your integration with a *new* idempotency key and metadata naming the
  original job ID. Reusing the original key returns the original job instead of
  printing, which is correct behaviour and not what you want here.
- **Reprint retained content on the node.** The node's local queue view offers
  a confirmed reprint for terminal attempts, including `delivery_uncertain`,
  when the printer is still present and the content file is still retained. It
  requires explicit confirmation, and the new attempt's identity is derived
  from the original job ID plus the caller's idempotency key, so replaying the
  browser request cannot produce a second print. The result is a node-local
  attempt titled `Reprint — <original title>`; it is not a cloud job and will
  not appear in `GET /v1/jobs`. Reaching it is described in
  [`local-agent-control.md`](../architecture/local-agent-control.md).

### 5. Close the loop yourself

Nothing in Piqae records that a human resolved an uncertain job. The job stays
`delivery_uncertain` permanently, and `delivery_uncertain_alerted_at` is an
idempotency fence — it guarantees the sweep surfaces a job once — not a
resolution flag. Your tracker is the system of record for the decision, the
evidence behind it, and who made it.

## Configuration

| Setting | Default | Notes |
| --- | --- | --- |
| `PIQAE_DELIVERY_UNCERTAIN_ALERT_SECONDS` | `900` | Age at which an uncertain job is surfaced as unresolved. Clamped to 60–86400 seconds; an unparseable value falls back to the default. Also accepted as `SPOOL_DELIVERY_UNCERTAIN_ALERT_SECONDS` |
| `PIQAE_SERVICE_ROLE` | `all` | The sweep and the webhook worker run only in the `worker` and `all` roles. A deployment split into `api` and `sync` roles must run at least one worker or nothing is ever surfaced or delivered |

The sweep runs once a minute and claims at most 50 jobs per pass, so worst-case
detection latency is the threshold plus about a minute, and a backlog drains at
50 jobs per minute rather than storming the receiver.

Treat the threshold as a debounce on human attention rather than a timeout:
the state is terminal and the job will never leave it on its own. Raise it if
your printers are attended and people notice problems before a ticket would
help. Lower it toward the floor if printing is unattended and nobody is going
to look until the ticket exists.

## Known gaps

Stated plainly, because assuming any of these work would cost a document.

- There is no dashboard view of uncertain jobs. Reconciliation is
  `GET /v1/jobs?state=delivery_uncertain`, paginated with `after`.
- There is no server-side reprint and no recorded link from a reprinted job
  back to the uncertain one beyond metadata you set yourself.
- Nothing marks an uncertain job resolved.
- A job is surfaced as unresolved once. If your receiver was down for the full
  ~31-hour retry window and the dead-lettered delivery was never replayed,
  nothing re-surfaces it. Run the reconciliation query on a schedule.
- The node-side uncertainty deadline is a fixed ten-minute constant. The
  workspace-configurable timers listed in
  [`reliability-and-job-lifecycle.md`](reliability-and-job-lifecycle.md) are a
  requirement, not a description of today's behaviour.
- None of this establishes that ink reached paper. `completed_reported` does
  not either. Piqae reports the strongest evidence it has and refuses to
  overstate it.
