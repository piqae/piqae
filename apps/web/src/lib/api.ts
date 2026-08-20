import { PiqaeClient } from '@piqae/sdk';
import type {
  DashboardAccount,
  DashboardAgent,
  DashboardApiKey,
  DashboardJob,
  DashboardJobEvent,
  DashboardMeta,
  DashboardOverview,
  DashboardPage,
  DashboardPrinter,
  DashboardWebhook
} from './view-types';
import * as demo from './demo-data';

export interface DashboardApi {
  meta(): Promise<DashboardMeta>;
  platformEnabled(): Promise<boolean>;
  enablePlatform(): Promise<{ enabled: true; secret: string }>;
  platformCredential(): Promise<DashboardApiKey | null>;
  rotatePlatformCredential(): Promise<DashboardApiKey & { secret: string }>;
  revokePlatformCredential(): Promise<void>;
  overview(): Promise<DashboardOverview>;
  agents(): Promise<DashboardPage<DashboardAgent>>;
  printers(): Promise<DashboardPage<DashboardPrinter>>;
  jobs(): Promise<DashboardPage<DashboardJob>>;
  job(id: string): Promise<DashboardJob | null>;
  jobEvents(id: string): Promise<DashboardPage<DashboardJobEvent>>;
  webhooks(): Promise<DashboardPage<DashboardWebhook>>;
  apiKeys(): Promise<DashboardPage<DashboardApiKey>>;
  accounts(): Promise<DashboardPage<DashboardAccount>>;
  account(externalId: string): Promise<DashboardAccount | null>;
}

const page = <T>(data: T[]): DashboardPage<T> => ({ data, nextCursor: null });
const delay = <T>(value: T): Promise<T> =>
  new Promise((resolve) => setTimeout(() => resolve(value), 60));

export const mockApi: DashboardApi = {
  meta: () =>
    delay({
      deployment: 'local',
      version: '0.1.0',
      auth: { provider: 'none', workspaceSwitching: false, invitations: false },
      billing: { enabled: false },
      updates: { officialFeed: true, customFeed: true },
      platform: { accounts: true }
    }),
  platformEnabled: () => delay(true),
  enablePlatform: () => delay({ enabled: true, secret: 'demo-platform-secret' }),
  platformCredential: () =>
    delay({
      id: '00000000-0000-7000-8000-000000000001',
      name: 'Piqae platform integration',
      prefix: 'piq_platform_00000000-0000-7000-8000-000000000001',
      environment: 'platform',
      kind: 'platform',
      scopes: [],
      lastUsedAt: null,
      createdAt: '2026-08-20T00:00:00.000Z'
    }),
  rotatePlatformCredential: async () => {
    throw new Error('Demo mode does not rotate credentials.');
  },
  revokePlatformCredential: async () => {
    throw new Error('Demo mode does not revoke credentials.');
  },
  overview: () =>
    delay({
      agents: {
        total: demo.agents.length,
        online: demo.agents.filter((node) => node.state === 'online').length,
        degraded: demo.agents.filter((node) => node.state === 'degraded').length
      },
      printers: {
        total: demo.printers.length,
        online: demo.printers.filter((printer) => printer.state === 'online').length,
        attention: demo.printers.filter((printer) => printer.state !== 'online').length
      },
      jobs: {
        recent: demo.jobs.length,
        active: demo.jobs.filter((job) =>
          [
            'created',
            'content_pending',
            'queued',
            'waiting_for_agent',
            'agent_accepted',
            'queued_local',
            'spooling',
            'printing'
          ].includes(job.state)
        ).length,
        failed: demo.jobs.filter((job) => job.state.startsWith('failed')).length,
        uncertain: demo.jobs.filter((job) => job.state === 'delivery_uncertain').length
      }
    }),
  agents: () => delay(page(demo.agents)),
  printers: () => delay(page(demo.printers)),
  jobs: () => delay(page(demo.jobs)),
  job: (id) => delay(demo.jobs.find((job) => job.id === id) ?? null),
  jobEvents: (id) =>
    delay(page(demo.jobEvents.map((event) => ({ ...event, jobId: id })))),
  webhooks: () => delay(page(demo.webhooks)),
  apiKeys: () => delay(page(demo.apiKeys)),
  accounts: () => delay(page(demo.accounts)),
  account: (externalId) =>
    delay(demo.accounts.find((account) => account.externalId === externalId) ?? null)
};

/**
 * The dashboard view model is intentionally richer than the current public
 * OpenAPI. This adapter only derives fields represented by that contract;
 * diagnostics and usage screens remain disabled until their public endpoints
 * are added.
 */
