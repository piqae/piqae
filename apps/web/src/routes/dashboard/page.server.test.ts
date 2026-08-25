import { beforeEach, describe, expect, it, vi } from 'vitest';

const { createJob, dashboardSource, listPrinters } = vi.hoisted(() => ({
  createJob: vi.fn(),
  dashboardSource: vi.fn(),
  listPrinters: vi.fn()
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
      accounts: async () => emptyPage
    }
  });
  return {
    url: new URL(`https://piqae.test/dashboard${search}`),
    parent: async () => ({ meta: { platform: { accounts: false } } })
  } as never;
}

describe('dashboard state addressing', () => {
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
    expect(await load(loadEvent(''))).toMatchObject({ view: 'jobs', stateFilter: 'all' });
  });
});
