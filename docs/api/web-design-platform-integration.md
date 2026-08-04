# Web design platform integration

**Status:** reference architecture for an implemented developer-preview API.
Platform service accounts remain **Disabled** as a production support claim in
the [support matrix](../../release/support-matrix.yaml). Use this flow only for
controlled evaluation until its tenant-isolation, audit, redaction, native
packaging, and fleet-soak release gates pass.

This guide joins the account, node, printer, stock, target, upload, job, and
webhook contracts into one integration for a multi-tenant web design product.
It does not make the browser a trusted Piqae client and it does not treat an
operating-system spooler acknowledgement as proof that paper was produced.

## Architecture and trust boundary

```text
designer browser
    |  product session; design and print intent
    v
design platform backend/BFF
    |  server-only piq_platform_ credential
    |  trusted organisation -> Piqae account mapping
    v
Piqae account (one per customer organisation)
    +-- Test environment: onboarding and virtual checks
    +-- Live environment: nodes, printers, stocks, targets and jobs
            |
            v
       durable customer node -> installed OS driver -> printer
```

Keep the platform credential in the backend's secret manager. Never send it,
tenant-selection headers, webhook secrets, enrolment tokens, document URLs, or
native profile payloads to browser JavaScript. The browser calls the design
platform's own authorization-checked endpoints; the backend resolves the
tenant from that authenticated session.

Use one immutable, non-personal identifier from the platform database as the
Piqae external ID. Do not accept an external ID, workspace ID, or environment
choice directly from a browser request.

```ts
const account = await platform.accounts.getOrCreate(session.organisation.id, {
  name: session.organisation.name
});

// Live is an explicit server-side business decision. Use account.test while
// onboarding or exercising virtual printers.
const piqae = account.live;
```

Maintain a local mapping containing the platform organisation ID, Piqae account
ID, Test and Live environment IDs, account status, and last reconciliation
time. `getOrCreate` is safe to repeat with the same external ID. Never key this
mapping by an email address, display name, or mutable slug.

## The browser-facing product contract

Expose a small BFF contract shaped for the design product rather than proxying
the complete Piqae API. A useful minimum is:

| Product operation | Backend Piqae work |
| --- | --- |
| List print templates | join stocks, targets, bindings, readiness, printers, and profile summaries |
| Get template specification | return portable geometry plus a platform-owned specification fingerprint |
| Get connection state | list nodes and eligible printers; return a product onboarding action when absent |
| Submit print | authorize the design, render/preflight PDF, upload, and create one idempotent job |
| List/retrieve print attempts | list or retrieve jobs and map the full Piqae state without erasing detail |
| Cancel | request cancellation; show the returned state rather than promising physical cancellation |
| Reconcile uncertain delivery | record an authorized operator decision; a replacement is a new print attempt |

Apply the product's roles and location rules before returning printers or
templates. Piqae's tenant boundary does not replace application-level rules
such as which store, team, or user may use a particular target.

## Discovering design specifications

Use logical targets as the customer-facing printing choice. A target associates
a stable business stock with exact printer/profile revisions and can select an
eligible primary or standby binding before native acceptance.

For each template selection:

1. list or retrieve its stock and target;
2. load `targets.bindings(target.id)` and `targets.readiness(target.id)`;
3. retrieve the selected binding's printer;
4. select the binding's exact immutable profile revision;
5. derive the canvas from stock width, height, orientation, bleed, safe area,
   gap, and marks that are actually present;
6. compare stock geometry with `profile.summary.dimensions_mm` and block when
   it is absent or materially incompatible; and
7. expose only portable profile facts such as media, source, colour, duplex,
   and resolution.

Do not expose or reproduce opaque PrintCore, DEVMODE, PrintTicket, PostScript,
or vendor-driver settings. The installed driver remains authoritative. Driver
capabilities and configured dimensions also cannot prove that an operator
loaded the expected physical roll, sheet, tray, ink, or finishing hardware.

### Specification revision and saved designs

