import { beforeEach, describe, expect, it, vi } from 'vitest';

const { createJob, dashboardSource, listPrinters, resolveUncertain, requestNodeRefresh, removeNode } = vi.hoisted(() => ({
  createJob: vi.fn(),
  dashboardSource: vi.fn(),
  listPrinters: vi.fn(),
  resolveUncertain: vi.fn(),
  requestNodeRefresh: vi.fn(),
  removeNode: vi.fn()
}));

vi.mock('$lib/server/dashboard-data', () => ({
  dashboardMode: () => 'live',
  dashboardSdk: () => ({
    jobs: { create: createJob },
    printers: { list: listPrinters }
  }),
  dashboardSource,
  preventSecretCaching: vi.fn(),
  presentDashboardError: (error: unknown) => ({
    message: error instanceof Error ? error.message : 'Request failed.'
  })
}));

import { actions, load } from './+page.server';

const createPrintJob = actions.createPrintJob!;
const resolveUncertainJob = actions.resolveUncertainJob!;
const requestNodeRefreshAction = actions.requestNodeRefresh!;
const removeNodeAction = actions.removeNode!;

function event(form: FormData) {
  return {
    request: { formData: async () => form },
    url: new URL('https://piqae.test/dashboard'),
    locals: {}
  } as never;
}

function validForm(content = '%PDF-1.7\nfixture') {
  const form = new FormData();
  const bytes = new TextEncoder().encode(content);
  const document = new File([bytes], 'packing-slip.pdf', { type: 'application/pdf' });
  Object.defineProperty(document, 'arrayBuffer', {
    value: async () => bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength)
  });
  form.set('printer_id', 'prt_01');
  form.set('profile_id', 'prof_a4');
  form.set('title', 'Packing slip');
  form.set('copies', '2');
  form.set('document', document);
  return form;
}

describe('dashboard PDF printing', () => {
  beforeEach(() => {
    createJob.mockReset();
    listPrinters.mockReset();
    listPrinters.mockResolvedValue({
      data: [
        {
          id: 'prt_01',
          state: 'online',
          profiles: [
            {
              profile_id: 'prof_a4',
              revision: 7,
              status: 'ready',
              options: { paper: 'A4', copies: 1, native_options: { InputSlot: 'Tray2' } }
            }
          ]
        }
      ]
    });
    createJob.mockResolvedValue({ id: 'job_01', state: 'queued' });
  });

  it('resolves the profile server-side and registers the selected PDF', async () => {
    const result = await createPrintJob(event(validForm()));

    expect(result).toMatchObject({ mutation: 'createPrintJob', createdJobId: 'job_01' });
    expect(createJob).toHaveBeenCalledWith(
      expect.objectContaining({
        printer_id: 'prt_01',
        title: 'Packing slip',
        content_type: 'pdf',
        content: { type: 'base64', data: expect.any(String) },
        options: {
          paper: 'A4',
          copies: 2,
          native_options: { InputSlot: 'Tray2' }
        },
        metadata: { profile_id: 'prof_a4', profile_revision: '7' }
      }),
      expect.stringMatching(/^dashboard-/)
    );
  });

  it('rejects content that merely has a PDF filename', async () => {
    const result = await createPrintJob(event(validForm('not a PDF')));

    expect(result).toMatchObject({ status: 415 });
    expect(createJob).not.toHaveBeenCalled();
  });

  it('rejects a profile that is no longer ready', async () => {
    listPrinters.mockResolvedValue({
      data: [{ id: 'prt_01', state: 'online', profiles: [{ profile_id: 'prof_a4', status: 'stale' }] }]
    });

    const result = await createPrintJob(event(validForm()));

    expect(result).toMatchObject({ status: 409 });
    expect(createJob).not.toHaveBeenCalled();
  });
});

describe('uncertain delivery resolution', () => {
  it('passes the exact idempotency request through and reports pending node acknowledgement', async () => {
    resolveUncertain.mockResolvedValue({ state: 'pending_node_ack', replacementJobId: null });
    dashboardSource.mockReturnValue({
      api: { resolveUncertainJob: resolveUncertain }
    });
    const form = new FormData();
    form.set('job_id', 'job_01J00000000000000000000000');
    form.set('resolution', 'acknowledge_missing');
    form.set('note', 'Checked the output tray and accepted the missing document.');
    form.set('request_id', 'resolve-browser-request-1');

    const result = await resolveUncertainJob(event(form));

    expect(result).toMatchObject({
      mutation: 'resolveUncertainJob',
      resolutionState: 'pending_node_ack'
    });
    expect(resolveUncertain).toHaveBeenCalledWith(
      'job_01J00000000000000000000000',
      'acknowledge_missing',
      'Checked the output tray and accepted the missing document.',
      'resolve-browser-request-1'
    );
  });
});

