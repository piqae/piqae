# Platform accounts

**Status:** implemented preview; Disabled in the
[support matrix](../../release/support-matrix.yaml) until the account server
routes and SDK facade have complete tenant, revocation, audit, and redaction
release evidence.

Platform accounts give a trusted SaaS backend one server-side Spool credential
while keeping every customer in a separate workspace. The intended integration
should feel like this:

```ts
import { readFile } from 'node:fs/promises';
import { SpoolPlatform } from '@spool/sdk';

const platformKey = process.env.SPOOL_PLATFORM_KEY;
if (!platformKey) throw new Error('SPOOL_PLATFORM_KEY is required');
const platform = new SpoolPlatform({ platformKey });

// Stable in your system: do not use a display name or browser-supplied ID.
const account = await platform.accounts.getOrCreate('org_01JQ8K8M6Q', {
  name: 'Northwind Foods',
  metadata: { plan: 'pro', region: 'nz' }
});

const printers = await account.printers.list({ limit: 25 });
const printer = printers.data.find((item) => item.state === 'online');
if (!printer) throw new Error('No online printer for Northwind Foods');

const pdf = await readFile('./shipping-label.pdf');
const job = await account.printPdf({
  printerId: printer.id,
  title: 'Order 10428 shipping label',
  pdf,
  metadata: { order_id: '10428' },
  idempotencyKey: 'northwind-order-10428-label-v1'
});

const webhook = await account.webhooks.create({
  url: 'https://example.com/webhooks/spool',
  events: ['job.updated']
});
// Store webhook.secret now; Spool returns it only once.
```

`SpoolPlatform` and the account facade are implemented in the repository SDK.
For lower-level or existing integrations, construct an account-scoped tenant
client directly:

```ts
const spool = new SpoolClient({
  platformKey,
  platformContext: {
    workspaceId: account.id,
    environmentId: account.environments.live.id
  }
});
```

## The two request modes

Headless account management uses only:

```text
Authorization: Bearer spl_platform_...
```

It must not send tenant-selection headers. Account-scoped printing uses the
same verified bearer plus the workspace and environment IDs returned by the
account response:

```text
X-Spool-Workspace-Id: wsp_...
X-Spool-Environment-Id: env_...
```

The SDK owns those headers. Do not let callers add or override them. Resolve an
account from your authenticated server-side organisation mapping, then create
the account-scoped client. A printer, job, target, profile, or browser parameter
is never a trusted source of tenant identity.

An ordinary `spl_test_...` or `spl_live_...` API key cannot select a workspace.
Adding platform-selection headers to an ordinary key fails authentication.
The hosted customer dashboard may call the same management routes from its
trusted SvelteKit server using an authorised human owner session. That session
is never converted into or shown as a platform key, and ordinary tenant API
keys remain invalid for account management.

## Accounts and external IDs

Use an immutable identifier from your own database:

```text
org_01JQ8K8M6Q
```

External IDs are 1–120 characters and use letters, numbers, `_`, `.`, `:`, or
`-`, beginning with a letter or number. They are scoped to the platform service
account. Never use an email address, mutable slug, company name, or ID accepted
directly from a browser.

`PUT /v1/platform/accounts/{external_id}` is the get-or-create operation. The
first successful call creates:

- one isolated Spool workspace;
- one Test environment;
- one Live environment; and
- exact grants for the calling platform identity.

The HTTP equivalent is deliberately small:

```console
curl --request PUT \
  "https://api.spool.dev/v1/platform/accounts/org_01JQ8K8M6Q" \
  --header "Authorization: Bearer $SPOOL_PLATFORM_KEY" \
  --header "Content-Type: application/json" \
  --data '{"name":"Northwind Foods","metadata":{"plan":"pro"}}'
```

Repeating the same external ID updates safe account name/metadata without
creating another tenant. Metadata is limited to 20 string values of at most 500
characters each. Keep secrets and personal print data out of it.

`GET /v1/platform/accounts/{external_id}` retrieves one account and
`GET /v1/platform/accounts` lists accounts owned by that platform identity.

## Test and Live

Every account has both environments:

```ts
account.environments.test.id;
account.environments.live.id;
```

They are hard tenant boundaries, not labels. Nodes, printers, profiles, API
keys, uploads, jobs, webhooks, events, quotas, and idempotency records belong to
one environment.

Use Test for onboarding, virtual printers, template validation, and integration
checks. Use Live only for real production jobs. A Test job cannot target a Live
printer, even when both environments belong to the same account.

