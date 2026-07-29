# @spool/sdk

Typed, dependency-free TypeScript client for Spool's native API. It works in
Node.js, browsers, serverless runtimes, and against a local-only agent.

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

Trusted multi-workspace SaaS backends use a distinct platform key and an
explicit grant context:

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
