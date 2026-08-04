# Cloud billing and usage

Piqae Cloud has one Free plan and one Pro plan. The control plane owns the
entitlement and usage projection; marketing content and Stripe Price objects
must match this contract before Checkout is enabled.

| Plan | Price | Included reported-complete Live jobs | Nodes | Overage |
| --- | --- | ---: | ---: | --- |
| Free | USD $0 | 100/month | 1 | None |
| Pro monthly | USD $9/month | 25,000/month | 25 | USD $0.25/additional 1,000 |
| Pro annual | USD $90/year | 300,000/annual Stripe period | 25 | USD $0.25/additional 1,000 |

Workspace members and virtual Test jobs are not metered. Pro includes platform
customer accounts; their reported-complete Live-job usage is charged to the owning
platform workspace.

## What counts

A job adds one unit only when the node reports `completed_reported` for the
first time in a Live environment. The database enforces one billable unit per
job. These do not add a unit:

- Test-environment jobs;
- job registration or spooler acceptance without reported completion;
- idempotent API retries;
- node lease or download retries;
- `failed_retryable`, `failed_terminal`, `blocked`, `cancelled`, `expired`, or
  `delivery_uncertain` terminal outcomes.

`completed_reported` is the strongest completion signal Piqae receives from a
node, operating system, driver, or printer. It is not independent proof that
ink reached paper.

The July 2026 cutover retains immutable legacy `print_job_accepted` ledger rows
for audit and invoice continuity. New usage rows use
`print_job_reported_complete`; the cross-kind uniqueness constraint prevents a
job from being counted twice.

## Read current usage

```console
curl https://api.piqae.example/v1/billing/summary \
  -H "Authorization: Bearer $PIQAE_API_KEY"
```

Use `GET /v1/usage?month=YYYY-MM` for a tenant-scoped UTC calendar month. The
billing summary includes the effective entitlement, subscription state,
current-period reported-complete usage, active-node count, and whether new Cloud
jobs may be accepted.

The TypeScript SDK exposes the same server projection:

```ts
const billing = await piqae.billing.summary();
const july = await piqae.usage.retrieve('2026-07');
```

## Quotas and payment state

- Free rejects new Live job registrations with `quota_exceeded` after its
  included usage is exhausted. Existing durable jobs continue.
- Pro records overage and does not stop printing because included usage was
  exceeded.
- A past-due Pro subscription receives the configured grace period. After the
  grace period, new Cloud jobs are rejected while already durable jobs
  continue.
- Test and self-hosted printing are not blocked by Piqae Cloud billing.

Always use an idempotency key when creating jobs. A retry of an existing
idempotent request remains readable even when the workspace later reaches a
quota.

## WorkOS and Stripe responsibilities

WorkOS authenticates people and supplies the hosted organisation claim. The
control plane maps that claim to a Piqae workspace; a WorkOS organisation ID
must never be treated as a Piqae workspace ID without that verified mapping.

Stripe supplies Checkout, Customer Portal, subscription state, and metered
invoice calculation. Signed Stripe webhooks update the Piqae billing
projection idempotently. The immutable Piqae usage ledger remains authoritative
for printing and is exported to Stripe with stable event identifiers. Checkout
fails closed when the workspace already has a non-terminal subscription, so
plan changes and payment recovery go through Customer Portal rather than
creating duplicate subscriptions.

Piqae snapshots the ending period on renewal and its worker submits durable
overage exports every minute. Production Stripe configuration must keep
usage-based subscription-cycle invoices in draft for a 72-hour finalization
grace period; the private-beta gate proves the export appears on a test-clock
invoice before enabling live Checkout. See
[Production release](../operations/production-release.md#one-time-stripe-setup).

Self-hosted deployments report billing capability disabled and make no Stripe
or WorkOS billing calls.
