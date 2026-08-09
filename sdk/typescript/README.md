# @piqae/sdk

Typed, dependency-free TypeScript client for Piqae's native API. It works in
Node.js, browsers, serverless runtimes, and against a local-only agent.

Install the public package from npm:

```console
pnpm add @piqae/sdk
```

Release tarballs and checksums are also attached to `sdk-vX.Y.Z` GitHub
Releases, and the package is mirrored to GitHub Packages for authenticated
GitHub consumers.

## SaaS platforms

Use one server-only platform key and one immutable external ID for each
customer:

```ts
import { readFile } from 'node:fs/promises';
import { PiqaePlatform } from '@piqae/sdk';

const piqae = new PiqaePlatform({
  platformKey: process.env.PIQAE_PLATFORM_KEY!
});

const customer = await piqae.accounts.getOrCreate('org_01JQ8K8M6Q', {
  name: 'Northwind Foods'
});

const printer = (await customer.printers.list()).data[0];
const job = await customer.printPdf({
  printerId: printer.id,
  title: 'Order 481 label',
  pdf: await readFile('./label.pdf'),
  idempotencyKey: 'northwind-order-481-label-v1'
});
```

Customer calls use Live by default. Use `customer.test` for onboarding and
automated checks. The account client exposes the normal `nodes`, `printers`,
`stocks`, `targets`, `uploads`, `jobs`, `apiKeys`, and `webhooks` resources.

Create, retrieve, list, or archive customer accounts through
`piqae.accounts`. Creating, rotating, and revoking the first platform key
remains an operator or hosted account-setup action. Never expose it to browser,
mobile, desktop, or node code.

## Preview embedded node onboarding

A platform backend can create a short-lived connection session through
`customer.nodes.createConnectSession(...)`, render its `connect_url`, and poll
the session until it reports `connected`. The one-time capability must remain
opaque to browser code and logs. The native app shows server-resolved workspace
and requesting-service identity, asks the user to select printers, and proves
possession of the existing installation key before a connector is added.

This onboarding path is Preview. The macOS source implements the native link and
consent flow, but no signed/notarised artifact is currently published from this
repository. Windows and Linux expose download/manual fallback choices only; they
do not yet provide the same native link flow. A download URL in the API is a
navigation choice, not evidence of an available or Supported installer.

## Preview content encryption

`encryptJobContent()` creates a versioned client-side AES-GCM envelope and
wraps its one-time content key to dedicated node P-256 ECDH keys using
HKDF-SHA-256 and authenticated AES-256-GCM key wrapping. It
authenticates the content type, target, profile revision, and expiry.
`printers.contentEncryptionKey()` discovers a tenant-scoped recipient and
`jobs.createEncrypted(input, envelope, idempotencyKey)` uploads ciphertext and
submits the encrypted manifest. The node verifies and decrypts locally; the
ordinary PDF/RAW path remains available for compatibility. This implemented
path remains Preview and is Disabled as a production support claim pending
independent cryptographic review, hardware-backed key work, signed native
releases, crash/soak evidence and physical fixtures.
See `docs/api/content-confidential-printing.md` in the repository.

## One workspace

```ts
import { PiqaeClient } from '@piqae/sdk';

const piqae = new PiqaeClient({ apiKey: process.env.PIQAE_API_KEY });

const job = await piqae.jobs.create(
  {
    printer_id: 'prt_01K...',
    title: 'Order 481 label',
    content_type: 'pdf',
    content: { type: 'uri', uri: 'https://example.com/labels/481.pdf' },
    options: { paper: '4x6', fit_to_page: true }
  },
  'order-481-label'
);

console.log(job.id, job.state);
```

Usage is counted only when a Live job is first accepted by the operating-system
spooler. Read the effective subscription period and its immutable usage ledger
without duplicating billing rules in your application:

```ts
const billing = await piqae.billing.summary();
const july = await piqae.usage.retrieve('2026-07');

console.log(billing.plan, billing.usage.reported_complete_live_jobs);
console.log(july.period_start, july.period_end);
```