Choose the environment in server-owned configuration. Never accept `"live"`
from an untrusted request without checking that the authenticated user and
business operation are permitted to print.

## Documents and jobs

Prefer uploads over Base64 for PDFs:

1. read or stream the PDF server-side;
2. calculate exact byte length and SHA-256;
3. create the upload;
4. PUT the binary body to the returned location;
5. create the job with `content.type = "upload"`.

The current upload contract accepts up to 50 MiB and verifies both length and
digest. Never forward the Spool bearer to an absolute, time-limited object-store
URL; send only the returned `upload_headers`. The SDK handles relative Spool
proxy URLs with authentication.

Choose either a concrete `printer_id` or a logical `target_id`, never both.
Targets are preferable when a stock/profile can fail over safely before node
acceptance.

A created job is durably registered; it does not prove paper was produced.
Observe events through signed webhooks or job polling. Treat
`delivery_uncertain` as an operator-reconciliation state, not success.

## Retries and idempotency

Use bounded retries with jitter only for transport errors, `429`, and responses
explicitly marked retryable.

- Account get-or-create is idempotent by external ID.
- Job creation requires a stable `Idempotency-Key`. Reuse the same key and
  identical request after an ambiguous response.
- Upload creation is not a substitute for job idempotency. Retain the returned
  upload ID and reconcile it with `GET /v1/uploads/{upload_id}`.
- Webhook creation is not documented as idempotent. After an ambiguous
  response, list endpoints before creating another.

Never append random text to escape `409 idempotency_conflict`. Decide whether
the operation is a retry, replacement, or intentionally additional print.

## Webhooks

Create webhooks on the account-scoped Test or Live client. A Live webhook does
not receive Test events. The `whsec_...` signing secret is returned once; store
it in your secret manager keyed by external account and environment.

Verify the signature against the exact raw request body before parsing JSON or
performing side effects. Deliveries are at least once, so deduplicate by event
ID. See [webhooks](webhooks.md).

## Archive semantics

`DELETE /v1/platform/accounts/{external_id}` archives the account and revokes
its Test and Live platform grants. It is idempotent.

Archive means:

- new platform requests for either environment stop;
- new jobs cannot be registered through that platform identity;
- already durable jobs remain available to workers to finish their lifecycle;
- job/event/audit history is retained according to configured retention; and
- the workspace is not synchronously erased.

Do not treat archive as a GDPR deletion primitive or assume that calling
get-or-create unarchives the account. V1 has no public unarchive contract.

## Secrets and deployment

The platform key is a high-impact server credential:

- keep it in a server-side secret manager;
- never expose it to browser JavaScript, mobile/desktop clients, native nodes,
  URLs, analytics, logs, traces, support bundles, or print metadata;
- use separate identities for production, previews, migrations, and support;
- rotate immediately after suspected exposure; and
- apply least-privilege grants.

Spool Cloud and self-hosted Spool use the same account and tenant request
contract. Self-hosting does not require WorkOS or a Spool Cloud account. Point
the client at the self-hosted HTTPS `baseUrl`, provision the platform identity
with the local operator CLI, and retain the same external IDs.

## What remains operator-only

These remain database-backed `spoolctl` operations rather than public platform
APIs:

- create the platform service-account credential;
- rotate or revoke the whole credential;
- delete an already revoked credential with explicit confirmation;
- grant or revoke access to a pre-existing workspace/environment; and
- customize grant scopes or expiry outside the account get-or-create policy.

For a self-hosted deployment, create the first identity against an existing
owner workspace and one of its environments:

```console
export SPOOL_DATABASE_URL='postgres://...'

cargo run -p spoolctl -- platform create \
  --name fulfilment-production \
  --workspace wsp_... \
  --environment env_... \
  --scopes api_keys_write
```

The command prints `service_account_id` and `credential` once. Store the
credential directly in a secret manager; do not copy it into shell history,
CI output, screenshots, analytics, or support tickets. Customer-account
get-or-create grants the standard account scopes to its newly created Test and
Live environments.

Rotate or immediately revoke the identity by its non-secret ID:

```console
cargo run -p spoolctl -- platform rotate \
  --service-account 019...

cargo run -p spoolctl -- platform revoke \
  --service-account 019...
```

The credential is printed once at creation or rotation. Spool stores only its
Argon2 verifier.

The complete authorization feature remains Disabled until the checked-in
[release evidence policy](../operations/platform-service-account-release-evidence.md)
passes. Contract presence is not production evidence.
