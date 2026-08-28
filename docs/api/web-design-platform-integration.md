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

## What is embedded and what remains native

The clean integration is not an iframe of the complete Piqae dashboard and it
is not an SDK running with a platform key in the customer's browser. It is a
coordinated product journey with four visible or operational surfaces:

| Surface | Owner | Responsibility |
| --- | --- | --- |
| Host product UI | Integrator | Customer identity, product authorization, template/design experience, print intent, status presentation, and business audit trail. |
| Host product backend/BFF | Integrator | Piqae account mapping, platform credential, environment policy, rendering/preflight, uploads, idempotent jobs, webhooks, and reconciliation. |
| Piqae-hosted connection handoff | Piqae | Short-lived connection session, verified download/application-link routing, and security identity that must not be imitated by partner UI. |
| Piqae native app and durable node | Piqae plus local operator | Explicit local consent, printer selection, OS permissions, driver/profile capture, durable offline queue, native handoff, and local recovery. |

The host product should own the surrounding journey and vocabulary while
Piqae remains visible at security boundaries: installer identity, operating
system permission prompts, one-time connection approval, native diagnostics,
and support evidence. Do not visually spoof those boundaries or describe a
download as a connected printer.

The normal user should not need to understand workspaces, environment IDs,
API keys, connector grants, upload objects, leases, or spooler identifiers.
Translate those into product concepts such as organisation, test setup,
production setup, printer computer, print destination, print attempt, and
needs attention. Preserve the exact underlying IDs and states in the backend
for audit and support.

## Recommended end-user journey

The shortest safe path is progressive: users design first, connect printing
only when they need it, validate setup in Test, and deliberately promote a
known destination to Live.

### 1. Product administrator enables printing

From the authenticated organisation context, the backend gets or creates the
Piqae account using the host organisation's immutable ID. The host stores the
returned account/Test/Live mapping and shows printing as **Not set up**. This
step is server-only and should not expose Piqae credentials or tenant selectors.

If the same administrator repeats setup, reconcile the existing mapping; do
not create a second account. If a mutable organisation slug changes, retain
the original external ID mapping.

### 2. Operator connects a printer computer

Show one primary action, **Connect a printer computer**, to an authorized
operator. The backend creates a short-lived Test connect session and returns
only the intended navigation URL to that user's browser. The operator installs
or opens the native app, reviews the requesting service and customer identity,
selects concrete printers, and approves the connector locally.

The host UI polls the opaque session ID at a bounded interval and presents
distinct results:

- **Waiting for approval** while the session is pending;
- **Connected** only after the API reports `connected` and a node ID;
- **Connection expired** with an action to create a new session; or
- a platform-specific manual/download path when native handoff is unavailable.

Never treat opening the URL, copying a code, downloading a package, or seeing
a process online as equivalent to successful tenant connection.

### 3. Operator configures a destination

After discovery, guide the operator through one deliberate setup wizard:

1. choose a discovered OS printer;
2. open the real installed-driver controls through the native app;
3. capture a named immutable profile revision;
4. define or select the business stock and portable geometry;
5. create a logical target and bind the exact printer/profile revision;
6. inspect readiness reasons; and
7. run the approved Test fixture before publishing the destination.

The host product may present the resulting target as “62 × 29 product label”,
but it must not recreate the vendor driver UI or claim that the expected roll
is physically loaded. Only options explicitly marked safe may be varied per
job.

### 4. Designer uses a stable product destination

The designer selects a business target rather than a machine queue. The host
loads `designSpecification`, builds the canvas from stock geometry, and saves
the `specification_revision` plus the relevant portable snapshot with the
design. If printing is not yet configured, design work can continue while the
product shows that production output still needs setup.

Before final output, re-fetch the specification. A changed revision triggers a
comparison and explicit resolution; it never silently rescales artwork or
falls back to another native profile.

### 5. User submits one print intent

The host backend authorizes the actor, organisation, design version, target,
quantity, and environment; renders and preflights the exact bytes; uploads
them; and registers one job with a stable print-attempt idempotency key. The UI
can immediately show **Queued** after durable registration, with copy such as
“Piqae has accepted this print attempt” rather than “Printed”.