Test jobs and idempotent retries do not increment usage. Self-hosted deployments
report managed billing as disabled and make no Stripe calls.

For private PDFs, declare the exact length and SHA-256 digest, then stream the
binary body without Base64:

```ts
const file = new Blob([pdfBytes], { type: 'application/pdf' });
const upload = await piqae.uploads.createAndPut(
  {
    media_type: 'application/pdf',
    byte_length: file.size,
    sha256: pdfSha256
  },
  file
);

await piqae.jobs.create(
  {
    target_id: 'tgt_01K...',
    title: 'Order 481 label',
    content_type: 'pdf',
    content: { type: 'upload', upload_id: upload.id }
  },
  'order-481-label'
);
```

`stocks`, `printers`, and `targets` expose portable geometry, immutable profile
summaries, safe overrides, and current target readiness. Vendor-native settings
are display-only facts captured by the node.

Design editors can fetch those constraints atomically and retain the revision
with the saved artwork:

```ts
const spec = await customer.targets.designSpecification('tgt_01K...');
if (spec.readiness.status !== 'ready') throw new Error('Print setup is not ready');
console.log(spec.stock?.attributes, spec.destinations, spec.specification_revision);
```

Build normalized job-scoped settings without exposing driver-native keys:

```ts
import { PrintIntentBuilder, preliminarilyValidatePrintIntent } from '@piqae/sdk';

const capabilities = await piqae.printers.capabilities('prt_01K...');
const intent = PrintIntentBuilder.create({
  printerId: capabilities.printer_id,
  capabilityRevision: capabilities.revision,
  documentManifest: {
    page_count: 1,
    page_boxes: [{ width_mm: 100, height_mm: 150 }],
    color_spaces: ['DeviceCMYK'],
    separations: ['White'],
    scaling: 'none'
  }
}).semantic('media.sensing', 'black_mark').build();

const preliminary = preliminarilyValidatePrintIntent(intent, capabilities);
const authoritative = await piqae.printIntents.validate(intent);
```

Preliminary validation only supports responsive UI. Server and node validation
remain authoritative. `jobs.createEncryptedResolved()` verifies that resolved
portable options are bound by encrypted-job v3 AAD. It deliberately rejects
unbound intents and ticket digests; use ordinary submission when the server must
enforce workflow-revision provenance.

Reconcile queues with exact server-side filters:

```ts
const failed = await customer.jobs.list({
  state: 'failed_retryable',
  metadata_key: 'order_id',
  metadata_value: '481'
});
```

Verify webhooks before parsing their JSON. Pass the exact raw body; the default
five-minute tolerance rejects stale signatures. Persist each delivery ID and
reject duplicates separately, because a valid delivery can be replayed within
that tolerance window:

```ts
import { verifyWebhookSignature } from '@piqae/sdk';

const valid = await verifyWebhookSignature(secret, rawBody, request.headers);
if (!valid) throw new Error('Invalid Piqae webhook signature');
```

Lower-level trusted integrations can also construct an explicit account grant
context:

```ts
const customerRecord = await applicationDatabase.customers.findById('customer_481');
const customerPiqae = new PiqaeClient({
  platformKey: process.env.PIQAE_PLATFORM_KEY,
  platformContext: {
    workspaceId: customerRecord.piqaeWorkspaceId,
    environmentId: customerRecord.piqaeEnvironmentId
  }
});
```

Ordinary API keys cannot set a platform context. The SDK strips tenant-selection
headers from ordinary custom headers and never sends them to absolute signed
upload URLs.

Set `baseUrl` for self-hosted deployments or
`http://127.0.0.1:39100` for local-only mode.

For complete request structures and server-side validation—not just the
TypeScript surface—read the repository's
[jobs, content, options, and capabilities guide](../../docs/api/jobs-content-and-capabilities.md).
For a multi-tenant product lifecycle from customer provisioning through
offboarding, read the
[web design platform integration guide](../../docs/api/web-design-platform-integration.md).
