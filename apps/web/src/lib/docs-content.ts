export interface DocBlock {
  heading?: string;
  body?: string;
  code?: string;
  language?: string;
  bullets?: string[];
  callout?: { tone: 'info' | 'warning'; title: string; body: string };
}

export interface Doc {
  slug: string;
  group: string;
  title: string;
  description: string;
  blocks: DocBlock[];
}

export const docs: Doc[] = [
  {
    slug: 'quickstart',
    group: 'Get started',
    title: 'Print in under ten minutes',
    description: 'Create an API key, enrol an agent, choose a printer, and submit a durable PDF job.',
    blocks: [
      {
        heading: '1. Start Spool',
        body: 'Use Spool Cloud, run the self-hosted Docker Compose stack, or start an agent in local-only mode. Cloud and self-hosted agents need outbound HTTPS access only.'
      },
      {
        code: `# Local-only mode
spool-agent --mode local

# Or enrol with a hosted/self-hosted control plane
spool-agent enrol \\
  --server https://api.spool.example \\
  --token spl_enr_...`,
        language: 'shell'
      },
      {
        heading: '2. Find a printer',
        body: 'After enrolment, installed operating-system queues appear automatically. Use the native printer ID rather than the driver display name in application code.',
        code: `curl https://api.spool.dev/v1/printers \\
  -H "Authorization: Bearer $SPOOL_API_KEY"`,
        language: 'shell'
      },
      {
        heading: '3. Submit a PDF',
        code: `curl https://api.spool.dev/v1/jobs \\
  -H "Authorization: Bearer $SPOOL_API_KEY" \\
  -H "Idempotency-Key: order-481-label" \\
  -H "Content-Type: application/json" \\
  -d '{
    "printer_id": "prt_01K...",
    "title": "Order 481 label",
    "content_type": "pdf",
    "content": {
      "type": "uri",
      "uri": "https://example.com/labels/481.pdf"
    },
    "options": {
      "paper": "4x6",
      "fit_to_page": true,
      "copies": 1
    }
  }'`,
        language: 'shell'
      },
      {
        callout: {
          tone: 'info',
          title: 'A 201 response means durable registration',
          body: 'It does not mean the document printed. Follow the job resource, SSE stream, or a signed webhook until the state reaches accepted_by_spooler or a later observation.'
        }
      }
    ]
  },
  {
    slug: 'jobs',
    group: 'Core concepts',
    title: 'Jobs and delivery truth',
    description: 'Understand durable registration, agent acceptance, operating-system handoff, and uncertain delivery.',
    blocks: [
      {
        heading: 'Two durable queues',
        body: 'The control plane retains responsibility while an agent is offline. The agent acknowledges a claim only after writing it to its local SQLite queue. This boundary prevents a successful API response from becoming a lost print.'
      },
      {
        heading: 'Idempotency',
        body: 'Send a unique Idempotency-Key for every logical job. Retrying an identical request returns its original result; reusing the key with a different body returns a conflict.',
        code: `const job = await spool.jobs.create(
  {
    printer_id: 'prt_01K...',
    title: 'Packing slip',
    content_type: 'pdf',
    content: { type: 'uri', uri: documentUrl }
  },
  \`order-\${order.id}-packing-slip\`
);`,
        language: 'typescript'
      },
      {
        heading: 'The handoff boundary',
        bullets: [
          'registered: the service durably owns the job.',
          'agent_accepted: a specific machine durably owns a local copy.',
          'accepted_by_spooler: the OS queue accepted the submission.',
          'completed_reported: the OS reports completion; physical output may still be unknowable.',
          'delivery_uncertain: a crash occurred at the handoff boundary and automatic retry could duplicate output.'
        ]
      },
      {
        callout: {
          tone: 'warning',
          title: 'Never automatically retry delivery_uncertain',
          body: 'An operator must decide whether to mark the original delivered or create a linked reprint that explicitly accepts duplicate risk.'
        }
      }
    ]
  },
  {
    slug: 'api-keys',
    group: 'Authentication',
    title: 'API keys',
    description: 'Use environment-bound, least-privilege credentials without exposing browser sessions or device keys.',
    blocks: [
      {
        heading: 'Authenticate',
        body: 'Send the secret as a Bearer token. The prefix identifies whether a credential belongs to a test or live environment.',
        code: `Authorization: Bearer spl_live_...`,
        language: 'http'
      },
      {
        heading: 'Scopes',
        bullets: [
          'jobs:read and jobs:write',
          'printers:read and printers:write',
          'agents:read and agents:write',
          'webhooks:read and webhooks:write',
          'usage:read and audit:read'
        ]
      },
      {
        callout: {
          tone: 'warning',
          title: 'Secrets are shown once',
          body: 'Spool stores only a one-way hash. Keep keys in a secret manager, never in browser code, source control, logs, or document metadata.'
        }
      }
    ]
  },
  {
    slug: 'webhooks',
    group: 'Events',
    title: 'Signed webhooks',
    description: 'Receive durable, at-least-once event delivery with replayable attempts.',
    blocks: [
      {
        heading: 'Verify before parsing',
        body: 'Compute HMAC-SHA256 over the timestamp and exact raw request bytes. Reject stale timestamps and compare signatures in constant time.',
        code: `const payload = \`\${timestamp}.\${rawBody}\`;
const expected = createHmac('sha256', secret)
  .update(payload)
  .digest('hex');

if (!timingSafeEqual(Buffer.from(expected), Buffer.from(signature))) {
  throw new Error('Invalid Spool signature');
}`,
        language: 'typescript'
      },
      {
        heading: 'Delivery model',
        bullets: [
          'Events are delivered at least once; deduplicate by event ID.',
          'Any 2xx response acknowledges an attempt.',
          'Failures retry with exponential backoff for up to 24 hours.',
          'Dead-letter attempts remain visible and can be replayed without changing the event ID.'
        ]
      }
    ]
  },
  {
    slug: 'sdk',
    group: 'Libraries',
    title: 'TypeScript SDK',
    description: 'A dependency-free client for Node.js, browsers, serverless functions, and local-only agents.',
    blocks: [
      {
        heading: 'Install',
        code: `pnpm add @spool/sdk`,
        language: 'shell'
      },
      {
        heading: 'Create a client',
        code: `import { SpoolClient } from '@spool/sdk';

const spool = new SpoolClient({
  apiKey: process.env.SPOOL_API_KEY,
  // Override for self-hosted or local-only Spool.
  baseUrl: process.env.SPOOL_API_URL
});`,
        language: 'typescript'
      },
      {
        heading: 'Structured failures',
        body: 'SpoolError exposes a stable code, HTTP status, request ID, retryable flag, and structured details. Do not branch on the human-readable message.'
      }
    ]
  },
  {
    slug: 'self-host',
    group: 'Deployment',
    title: 'Self-host Spool',
    description: 'Run the same control plane using PostgreSQL, S3-compatible storage, and generic OIDC.',
    blocks: [
      {
        heading: 'Required services',
        bullets: [
          'spool-server OCI image',
          'PostgreSQL 16 or later',
          'S3, R2, MinIO, or filesystem object storage',
          'Generic OIDC, WorkOS, or one-time local-owner bootstrap',
          'SvelteKit Node artifact or separately deployed Vercel frontend'
        ]
      },
      {
        heading: 'Build the self-hosted web artifact',
        code: `pnpm --filter @spool/web build:self-host
node apps/web/build-node`,
        language: 'shell'
      },
      {
        heading: 'Operational baseline',
        body: 'Run migrations as an explicit job, configure health probes, back up PostgreSQL and signing keys, apply object lifecycle rules, and exercise restoration before production traffic.'
      }
    ]
  },
  {
    slug: 'printnode-migration',
    group: 'Migration',
    title: 'Move from PrintNode',
    description: 'Keep your existing printing integration while adopting richer Spool observability incrementally.',
    blocks: [
      {
        heading: 'Change the origin and credential',
        body: 'The compatibility hostname implements documented PrintNode printing routes at the root. Keep HTTP Basic authentication and replace only the API origin and key.',
        code: `const printNode = new PrintNode({
  apiKey: process.env.SPOOL_API_KEY,
  baseUrl: 'https://compat.spool.example'
});`,
        language: 'typescript'
      },
      {
        heading: 'Compatibility scope',
        bullets: [
          'Computers, printers, print jobs, states, and printing webhooks are V1.',
          'PDF and RAW URI/base64 modes are supported.',
          'Scales and integrator child accounts arrive in V1.1.',
          'Native Spool states provide more truth than the five-state compatibility projection.'
        ]
      }
    ]
  }
];

export const docBySlug = new Map(docs.map((doc) => [doc.slug, doc]));