Disable accidental double submission in the browser, but rely on server-side
idempotency for correctness. A user choosing **Print again** creates a new,
linked business attempt; it is not an HTTP retry of the old request.

### 6. Product follows and explains delivery

The backend consumes signed webhooks and runs reconciliation independent of
the browser. The UI groups states into waiting, processing, printing, reported
complete, needs attention, cancelling, and ended, while retaining the exact
Piqae state and event timeline.

Give each actionable condition one clear next step: reconnect an offline node,
ask an operator to fix a blocked native queue, recapture a stale profile, or
reconcile uncertain delivery. Never automatically reprint
`delivery_uncertain`.

### 7. Administrators maintain and offboard

Expose connection health, destination readiness, profile revision, last
successful validation, and revocation to authorized administrators. Connector
revocation removes only that tenant projection; it must not reveal or disturb
other integrators using the same physical installation.

When the customer disables printing, stop new host-product submissions first,
allow already durable attempts to reach a final state, archive the Piqae
account, retain a non-reusable mapping tombstone, and follow the operator's
separate retention/deletion process.

## Integration lifecycle and systems of record

| Phase | Host product record | Piqae record | Completion condition |
| --- | --- | --- | --- |
| Provision | Organisation and immutable external ID | Account plus Test/Live environments | Mapping stored and repeatable |
| Connect | Setup session reference and initiating actor | Expiring connect session and connector grant | API reports connected node ID |
| Discover | Product-visible location/role mapping | Tenant-scoped node and printer snapshots | Intended queue is visible and online |
| Configure | Business destination/template association | Stock, target, binding, immutable profile | Readiness selects an exact binding |
| Validate | Test result and operator acknowledgement | Test job and complete event history | Declared fixture passes required evidence |
| Design | Artwork version and portable geometry snapshot | Design specification revision | Saved constraints match current revision |
| Submit | One immutable print-attempt record | Upload and idempotently created job | Job ID durably stored by both systems |
| Observe | Business status plus deduplicated webhook ledger | Job state and ordered events | Terminal or explicitly reconciled outcome |
| Recover | Authorized decision and replacement relationship | Original uncertain/failed job plus optional new job | No ambiguous automatic duplicate |
| Revoke/offboard | Disabled integration and retention tombstone | Revoked connector or archived account | New access stopped; durable work handled |

The host product is authoritative for who may print, what business action the
output represents, design versions, and operator decisions. Piqae is
authoritative for tenant-scoped printing resources, durable job/event history,
node acceptance, and observed native state. The installed OS driver is
authoritative for vendor settings. None is independently authoritative for
physical paper delivery without stronger hardware evidence.

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
| List print templates | list targets/stocks and apply product roles and location visibility |
| Get template specification | retrieve the consolidated portable design specification and its revision |
| Get connection state | list nodes and eligible printers; return a product onboarding action when absent |
| Submit print | authorize the design, render/preflight PDF, upload, and create one idempotent job |
| List/retrieve print attempts | list or retrieve jobs and map the full Piqae state without erasing detail |
| Cancel | request cancellation; show the returned state rather than promising physical cancellation |
| Reconcile uncertain delivery | record an authorized operator decision; a replacement is a new print attempt |

Apply the product's roles and location rules before returning printers or
templates. Piqae's tenant boundary does not replace application-level rules
such as which store, team, or user may use a particular target.

A practical host API can remain small and stable even as Piqae evolves:

| Host endpoint example | Required authorization and behavior |
| --- | --- |
| `GET /printing/setup` | Administrator/operator role; return mapped setup/readiness state without secrets. |
| `POST /printing/connect-sessions` | Operator role, CSRF protection and rate limit; create one short-lived session for the authenticated organisation. |
| `GET /printing/connect-sessions/:id` | Same organisation and authorized actor; return only product-safe state. |
| `GET /printing/destinations` | Apply organisation/location visibility; return targets and portable design summaries, not opaque native blobs. |
| `GET /printing/destinations/:id/specification` | Authorize target ownership; return geometry, revision and actionable readiness. |
| `POST /printing/attempts` | Authorize design/version/quantity/environment, preflight server-side, create one immutable idempotent attempt. |
| `GET /printing/attempts/:id` | Authorize business-record visibility; return grouped and exact state plus a redacted timeline. |
| `POST /printing/attempts/:id/cancel` | Authorized operator; relay cancellation as a request and display the returned state. |
| `POST /printing/attempts/:id/replacements` | Authorized operator plus reason; create a new linked attempt, never mutate/reuse the old key. |

