# Headless SaaS quickstart

**Status:** implemented developer preview. Platform account routes and the SDK
facade remain Disabled as a production support claim until release evidence
passes.

1. Create one server-only platform credential with `spoolctl`.
2. Store it as `SPOOL_PLATFORM_KEY` in your secret manager.
3. Resolve your authenticated customer to one immutable external ID.
4. Get or create its Spool account.
5. Select Test or Live from trusted server-side policy.
6. Use the returned workspace/environment IDs for printer, upload, job, and
   webhook calls.

Hosted-first SDK:

```ts
import { readFile } from 'node:fs/promises';
import { SpoolPlatform } from '@spool/sdk';

const platform = new SpoolPlatform({
  platformKey: process.env.SPOOL_PLATFORM_KEY!
});

const account = await platform.accounts.getOrCreate(customer.id, {
  name: customer.name
});

const printers = await account.printers.list();
const job = await account.printPdf({
  printerId: printers.data[0].id,
  title: `Order ${order.number} label`,
  pdf: await readFile('./label.pdf'),
  idempotencyKey: `order-${order.id}-label-v1`
});
```

Use `account.test` for onboarding and virtual-printer checks. Account calls use
Live by default.

Lower-level account-scoped client:

```ts
const spool = new SpoolClient({
  platformKey: process.env.SPOOL_PLATFORM_KEY!,
  platformContext: {
    workspaceId: account.id,
    environmentId: account.environments.live.id
  }
});
```

Send binary PDFs through `uploads.createAndPut`, then call `jobs.create` with a
stable idempotency key and `content: { type: "upload", upload_id: upload.id }`.
Create and verify a signed webhook for lifecycle status.

Never put the platform key or tenant-selection inputs in browser code. Archive
stops new platform access but lets durable jobs finish; it is not immediate
data deletion.

See [Platform accounts](platform-service-accounts.md) for the complete example,
retry rules, Test/Live boundary, archive behavior, self-hosting, and CLI-only
operations.
