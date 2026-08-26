import { fail } from "@sveltejs/kit";
import type { Actions, PageServerLoad, RequestEvent } from "./$types";
import {
  dashboardMode,
  dashboardSdk,
  dashboardSource,
  preventSecretCaching,
  presentDashboardError,
} from "$lib/server/dashboard-data";
import {
  isOperationalView,
  resolveStateFilter,
} from "$lib/dashboard-navigation";
import type { DashboardApi } from "$lib/api";
import type {
  DashboardAccount,
  DashboardAgent,
  DashboardCustomerOperations,
  DashboardDestination,
  DashboardJob,
  DashboardJobEvent,
  DashboardNodeDiagnostic,
  DashboardNodeRuntimeObservation,
  DashboardNodeWakeHint,
  DashboardOverview,
  DashboardPrinter,
  DashboardPrinterRoute,
  DashboardRouteObservation,
} from "$lib/view-types";

const MAX_DASHBOARD_PDF_BYTES = 50 * 1024 * 1024;

function encodeBase64(bytes: Uint8Array): string {
  let value = "";
  const chunkSize = 32_768;
  for (let offset = 0; offset < bytes.length; offset += chunkSize) {
    value += String.fromCharCode(...bytes.subarray(offset, offset + chunkSize));
  }
  return btoa(value);
}

const emptyOverview: DashboardOverview = {
  agents: { total: 0, online: 0, degraded: 0 },
  printers: { total: 0, online: 0, attention: 0 },
  jobs: { recent: 0, active: 0, failed: 0, uncertain: 0 },
};

export type OperationalDetail =
  | {
      kind: "job";
      job: DashboardJob;
      events: DashboardJobEvent[];
      printer: DashboardPrinter | null;
      agent: DashboardAgent | null;
    }
  | {
      kind: "destination";
      destination: DashboardDestination;
      routes: DashboardPrinterRoute[];
    }
  | {
      kind: "route";
      route: DashboardPrinterRoute;
      destination: DashboardDestination | null;
    }
  | {
      kind: "printer";
      printer: DashboardPrinter;
      agent: DashboardAgent | null;
      jobs: DashboardJob[];
    }
  | {
      kind: "node";
      node: DashboardAgent;
      printers: DashboardPrinter[];
      runtime: DashboardNodeRuntimeObservation | null;
      wakeHints: Promise<{
        hints: DashboardNodeWakeHint[];
        dataError: ReturnType<typeof presentDashboardError> | null;
      }>;
      diagnostics: Promise<{
        reports: DashboardNodeDiagnostic[];
        dataError: ReturnType<typeof presentDashboardError> | null;
      }>;
    }
  | { kind: "customer"; account: DashboardAccount }
  | { kind: "missing"; label: string };

type LoadedLists = {
  jobs: DashboardJob[];
  printers: DashboardPrinter[];
  agents: DashboardAgent[];
  accounts: DashboardAccount[];
  destinations: DashboardDestination[];
  routes: DashboardPrinterRoute[];
  routeObservations: DashboardRouteObservation[];
  runtimeObservations: DashboardNodeRuntimeObservation[];
};

type OperationsScope = "customers" | "own";

const RESOURCE_ID_PREFIX = /^(?:agt|ptr|job)_/;

function sameResourceId(
  left: string | null | undefined,
  right: string | null | undefined,
): boolean {
  if (!left || !right) return false;
  return (
    left === right ||
    left.replace(RESOURCE_ID_PREFIX, "") ===
      right.replace(RESOURCE_ID_PREFIX, "")
  );
}

function overviewFor(
  lists: Pick<LoadedLists, "jobs" | "printers" | "agents">,
): DashboardOverview {
  const uncertain = lists.jobs.filter(
    (job) => job.state === "delivery_uncertain" && !job.deliveryResolution,
  );
  return {
    agents: {
      total: lists.agents.length,
      online: lists.agents.filter((agent) => agent.state === "online").length,
      degraded: lists.agents.filter((agent) => agent.state === "degraded")
        .length,
    },
    printers: {
      total: lists.printers.length,
      online: lists.printers.filter((printer) => printer.state === "online")
        .length,
      attention: lists.printers.filter((printer) => printer.state !== "online")
        .length,
    },
    jobs: {
      recent: lists.jobs.length,
      active: lists.jobs.filter(
        (job) =>
          ![
            "completed_reported",
            "cancelled",
            "expired",
            "failed_terminal",
          ].includes(job.state),
      ).length,
      failed: lists.jobs.filter((job) => job.state.startsWith("failed")).length,
      uncertain: uncertain.length,
      oldestUncertainSince:
        uncertain
          .map((job) => job.deliveryUncertainSince)
          .filter((value): value is string => typeof value === "string")
          .sort()[0] ?? null,
    },
  };
}

