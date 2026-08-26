# Production release and promotion

**Status:** Railway private-preview deployment and managed-HA foundations are
implemented; Piqae Cloud is not yet approved for a 99.95% availability claim.

The release decision is fail-closed. A successful build, Helm render, Terraform
validation, or virtual print does not approve a production promotion.

## One-command preflight

Run repository-only checks during development:

```console
./deploy/production-check.sh structural
```

Run the Railway private-beta release gate with protected, populated
configuration and external evidence:

```console
PIQAE_PRODUCTION_RAILWAY_ENV_FILE=/protected/railway-production-web.env \
PIQAE_PRODUCTION_EVIDENCE_DIR=/protected/release-evidence \
  ./deploy/production-check.sh release
```

The command does not deploy, publish, sign, or print. It rejects missing
configuration, example domains, unfinished hosted integrations, absent
external evidence, and unsafe rollout semantics without printing secret
values.

The future managed-HA Cloud Run/GCP target uses
`./deploy/production-check.sh managed-ha` and additionally requires
`PIQAE_PRODUCTION_TFVARS_FILE` plus regional DR evidence. Those scale-up gates
do not block the controlled Railway private beta.

## Current gate classification

| Area | Classification | Repository evidence | Remaining release evidence |
| --- | --- | --- | --- |
| Compose | Preview | `deploy/self-host/docker-compose.yml`, readiness rendering | backup/restore and upgrade drill |
| Helm | Preview | digest example, pre-upgrade migration Job, PDB/HPA/topology/network policy | Kind install/upgrade/rollback and disruption run |
| Cloud Run | Preview foundation | role-separated two-region Terraform, guarded API/sync invocation, readiness, staged two-region promotion and rollback workflow | rehearsed promotion and failure evidence |
| Cloud SQL | Preview foundation | Enterprise Plus regional HA, PITR, retained backups, DR replica, generated database identity and authenticated Cloud Run connector | fenced promotion and restore rehearsal |
| GCS | Preview foundation | native ADC-backed runtime adapter, bucket IAM and versioned dual-region bucket | deployed end-to-end object and retention checks |
| WorkOS | Preview | OIDC validation and hosted environment contract | production tenant/session/invitation run |
| Stripe | Preview | canonical Free/Pro checkout validation, signed idempotent webhook projection and durable overage exporter | live-mode price/webhook/meter replay evidence |
| Sentry | Preview | server/browser SDK, aggressive PII redaction and release-bound source-map upload | production project and redaction evidence |
| Native signing | Partial Preview evidence | `v0.1.11` macOS Developer ID signing, notarisation, stapling, signed appcast, checksum, SBOM and repository-bound provenance | Windows Microsoft Artifact Signing identity/profile and signed candidate; macOS/Windows clean-install, update and rollback evidence |
| Availability | Blocked externally | lease/outbox architecture and HA foundations | regional DR rehearsal and 30-day no-loss soak |

`release/support-matrix.yaml` remains the public source of support claims.

## Railway private-preview promotion

Railway is the canonical current web, API, and worker host. Promotion is
staging-first:

The reviewed `release/product-release.yaml` is authoritative for coupled
component order. A Shopify release that consumes a new API contract cannot be
promoted independently: migrations and the compatible control plane go first,
then workers and web, then Shopify, and finally a desktop-node canary. Every
component records the same source commit and immutable artifact/deployment ID
in the release evidence. Rollback restores application digests; database
migrations are forward-only and must remain compatible with N and N-1.

1. Build and attest the web and server candidate from one reviewed commit.
2. Deploy that commit to the isolated Railway `staging` environment.
3. Run backward-compatible migrations exactly once, then deploy staging API,
   worker, and web.
4. Verify `/v1/health`, `/v1/ready`, `/v1/meta`, authenticated dashboard
   access, object digest fetch, fake-print lifecycle, webhook delivery,
   reconnect, queue age, and tenant isolation.
5. Back up the production database and referenced objects and bind them into
   one restore checkpoint.
6. Record the staging commit, build/deployment IDs, migration version, checks,
   approver, and observation window.
7. Promote that exact commit to production. Do not rebuild an unreviewed tree.
8. Run the compatible production migration once; keep ordinary replicas from
   running DDL.
9. Deploy the API and verify readiness before the worker. Never percentage
   split worker revisions.
10. Deploy the web only after the production API contract is healthy.
11. Keep every previous Railway deployment selectable until the observation
    window closes.