Do not implement a generic authenticated `/piqae/*` proxy. A narrow BFF keeps
tenant resolution, role checks, environment selection, quantity policy,
redaction, and lifecycle language under the host application's control.

## Discovering design specifications

During node connection, the operator can allow all printers on the computer or
limit the integration to selected printers. For a managed design editor,
`all_local_printers` is usually the cleanest choice: newly installed printers
appear automatically without reconnecting the node. Use
`selected_printers` when a site requires printer-by-printer separation.

Discovery remains authorization-scoped. The editor receives only the printers,
stocks, capabilities, and captured driver profiles available to its connector,
even when the same physical node is connected to other platforms.

Use logical targets as the customer-facing printing choice. A target associates
a stable business stock with exact printer/profile revisions and can select an
eligible primary or standby binding before native acceptance.

For each template selection, call
`targets.designSpecification(target.id)`. The consolidated projection contains
the target, stock, readiness, exact binding/printer/profile destinations, and a
`specification_revision`. It prevents each integrator from implementing a
different multi-request join. Use its fields as follows:

1. identify the readiness-selected binding and its destination;
2. retain the binding's exact immutable profile revision;
3. derive the canvas from stock width, height, orientation, bleed, safe area,
   gap, and marks that are actually present;
4. compare stock geometry with `profile.summary.dimensions_mm` and block when
   it is absent or materially incompatible; and
5. require the destination's `media_compatibility.status` to be `ready` and
   show its actionable reasons otherwise; and
6. expose only portable profile facts such as media, source, colour, duplex,
   and resolution.

Do not expose or reproduce opaque PrintCore, DEVMODE, PrintTicket, PostScript,
or vendor-driver settings. The installed driver remains authoritative. Driver
capabilities and configured dimensions also cannot prove that an operator
loaded the expected physical roll, sheet, tray, ink, or finishing hardware.
The `loaded_media` member is evidence rather than a default: it includes the
source, confidence, observation and expiry timestamps, and exact stock
revision. Piqae authorizes a new handoff only from fresh `reported` or
`operator_confirmed` evidence with current calibration. Absence, unknown or
inferred confidence, and observations older than 15 minutes fail closed.

### Specification revision and saved designs

Printer profile revisions are immutable, but stocks and targets can be updated.
Store the returned `specification_revision` and the portable specification
snapshot when a design is saved:

```json
{
  "stock_id": "stk_...",
  "target_id": "tgt_...",
  "specification_revision": "...",
  "binding_id": "tbd_...",
  "printer_id": "prn_...",
  "profile_id": "prf_...",
  "profile_revision": 4,
  "geometry": {"width_mm": 62, "height_mm": 29, "bleed_mm": 1.5}
}
```

Before printing, retrieve the specification again and compare its revision. If
it changed, compare the saved and current snapshots, require preflight, and ask
for an explicit user decision when production or design constraints changed.
Never silently scale an existing design or silently fall back from its pinned
native profile. The revision is a change detector for the current projection,
not proof that the correct physical stock is loaded.

## No-node onboarding

When `nodes.list()` has no usable Live node, show a single product action such
as **Connect a printer computer**. From the trusted backend, create an
account-scoped connect session:

```ts
const session = await account.connectSessions.create({
  name: 'Packing room',
  return_url: 'https://design.example.com/settings/printing',
  expires_in_seconds: 600
});
```

The implemented-preview session lasts 60–900 seconds and returns a one-time
`connect_url` plus macOS, Windows, and Linux download choices. New sessions use
the verified `https://app.piqae.com/connect` link shape;
the old `piqae://connect` transport is deprecated compatibility for existing
Preview builds and is no longer emitted. Send only the connect URL to the
intended authenticated user. Its URL fragment contains a
short-lived enrolment capability; never log, persist in analytics, place in a
referrer-bearing query string, or reveal it to another customer. Return URLs
must use HTTPS, except localhost HTTP during development, and cannot contain
credentials or fragments.

