import type {
  DashboardAccount,
  DashboardAgent,
  DashboardApiKey,
  DashboardJob,
  DashboardJobEvent,
  DashboardPrinter,
  DashboardWebhook
} from './view-types';

const now = new Date('2026-07-29T04:24:00.000Z');
const ago = (minutes: number) => new Date(now.getTime() - minutes * 60_000).toISOString();

export const agents: DashboardAgent[] = [
  {
    id: 'agt_01K0A',
    name: 'Packing desk',
    state: 'online',
    os: 'windows',
    architecture: 'x86_64',
    version: '0.1.0',
    protocolVersion: '1',
    lastSeenAt: ago(0),
    queueDepth: 4,
    printerCount: 3,
    labels: ['warehouse', 'labels']
  },
  {
    id: 'agt_01K0B',
    name: 'Roastery office',
    state: 'online',
    os: 'macos',
    architecture: 'aarch64',
    version: '0.1.0',
    protocolVersion: '1',
    lastSeenAt: ago(1),
    queueDepth: 1,
    printerCount: 4,
    labels: ['office']
  },
  {
    id: 'agt_01K0C',
    name: 'Dispatch Pi',
    state: 'degraded',
    os: 'linux',
    architecture: 'aarch64',
    version: '0.1.0',
    protocolVersion: '1',
    lastSeenAt: ago(3),
    queueDepth: 2,
    printerCount: 2,
    labels: ['dispatch', 'arm64']
  },
  {
    id: 'agt_01K0D',
    name: 'Test workstation',
    state: 'offline',
    os: 'windows',
    architecture: 'x86_64',
    version: '0.0.9',
    protocolVersion: '1',
    lastSeenAt: ago(124),
    queueDepth: 0,
    printerCount: 3,
    labels: ['test']
  }
];

const capabilities = {
  color: false,
  duplex: false,
  copies: 99,
  papers: ['4x6', 'A6', 'Custom.100x150mm'],
  dpis: ['203x203', '300x300'],
  source: 'windows_driver',
  revision: 'cap_01J9Z',
  observedAt: ago(2)
};

export const printers: DashboardPrinter[] = [
  {
    id: 'prt_01K0A',
    agentId: 'agt_01K0A',
    name: 'Zebra ZD421 — Packing',
    description: '4 × 6 shipping labels',
    location: 'Packing desk',
    state: 'online',
    stateReasons: [],
    isDefault: true,
    queueDepth: 3,
    lastSeenAt: ago(0),
    capabilityRevision: 1,
    nativeOptions: {},
    profiles: [
      {
        profileId: 'prf_zebra_shipping',
        revision: 3,
        name: '4 × 6 shipping',
        isDefault: true,
        options: { paper: '4x6', dpi: '203x203', copies: 1 },
        status: 'ready',
        nativeKind: 'windows_devmode',
        nativeDigest: 'sha256:772a',
        driverName: 'ZDesigner ZD421-203dpi ZPL',
        driverVersion: '10.5.1',
        summary: {
          paper: '4 × 6 shipping',
          dimensionsMm: [101.6, 152.4],
          source: 'Roll',
          media: 'Gap',
          color: 'Mono',
          resolution: '203 × 203 dpi'
        },
        stockId: 'stk_shipping_4x6',
        safeOverrides: ['copies', 'pages'],
        lastValidatedAt: ago(14),
        lastTestJobId: 'job_test_zebra',
        published: true
      }
    ],
    capabilities
  },
  {
    id: 'prt_01K0B',
    agentId: 'agt_01K0A',
    name: 'Brother QL-820NWB',
    description: 'Product labels',
    location: 'Packing desk',
    state: 'degraded',
    stateReasons: ['media_low'],
    isDefault: false,
    queueDepth: 1,
    lastSeenAt: ago(1),
    capabilityRevision: 1,
    nativeOptions: {},
    profiles: [
      {
        profileId: 'prf_brother_product',
        revision: 1,
        name: '62 mm product label',
        isDefault: true,
        options: { paper: '62mm', copies: 1 },
        status: 'needs_test',
        nativeKind: 'portable_options',
        nativeDigest: null,
        driverName: null,
        driverVersion: null,
        summary: {
          paper: '62 mm',
          dimensionsMm: null,
          source: 'Roll',
          media: null,
          color: 'Mono',
          resolution: null
        },
        stockId: 'stk_product_62',
        safeOverrides: ['copies'],
        lastValidatedAt: null,
        lastTestJobId: null,
        published: false
      }
    ],
    capabilities: { ...capabilities, papers: ['62mm', '29mm'], source: 'windows_driver' }
  },
  {
    id: 'prt_01K0C',
    agentId: 'agt_01K0B',
    name: 'Office LaserJet',
    description: 'A4 documents',
    location: 'Upstairs office',
    state: 'online',
    stateReasons: [],
    isDefault: true,
    queueDepth: 0,
    lastSeenAt: ago(1),
    capabilityRevision: 1,
    nativeOptions: {},
    profiles: [
      {
        profileId: 'prf_office_colour',
        revision: 2,
        name: 'A4 colour',
        isDefault: true,
        options: { paper: 'A4', color: true, duplex: 'one-sided', copies: 1 },
        status: 'ready',
        nativeKind: 'macos_printcore',
        nativeDigest: 'sha256:b061',
        driverName: 'HP OfficeJet Pro',
        driverVersion: '4.2',
        summary: {
          paper: 'A4',
          dimensionsMm: [210, 297],
          source: 'Automatic',
          media: 'Plain paper',
          color: 'Colour',
          resolution: '600 dpi'
        },
        stockId: 'stk_a4_plain',
        safeOverrides: ['copies', 'pages'],
        lastValidatedAt: ago(18),
        lastTestJobId: 'job_test_hp_colour',
        published: true
      },
      {
        profileId: 'prf_office_mono',
        revision: 1,
        name: 'A4 mono draft',
        isDefault: false,
        options: { paper: 'A4', color: false, duplex: 'one-sided', copies: 1 },
        status: 'ready',
        nativeKind: 'macos_printcore',
        nativeDigest: 'sha256:508d',
        driverName: 'HP OfficeJet Pro',
        driverVersion: '4.2',
        summary: {
          paper: 'A4',
          dimensionsMm: [210, 297],
          source: 'Automatic',
          media: 'Plain paper',
          color: 'Monochrome draft',
          resolution: '300 dpi'
        },
        stockId: 'stk_a4_plain',
        safeOverrides: ['copies', 'pages'],
        lastValidatedAt: ago(16),
        lastTestJobId: 'job_test_hp_mono',
        published: true
      }
    ],
    capabilities: {
      ...capabilities,
      color: true,
      duplex: true,
      papers: ['A4', 'A5', 'Letter'],
      source: 'cups'
    }
  },
  {
    id: 'prt_01K0D',
    agentId: 'agt_01K0C',
    name: 'Dispatch Zebra',
    description: 'Courier labels',
    location: 'Dispatch',
    state: 'offline',
    stateReasons: ['not_connected'],
    isDefault: false,
    queueDepth: 2,
    lastSeenAt: ago(7),
    capabilityRevision: 1,
    nativeOptions: {},
    profiles: [],
    capabilities: { ...capabilities, source: 'cups' }
  }
];

