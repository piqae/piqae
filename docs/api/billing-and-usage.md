# Cloud billing and usage

Spool Cloud has one Free plan and one Pro plan. The control plane owns the
entitlement and usage projection; marketing content and Stripe Price objects
must match this contract before Checkout is enabled.

| Plan | Price | Included Live jobs | Nodes | Overage |
| --- | --- | ---: | ---: | --- |
| Free | USD $0 | 100/month | 1 | None |
| Pro monthly | USD $9/month | 25,000/month | 25 | USD $0.25/additional 1,000 |
| Pro annual | USD $90/year | 300,000/annual Stripe period | 25 | USD $0.25/additional 1,000 |

Workspace members and virtual Test jobs are not metered. Pro includes platform
customer accounts; their Live accepted-job usage is charged to the owning
platform workspace.

## What counts

A job adds one unit only when the node reports
`accepted_by_spooler` for the first time in a Live environment. The database
enforces one acceptance unit per job. These do not add another unit:

- Test-environment jobs;
- job registration without spooler acceptance;
- idempotent API retries;
- node lease or download retries;
- later `spooling`, `printing`, `completed_reported`, or
  `delivery_uncertain` events.

`accepted_by_spooler` proves operating-system handoff, not physical output.

## Read current usage

```console
curl https://api.spool.example/v1/billing/summary \
  -H "Authorization: Bearer $SPOOL_API_KEY"
```

Use `GET /v1/usage?month=YYYY-MM` for a tenant-scoped UTC calendar month. The
billing summary includes the effective entitlement, subscription state,
current-period accepted-job usage, active-node count, and whether new Cloud
jobs may be accepted.

The TypeScript SDK exposes the same server projection:

```ts
const billing = await spool.billing.summary();
const july = await spool.usage.retrieve('2026-07');
```

## Quotas and payment state

- Free rejects new Live job registrations with `quota_exceeded` after its
  included usage is exhausted. Existing durable jobs continue.
- Pro records overage and does not stop printing because included usage was
  exceeded.
- A past-due Pro subscription receives the configured grace period. After the
  grace period, new Cloud jobs are rejected while already durable jobs
  continue.
- Test and self-hosted printing are not blocked by Spool Cloud billing.

Always use an idempotency key when creating jobs. A retry of an existing
idempotent request remains readable even when the workspace later reaches a
quota.

## WorkOS and Stripe responsibilities

WorkOS authenticates people and supplies the hosted organisation claim. The
control plane maps that claim to a Spool workspace; a WorkOS organisation ID
must never be treated as a Spool workspace ID without that verified mapping.

Stripe supplies Checkout, Customer Portal, subscription state, and metered
invoice calculation. Signed Stripe webhooks update the Spool billing
projection idempotently. The immutable Spool usage ledger remains authoritative
for printing and is exported to Stripe with stable event identifiers.

Self-hosted deployments report billing capability disabled and make no Stripe
or WorkOS billing calls.