Printer profile revisions are immutable, but stocks and targets can be updated.
The current API does not provide one immutable, typed "design specification
revision" resource. Until it does, the design platform should create its own
canonical specification snapshot and fingerprint when a design is saved:

```json
{
  "stock_id": "stk_...",
  "stock_updated_at": "...",
  "target_id": "tgt_...",
  "target_updated_at": "...",
  "binding_id": "tbd_...",
  "printer_id": "prn_...",
  "profile_id": "prf_...",
  "profile_revision": 4,
  "geometry": {"width_mm": 62, "height_mm": 29, "bleed_mm": 1.5}
}
```

Canonicalize the selected portable fields and hash them in the platform. Store
both snapshot and hash with the design. Before printing, reconstruct the current
specification and compare it. If it changed, require preflight and an explicit
user decision; never silently scale an existing design or silently fall back
from its pinned native profile. This is an integrator-owned safeguard, not a
Piqae-provided revision guarantee.

## No-node onboarding

When `nodes.list()` has no usable Live node, show a single product action such
as **Connect a printer computer**. The public downloads page detects the
visitor's OS and exposes only verified artifacts, and browser pairing uses a
short-lived human approval flow. Keep Piqae visible where the installer,
operating-system permissions, code confirmation, or security identity requires
it; surrounding product copy can remain partner-oriented.

The current contract does **not** provide a partner-scoped onboarding-session
API, an SDK method for OS-specific download metadata, partner return URLs, or a
pairing-complete webhook. Therefore do not promise a fully embedded or
white-labelled install. Link to the canonical downloads/pairing experience,
then poll the account-scoped node/printer lists with a bounded interval after
the user returns. Browser pairing approval must execute in the intended
customer workspace; do not approve pairing using a platform credential in
browser code.

After a node appears, guide the customer to:

1. confirm the node name and online state;
2. verify discovered printers;
3. capture and validate driver-native profiles;
4. define stock geometry;
5. bind and publish targets; and
6. run Test/virtual checks before an explicitly authorized physical test.

An installed node that is offline, unauthorized, paused, or attached to the
wrong account is different from no installation. Preserve those states in the
UI and give a corrective action rather than repeatedly offering the download.

## Rendering, upload, and job creation

Render the final document in the trusted platform backend or a controlled
rendering service. Define and test the platform's own PDF contract: page boxes,
physical dimensions, orientation, bleed, scaling policy, embedded fonts,
colour expectations, image resolution, and multi-page policy. Piqae transports
and hands the document to the selected native profile; it does not validate the
visual design against those editorial rules.

For each user print action:

1. authorize the account, target, design revision, quantity, and environment;
2. re-check target readiness and the saved specification fingerprint;
3. render and preflight the exact PDF;
4. compute its byte length and SHA-256 digest;
5. create and PUT a tenant-scoped upload; and
6. create the job using `target_id` and a stable idempotency key.

Prefer a target to a concrete printer when pre-acceptance failover is safe. Do
not provide both. A useful idempotency key identifies one intended print
attempt, for example `print_attempt_<platform-attempt-id>`. On an ambiguous
HTTP result, repeat the identical request with that key. A user-authorized
reprint is a new attempt and a new key; randomizing a key to escape a conflict
can duplicate physical output.

Store at least the platform print-attempt ID, Piqae job ID, idempotency key,
account/environment, design and specification fingerprints, target, requested
quantity, initiating actor, and timestamps. Do not put secrets or sensitive
document content in Piqae metadata.

## Live status and queue presentation

Create an account- and environment-scoped `job.updated` webhook. Verify its
signature against the exact raw body before parsing, durably record the event,
deduplicate by event ID, and only then return 2xx. Deliveries are at least once.
Use polling to reconcile non-terminal jobs and gaps; browser EventSource is not
a substitute for the backend webhook record.

Do not flatten Piqae state into only `success` and `failed`. A product UI may
group states while retaining the original state and event history:

