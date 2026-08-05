import { fail } from '@sveltejs/kit';
import type { Actions, PageServerLoad, RequestEvent } from './$types';
import {
  dashboardMode,
  dashboardSdk,
  dashboardSource,
  preventSecretCaching,
  presentDashboardError
} from '$lib/server/dashboard-data';
import { isOperationalView } from '$lib/dashboard-navigation';
import type { DashboardApi } from '$lib/api';
import type {
  DashboardAccount,
  DashboardAgent,
  DashboardJob,
  DashboardJobEvent,
  DashboardOverview,
  DashboardPrinter
} from '$lib/view-types';

const emptyOverview: DashboardOverview = {
  agents: { total: 0, online: 0, degraded: 0 },
  printers: { total: 0, online: 0, attention: 0 },
  jobs: { recent: 0, active: 0, failed: 0, uncertain: 0 }
};

export type OperationalDetail =
  | {
      kind: 'job';
      job: DashboardJob;
      events: DashboardJobEvent[];
      printer: DashboardPrinter | null;
      agent: DashboardAgent | null;
    }
  | {
      kind: 'printer';
      printer: DashboardPrinter;
      agent: DashboardAgent | null;
      jobs: DashboardJob[];
    }
  | { kind: 'node'; node: DashboardAgent; printers: DashboardPrinter[] }
  | { kind: 'customer'; account: DashboardAccount }
  | { kind: 'missing'; label: string };

type LoadedLists = {
  jobs: DashboardJob[];
  printers: DashboardPrinter[];
  agents: DashboardAgent[];
  accounts: DashboardAccount[];
};

export const load: PageServerLoad = async (event) => {
  const { meta } = await event.parent();
  const { api } = dashboardSource(event);
  let platformEnabled = false;
  if (meta.platform.accounts) {
    try {
      platformEnabled = await api.platformEnabled();
    } catch {
      // Platform status must never make the ordinary operational dashboard
      // unavailable. Account operations remain hidden and fail closed.
    }
  }
  const effectiveMeta = {
    ...meta,
    platform: { accounts: meta.platform.accounts && platformEnabled }
  };
  const requestedView = event.url.searchParams.get('view');
  const view = isOperationalView(requestedView, effectiveMeta) ? requestedView : 'jobs';

  try {
    const [overview, jobs, printers, agents, accounts] = await Promise.all([
      api.overview(),
      api.jobs(),
      api.printers(),
      api.agents(),
      effectiveMeta.platform.accounts
        ? api.accounts()
        : Promise.resolve({ data: [] as DashboardAccount[], nextCursor: null })
    ]);

    const lists: LoadedLists = {
      jobs: jobs.data,
      printers: printers.data,
      agents: agents.data,
      accounts: accounts.data
    };

    return {
      view,
      platformEnabled,
      overview,
      ...lists,
      detail: await loadDetail(event, api, lists),
      dataError: null
    };
  } catch (error) {
    return {
      view,
      platformEnabled,
      overview: emptyOverview,
      jobs: [],
      printers: [],
      agents: [],
      accounts: [],
      detail: null,
      dataError: presentDashboardError(error)
    };
  }
};

/**
 * Detail lives in a drawer on this page rather than on its own route, so the
 * selected entity comes from the query string and is resolved alongside the
 * list. Only the job drawer needs a further round trip — its event timeline is
 * not part of any list payload.
 */
async function loadDetail(
  event: RequestEvent,
  api: DashboardApi,
  loaded: LoadedLists
): Promise<OperationalDetail | null> {
  const params = event.url.searchParams;

  const jobId = params.get('job');
  if (jobId) {
    const [job, events] = await Promise.all([api.job(jobId), api.jobEvents(jobId)]);
    if (!job) return { kind: 'missing', label: 'job' };
    return {
      kind: 'job',
      job,
      events: events.data,
      printer: loaded.printers.find((printer) => printer.id === job.printerId) ?? null,
      agent: loaded.agents.find((agent) => agent.id === job.agentId) ?? null
    };
  }

  const printerId = params.get('printer');
  if (printerId) {
    const printer = loaded.printers.find((candidate) => candidate.id === printerId);
    if (!printer) return { kind: 'missing', label: 'printer' };
    return {
      kind: 'printer',
      printer,
      agent: loaded.agents.find((agent) => agent.id === printer.agentId) ?? null,
      jobs: loaded.jobs.filter((job) => job.printerId === printer.id)
    };
  }

  const nodeId = params.get('node');
  if (nodeId) {
    const node = loaded.agents.find((candidate) => candidate.id === nodeId);
    if (!node) return { kind: 'missing', label: 'node' };
    return {
      kind: 'node',
      node,
      printers: loaded.printers.filter((printer) => printer.agentId === node.id)
    };
  }

  const customerId = params.get('customer');
  if (customerId) {
    const account = loaded.accounts.find((candidate) => candidate.externalId === customerId);
    if (!account) return { kind: 'missing', label: 'customer' };
    return { kind: 'customer', account };
  }

  return null;
}

export const actions: Actions = {
  createEnrolment: async (event) => {
    preventSecretCaching(event);
    if (dashboardMode() !== 'live') {
      return fail(400, {
        mutation: 'createEnrolment',
        error: { message: 'Node enrolment is disabled while demo data is active.' }
      });
    }
    const data = await event.request.formData();
    const name = String(data.get('name') ?? '').trim();
    const expiresInSeconds = Number(data.get('expires_in_seconds') ?? 600);
    if (name && (name.length < 2 || name.length > 120)) {
      return fail(400, {
        mutation: 'createEnrolment',
        error: { message: 'A custom node name must be between 2 and 120 characters.' }
      });
    }
    if (!Number.isInteger(expiresInSeconds) || expiresInSeconds < 60 || expiresInSeconds > 900) {
      return fail(400, {
        mutation: 'createEnrolment',
        error: { message: 'Expiry must be between 60 and 900 seconds.' }
      });
    }
    try {
      const enrolment = await dashboardSdk(event).connectSessions.create({
        ...(name ? { name } : {}),
        expires_in_seconds: expiresInSeconds,
        return_url: new URL('/dashboard?view=nodes', event.url.origin).toString()
      });
      if (!enrolment.connect_url) throw new Error('The connection session did not provide a link.');
      return {
        mutation: 'createEnrolment',
        enrolment: {
          id: enrolment.id,
          connectUrl: enrolment.connect_url,
          expiresAt: enrolment.expires_at
        }
      };
    } catch (error) {
      return fail(502, {
        mutation: 'createEnrolment',
        error: { message: presentDashboardError(error).message }
      });
    }
  },

  cancelJob: async (event) => {
    if (dashboardMode() !== 'live') {
      return fail(400, {
        mutation: 'cancelJob',
        error: { message: 'Job cancellation is disabled while demo data is active.' }
      });
    }
    const data = await event.request.formData();
    const jobId = String(data.get('job_id') ?? '').trim();
    if (!jobId) {
      return fail(400, {
        mutation: 'cancelJob',
        error: { message: 'A job identifier is required.' }
      });
    }
    try {
      const job = await dashboardSdk(event).jobs.cancel(jobId);
      return { mutation: 'cancelJob', cancelledJobId: job.id, state: job.state };
    } catch (error) {
      return fail(409, {
        mutation: 'cancelJob',
        error: { message: presentDashboardError(error).message }
      });
    }
  }
};