async function loadCustomerOperations(api: DashboardApi) {
  const loaded: DashboardCustomerOperations[] = [];
  const seen = new Set<string>();
  let after: string | undefined;
  let complete = false;
  for (let page = 0; page < 100; page += 1) {
    const result = await api.customerOperations(after);
    loaded.push(...result.data);
    if (!result.hasMore) {
      complete = true;
      break;
    }
    if (!result.nextCursor || seen.has(result.nextCursor)) {
      throw new Error(
        "Piqae customer operations pagination returned an invalid cursor.",
      );
    }
    seen.add(result.nextCursor);
    after = result.nextCursor;
  }
  if (!complete)
    throw new Error("Piqae customer operations exceeded its pagination bound.");
  return {
    jobs: loaded.flatMap((entry) => entry.jobs),
    printers: loaded.flatMap((entry) => entry.printers),
    agents: loaded.flatMap((entry) => entry.agents),
    destinations: loaded.flatMap((entry) => entry.destinations),
    routes: loaded.flatMap((entry) => entry.routes),
    routeObservations: loaded.flatMap((entry) => entry.routeObservations),
    runtimeObservations: loaded.flatMap(
      (entry) => entry.runtimeObservations ?? [],
    ),
  };
}

async function loadRuntimeObservations(api: DashboardApi) {
  if (typeof api.nodeRuntimeObservations !== "function") {
    return { data: [] as DashboardNodeRuntimeObservation[], dataError: null };
  }
  try {
    const result = await api.nodeRuntimeObservations();
    return { data: result.data, dataError: null };
  } catch (error) {
    return {
      data: [] as DashboardNodeRuntimeObservation[],
      dataError: presentDashboardError(error),
    };
  }
}

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
    platform: { accounts: meta.platform.accounts && platformEnabled },
  };
  const requestedView = event.url.searchParams.get("view");
  const view = isOperationalView(requestedView, effectiveMeta)
    ? requestedView
    : "jobs";
  // The state narrowing is part of the address, not component state, so a link
  // such as ?view=jobs&state=delivery_uncertain lands on exactly those jobs.
  const stateFilter = resolveStateFilter(
    event.url.searchParams.get("state"),
    view,
  );

  try {
    const managedExternalId = event.url.searchParams.get("managed_customer");
    const managedAccount =
      managedExternalId && view !== "customers"
        ? await api.account(managedExternalId)
        : null;
    if (managedExternalId && view !== "customers" && !managedAccount) {
      throw new Error(
        "That managed customer is unavailable or is not owned by this workspace.",
      );
    }
    if (managedAccount && managedAccount.status !== "active") {
      throw new Error(
        "Archived or suspended managed customers cannot be operated.",
      );
    }
    const operationalApi = managedAccount
      ? api.managedWorkspace(managedAccount)
      : api;
    const accounts = effectiveMeta.platform.accounts
      ? await api.accounts()
      : { data: [] as DashboardAccount[], nextCursor: null };
    const requestedScope = event.url.searchParams.get("scope");
    const scope: OperationsScope =
      !managedAccount &&
      effectiveMeta.platform.accounts &&
      requestedScope !== "own"
        ? "customers"
        : "own";
    const [
      ownOverview,
      ownJobs,
      ownPrinters,
      ownAgents,
      ownDestinations,
      ownRoutes,
      ownRuntimes,
    ] = await Promise.all([
      operationalApi.overview(),
      operationalApi.jobs(),
      operationalApi.printers(),
      operationalApi.agents(),
      operationalApi.destinations(),
      operationalApi.routes(),
      loadRuntimeObservations(operationalApi),
    ]);
    const ownHasResources =
      ownAgents.data.length > 0 ||
      ownPrinters.data.length > 0 ||
      ownJobs.data.length > 0;
    const customerOperations =
      scope === "customers" && !managedAccount
        ? await loadCustomerOperations(api)
        : null;
    const jobs = customerOperations
      ? { data: customerOperations.jobs, nextCursor: null }
      : ownJobs;
    const printers = customerOperations
      ? { data: customerOperations.printers, nextCursor: null }
      : ownPrinters;
    const agents = customerOperations
      ? { data: customerOperations.agents, nextCursor: null }
      : ownAgents;
    const destinations = customerOperations
      ? { data: customerOperations.destinations, nextCursor: null }
      : ownDestinations;
    const routes = customerOperations
      ? { data: customerOperations.routes, nextCursor: null }
      : ownRoutes;

    const lists: LoadedLists = {
      jobs: jobs.data,
      printers: printers.data,
      agents: agents.data,
      accounts: accounts.data,
      destinations: destinations.data,
      routes: routes.data,
      routeObservations: customerOperations
        ? customerOperations.routeObservations
        : routes.data.flatMap((route) =>
            route.latestObservation ? [route.latestObservation] : [],
          ),
      runtimeObservations: customerOperations
        ? customerOperations.runtimeObservations
        : ownRuntimes.data,
    };

    const overview = customerOperations ? overviewFor(lists) : ownOverview;

    return {
      view,
      stateFilter,
      platformEnabled,
      managedAccount,
      scope,
      ownHasResources,
      overview,
      ...lists,
      runtimeDataError: customerOperations ? null : ownRuntimes.dataError,
      detail: await loadDetail(event, operationalApi, lists),
      dataError: null,
    };
  } catch (error) {
    return {
      view,
      stateFilter,
      platformEnabled,
      managedAccount: null,
      scope: "own" as OperationsScope,
      ownHasResources: false,
      overview: emptyOverview,
      jobs: [],
      printers: [],
      agents: [],
      accounts: [],
      destinations: [],
      routes: [],
      routeObservations: [],
      runtimeObservations: [],
      runtimeDataError: null,
      detail: null,
      dataError: presentDashboardError(error),
    };
  }
};