| Product group | Representative Piqae states | Product meaning |
| --- | --- | --- |
| Waiting | `registered`, `content_pending`, `waiting_for_agent` | Durable but not accepted by a node |
| Processing | `agent_downloading`, `agent_accepted`, `queued_local`, `preparing`, `rendering` | Node has begun local work |
| Printing | `spool_intent`, `accepted_by_spooler`, `spooling`, `printing` | Native handoff is in progress; output is not proven |
| Reported complete | `completed_reported` | Strongest driver/spooler report, not independent physical proof |
| Needs attention | `blocked`, `failed_retryable`, `delivery_uncertain` | Operator or bounded policy action is required |
| Ended | `failed_terminal`, `cancelled`, `expired` | This attempt will not progress |

Use the job event stream for the detailed timeline. Cancellation is a request:
once native handoff has started, it may be too late to prevent output.
`delivery_uncertain` must never trigger automatic resubmission because doing so
may duplicate labels or documents. Require an operator to inspect the physical
printer, native queue, and stock, then record whether to accept the result or
create a separately audited replacement attempt.

## Recovery and reconciliation

The design platform should run a bounded reconciler independent of browser
sessions:

- retrieve attempts with no terminal state or a stale webhook timestamp;
- fetch the current job and append missing events to the platform record;
- retry only transport errors, `429`, and explicitly retryable responses with
  capped exponential backoff and jitter;
- reconcile interrupted uploads by retained upload ID rather than creating
  unbounded replacements;
- monitor webhook pending age and delivery failures, and replay only after the
  receiver is idempotent; and
- alert on offline nodes, stale profiles, readiness loss, queue age,
  `failed_retryable`, and `delivery_uncertain`.

Never infer delivery from a browser timeout, an SDK promise resolving, job
registration, node acceptance, or spooler acceptance. Preserve Piqae's event
history alongside the platform's business audit trail.

## Offboarding and data lifecycle

Stop new product print actions first, then call
`platform.accounts.archive(externalId)`. Archive revokes new platform access
to Test and Live but allows already durable jobs to reach a final state. It is
idempotent, does not synchronously erase the workspace, and is not a GDPR or
privacy deletion API. V1 has no public unarchive contract.

Before archive, record the account and outstanding job IDs needed for the
platform's retention and support obligations. After archive, keep the customer
mapping tombstone so the same external ID is not accidentally reused for a
different organisation. Complete deletion/export, retention expiry, legal
hold, and webhook shutdown through the deployment operator's documented data
governance process; do not claim the SDK archive call completed them.

## Production evidence gate

An integration is not production-ready merely because its happy path works.
Before general availability, record evidence for all of the following against
a reviewed release:

| Gate | Required evidence |
| --- | --- |
| Tenant isolation | cross-resource, concurrent request, selector-rejection, revocation, attribution, and redaction suites |
| Durable delivery | database/object-store recovery, lease expiry, reconnect, N/N-1 protocol, and duplicate-prevention soak |
| Webhooks | signature rejection, durable deduplication, retries, replay, backlog alerts, and receiver recovery |
| Rendering | fixture PDFs for every supported template class and explicit no-scaling assertions |
| Native platforms | checked-in support tier, signed package evidence, clean install/upgrade/rollback, and named physical-printer fixtures |
| Operations | quotas, retention, backup/restore, disaster recovery, credential rotation, incident response, and offboarding rehearsal |
| Product UX | no-node, offline, paused, unauthorized, wrong-account, no-ready-target, retryable failure, and uncertain-delivery paths |

At the time of writing, platform service accounts are Disabled, native
platforms are Preview or Disabled, and durable/offline routing features remain
Preview. The authoritative current result is always the checked-in
[support matrix](../../release/support-matrix.yaml), not this narrative guide.

## Related contracts

- [Platform accounts](platform-service-accounts.md)
- [Uploads and design applications](uploads-and-design-apps.md)
- [Webhooks](webhooks.md)
- [Jobs and statuses](../printing/jobs-and-statuses.md)
- [Pairing and enrolment](../nodes/pairing.md)
- [Reliability and lifecycle](../operations/reliability-and-job-lifecycle.md)
- [Production release](../operations/production-release.md)