describe('scoped node operations', () => {
  const account = {
    id: 'wsp_child',
    externalId: 'shopify:store-1',
    name: 'Store one',
    status: 'active' as const,
    metadata: {},
    environments: { testId: 'env_test', liveId: 'env_live' },
    createdAt: '2026-08-27T00:00:00.000Z',
    updatedAt: '2026-08-27T00:00:00.000Z'
  };

  beforeEach(() => {
    requestNodeRefresh.mockReset();
    removeNode.mockReset();
  });

  it('requests an advisory refresh through the resolved managed tenant', async () => {
    requestNodeRefresh.mockResolvedValue({
      id: 'wkh_01',
      nodeId: 'agt_01',
      reason: 'operator_request',
      deliveryChannel: null,
      status: 'pending',
      requestedAt: '2026-08-27T00:00:00.000Z',
      expiresAt: '2026-08-27T00:05:00.000Z',
      observedAt: null
    });
    const managedApi = { requestNodeRefresh };
    dashboardSource.mockReturnValue({
      api: {
        account: vi.fn(async () => account),
        managedWorkspace: vi.fn(() => managedApi)
      }
    });
    const form = new FormData();
    form.set('managed_customer', account.externalId);
    form.set('node_id', 'agt_01');

    const result = await requestNodeRefreshAction(event(form));

    expect(result).toMatchObject({ mutation: 'requestNodeRefresh', nodeRefreshHint: { status: 'pending' } });
    expect(requestNodeRefresh).toHaveBeenCalledWith('agt_01', expect.stringMatching(/^dashboard-refresh-/));
  });

  it('requires the scoped node name before removing a managed projection', async () => {
    removeNode.mockResolvedValue({ alreadyRemoved: false });
    const managedApi = {
      agents: vi.fn(async () => ({ data: [{ id: 'agt_01', name: 'Kitchen iPad' }], nextCursor: null })),
      removeNode
    };
    dashboardSource.mockReturnValue({
      api: {
        account: vi.fn(async () => account),
        managedWorkspace: vi.fn(() => managedApi)
      }
    });
    const wrong = new FormData();
    wrong.set('managed_customer', account.externalId);
    wrong.set('node_id', 'agt_01');
    wrong.set('expected_node_name', 'Kitchen iPad');
    wrong.set('confirmation', 'Kitchen Mac');
    await expect(removeNodeAction(event(wrong))).resolves.toMatchObject({ status: 400 });
    expect(removeNode).not.toHaveBeenCalled();

    const confirmed = new FormData();
    confirmed.set('managed_customer', account.externalId);
    confirmed.set('node_id', 'agt_01');
    confirmed.set('expected_node_name', 'Caller supplied name is not authoritative');
    confirmed.set('confirmation', 'Kitchen iPad');
    await expect(removeNodeAction(event(confirmed))).resolves.toMatchObject({
      mutation: 'removeNode',
      removedNodeId: 'agt_01'
    });
    expect(removeNode).toHaveBeenCalledWith('agt_01');
  });

  it('does not remove a node absent from the server-resolved managed workspace', async () => {
    const managedApi = {
      agents: vi.fn(async () => ({ data: [], nextCursor: null })),
      removeNode
    };
    dashboardSource.mockReturnValue({
      api: {
        account: vi.fn(async () => account),
        managedWorkspace: vi.fn(() => managedApi)
      }
    });
    const form = new FormData();
    form.set('managed_customer', account.externalId);
    form.set('node_id', 'agt_missing');
    form.set('expected_node_name', 'Kitchen iPad');
    form.set('confirmation', 'Kitchen iPad');

    await expect(removeNodeAction(event(form))).resolves.toMatchObject({ status: 404 });
    expect(removeNode).not.toHaveBeenCalled();
  });
});

const emptyPage = { data: [], nextCursor: null };

