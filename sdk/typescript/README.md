# @spool/sdk

Typed, dependency-free TypeScript client for Spool's native API. It works in
Node.js, browsers, serverless runtimes, and against a local-only agent.

## SaaS platforms

Use one server-only platform key and one immutable external ID for each
customer:

```ts
import { readFile } from 'node:fs/promises';
import { SpoolPlatform } from '@spool/sdk';

const spool = new SpoolPlatform({
  platformKey: process.env.SPOOL_PLATFORM_KEY!
});

const customer = await spool.accounts.getOrCreate('org_01JQ8K8M6Q', {
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
`spool.accounts`. Creating, rotating, and revoking the first platform key
remains an operator or hosted account-setup action. Never expose it to browser,
mobile, desktop, or node code.

## One workspace

```ts
import { SpoolClient } from '@spool/sdk';

const spool = new SpoolClient({ apiKey: process.env.SPOOL_API_KEY });

const job = await spool.jobs.create(
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
const billing = await spool.billing.summary();
const july = await spool.usage.retrieve('2026-07');

console.log(billing.plan, billing.usage.accepted_live_jobs);
console.log(july.period_start, july.period_end);
```

Test jobs and idempotent retries do not increment usage. Self-hosted deployments
report managed billing as disabled and make no Stripe calls.

For private PDFs, declare the exact length and SHA-256 digest, then stream the
binary body without Base64:

```ts
const file = new Blob([pdfBytes], { type: 'application/pdf' });
const upload = await spool.uploads.createAndPut(
  {
    media_type: 'application/pdf',
    byte_length: file.size,
    sha256: pdfSha256
  },
  file
);

await spool.jobs.create(
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

Lower-level trusted integrations can also construct an explicit account grant
context:

```ts
const customerSpool = new SpoolClient({
  platformKey: process.env.SPOOL_PLATFORM_KEY,
  platformContext: {
    workspaceId: customer.spoolWorkspaceId,
    environmentId: customer.spoolEnvironmentId
  }
});
```

Ordinary API keys cannot set a platform context. The SDK strips tenant-selection
headers from ordinary custom headers and never sends them to absolute signed
upload URLs.

Set `baseUrl` for self-hosted deployments or
`http://127.0.0.1:39100` for local-only mode.