async function managedSelection(event: RequestEvent, data: FormData) {
  const externalId = String(data.get("managed_customer") ?? "").trim();
  if (!externalId) return null;
  const api = dashboardSource(event).api;
  const account = await api.account(externalId);
  if (!account || account.status !== "active") {
    throw new Error("That managed customer is unavailable or is not active.");
  }
  return {
    api: api.managedWorkspace(account),
    sdk: dashboardSdk(event, {
      workspaceId: account.id,
      environmentId: account.environments.liveId,
    }),
    account,
  };
}

/**
 * Detail lives in a drawer on this page rather than on its own route, so the
 * selected entity comes from the query string and is resolved alongside the
 * list. Only the job drawer needs a further round trip — its event timeline is
 * not part of any list payload.
 */
async function loadDetail(
  event: RequestEvent,
  api: DashboardApi,
  loaded: LoadedLists,
): Promise<OperationalDetail | null> {
  const params = event.url.searchParams;

  const jobId = params.get("job");
  if (jobId) {
    const [job, events] = await Promise.all([
      api.job(jobId),
      api.jobEvents(jobId),
    ]);
    if (!job) return { kind: "missing", label: "job" };
    return {
      kind: "job",
      job,
      events: events.data,
      printer:
        loaded.printers.find((printer) =>
          sameResourceId(printer.id, job.printerId),
        ) ?? null,
      agent:
        loaded.agents.find((agent) => sameResourceId(agent.id, job.agentId)) ??
        null,
    };
  }

  const printerId = params.get("printer");
  if (printerId) {
    const printer = loaded.printers.find((candidate) =>
      sameResourceId(candidate.id, printerId),
    );
    if (!printer) return { kind: "missing", label: "printer" };
    return {
      kind: "printer",
      printer,
      agent:
        loaded.agents.find((agent) =>
          sameResourceId(agent.id, printer.agentId),
        ) ?? null,
      jobs: loaded.jobs.filter((job) =>
        sameResourceId(job.printerId, printer.id),
      ),
    };
  }

  const destinationId = params.get("destination");
  if (destinationId) {
    const destination = loaded.destinations.find(
      (candidate) => candidate.id === destinationId,
    );
    if (!destination) return { kind: "missing", label: "destination" };
    return {
      kind: "destination",
      destination,
      routes: loaded.routes.filter(
        (route) => route.physicalDestinationId === destination.id,
      ),
    };
  }

  const routeId = params.get("route");
  if (routeId) {
    const route = loaded.routes.find((candidate) => candidate.id === routeId);
    if (!route) return { kind: "missing", label: "route" };
    return {
      kind: "route",
      route,
      destination:
        loaded.destinations.find(
          (candidate) => candidate.id === route.physicalDestinationId,
        ) ?? null,
    };
  }

  const nodeId = params.get("node");
  if (nodeId) {
    const node = loaded.agents.find((candidate) =>
      sameResourceId(candidate.id, nodeId),
    );
    if (!node) return { kind: "missing", label: "node" };
    return {
      kind: "node",
      node,
      printers: loaded.printers.filter((printer) =>
        sameResourceId(printer.agentId, node.id),
      ),
      runtime:
        loaded.runtimeObservations.find((runtime) =>
          sameResourceId(runtime.nodeId, node.id),
        ) ?? null,
      wakeHints: nodeWakeHints(api, node.id),
      // Streamed: a diagnostics outage must never block the node drawer.
      diagnostics: nodeDiagnostics(api, node.id),
    };
  }

  const customerId = params.get("customer");
  if (customerId) {
    const account = loaded.accounts.find(
      (candidate) => candidate.externalId === customerId,
    );
    if (!account) return { kind: "missing", label: "customer" };
    return { kind: "customer", account };
  }

  return null;
}

