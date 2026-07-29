# Spool Cloud

**Status:** paid private-beta implementation; not yet a Supported public SaaS
release.

Spool Cloud is the normal developer path. It supplies hosted identity, durable
job registration, object storage, live status, signed node downloads, updates,
monitoring, backups, and billing. Your application uses the same API and SDK as
a self-hosted deployment.

## Choose your backend model

- **Platform account:** your SaaS serves multiple customer organisations. Keep
  one `spl_platform_...` key on your server and map each customer to an isolated
  Spool account with Test and Live environments.
- **Workspace API key:** one organisation adds printing to its own backend. Use
  an environment-bound `spl_test_...` or `spl_live_...` key.
- **Interactive operator:** a person uses the dashboard and native tray app to
  pair nodes, capture profiles, monitor queues, and diagnose failures.

See the [headless SaaS quickstart](../api/platform-headless-quickstart.md) or
[single-workspace API quickstart](../api/quickstart.md). The
[billing and usage guide](../api/billing-and-usage.md) defines the exact
billable event, quotas, and hosted identity/billing boundary.

## Connect the first node

1. Create or choose a workspace in the hosted dashboard.
2. Select Test while validating an integration.
3. Open **Nodes → Add node** and download the signed macOS or Windows build.
4. Install the thin tray application and choose **Pair in browser**.
5. Approve the displayed hostname, platform, node name, workspace, and
   environment.
6. Wait for operating-system printers to appear.
7. Open the native profile editor, select the installed driver settings, name
   the profile and stock, then save without printing.
8. Send the virtual or named test page before using customer documents.

The node stores a device key in macOS Keychain or Windows DPAPI. It does not
retain the person’s WorkOS session, and signing out of the browser does not
disconnect it.

## Add the SDK

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
  title: 'Order 10428 label',
  pdf: await readFile('./label.pdf'),
  idempotencyKey: 'northwind-order-10428-label-v1'
});
```

Use `customer.test` for onboarding and automated checks; account calls use Live
by default. Select the environment in trusted server policy, never from a raw
browser parameter.

## Follow delivery

Create a signed `job.updated` webhook and record each event before returning
2xx. A successful job-creation response means Spool durably owns the job. If no
eligible node is online, the control plane retains it until a node reconnects,
the job expires, or it is cancelled.

The operating-system spooler may report acceptance or completion without
proving physical output. Preserve `delivery_uncertain` and offer an explicit
linked reprint instead of silently retrying across that boundary.

## Hosted infrastructure status

The repository includes the Cloud Run, PostgreSQL, object-storage, WorkOS, and
observability foundations used by the intended service. Infrastructure code is
not itself production evidence. Availability, database failover, signed native
packages, physical printer certification, update rollback, and soak gates
remain governed by the checked-in support matrix and release evidence policy.
