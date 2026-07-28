import { SpoolClient } from '@spool/sdk';
import type {
  DashboardAgent,
  DashboardApiKey,
  DashboardJob,
  DashboardJobEvent,
  DashboardOverview,
  DashboardPage,
  DashboardPrinter,
  DashboardWebhook
} from './view-types';
import * as demo from './demo-data';

export interface DashboardApi {
  overview(): Promise<DashboardOverview>;
  agents(): Promise<DashboardPage<DashboardAgent>>;
  printers(): Promise<DashboardPage<DashboardPrinter>>;
  jobs(): Promise<DashboardPage<DashboardJob>>;
  job(id: string): Promise<DashboardJob | null>;
  jobEvents(id: string): Promise<DashboardPage<DashboardJobEvent>>;
  webhooks(): Promise<DashboardPage<DashboardWebhook>>;
  apiKeys(): Promise<DashboardPage<DashboardApiKey>>;
}

const page = <T>(data: T[]): DashboardPage<T> => ({ data, nextCursor: null });
const delay = <T>(value: T): Promise<T> =>
  new Promise((resolve) => setTimeout(() => resolve(value), 60));

export const mockApi: DashboardApi = {
  overview: () => delay(demo.overview),
  agents: () => delay(page(demo.agents)),
  printers: () => delay(page(demo.printers)),
  jobs: () => delay(page(demo.jobs)),
  job: (id) => delay(demo.jobs.find((job) => job.id === id) ?? null),
  jobEvents: (id) =>
    delay(page(demo.jobEvents.map((event) => ({ ...event, jobId: id })))),
  webhooks: () => delay(page(demo.webhooks)),
  apiKeys: () => delay(page(demo.apiKeys))
};

/**
 * The dashboard view model is intentionally richer than the current public
 * OpenAPI. This adapter only derives fields represented by that contract;
 * diagnostics, usage, and API-key screens remain disabled until their public
 * admin endpoints are added.
 */
export function createLiveApi(fetcher: typeof fetch, baseUrl: string): DashboardApi {
  const client = new SpoolClient({
    baseUrl,
    fetch: fetcher,
    headers: { 'x-spool-dashboard': '1' }
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
    capabilities: {
      color: printer.capabilities.color === true,
      duplex: printer.capabilities.duplex === true,
      copies: Number(printer.capabilities.copies ?? 1),
      papers: Array.isArray(printer.capabilities.papers)
        ? printer.capabilities.papers.map(String)
        : [],
      dpis: Array.isArray(printer.capabilities.dpis)
        ? printer.capabilities.dpis.map(String)
        : [],
      source: String(printer.capabilities.source ?? 'driver'),
      revision: String(printer.capabilities.revision ?? 'unknown'),
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

  return {
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
          today: jobPage.data.length,
          active: jobPage.data.filter((job) => !job.state.startsWith('failed')).length,
          failed: jobPage.data.filter((job) => job.state.startsWith('failed')).length,
          uncertain: jobPage.data.filter((job) => job.state === 'delivery_uncertain').length
        },
        pickupLatencyP95Ms: 0
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
    apiKeys: async () => page([])
  };
}
