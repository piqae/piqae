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
    group: 'Hosted',
    title: 'Print in under ten minutes',
    description: 'Prepare the SDK integration, connect a customer, and send a durable PDF job from your server.',
    blocks: [
      {
        heading: 'Install after the first SDK release',
        body: 'The public npm package has not yet been released; the command below becomes available with the first SDK release. Repository development uses the workspace package at sdk/typescript. Use Piqae Cloud unless you have a specific reason to operate the control plane yourself. Platform keys are server credentials and must never be used in a browser, mobile app, or distributed desktop client.',
        code: `pnpm add @piqae/sdk

# Server-only secret
PIQAE_PLATFORM_KEY=piq_platform_...`,
        language: 'shell'
      },
      {
        heading: 'Connect one customer',
        code: `import { PiqaePlatform } from '@piqae/sdk';

const piqae = new PiqaePlatform({
  platformKey: process.env.PIQAE_PLATFORM_KEY!
});

const customer = await piqae.accounts.getOrCreate('org_01JQ8K8M6Q', {
  name: 'Northwind Foods',
  metadata: { plan: 'pro' }
});`,
        language: 'typescript'
      },
      {
        heading: 'Print one PDF',
        code: `const printer = (await customer.printers.list()).data[0];

const job = await customer.printPdf({
  printerId: printer.id,
  title: 'Order 10428 shipping label',
  pdf: await readFile('./shipping-label.pdf'),
  idempotencyKey: 'northwind-order-10428-label-v1',
  metadata: { order_id: '10428' }
});`,
        language: 'typescript'
      },
      {
        callout: {
          tone: 'info',
          title: 'The SDK defaults customer calls to Live',
          body: 'Use customer.test during onboarding and automated checks. Pick the environment in trusted server code; never accept a raw Live/Test choice from an untrusted client.'
        }
      },
      {
        callout: {
          tone: 'warning',
          title: 'A 201 response means durable registration',
          body: 'Follow job.updated webhooks until operating-system handoff or a later observation. A printer or driver may be unable to prove that ink reached paper.'
        }
      }
    ]
  },
  {
    slug: 'integration-models',
    group: 'Hosted',
    title: 'Choose an integration',
    description: 'Use a platform account, a workspace API key, or an interactive node flow without mixing their trust boundaries.',
    blocks: [
      {
        heading: 'Headless platform',
        body: 'Best for SaaS products, fulfilment systems, marketplaces, and design tools serving many customer organisations. Your backend keeps one platform key, maps each authenticated organisation to an immutable external ID, and manages isolated customer accounts through the API or SDK.',
        bullets: [
          'Create, retrieve, list, and archive customer accounts.',
          'Use isolated Test and Live environments for every customer.',
          'List nodes, printers, profiles, targets, jobs, API keys, and webhooks.',
          'Send PDFs without implementing upload or digest plumbing.',
          'Keep all platform and tenant selection decisions on the server.'
        ]
      },
      {
        heading: 'Single-workspace backend',
        body: 'Best for one company adding printing to its own application. Create a scoped Live or Test API key in Developers, then use PiqaeClient directly.',
        code: `import { PiqaeClient } from '@piqae/sdk';

const piqae = new PiqaeClient({
  apiKey: process.env.PIQAE_API_KEY!
});

const printers = await piqae.printers.list();
const jobs = await piqae.jobs.list({ limit: 25 });`,
        language: 'typescript'
      },
      {
        heading: 'Interactive desktop or operator flow',
        body: 'Best when a person installs a node, chooses native driver settings, or monitors a local queue. Pair the thin tray app in a browser. The node keeps a device credential in Keychain or DPAPI and does not retain the person’s web session.'
      },
      {
        heading: 'Local-only',
        body: 'Best for a single machine that must print without any control plane. Use the loopback API and local SQLite queue. Platform accounts, workspace members, and hosted webhooks are intentionally absent.'
      }
    ]
  },
  {
    slug: 'platform-accounts',
    group: 'Platform APIs',
    title: 'Customer accounts',
    description: 'Give every customer an isolated workspace while keeping integration code compact.',
    blocks: [
      {
        heading: 'Use your immutable external ID',
        body: 'Use a non-personal identifier from your own database. Never use a mutable slug, email address, display name, printer ID, or value accepted directly from browser input.',
        code: `const customer = await piqae.accounts.getOrCreate(customer.id, {
  name: customer.displayName,
  metadata: {
    billing_tier: customer.plan,
    home_region: customer.region
  }
});`,
        language: 'typescript'
      },
      {
        heading: 'Test and Live are isolated',
        code: `// Live is the default.
await customer.printers.list();

// Test has separate nodes, printers, jobs and webhooks.
await customer.test.printers.list();`,
        language: 'typescript'
      },
      {
        heading: 'Manage the lifecycle',
        code: `const accounts = await piqae.accounts.list();
const sameCustomer = await piqae.accounts.retrieve(customer.externalId);
await piqae.accounts.archive(customer.externalId);`,
        language: 'typescript'
      },
      {
        body: 'Archive blocks new platform access and revokes both environment grants. Already accepted jobs retain their durable lifecycle. Archive is not immediate data deletion.'
      },
      {
        heading: 'Dashboard and API',
        body: 'Platform operators can view customer accounts and environment health in the hosted dashboard. Application code should use the SDK or API. The browser dashboard uses the signed-in human session; it never receives the platform key.'
      }
    ]
  },
  {
    slug: 'common-patterns',
    group: 'Platform APIs',
    title: 'Common printing patterns',
    description: 'Build labels, receipts, documents, and design experiences around printer profiles rather than vendor settings.',
    blocks: [
      {
        heading: 'Shipping and fulfilment labels',
        body: 'Create one native profile for each stock or finishing setup. Store the profile-backed target ID with your warehouse configuration, then send the order PDF with a stable order/label idempotency key.',
        code: `await customer.printPdf({
  targetId: warehouse.targets.shippingLabel4x6,
  title: \`Order \${order.number} label\`,
  pdf,
  idempotencyKey: \`order-\${order.id}-label-v2\`
});`,
        language: 'typescript'
      },
      {
        heading: 'Point of sale',
        body: 'Use a target that can route to an available node and printer. Keep the receipt request short-lived, show current node/printer health before checkout, and provide an explicit reprint action when delivery becomes uncertain.'
      },
      {
        heading: 'Batch documents',
        body: 'Create one job per independently retryable document. Bound concurrency in your application and use webhooks to drive progress rather than polling every job.'
      },
      {
        heading: 'Design and template applications',
        body: 'List profiles and stock metadata before rendering. Use the profile’s page dimensions, printable area, orientation, stock identifier, and native validation state to size the canvas. The installed driver remains authoritative for trays, colour, cutters, black-mark sensors, and vendor PostScript options.'
      },
      {
        heading: 'Multiple nodes',
        body: 'A physical printer exposed by two computers is two node-specific printer resources. Send to a printer for exact placement, or to a target for an explicit routing policy. Piqae does not silently reroute a pinned printer job.'
      }
    ]
  },
  {
    slug: 'printers-and-profiles',
    group: 'Platform APIs',
    title: 'Printers, profiles, and stock',
    description: 'Discover what is available and expose safe print choices to your application.',
    blocks: [
      {
        heading: 'List available resources',
        code: `const [nodes, printers, stocks, targets] = await Promise.all([
  customer.agents.list(),
  customer.printers.list(),
  customer.stocks.list(),
  customer.targets.list()
]);`,
        language: 'typescript'
      },
      {
        heading: 'Profiles are native snapshots',
        body: 'A profile is an immutable capture of operating-system driver settings on one node. The same OS printer can have several profiles—for example A4 colour, A4 duplex, tray 2 letterhead, or an OKI label stock with vendor-specific calibration.'
      },
      {
        heading: 'What the web app should show',
        bullets: [
          'Friendly profile and stock name.',
          'Page dimensions, orientation, colour, duplex, copies policy, and printable area.',
          'Node and printer online state.',
          'Native validation and last successful test.',
          'A stable target ID when routing rather than a specific OS queue is desired.'
        ]
      },
      {
        callout: {
          tone: 'info',
          title: 'Do not recreate complex drivers in your web UI',
          body: 'Open the operating system’s native printer panel to edit advanced settings. Piqae stores and replays the opaque native snapshot and exposes only portable metadata needed by an application.'
        }
      }
    ]
  },
  {
    slug: 'jobs',
    group: 'Reliability',
    title: 'Jobs and delivery truth',
    description: 'Understand offline nodes, durable queues, large documents, retries, and uncertain delivery.',
    blocks: [
      {
        heading: 'When no node is online',
        body: 'The control plane durably retains the job. An eligible node claims it after reconnecting, writes the claim and document reference into its local SQLite queue, then acknowledges ownership. Expiry and cancellation still apply while offline.'
      },
      {
        heading: 'Large documents',
        body: 'Upload binary content directly to object storage using the SDK. The SDK computes SHA-256, sends no Piqae credential to the signed upload URL, and verifies the upload before job creation. The hosted V1 limit is 50 MiB per document; stream from disk in production when the runtime supports it.'
      },
      {
        heading: 'Idempotency',
        body: 'Use one stable key for one logical print. Retrying an identical request returns the original job. Reusing the key with different content returns a conflict.',
        code: `await customer.printPdf({
  printerId,
  title: 'Packing slip',
  pdf,
  idempotencyKey: \`order-\${order.id}-packing-slip-v1\`
});`,
        language: 'typescript'
      },
      {
        heading: 'Lifecycle',
        bullets: [
          'registered: the service durably owns the job.',
          'agent_accepted: one node durably owns a local copy.',
          'accepted_by_spooler: the operating-system queue accepted the submission.',
          'completed_reported: the OS reports completion; physical output may remain unknowable.',
          'delivery_uncertain: a crash crossed the handoff boundary and automatic retry could duplicate output.'
        ]
      },
      {
        callout: {
          tone: 'warning',
          title: 'Never hide uncertain delivery',
          body: 'Do not automatically retry delivery_uncertain. Let an operator mark the original delivered or create a linked reprint that explicitly accepts duplicate risk.'
        }
      }
    ]
  },
  {
    slug: 'webhooks',
    group: 'Reliability',
    title: 'Webhooks and live status',
    description: 'Receive signed, at-least-once lifecycle events without running a polling loop.',
    blocks: [
      {
        heading: 'Create a webhook',
        code: `const webhook = await customer.webhooks.create({
  url: 'https://example.com/webhooks/piqae',
  events: ['job.updated']
});

// Save webhook.secret now. It is returned once.`,
        language: 'typescript'
      },
      {
        heading: 'Delivery rules',
        bullets: [
          'Verify the timestamp and HMAC signature over the exact raw request body.',
          'Deduplicate at-least-once delivery by event ID.',
          'Return 2xx only after your application durably records the event.',
          'Treat status changes as monotonic evidence, not proof of physical output.',
          'Use API reads to reconcile after downtime or an event gap.'
        ]
      }
    ]
  },
  {
    slug: 'api-and-sdk',
    group: 'Reference',
    title: 'API and SDK',
    description: 'Use one small TypeScript client or the same versioned HTTP contract from any backend.',
    blocks: [
      {
        heading: 'TypeScript',
        code: `import {
  PiqaeClient,   // one workspace/environment
  PiqaePlatform, // many customer accounts
  PiqaeError
} from '@piqae/sdk';`,
        language: 'typescript'
      },
      {
        heading: 'HTTP',
        body: 'The native API is JSON over HTTPS under /v1. Platform account management uses only the platform bearer. Tenant operations add the exact workspace and environment selected by server-owned application state. Use Idempotency-Key on mutating requests that may be retried.'
      },
      {
        heading: 'Errors',
        body: 'PiqaeError exposes a stable code, HTTP status, request ID, retryable flag, and structured details. Branch on the code, never on the human-readable message. Retry only operations marked retryable and preserve the same idempotency key.'
      },
      {
        heading: 'Initial setup remains deliberate',
        body: 'Create, rotate, or revoke the first platform credential with the operator CLI or hosted account setup. Routine customer accounts, nodes, printers, jobs, API keys, webhooks, and usage are API/SDK resources.'
      }
    ]
  },
  {
    slug: 'legacy-compatibility',
    group: 'Reference',
    title: 'Migrate from the legacy provider',
    description: 'Change the API origin for the tested compatibility subset, then adopt native Piqae resources incrementally.',
    blocks: [
      {
        heading: 'Compatibility endpoint',
        code: `const legacyClient = new LegacyPrintClient({
  apiKey: process.env.PIQAE_API_KEY,
  baseUrl: 'https://compat.piqae.example'
});`,
        language: 'typescript'
      },
      {
        heading: 'Scope',
        bullets: [
          'Computers, printers, print jobs, states, and printing webhooks are the V1 compatibility focus.',
          'PDF and RAW URI/base64 modes are supported only where the checked-in compatibility tests say so.',
          'Platform customer accounts use the native Piqae SDK/API.',
          'Native states expose more delivery evidence than the compatibility projection.'
        ]
      }
    ]
  },
  {
    slug: 'open-source',
    group: 'Project',
    title: 'Open source and self-hosting',
    description: 'Run the complete printing control plane yourself without WorkOS, Stripe, a licence server, or phone-home.',
    blocks: [
      {
        heading: 'Choose a deployment',
        bullets: [
          'Docker Compose for development and normal small installations.',
          'Helm with external PostgreSQL and S3-compatible storage for highly available deployments.',
          'Local-only node and loopback API when no server is needed.',
          'The same signed node can connect to Piqae Cloud or your HTTPS control plane.'
        ]
      },
      {
        heading: 'Start with Compose',
        code: `cd deploy/self-host
cp .env.example .env
# Replace every placeholder.
docker compose --env-file .env up -d`,
        language: 'shell'
      },
      {
        heading: 'What remains complete',
        body: 'Self-hosted Piqae includes printing, native profiles, durable queues, platform accounts, API keys, webhooks, diagnostics, update policy, local-owner access, and generic OIDC. Hosted convenience—not withheld printing capability—is the commercial product.'
      },
      {
        callout: {
          tone: 'warning',
          title: 'Compose is not highly available',
          body: 'Production operators must back up PostgreSQL and object storage together, pin image digests, run migrations explicitly, monitor queue age, and prove restore and upgrade procedures.'
        }
      }
    ]
  },
  {
    slug: 'contributing',
    group: 'Project',
    title: 'Contributing',
    description: 'Create a virtual first print from a fresh checkout without touching physical hardware.',
    blocks: [
      {
        heading: 'Set up',
        code: `git clone https://github.com/piqae/piqae.git
cd piqae
cargo xtask doctor
cargo xtask dev`,
        language: 'shell'
      },
      {
        heading: 'Work safely',
        bullets: [
          'Read AGENTS.md and the nearest scoped instructions before editing.',
          'Use the virtual node and fake printers unless a named physical test is explicitly authorised.',
          'Update OpenAPI before public route or schema code.',
          'Use append-only PostgreSQL migrations with cross-tenant tests.',
          'Run cargo xtask test changed before a focused DCO-signed commit.'
        ]
      },
      {
        heading: 'Submit',
        body: 'Open an issue or RFC for protocol, profile, compatibility, or deployment changes. Keep pull requests small, include failure-path evidence, preserve the distinction between spooler acceptance and physical delivery, and never use customer documents as fixtures.'
      }
    ]
  }
];

export const docBySlug = new Map(docs.map((doc) => [doc.slug, doc]));