function loadEvent(search: string) {
  dashboardSource.mockReturnValue({
    api: {
      platformEnabled: async () => false,
      overview: async () => ({
        agents: { total: 0, online: 0, degraded: 0 },
        printers: { total: 0, online: 0, attention: 0 },
        jobs: { recent: 0, active: 0, failed: 0, uncertain: 0 }
      }),
      jobs: async () => emptyPage,
      printers: async () => emptyPage,
      agents: async () => emptyPage,
      destinations: async () => emptyPage,
      routes: async () => emptyPage,
      accounts: async () => emptyPage
    }
  });
  return {
    url: new URL(`https://piqae.test/dashboard${search}`),
    parent: async () => ({ meta: { platform: { accounts: false } } })
  } as never;
}

describe('dashboard state addressing', () => {
  it('keeps node operations visible when only runtime telemetry is unavailable', async () => {
    dashboardSource.mockReturnValue({
      api: {
        platformEnabled: async () => false,
        overview: async () => ({
          agents: { total: 1, online: 1, degraded: 0 },
          printers: { total: 0, online: 0, attention: 0 },
          jobs: { recent: 0, active: 0, failed: 0, uncertain: 0 }
        }),
        jobs: async () => emptyPage,
        printers: async () => emptyPage,
        agents: async () => ({ data: [{ id: 'agt_01', name: 'Warehouse Mac' }], nextCursor: null }),
        destinations: async () => emptyPage,
        routes: async () => emptyPage,
        accounts: async () => emptyPage,
        nodeRuntimeObservations: async () => { throw new Error('runtime projection unavailable'); }
      }
    });
    const result = await load({
      url: new URL('https://piqae.test/dashboard?view=nodes'),
      parent: async () => ({ meta: { platform: { accounts: false } } })
    } as never);

    expect(result).toMatchObject({
      dataError: null,
      agents: [{ id: 'agt_01' }],
      runtimeObservations: [],
      runtimeDataError: { message: 'runtime projection unavailable' }
    });
  });

  it('carries an uncertain-delivery filter from the URL into the view model', async () => {
    const data = await load(loadEvent('?view=jobs&state=delivery_uncertain'));

    expect(data).toMatchObject({ view: 'jobs', stateFilter: 'delivery_uncertain' });
  });

  it('widens a state that does not apply to the requested view', async () => {
    expect(await load(loadEvent('?view=printers&state=delivery_uncertain'))).toMatchObject({
      view: 'printers',
      stateFilter: 'all'
    });
    expect(await load(loadEvent('?view=jobs&state=nonsense'))).toMatchObject({
      stateFilter: 'all'
    });
    expect(await load(loadEvent(''))).toMatchObject({
      view: 'jobs',
      stateFilter: 'all',
      managedAccount: null,
      dataError: null
    });
  });
});

