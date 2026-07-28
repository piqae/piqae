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

Set `baseUrl` for self-hosted deployments or
`http://127.0.0.1:39100` for local-only mode.