A failed production application deployment rolls back to the prior Railway
deployment. Migrations do not roll back automatically. Durably registered jobs
remain in PostgreSQL, already leased jobs remain in node SQLite, and ambiguous
native handoffs remain `delivery_uncertain`.

This one-region preview is operational for controlled use, but it is not
highly available and does not support a 99.95% claim. See
[Railway low-cost private preview](railway-private-preview.md).

## Required managed-HA production order

1. Build and attest immutable server, migration, web, macOS, and Windows
   candidates from the same reviewed commit.
2. Verify SBOMs, checksums, repository-bound provenance, code signatures,
   update metadata, the declared server/schema compatibility matrix, the
   fail-closed node projection behavior, and the support matrix. N/N-1 is not a
   blanket native-handoff promise.
3. Back up PostgreSQL and object storage and prove the restore checkpoint is
   readable.
4. Run backward-compatible migrations exactly once with the migration image.
   Do not let ordinary API/sync/worker replicas run DDL.
5. Deploy the new server revision with zero unavailable capacity. Keep the
   previous digest addressable and do not move all traffic immediately.
6. Gate traffic on `/v1/ready`; verify job registration, node sync, object
   digest fetch, webhook delivery, queue age, and tenant isolation using
   non-physical canaries.
7. Shift traffic in bounded stages, observing errors, pickup latency, event
   propagation, database pressure, object failures, and duplicate-handoff
   alarms at each stage.
8. Hand worker services over one region at a time without percentage-splitting
   worker revisions. PostgreSQL outbox leases remain the duplicate-processing
   boundary while both regions are available.
9. Promote the web only after the API contract is healthy. Then release a small
   signed node canary cohort. Require current route projection health and fresh
   route telemetry; widening while jobs are held with
   `node_upgrade_required` is a failed gate, not a reason to bypass fencing.
10. Record the exact commit, digests, six prior and six promoted Cloud Run
   revisions, configuration revision, migration version,
   evidence links, approver, and observation window.

The protected promotion workflow requires the previous API, sync, and worker
revision in both regions. It creates all six candidates without traffic, checks
API/sync candidate readiness directly, stages API/sync at 5%, 25%, and 100%,
then hands workers over one region at a time. A failure restores every service
whose candidate was created. Database migrations are not reversed; they must
remain compatible with both application versions.

## Rollback and database rules

Application rollback restores the previous immutable digest and traffic split.
It must not run down-migrations automatically. Every migration must remain
compatible with N and N-1 server versions throughout the rollout. If a schema
change cannot be expanded and contracted safely, stop the release and schedule
a separately rehearsed maintenance operation.

When a release adds destination-route reservations, older nodes may continue
to report presence and inventory but must not receive new handoffs until they
publish a current route projection. Existing locally accepted work remains on
its original node. A rollback plan must preserve that safe hold and must never
reroute an ambiguous post-spooler attempt merely to restore throughput.

During regional database promotion, fence the old writer before accepting
writes in the secondary region. Reconcile jobs near the recovery point by
event and native spooler evidence; never bulk-resubmit `delivery_uncertain`
jobs. A control-plane outage leaves durably registered jobs waiting, and
connected nodes preserve already leased work in their local SQLite queues.

## Hosted configuration contract

[`apps/web/.env.example`](../../apps/web/.env.example) is the dashboard runtime
contract; protected values live in separate Railway staging and production
environments. Piqae Cloud has exactly Free and Pro plans. The production
preflight requires WorkOS, Stripe, Sentry, public domains and release metadata,
while rejecting blank values and non-HTTPS origins.

### One-time WorkOS setup

1. Create separate WorkOS applications for Preview and Production.
2. Configure each production role (`owner`, `admin`, `developer`, `operator`,
   `viewer`, and `billing`) and grant the API permission names Piqae expects,
   including `usage_read` for billing pages. Keep server-side permission checks
   authoritative.
3. Set the organisation claim to `org_id` and permissions claim to
   `permissions`; set the exact issuer, JWKS URL, and application binding in
   protected Terraform variables.
4. Add only the production site callback and logout URLs. Store the AuthKit
   variables as protected Railway production web-service variables.
5. Prove workspace switching, role refresh, removal, and cross-workspace denial
   with two real organisations before enabling public signup.

### One-time Stripe setup

Create one Pro product with four recurring Prices:

| Price | Amount | Interval | Usage |
| --- | ---: | --- | --- |
| Pro monthly base | USD $9.00 | month | licensed |
| Pro annual base | USD $90.00 | year | licensed |
| Pro monthly overage | USD $0.25 | month | metered |
| Pro annual overage | USD $0.25 | year | metered |