export function createLiveApi(
  fetcher: typeof fetch,
  baseUrl: string,
  apiKey?: string
): DashboardApi {
  const client = new PiqaeClient({
    baseUrl,
    fetch: fetcher,
    ...(apiKey ? { apiKey } : {}),
    headers: { 'x-piqae-dashboard': '1' }
  });

  const toAgent = (agent: Awaited<ReturnType<typeof client.agents.list>>[number]): DashboardAgent => ({
    id: agent.id,
    name: agent.name,
    state:
      agent.state === 'connected'
        ? 'online'
        : agent.state === 'disconnected'
          ? 'offline'
          : agent.state,
    os: agent.platform.toLowerCase().includes('win')
      ? 'windows'
      : agent.platform.toLowerCase().includes('mac')
        ? 'macos'
        : 'linux',
    architecture: agent.platform.split('/')[1] ?? 'unknown',
    version: agent.version,
    protocolVersion: '1',
    lastSeenAt: agent.last_seen_at,
    queueDepth: 0,
    printerCount: 0,
    labels: []
  });

  const toPrinter = (
    printer: Awaited<ReturnType<typeof client.printers.list>>['data'][number]
  ): DashboardPrinter => ({
    id: printer.id,
    agentId: printer.agent_id,
    name: printer.name,
    description: null,
    location: null,
    state: printer.state === 'busy'
      ? 'online'
      : printer.state === 'paper_out' || printer.state === 'error'
        ? 'degraded'
        : printer.state,
    stateReasons: ['paper_out', 'error'].includes(printer.state) ? [printer.state] : [],
    isDefault: false,
    queueDepth: 0,
    lastSeenAt: printer.updated_at,
    capabilityRevision: printer.capability_revision,
    nativeOptions: printer.native_options,
    profiles: printer.profiles.map((profile) => ({
      profileId: profile.profile_id,
      revision: profile.revision,
      name: profile.name,
      isDefault: profile.is_default,
      options: profile.options,
      status: profile.status ?? 'needs_test',
      nativeKind: profile.native_kind ?? 'portable_options',
      nativeDigest: profile.native_digest ?? null,
      driverName: profile.driver_fingerprint?.driver_name ?? null,
      driverVersion: profile.driver_fingerprint?.driver_version ?? null,
      summary: {
        paper: profile.summary?.paper ?? profile.options.paper ?? null,
        dimensionsMm: profile.summary?.dimensions_mm ?? null,
        source: profile.summary?.source ?? profile.options.bin ?? null,
        media: profile.summary?.media ?? profile.options.media ?? null,
        color:
          profile.summary?.color ??
          (profile.options.color === undefined ? null : profile.options.color ? 'Colour' : 'Mono'),
        resolution: profile.summary?.resolution ?? profile.options.dpi ?? null
      },
      stockId: profile.stock_id ?? null,
      safeOverrides: profile.safe_overrides ?? ['copies', 'pages'],
      lastValidatedAt: profile.last_validated_at ?? null,
      lastTestJobId: profile.last_test_job_id ?? null,
      published: profile.published ?? true
    })),
    capabilities: {
      color: printer.capabilities.color === true,
      duplex: printer.capabilities.duplex === true,
      copies: Number(printer.capabilities.copies ?? 1),
      papers: Object.keys(printer.capabilities.papers),
      dpis: printer.capabilities.dpis,
      source: 'driver',
      revision: String(printer.capability_revision),
      observedAt: printer.updated_at
    }
  });

  const toJob = (job: Awaited<ReturnType<typeof client.jobs.retrieve>>): DashboardJob => ({
    id: job.id,
    printerId: job.printer_id,
    agentId: '',
    title: job.title,
    source: job.source ?? null,
    contentFormat: job.content_type,
    state: job.state,
    reasonCode: null,
    message: null,
    authority: 'service',
    nativeJobId: null,
    createdAt: job.created_at,
    updatedAt: job.created_at,
    expiresAt: job.expires_at,
    contentRetained: true
  });

  const platformRequest = async (path: string, init: RequestInit = {}): Promise<Response> =>
    fetcher(`${baseUrl.replace(/\/$/, '')}${path}`, {
      ...init,
      headers: {
        accept: 'application/json',
        'x-piqae-dashboard': '1',
        ...(apiKey ? { authorization: `Bearer ${apiKey}` } : {})
      }
    });

  return {
    meta: async () => {
      const response = await fetcher(`${baseUrl.replace(/\/$/, '')}/v1/meta`, {
        headers: { accept: 'application/json' }
      });
      if (!response.ok) {
        throw new Error(`Piqae metadata request failed with HTTP ${response.status}.`);
      }
      return parseDashboardMeta(await response.json());
    },
    overview: async () => {
      const [agentList, printerPage, jobPage] = await Promise.all([
        client.agents.list(),
        client.printers.list({ limit: 100 }),
        client.jobs.list({ limit: 100 })
      ]);
      return {
        agents: {
          total: agentList.length,
          online: agentList.filter((agent) => agent.state === 'connected').length,
          degraded: agentList.filter((agent) => agent.state === 'degraded').length
        },
        printers: {
          total: printerPage.data.length,
          online: printerPage.data.filter((printer) => printer.state === 'online').length,
          attention: printerPage.data.filter((printer) => printer.state !== 'online').length
        },
        jobs: {
          recent: jobPage.data.length,
          active: jobPage.data.filter((job) =>
            [
              'created',
              'content_pending',
              'queued',
              'waiting_for_agent',
              'agent_accepted',
              'queued_local',
              'spooling',
              'printing'
            ].includes(job.state)
          ).length,
          failed: jobPage.data.filter((job) => job.state.startsWith('failed')).length,
          uncertain: jobPage.data.filter((job) => job.state === 'delivery_uncertain').length
        },
      };
    },
    agents: async () => page((await client.agents.list()).map(toAgent)),
    printers: async () => {
      const result = await client.printers.list({ limit: 100 });
      return { data: result.data.map(toPrinter), nextCursor: result.next_cursor ?? null };
    },
    jobs: async () => {
      const result = await client.jobs.list({ limit: 100 });
      return { data: result.data.map(toJob), nextCursor: result.next_cursor ?? null };
    },
    job: async (id) => toJob(await client.jobs.retrieve(id)),
    jobEvents: async (id) =>
      page(
        (await client.jobs.events(id)).map((event) => ({
          id: event.id,
          jobId: event.job_id,
          sequence: event.sequence,
          type: `job.${event.state}`,
          state: event.state,
          observer: 'service',
          authority: 'service',
          reasonCode: event.reason ?? null,
          message: event.message ?? event.state.replaceAll('_', ' '),
          occurredAt: event.occurred_at,
          receivedAt: event.occurred_at,
          details: {}
        }))
      ),
    webhooks: async () =>
      page(
        (await client.webhooks.list()).map((webhook) => ({
          id: webhook.id,
          url: webhook.url,
          description: null,
          events: webhook.events,
          enabled: webhook.enabled,
          status: webhook.enabled ? 'healthy' : 'disabled',
          lastDeliveryAt: null,
          createdAt: webhook.created_at
        }))
      ),
    apiKeys: async () =>
      page(
        (await client.apiKeys.list())
          .filter((apiKey) => apiKey.revoked_at === null)
          .map((apiKey) => ({
            id: apiKey.id,
            name: apiKey.name,
            prefix: apiKey.lookup_prefix,
            environment: apiKey.lookup_prefix.startsWith('piq_test_') ? 'test' : 'live',
            scopes: apiKey.scopes.map((scope) => scope.replace(/_([^_]*)$/, ':$1')),
            lastUsedAt: apiKey.last_used_at,
            createdAt: apiKey.created_at
          }))
      ),
    platformEnabled: async () => {
      const response = await platformRequest('/v1/platform/status');
      if (!response.ok) {
        throw new Error(`Piqae platform status request failed with HTTP ${response.status}.`);
      }
      const value: unknown = await response.json();
      return isRecord(value) && value.enabled === true;
    },
    enablePlatform: async () => {
      const response = await platformRequest('/v1/platform/enable', { method: 'POST' });
      if (!response.ok) {
        throw new Error(`Piqae platform enablement request failed with HTTP ${response.status}.`);
      }
      const value: unknown = await response.json();
      if (!isRecord(value) || value.enabled !== true || typeof value.secret !== 'string') {
        throw new Error('Piqae platform enablement response was invalid.');
      }
      return { enabled: true, secret: value.secret };
    },
    platformCredential: async () => {
      const response = await platformRequest('/v1/platform/credential');
      if (response.status === 404) return null;
      if (!response.ok) {
        throw new Error(`Piqae platform credential request failed with HTTP ${response.status}.`);
      }
      return parsePlatformCredential(await response.json());
    },
    rotatePlatformCredential: async () => {
      const response = await platformRequest('/v1/platform/credential', { method: 'POST' });
      if (!response.ok) {
        throw new Error(`Piqae platform rotation failed with HTTP ${response.status}.`);
      }
      const value: unknown = await response.json();
      if (!isRecord(value) || typeof value.secret !== 'string') {
        throw new Error('Piqae platform rotation response was invalid.');
      }
      return { ...parsePlatformCredential(value), secret: value.secret };
    },
    revokePlatformCredential: async () => {
      const response = await platformRequest('/v1/platform/credential', { method: 'DELETE' });
      if (!response.ok) {
        throw new Error(`Piqae platform revocation failed with HTTP ${response.status}.`);
      }
    },
    accounts: async () => {
      const response = await platformRequest('/v1/platform/accounts');
      if (!response.ok) {
        throw new Error(`Piqae customer account request failed with HTTP ${response.status}.`);
      }
      const value: unknown = await response.json();
      if (!Array.isArray(value)) {
        throw new Error('Piqae customer account response was not a list.');
      }
      return page(value.map(parseDashboardAccount));
    },
    account: async (externalId) => {
      const response = await platformRequest(
        `/v1/platform/accounts/${encodeURIComponent(externalId)}`
      );
      if (response.status === 404) return null;
      if (!response.ok) {
        throw new Error(`Piqae customer account request failed with HTTP ${response.status}.`);
      }
      return parseDashboardAccount(await response.json());
    }
  };
}