async function nodeDiagnostics(api: DashboardApi, nodeId: string) {
  try {
    return { reports: await api.nodeDiagnostics(nodeId), dataError: null };
  } catch (error) {
    return { reports: [], dataError: presentDashboardError(error) };
  }
}

async function nodeWakeHints(api: DashboardApi, nodeId: string) {
  try {
    return { hints: await api.nodeWakeHints(nodeId), dataError: null };
  } catch (error) {
    return { hints: [], dataError: presentDashboardError(error) };
  }
}

export const actions: Actions = {
  removeNode: async (event) => {
    if (dashboardMode() !== "live") {
      return fail(400, {
        mutation: "removeNode",
        error: {
          message: "Node removal is disabled while demo data is active.",
        },
      });
    }
    const data = await event.request.formData();
    const nodeId = String(data.get("node_id") ?? "").trim();
    const confirmation = String(data.get("confirmation") ?? "").trim();
    if (!nodeId || !confirmation) {
      return fail(400, {
        mutation: "removeNode",
        error: { message: "Type the node name to confirm removal." },
      });
    }
    try {
      const managed = await managedSelection(event, data);
      const scopedApi = managed?.api ?? dashboardSource(event).api;
      const nodes = await scopedApi.agents();
      const node = nodes.data.find((candidate) =>
        sameResourceId(candidate.id, nodeId),
      );
      if (!node) {
        return fail(404, {
          mutation: "removeNode",
          error: {
            message: "That node is not present in the selected workspace.",
          },
        });
      }
      if (confirmation !== node.name) {
        return fail(400, {
          mutation: "removeNode",
          error: { message: `Type “${node.name}” exactly to confirm removal.` },
        });
      }
      const result = await scopedApi.removeNode(nodeId);
      return {
        mutation: "removeNode",
        removedNodeId: nodeId,
        alreadyRemoved: result.alreadyRemoved,
      };
    } catch (error) {
      return fail(502, {
        mutation: "removeNode",
        error: { message: presentDashboardError(error).message },
      });
    }
  },

  requestNodeRefresh: async (event) => {
    if (dashboardMode() !== "live") {
      return fail(400, {
        mutation: "requestNodeRefresh",
        error: {
          message:
            "Node refresh requests are disabled while demo data is active.",
        },
      });
    }
    const data = await event.request.formData();
    const nodeId = String(data.get("node_id") ?? "").trim();
    if (!nodeId) {
      return fail(400, {
        mutation: "requestNodeRefresh",
        error: { message: "Select a node before requesting a refresh." },
      });
    }
    try {
      const managed = await managedSelection(event, data);
      const hint = await (
        managed?.api ?? dashboardSource(event).api
      ).requestNodeRefresh(nodeId, `dashboard-refresh-${crypto.randomUUID()}`);
      return {
        mutation: "requestNodeRefresh",
        nodeRefreshHint: hint,
      };
    } catch (error) {
      return fail(502, {
        mutation: "requestNodeRefresh",
        error: { message: presentDashboardError(error).message },
      });
    }
  },

  collectNodeDiagnostics: async (event) => {
    if (dashboardMode() !== "live") {
      return fail(400, {
        mutation: "collectNodeDiagnostics",
        error: {
          message: "Diagnostics are disabled while demo data is active.",
        },
      });
    }
    const data = await event.request.formData();
    const nodeId = String(data.get("node_id") ?? "").trim();
    if (!nodeId) {
      return fail(400, {
        mutation: "collectNodeDiagnostics",
        error: { message: "Select a node before collecting diagnostics." },
      });
    }
    try {
      const managed = await managedSelection(event, data);
      const { requestId } = await (
        managed?.api ?? dashboardSource(event).api
      ).collectNodeDiagnostics(nodeId);
      return {
        mutation: "collectNodeDiagnostics",
        diagnosticRequestId: requestId,
      };
    } catch (error) {
      return fail(502, {
        mutation: "collectNodeDiagnostics",
        error: { message: presentDashboardError(error).message },
      });
    }
  },

  createPrintJob: async (event) => {
    preventSecretCaching(event);
    if (dashboardMode() !== "live") {
      return fail(400, {
        mutation: "createPrintJob",
        error: { message: "Printing is disabled while demo data is active." },
      });
    }

    const data = await event.request.formData();
    const printerId = String(data.get("printer_id") ?? "").trim();
    const profileId = String(data.get("profile_id") ?? "").trim();
    const title = String(data.get("title") ?? "").trim();
    const copies = Number(data.get("copies") ?? 1);
    const document = data.get("document");

    if (!printerId || !profileId) {
      return fail(400, {
        mutation: "createPrintJob",
        error: { message: "Choose an available printer and print profile." },
      });
    }
    if (!(document instanceof File) || document.size === 0) {
      return fail(400, {
        mutation: "createPrintJob",
        error: { message: "Choose a PDF document to print." },
      });
    }
    if (document.size > MAX_DASHBOARD_PDF_BYTES) {
      return fail(413, {
        mutation: "createPrintJob",
        error: { message: "PDF documents must be 50 MiB or smaller." },
      });
    }
    if (!Number.isInteger(copies) || copies < 1 || copies > 100) {
      return fail(400, {
        mutation: "createPrintJob",
        error: { message: "Copies must be a whole number between 1 and 100." },
      });
    }

    try {
      const managed = await managedSelection(event, data);
      const client = managed?.sdk ?? dashboardSdk(event);
      const printers = await client.printers.list({ limit: 100 });
      const printer = printers.data.find(
        (candidate) => candidate.id === printerId,
      );
      const profile = printer?.profiles.find(
        (candidate) =>
          candidate.profile_id === profileId && candidate.status === "ready",
      );
      if (!printer || printer.state !== "online") {
        return fail(409, {
          mutation: "createPrintJob",
          error: {
            message:
              "That printer is no longer online. Refresh and choose another printer.",
          },
        });
      }
      if (!profile) {
        return fail(409, {
          mutation: "createPrintJob",
          error: {
            message:
              "That print profile is no longer ready. Refresh and choose another profile.",
          },
        });
      }

      const bytes = new Uint8Array(await document.arrayBuffer());
      if (
        bytes.length < 5 ||
        bytes[0] !== 0x25 ||
        bytes[1] !== 0x50 ||
        bytes[2] !== 0x44 ||
        bytes[3] !== 0x46 ||
        bytes[4] !== 0x2d
      ) {
        return fail(415, {
          mutation: "createPrintJob",
          error: { message: "The selected file is not a valid PDF document." },
        });
      }

      const job = await client.jobs.create(
        {
          printer_id: printerId,
          title:
            title || document.name.replace(/\.pdf$/i, "") || "PDF document",
          source: "piqae-dashboard",
          content_type: "pdf",
          content: { type: "base64", data: encodeBase64(bytes) },
          options: { ...profile.options, copies },
          deliveries: 1,
          expire_after_seconds: 3600,
          metadata: {
            profile_id: profile.profile_id,
            profile_revision: String(profile.revision),
          },
        },
        `dashboard-${crypto.randomUUID()}`,
      );
      return {
        mutation: "createPrintJob",
        createdJobId: job.id,
        state: job.state,
      };
    } catch (error) {
      return fail(502, {
        mutation: "createPrintJob",
        error: { message: presentDashboardError(error).message },
      });
    }
  },

  createEnrolment: async (event) => {
    preventSecretCaching(event);
    if (dashboardMode() !== "live") {
      return fail(400, {
        mutation: "createEnrolment",
        error: {
          message: "Node enrolment is disabled while demo data is active.",
        },
      });
    }
    const data = await event.request.formData();
    const name = String(data.get("name") ?? "").trim();
    const expiresInSeconds = Number(data.get("expires_in_seconds") ?? 600);
    if (name && (name.length < 2 || name.length > 120)) {
      return fail(400, {
        mutation: "createEnrolment",
        error: {
          message: "A custom node name must be between 2 and 120 characters.",
        },
      });
    }
    if (
      !Number.isInteger(expiresInSeconds) ||
      expiresInSeconds < 60 ||
      expiresInSeconds > 900
    ) {
      return fail(400, {
        mutation: "createEnrolment",
        error: { message: "Expiry must be between 60 and 900 seconds." },
      });
    }
    try {
      const managed = await managedSelection(event, data);
      const enrolment = await (
        managed?.sdk ?? dashboardSdk(event)
      ).connectSessions.create({
        ...(name ? { name } : {}),
        expires_in_seconds: expiresInSeconds,
        return_url: new URL(
          managed
            ? `/dashboard?view=nodes&managed_customer=${encodeURIComponent(managed.account.externalId)}`
            : "/dashboard?view=nodes",
          event.url.origin,
        ).toString(),
      });
      if (!enrolment.connect_url)
        throw new Error("The connection session did not provide a link.");
      return {
        mutation: "createEnrolment",
        enrolment: {
          id: enrolment.id,
          connectUrl: enrolment.connect_url,
          expiresAt: enrolment.expires_at,
        },
      };
    } catch (error) {
      return fail(502, {
        mutation: "createEnrolment",
        error: { message: presentDashboardError(error).message },
      });
    }
  },

  resolveUncertainJob: async (event) => {
    if (dashboardMode() !== "live") {
      return fail(400, {
        mutation: "resolveUncertainJob",
        error: {
          message:
            "Uncertain delivery resolution is disabled while demo data is active.",
        },
      });
    }
    const data = await event.request.formData();
    const jobId = String(data.get("job_id") ?? "").trim();
    const resolution = String(data.get("resolution") ?? "").trim();
    const note = String(data.get("note") ?? "").trim();
    const requestId = String(data.get("request_id") ?? "").trim();
    if (
      !jobId ||
      !requestId ||
      note.length < 1 ||
      note.length > 2_000 ||
      ![
        "acknowledge_printed",
        "acknowledge_missing",
        "cancelled",
        "reprint",
      ].includes(resolution)
    ) {
      return fail(400, {
        mutation: "resolveUncertainJob",
        error: { message: "Choose a resolution and include an operator note." },
      });
    }
    try {
      const managed = await managedSelection(event, data);
      const result = await (
        managed?.api ?? dashboardSource(event).api
      ).resolveUncertainJob(
        jobId,
        resolution as
          | "acknowledge_printed"
          | "acknowledge_missing"
          | "cancelled"
          | "reprint",
        note,
        requestId,
      );
      return {
        mutation: "resolveUncertainJob",
        resolvedJobId: jobId,
        resolutionState: result.state,
        replacementJobId: result.replacementJobId,
      };
    } catch (error) {
      return fail(409, {
        mutation: "resolveUncertainJob",
        error: { message: presentDashboardError(error).message },
      });
    }
  },

  cancelJob: async (event) => {
    if (dashboardMode() !== "live") {
      return fail(400, {
        mutation: "cancelJob",
        error: {
          message: "Job cancellation is disabled while demo data is active.",
        },
      });
    }
    const data = await event.request.formData();
    const jobId = String(data.get("job_id") ?? "").trim();
    if (!jobId) {
      return fail(400, {
        mutation: "cancelJob",
        error: { message: "A job identifier is required." },
      });
    }
    try {
      const managed = await managedSelection(event, data);
      const job = await (managed?.sdk ?? dashboardSdk(event)).jobs.cancel(
        jobId,
      );
      return {
        mutation: "cancelJob",
        cancelledJobId: job.id,
        state: job.state,
      };
    } catch (error) {
      return fail(409, {
        mutation: "cancelJob",
        error: { message: presentDashboardError(error).message },
      });
    }
  },
};