export const jobs: DashboardJob[] = [
  {
    id: 'job_01K0VY5YJ',
    printerId: 'prt_01K0A',
    agentId: 'agt_01K0A',
    title: 'Order #10842 shipping label',
    source: 'shopify-webhook',
    contentFormat: 'pdf',
    state: 'printing',
    reasonCode: null,
    message: 'Printing page 1 of 1',
    authority: 'os_queue',
    nativeJobId: '481',
    createdAt: ago(1),
    updatedAt: ago(0),
    expiresAt: null,
    contentRetained: true
  },
  {
    id: 'job_01K0VY4QP',
    printerId: 'prt_01K0B',
    agentId: 'agt_01K0A',
    title: 'Ethiopia Guji 250g × 12',
    source: 'cin7',
    contentFormat: 'pdf',
    state: 'blocked',
    reasonCode: 'media_low',
    message: 'Printer reports media low',
    authority: 'device',
    nativeJobId: '479',
    createdAt: ago(6),
    updatedAt: ago(2),
    expiresAt: null,
    contentRetained: true
  },
  {
    id: 'job_01K0VXZZB',
    printerId: 'prt_01K0A',
    agentId: 'agt_01K0A',
    title: 'Order #10841 shipping label',
    source: 'shopify-webhook',
    contentFormat: 'pdf',
    state: 'completed_reported',
    reasonCode: null,
    message: 'Completed according to the OS queue',
    authority: 'os_queue',
    nativeJobId: '478',
    createdAt: ago(9),
    updatedAt: ago(7),
    expiresAt: null,
    contentRetained: true
  },
  {
    id: 'job_01K0VXSQ2',
    printerId: 'prt_01K0D',
    agentId: 'agt_01K0C',
    title: 'NZ Post manifest #3301',
    source: 'dispatch',
    contentFormat: 'pdf',
    state: 'waiting_for_agent',
    reasonCode: 'agent_offline',
    message: 'Waiting for Dispatch Pi to reconnect',
    authority: 'service',
    nativeJobId: null,
    createdAt: ago(12),
    updatedAt: ago(7),
    expiresAt: new Date(now.getTime() + 48 * 60 * 60_000).toISOString(),
    contentRetained: true
  },
  {
    id: 'job_01K0VXP8M',
    printerId: 'prt_01K0C',
    agentId: 'agt_01K0B',
    title: 'Wholesale invoice C4-9012',
    source: 'xero',
    contentFormat: 'pdf',
    state: 'failed_terminal',
    reasonCode: 'malformed_pdf',
    message: 'PDF validation failed at object 14',
    authority: 'renderer',
    nativeJobId: null,
    createdAt: ago(19),
    updatedAt: ago(18),
    expiresAt: null,
    contentRetained: true
  },
  {
    id: 'job_01K0VXK4A',
    printerId: 'prt_01K0A',
    agentId: 'agt_01K0A',
    title: 'Order #10838 shipping label',
    source: 'shopify-webhook',
    contentFormat: 'pdf',
    state: 'delivery_uncertain',
    reasonCode: 'ambiguous_handoff',
    message: 'The agent restarted after handing the job to Windows',
    authority: 'agent',
    nativeJobId: null,
    createdAt: ago(31),
    updatedAt: ago(29),
    expiresAt: null,
    contentRetained: true
  }
];