export function parseDashboardMeta(value: unknown): DashboardMeta {
  const raw = isRecord(value) ? value : {};
  const auth = isRecord(raw.auth) ? raw.auth : {};
  const billing = isRecord(raw.billing) ? raw.billing : {};
  const updates = isRecord(raw.updates) ? raw.updates : {};
  const platform = isRecord(raw.platform) ? raw.platform : {};
  const deployment = ['cloud', 'self_hosted', 'local'].includes(String(raw.deployment))
    ? (raw.deployment as DashboardMeta['deployment'])
    : 'self_hosted';
  const provider = ['workos', 'local_owner', 'oidc', 'hybrid', 'none'].includes(
    String(auth.provider)
  )
    ? (auth.provider as DashboardMeta['auth']['provider'])
    : 'none';
  return {
    deployment,
    version: typeof raw.version === 'string' && raw.version !== '' ? raw.version : 'unknown',
    auth: {
      provider,
      workspaceSwitching: auth.workspace_switching === true,
      invitations: auth.invitations === true
    },
    billing: { enabled: billing.enabled === true },
    updates: {
      officialFeed: updates.official_feed === true,
      customFeed: updates.custom_feed === true
    },
    platform: { accounts: platform.accounts === true }
  };
}

export function parseDashboardAccount(value: unknown): DashboardAccount {
  if (!isRecord(value)) throw new Error('Piqae customer account response was invalid.');
  const environments = isRecord(value.environments) ? value.environments : {};
  const testEnvironment = isRecord(environments.test) ? environments.test : {};
  const liveEnvironment = isRecord(environments.live) ? environments.live : {};
  const metadata = isRecord(value.metadata) ? value.metadata : {};
  const status = ['active', 'suspended', 'cancelled'].includes(String(value.status))
    ? (value.status as DashboardAccount['status'])
    : null;
  const requiredStrings = [
    value.id,
    value.external_id,
    value.name,
    value.created_at,
    value.updated_at,
    testEnvironment.id,
    liveEnvironment.id
  ];
  if (!status || requiredStrings.some((item) => typeof item !== 'string' || item === '')) {
    throw new Error('Piqae customer account response was invalid.');
  }
  const safeMetadata = Object.fromEntries(
    Object.entries(metadata).filter(
      (entry): entry is [string, string] => typeof entry[1] === 'string'
    )
  );
  return {
    id: value.id as string,
    externalId: value.external_id as string,
    name: value.name as string,
    status,
    metadata: safeMetadata,
    environments: {
      testId: testEnvironment.id as string,
      liveId: liveEnvironment.id as string
    },
    createdAt: value.created_at as string,
    updatedAt: value.updated_at as string
  };
}

export function parsePlatformCredential(value: unknown): DashboardApiKey {
  if (!isRecord(value)) throw new Error('Piqae platform credential response was invalid.');
  const required = [value.id, value.name, value.lookup_prefix, value.created_at];
  if (required.some((item) => typeof item !== 'string' || item === '')) {
    throw new Error('Piqae platform credential response was invalid.');
  }
  if (value.last_used_at !== null && typeof value.last_used_at !== 'string') {
    throw new Error('Piqae platform credential response was invalid.');
  }
  return {
    id: value.id as string,
    name: value.name as string,
    prefix: value.lookup_prefix as string,
    environment: 'platform',
    kind: 'platform',
    scopes: [],
    lastUsedAt: value.last_used_at as string | null,
    createdAt: value.created_at as string
  };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}