The public downloads page selects the visitor's OS and consumes the fragment
in browser memory. It immediately removes the fragment from the address bar,
does not put the capability in web storage or page text, and exposes it only
through an explicit **Copy one-time connection code** fallback. Clear the
clipboard after manual setup.

The macOS source handles the verified HTTPS Universal Link, previews the invitation before
consumption, distinguishes the authenticated platform service account from the
customer workspace, requires an idle node and explicit initially-unchecked
printer selection, proves possession of the existing installation key, and
passes secrets to the agent only through bounded standard input. The native UI
ignores any fragment-supplied return URL and follows only the HTTPS return URL
bound in server-side invitation state after successful connector persistence.
The web origin publishes `/.well-known/apple-app-site-association` only when
`APPLE_TEAM_ID` is a valid ten-character signing Team ID; it fails closed with
503 when Universal Links are not configured. A browser fallback preserves the fragment only
across the same-origin `/connect` to `/downloads` navigation.

Windows and Linux do **not** currently have this application-link/consent UI.
They expose only the shared headless stdin transaction for controlled
development; use the copy/manual or normal pairing path and do not describe it
as seamless onboarding. A created session or downloaded binary never proves
connection completed. macOS itself remains Preview until signed-package,
clean-install, restart/recovery and physical-printer evidence is recorded.
Poll `account.connectSessions.retrieve(session.id)` with a bounded interval
until it reports `connected` with a node ID or `expired`, then refresh nodes and
printers. There is no pairing-complete webhook in this contract.

Connect sessions improve partner handoff, but do not make this a fully
white-labelled install and do not override the support matrix. Each returned
binary may still be Preview or Disabled. Keep Piqae visible where the
installer, operating-system permissions, code confirmation, or security
identity requires it; surrounding product copy can remain partner-oriented.
Browser approval and enrolment must execute in the intended customer account;
never expose the platform credential to accomplish it.

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
2. re-check target readiness, destination `media_compatibility`, and the saved
   `specification_revision`;
3. render and preflight the exact PDF;
4. compute its byte length and SHA-256 digest;
5. create and PUT a tenant-scoped upload; and
6. create the job using `target_id`, the current `specification_revision`, and
   a stable idempotency key.

PrintPacket target registration validates its declared paged, continuous, or
label media against the explicit business-stock kind and geometry plus the
immutable profile dimensions. Piqae pins the resolved binding/profile/stock
revisions and revalidates them, the driver/profile state, and loaded-stock
evidence after lease claim immediately before native responsibility can be
offered. A fence failure is a durable `job.updated` terminal event with
`target_configuration_changed`, `stock_not_loaded`, or
`document_media_incompatible`; no native responsibility was accepted. After
correction, the host must create an explicitly new attempt. This does not
collapse PDF and printer-native workflows or silently substitute a generic
profile.

Prefer a target to a concrete printer when pre-acceptance failover is safe. Do
not provide both. A useful idempotency key identifies one intended print
attempt, for example `print_attempt_<platform-attempt-id>`. On an ambiguous
HTTP result, repeat the identical request with that key. A user-authorized
reprint is a new attempt and a new key; randomizing a key to escape a conflict
can duplicate physical output.

Store at least the platform print-attempt ID, Piqae job ID, idempotency key,
account/environment, design revision and specification revision, target, requested
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
| Cancelling | `cancel_requested` | Cancellation was requested but output prevention is not yet confirmed |
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
- [Jobs, content, options, and printer capabilities](jobs-content-and-capabilities.md)
- [Uploads and design applications](uploads-and-design-apps.md)
- [Webhooks](webhooks.md)
- [Jobs and statuses](../printing/jobs-and-statuses.md)
- [Pairing and enrolment](../nodes/pairing.md)
- [Reliability and lifecycle](../operations/reliability-and-job-lifecycle.md)
- [Production release](../operations/production-release.md)