export const jobEvents: DashboardJobEvent[] = [
  {
    id: 'evt_01',
    jobId: jobs[0]?.id ?? '',
    sequence: 1,
    type: 'job.registered',
    state: 'registered',
    observer: 'control_plane',
    authority: 'service',
    reasonCode: null,
    message: 'Job and content registered durably',
    occurredAt: ago(1),
    receivedAt: ago(1),
    details: {}
  },
  {
    id: 'evt_02',
    jobId: jobs[0]?.id ?? '',
    sequence: 2,
    type: 'agent.accepted',
    state: 'agent_accepted',
    observer: 'spool_agent',
    authority: 'agent',
    reasonCode: null,
    message: 'Packing desk accepted the job into its durable queue',
    occurredAt: ago(1),
    receivedAt: ago(1),
    details: { queuePosition: 1 }
  },
  {
    id: 'evt_03',
    jobId: jobs[0]?.id ?? '',
    sequence: 3,
    type: 'spooler.accepted',
    state: 'accepted_by_spooler',
    observer: 'windows_spooler',
    authority: 'os_queue',
    reasonCode: null,
    message: 'Accepted by the Windows print queue as native job 481',
    occurredAt: ago(0),
    receivedAt: ago(0),
    details: { nativeJobId: 481 }
  },
  {
    id: 'evt_04',
    jobId: jobs[0]?.id ?? '',
    sequence: 4,
    type: 'spooler.printing',
    state: 'printing',
    observer: 'windows_spooler',
    authority: 'os_queue',
    reasonCode: null,
    message: 'Printing page 1 of 1',
    occurredAt: ago(0),
    receivedAt: ago(0),
    details: { page: 1, pages: 1 }
  }
];

export const webhooks: DashboardWebhook[] = [
  {
    id: 'whk_01K0A',
    url: 'https://orders.c4coffee.co.nz/hooks/spool',
    description: 'Order status updates',
    events: ['job.*'],
    enabled: true,
    status: 'healthy',
    lastDeliveryAt: ago(1),
    createdAt: ago(18_420)
  },
  {
    id: 'whk_01K0B',
    url: 'https://ops.example.com/printing',
    description: 'Fleet health',
    events: ['agent.*', 'printer.*'],
    enabled: true,
    status: 'failing',
    lastDeliveryAt: ago(92),
    createdAt: ago(4_210)
  }
];

export const apiKeys: DashboardApiKey[] = [
  {
    id: 'key_01K0A',
    name: 'Production orders',
    prefix: 'spl_live_9F4E',
    environment: 'live',
    scopes: ['jobs:read', 'jobs:write', 'printers:read'],
    lastUsedAt: ago(0),
    createdAt: ago(52_820)
  },
  {
    id: 'key_01K0B',
    name: 'Local development',
    prefix: 'spl_test_2A81',
    environment: 'test',
    scopes: ['jobs:read', 'jobs:write', 'printers:read'],
    lastUsedAt: ago(412),
    createdAt: ago(14_980)
  }
];

export const accounts: DashboardAccount[] = [
  {
    id: 'wsp_customer_north',
    externalId: 'customer:north-star',
    name: 'North Star Coffee',
    status: 'active',
    metadata: { plan: 'Pro', region: 'New Zealand' },
    environments: {
      testId: 'env_test_north',
      liveId: 'env_live_north'
    },
    createdAt: ago(86_400),
    updatedAt: ago(8)
  },
  {
    id: 'wsp_customer_atlas',
    externalId: 'customer:atlas-studio',
    name: 'Atlas Design Studio',
    status: 'active',
    metadata: { plan: 'Pro', integration: 'Design editor' },
    environments: {
      testId: 'env_test_atlas',
      liveId: 'env_live_atlas'
    },
    createdAt: ago(42_200),
    updatedAt: ago(94)
  },
  {
    id: 'wsp_customer_field',
    externalId: 'customer:field-goods',
    name: 'Field Goods',
    status: 'suspended',
    metadata: { plan: 'Free', reason: 'Payment review' },
    environments: {
      testId: 'env_test_field',
      liveId: 'env_live_field'
    },
    createdAt: ago(21_540),
    updatedAt: ago(1_320)
  }
];