The two metered Prices use the same Billing Meter event name configured as
`stripe_meter_event_name` in Terraform. Its customer key is
`stripe_customer_id`, value key is `value`, and aggregation is `sum`. Piqae
submits integer 1,000-job overage blocks—not raw print counts—after closing a
subscription period. The worker checks every 60 seconds, claims exports
durably, and retries transport failures without changing the Stripe event
identifier. Apply this Price metadata:

```text
piqae_plan=pro
piqae_metric=reported_complete_live_jobs_overage
piqae_included_jobs=25000       # monthly; use 300000 for annual
piqae_overage_unit=1000
```

Put each Price’s unique lookup key—not its displayed amount—in the matching
`STRIPE_PRICE_*` Railway production variable. Register the control-plane
endpoint `https://<api-host>/v1/integrations/stripe/webhook` for these exact
events:

```text
checkout.session.completed
customer.subscription.created
customer.subscription.updated
customer.subscription.deleted
invoice.paid
invoice.payment_failed
```

Put its signing secret in both the control-plane secret manager and the
protected release configuration.

In Stripe **Invoice settings**, add a 72-hour invoice-finalization grace-period
rule for subscription-cycle invoices with a metered Price. Stripe includes
late-reported usage only while the invoice remains draft, so this rule is a
billing correctness requirement rather than an optional buffer. Alert when the
oldest pending or failed Piqae usage export is 15 minutes old and block
promotion unless a test-clock renewal proves that:

1. the ending subscription period is snapshotted once;
2. its overage meter event reaches Stripe within the grace period;
3. replaying either Stripe webhook or worker claim does not duplicate usage;
4. the finalized test invoice contains the expected integer overage blocks.

Never depend on a failed `invoice.created` webhook to delay finalization.

Before promotion, call `/api/internal/pricing-drift` with the protected
`PRICING_DRIFT_SHARED_SECRET`. Any missing, duplicate, inactive, wrongly priced,
wrong-interval, wrong-meter, or wrong-metadata Price fails with HTTP 409.

### One-time Sentry setup

1. Create production Sentry project(s) for server and browser events. The
   runtime DSNs may be the same, but both `SENTRY_DSN` and
   `PUBLIC_SENTRY_DSN` must be configured for complete coverage.
2. Keep `sendDefaultPii` disabled. Piqae additionally removes users, request
   bodies, headers, cookies, query strings, console/UI breadcrumbs, local
   variables, and known credential fields from errors and transactions.
3. Give the build a least-privilege `SENTRY_AUTH_TOKEN`, organisation, project,
   and an immutable `SENTRY_RELEASE` tied to the promoted commit. If only part
   of this set is present, the build fails before Vite emits a candidate.
4. Source maps are generated as hidden artifacts, uploaded under that release,
   and removed from deploy output by the Sentry SvelteKit integration. Never
   put the upload token behind a `PUBLIC_` prefix.
5. Keep tracing sample rates at zero until the error-only synthetic check
   passes. Then set an approved per-environment rate and prove a transaction
   also contains no query values, credentials, or user identity.
6. Record a production event and source-map resolution using synthetic,
   non-customer data. Attach the redaction and release-association evidence to
   the production gate; code presence alone does not close it.

`deploy/terraform/examples/ha-production/terraform.tfvars.example` is not
deployable. A protected production tfvars file must select multi-region Cloud
Run, the global load balancer, Cloud SQL, dual-region GCS, and digest-pinned
images without placeholders. Secret values belong in a protected secret
manager and state backend, not Git or command output.

## External evidence records

The preflight expects one JSON record for each target-specific filename
declared in `release/production-readiness.json`. A record must identify the
gate, say `passed`, bind to a full commit, include a timestamp, and point to
access-controlled evidence. A locally created assertion is not acceptable
evidence. Signing, physical Windows/OKI tests, live WorkOS identity, live
Stripe test-clock billing, production Sentry redaction/source maps, and
independent security review remain open Railway private-beta gates. Regional
DR is an additional managed-HA gate. The 30-day production soak is a public
self-serve release gate rather than a prerequisite for controlled private
beta.

The live WorkOS record must cover workspace creation, invitation, role change,
workspace switching, removal, and session revocation without cross-workspace
data exposure. The live Stripe record must cover monthly and annual Checkout,
Portal, signed webhook replay, quota behavior, the 72-hour finalization rule,
renewal export, invoice overage, and payment-failure grace. The Sentry record
must prove server/browser event delivery, release association, source-map
resolution, and redaction using synthetic data only.