describe('managed customer selection', () => {
  it('defaults platform operators to tenant-attributed customer operations', async () => {
    const customerOperations = vi.fn(async () => ({
      data: [{
        customer: { id: 'wsp_child', externalId: 'c4beta', name: 'C4 Beta' },
        environment: { id: 'env_live', kind: 'live' },
        agents: [{ ...({ id: 'agt_child', name: 'Shop Mac', state: 'online' }), customer: { id: 'wsp_child', externalId: 'c4beta', name: 'C4 Beta' } }],
        printers: [], jobs: [], destinations: [], routes: [], routeObservations: []
      }],
      nextCursor: null,
      hasMore: false
    }));
    dashboardSource.mockReturnValue({
      api: {
        platformEnabled: async () => true,
        accounts: async () => ({ data: [], nextCursor: null }),
        customerOperations,
        overview: async () => ({
          agents: { total: 0, online: 0, degraded: 0 },
          printers: { total: 0, online: 0, attention: 0 },
          jobs: { recent: 0, active: 0, failed: 0, uncertain: 0 }
        }),
        jobs: async () => emptyPage,
        printers: async () => emptyPage,
        agents: async () => emptyPage,
        destinations: async () => emptyPage,
        routes: async () => emptyPage
      }
    });

    const data = await load({
      url: new URL('https://piqae.test/dashboard?view=nodes'),
      parent: async () => ({ meta: { platform: { accounts: true } } })
    } as never);

    expect(customerOperations).toHaveBeenCalledWith(undefined);
    expect(data).toMatchObject({
      scope: 'customers',
      ownHasResources: false,
      overview: { agents: { total: 1, online: 1 } },
      agents: [{ id: 'agt_child', customer: { externalId: 'c4beta' } }]
    });
  });

  it('resolves an owned account before using its isolated operational client', async () => {
    const childAgents = vi.fn(async () => ({
      data: [{ id: 'agt_child', name: 'Shop Mac' }],
      nextCursor: null
    }));
    const account = {
      id: 'wsp_child',
      externalId: 'shopify:gid://shopify/Shop/1',
      name: 'C4 Beta',
      status: 'active',
      metadata: {},
      environments: { testId: 'env_test', liveId: 'env_live' },
      createdAt: '2026-08-25T00:00:00.000Z',
      updatedAt: '2026-08-25T00:00:00.000Z'
    };
    const childApi = {
      overview: async () => ({
        agents: { total: 1, online: 1, degraded: 0 },
        printers: { total: 0, online: 0, attention: 0 },
        jobs: { recent: 0, active: 0, failed: 0, uncertain: 0 }
      }),
      jobs: async () => emptyPage,
      printers: async () => emptyPage,
      agents: childAgents,
      destinations: async () => emptyPage,
      routes: async () => emptyPage
    };
    const managedWorkspace = vi.fn(() => childApi);
    dashboardSource.mockReturnValue({
      api: {
        platformEnabled: async () => true,
        account: async (externalId: string) =>
          externalId === account.externalId ? account : null,
        managedWorkspace,
        accounts: async () => ({ data: [account], nextCursor: null })
      }
    });

    const data = await load({
      url: new URL(
        `https://piqae.test/dashboard?view=nodes&managed_customer=${encodeURIComponent(account.externalId)}`
      ),
      parent: async () => ({ meta: { platform: { accounts: true } } })
    } as never);

    expect(managedWorkspace).toHaveBeenCalledWith(account);
    expect(childAgents).toHaveBeenCalledOnce();
    expect(data).toMatchObject({
      managedAccount: account,
      agents: [{ id: 'agt_child', name: 'Shop Mac' }]
    });
  });

  it('resolves a canonical aggregate node link against the managed customer raw ID', async () => {
    const account = {
      id: 'wsp_child',
      externalId: 'c4beta',
      name: 'C4 Beta',
      status: 'active',
      metadata: {},
      environments: { testId: 'env_test', liveId: 'env_live' },
      createdAt: '2026-08-25T00:00:00.000Z',
      updatedAt: '2026-08-25T00:00:00.000Z'
    };
    const rawId = '01M0EG7NMZ58KZQGNC7Y5GR0SV';
    const childApi = {
      overview: async () => ({
        agents: { total: 1, online: 1, degraded: 0 },
        printers: { total: 0, online: 0, attention: 0 },
        jobs: { recent: 0, active: 0, failed: 0, uncertain: 0 }
      }),
      jobs: async () => emptyPage,
      printers: async () => emptyPage,
      agents: async () => ({ data: [{ id: rawId, name: 'Piqae node' }], nextCursor: null }),
      destinations: async () => emptyPage,
      routes: async () => emptyPage,
      nodeDiagnostics: async () => []
    };
    dashboardSource.mockReturnValue({
      api: {
        platformEnabled: async () => true,
        account: async () => account,
        managedWorkspace: () => childApi,
        accounts: async () => ({ data: [account], nextCursor: null })
      }
    });

    const data = await load({
      url: new URL(
        `https://piqae.test/dashboard?view=nodes&managed_customer=c4beta&node=agt_${rawId}`
      ),
      parent: async () => ({ meta: { platform: { accounts: true } } })
    } as never);

    expect(data).toMatchObject({
      detail: {
        kind: 'node',
        node: { id: rawId, name: 'Piqae node' }
      }
    });
  });

  it('fails closed before issuing child resource requests for an unowned customer', async () => {
    const managedWorkspace = vi.fn();
    const parentAgents = vi.fn();
    dashboardSource.mockReturnValue({
      api: {
        platformEnabled: async () => true,
        account: async () => null,
        managedWorkspace,
        agents: parentAgents
      }
    });

    const data = await load({
      url: new URL('https://piqae.test/dashboard?view=nodes&managed_customer=foreign'),
      parent: async () => ({ meta: { platform: { accounts: true } } })
    } as never);

    expect(managedWorkspace).not.toHaveBeenCalled();
    expect(parentAgents).not.toHaveBeenCalled();
    expect(data).toMatchObject({
      managedAccount: null,
      agents: [],
      dataError: { message: expect.stringMatching(/unavailable or is not owned/i) }
    });
  });
});
