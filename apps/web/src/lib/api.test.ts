import { describe, expect, it, vi } from 'vitest';
import { createLiveApi } from './api';

const uncertainJob = (id: string, since: string) => ({
  id,
  printer_id: 'prt_01',
  title: id,
  content_type: 'pdf',
  deliveries: 1,
  state: 'delivery_uncertain',
  created_at: '2026-08-25T10:00:00.000Z',
  expires_at: '2026-08-26T10:00:00.000Z',
  delivery_uncertain_since: since
});

describe('live dashboard overview', () => {
  it('reports PrintPacket ready only for the exact negotiated renderer contract', async () => {
    const capability = {
      renderer_abi: 'printpacket.pdf-renderer/v2',
      resource_abi: 'printpacket.resources/v1',
      persistent_cache: true,
      font_rendering: false,
      image_media_types: ['image/jpeg'],
      font_media_types: [],
      cached_resource_digests: [],
      print_packet: {
        negotiation_version: 2,
        supported_packet_versions: ['printpacket/v1'],
        feature_ids: [
          'media_paged', 'media_continuous', 'media_label', 'layout_flow',
          'layout_grid', 'layout_table', 'layout_regions', 'layout_keep_together',
          'data_expressions', 'data_repeat', 'image_jpeg', 'barcode_qr',
          'barcode_code128', 'typography_base14_windows1252'
        ],
        conformance_profiles: ['printpacket.conformance/core-v2'],
        output_profiles: [{
          id: 'printpacket.pdf-base14/v1',
          kind: 'pdf',
          media_type: 'application/pdf'
        }],
        deterministic: true,
        limits: {
          max_template_bytes: 1_048_576,
          max_input_bytes: 4_194_304,
          max_output_bytes: 52_428_800,
          max_pages: 1_000,
          max_resource_count: 100,
          max_resource_bytes: 4_194_304,
          max_total_resource_bytes: 12_582_912
        },
        resource_types: ['image/jpeg'],
        direct_offline: true,
        native_language_profiles: [],
        implementation_version: 'virtual-v1'
      }
    };
    const fetcher = vi.fn(async () => Response.json([
      {
        id: 'agt_ready', name: 'Ready node', platform: 'macos/arm64', state: 'connected',
        version: '0.1.0', last_seen_at: '2026-08-27T00:00:00Z', labels: [],
        document_render: capability
      },
      {
        id: 'agt_wrong_abi', name: 'Old renderer', platform: 'macos/arm64', state: 'connected',
        version: '0.1.0', last_seen_at: '2026-08-27T00:00:00Z', labels: [],
        document_render: { ...capability, renderer_abi: 'printpacket.pdf-renderer/v1' }
      },
      {
        id: 'agt_missing_feature', name: 'Missing feature', platform: 'macos/arm64', state: 'connected',
        version: '0.1.0', last_seen_at: '2026-08-27T00:00:00Z', labels: [],
        document_render: {
          ...capability,
          print_packet: { ...capability.print_packet, feature_ids: capability.print_packet.feature_ids.slice(1) }
        }
      },
      {
        id: 'agt_low_limit', name: 'Low limit', platform: 'macos/arm64', state: 'connected',
        version: '0.1.0', last_seen_at: '2026-08-27T00:00:00Z', labels: [],
        document_render: {
          ...capability,
          print_packet: {
            ...capability.print_packet,
            limits: { ...capability.print_packet.limits, max_resource_count: 99 }
          }
        }
      },
      {
        id: 'agt_no_jpeg', name: 'No JPEG', platform: 'macos/arm64', state: 'connected',
        version: '0.1.0', last_seen_at: '2026-08-27T00:00:00Z', labels: [],
        document_render: {
          ...capability,
          print_packet: { ...capability.print_packet, resource_types: [] }
        }
      }
    ]));

    await expect(
      createLiveApi(fetcher as typeof fetch, 'https://api.example.test').agents()
    ).resolves.toMatchObject({
      data: [
        { id: 'agt_ready', printPacket: { status: 'ready', reasons: [], directOffline: true } },
        { id: 'agt_wrong_abi', printPacket: { status: 'node_update_required', reasons: ['renderer_update_required'] } },
        { id: 'agt_missing_feature', printPacket: { status: 'node_update_required', reasons: ['semantic_features_missing'] } },
        { id: 'agt_low_limit', printPacket: { status: 'node_update_required', reasons: ['renderer_limits_insufficient'] } },
        { id: 'agt_no_jpeg', printPacket: { status: 'node_update_required', reasons: ['jpeg_resources_missing'] } }
      ]
    });
  });

  it('maps runtime availability and advisory refresh hints without upgrading them to remote wake', async () => {
    const fetcher = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = new URL(String(input));
      if (url.pathname === '/v1/nodes/runtime-observations') {
        return Response.json({
          data: [{
            node_id: 'agt_ipad',
            sequence: 9,
            host_mode: 'embedded_application',
            availability_class: 'background_opportunistic',
            lifecycle_state: 'background',
            accepts_cloud_jobs: false,
            execution_budget_ms: 24_000,
            wake_mechanisms: ['apns_background', 'bluetooth_accessory'],
            observed_at: '2026-08-27T00:00:00.000Z',
            expires_at: '2026-08-27T00:01:00.000Z',
            freshness: 'recent'
          }],
          next_cursor: null,
          has_more: false
        });
      }
      if (url.pathname === '/v1/nodes/agt_ipad/wake-hints' && init?.method === 'POST') {
        expect(new Headers(init.headers).get('idempotency-key')).toBe('refresh-1');
        expect(JSON.parse(String(init.body))).toEqual({
          reason: 'operator_request',
          expires_in_seconds: 300
        });
        return Response.json({
          id: 'wkh_01',
          node_id: 'agt_ipad',
          reason: 'operator_request',
          delivery_channel: 'connected_session',
          status: 'observed',
          requested_at: '2026-08-27T00:00:00.000Z',
          expires_at: '2026-08-27T00:05:00.000Z',
          observed_at: '2026-08-27T00:00:02.000Z'
        });
      }
      throw new Error(`Unexpected request: ${url}`);
    });
    const api = createLiveApi(fetcher as typeof fetch, 'https://api.example.test');

    await expect(api.nodeRuntimeObservations()).resolves.toMatchObject({
      data: [{
        nodeId: 'agt_ipad',
        hostMode: 'embedded_application',
        availabilityClass: 'background_opportunistic',
        acceptsCloudJobs: false,
        executionBudgetMs: 24_000
      }]
    });
    await expect(api.requestNodeRefresh('agt_ipad', 'refresh-1')).resolves.toMatchObject({
      deliveryChannel: 'connected_session',
      status: 'observed'
    });
  });

  it('treats an already absent node projection as a completed remove action', async () => {
    const fetcher = vi.fn(async () => Response.json(
      { error: { code: 'not_found', message: 'Node not found.' } },
      { status: 404 }
    ));
    await expect(
      createLiveApi(fetcher as typeof fetch, 'https://api.example.test').removeNode('agt_old')
    ).resolves.toEqual({ alreadyRemoved: true });
  });

  it('loads customer operations with immutable attribution and no tenant selector headers', async () => {
    const fetcher = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = new URL(String(input));
      expect(url.pathname).toBe('/v1/platform/operations');
      expect(url.searchParams.get('limit')).toBe('25');
      expect(url.searchParams.get('after')).toBe('c4beta');
      const headers = new Headers(init?.headers);
      expect(headers.get('x-piqae-dashboard')).toBe('1');
      expect(headers.has('x-piqae-managed-workspace-id')).toBe(false);
      return Response.json({
        data: [{
          customer: { id: 'wsp_child', external_id: 'c4beta', name: 'C4 Beta' },
          environment: { id: 'env_live', kind: 'live' },
          agents: [], printers: [], jobs: []
        }],
        next_cursor: 'next-shop',
        has_more: true
      });
    });

    await expect(
      createLiveApi(fetcher as typeof fetch, 'https://api.example.test', 'owner')
        .customerOperations('c4beta')
    ).resolves.toEqual({
      data: [{
        customer: { id: 'wsp_child', externalId: 'c4beta', name: 'C4 Beta' },
        environment: { id: 'env_live', kind: 'live' },
        agents: [], printers: [], jobs: [], destinations: [], routes: [], routeObservations: [], runtimeObservations: []
      }],
      nextCursor: 'next-shop',
      hasMore: true
    });
  });

  it('scopes managed customer resources without adding tenant selectors to platform requests', async () => {
    const fetcher = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = new URL(String(input));
      if (url.pathname.startsWith('/v1/platform/accounts/')) {
        const headers = new Headers(init?.headers);
        expect(headers.has('x-piqae-managed-workspace-id')).toBe(false);
        expect(headers.has('x-piqae-managed-environment-id')).toBe(false);
        return Response.json({
          id: 'wsp_child',
          external_id: 'shopify:gid://shopify/Shop/1',
          name: 'C4 Beta',
          status: 'active',
          metadata: {},
          environments: {
            test: { id: 'env_test' },
            live: { id: 'env_live' }
          },
          created_at: '2026-08-25T00:00:00.000Z',
          updated_at: '2026-08-25T00:00:00.000Z'
        });
      }
      if (url.pathname === '/v1/agents') {
        const headers = new Headers(init?.headers);
        expect(headers.get('x-piqae-dashboard')).toBe('1');
        expect(headers.get('x-piqae-managed-workspace-id')).toBe('wsp_child');
        expect(headers.get('x-piqae-managed-environment-id')).toBe('env_live');
        expect(headers.has('x-piqae-workspace-id')).toBe(false);
        expect(headers.has('x-piqae-environment-id')).toBe(false);
        return Response.json([]);
      }
      throw new Error(`Unexpected request: ${url}`);
    });
    const parent = createLiveApi(fetcher as typeof fetch, 'https://api.example.test', 'owner');
    const account = await parent.account('shopify:gid://shopify/Shop/1');
    expect(account).not.toBeNull();

    await expect(parent.managedWorkspace(account!).agents()).resolves.toEqual({
      data: [],
      nextCursor: null
    });
  });

  it('paginates the server-filtered uncertain set and maps its transition timestamp', async () => {
    const first = Array.from({ length: 100 }, (_, index) =>
      uncertainJob(`job_${index}`, '2026-08-25T09:00:00.000Z')
    );
    const fetcher = vi.fn(async (input: RequestInfo | URL) => {
      const url = new URL(String(input));
      if (url.pathname === '/v1/agents') return Response.json([]);
      if (url.pathname === '/v1/printers') {
        return Response.json({ data: [], next_cursor: null, has_more: false });
      }
      if (url.pathname === '/v1/jobs/job_detail') {
        return Response.json(uncertainJob('job_detail', '2026-08-25T08:00:00.000Z'));
      }
      if (url.pathname === '/v1/jobs' && url.searchParams.get('state') === 'delivery_uncertain') {
        return url.searchParams.get('after') === 'page_2'
          ? Response.json({
              data: [uncertainJob('job_100', '2026-08-25T07:00:00.000Z')],
              next_cursor: null,
              has_more: false
            })
          : Response.json({ data: first, next_cursor: 'page_2', has_more: true });
      }
      if (url.pathname === '/v1/jobs') {
        return Response.json({ data: [], next_cursor: null, has_more: false });
      }
      throw new Error(`Unexpected request: ${url}`);
    });
    const api = createLiveApi(fetcher as typeof fetch, 'https://api.example.test');

    await expect(api.overview()).resolves.toMatchObject({
      jobs: {
        uncertain: 101,
        oldestUncertainSince: '2026-08-25T07:00:00.000Z'
      }
    });
    await expect(api.job('job_detail')).resolves.toMatchObject({
      deliveryUncertainSince: '2026-08-25T08:00:00.000Z'
    });
    expect(
      fetcher.mock.calls.some(([input]) => new URL(String(input)).searchParams.get('after') === 'page_2')
    ).toBe(true);
    expect(
      fetcher.mock.calls.filter(
        ([input]) =>
          new URL(String(input)).pathname === '/v1/jobs' &&
          new URL(String(input)).searchParams.get('state') === 'delivery_uncertain'
      )
    ).toHaveLength(2);
  });

  it('fails closed when a paginated response claims another page without a cursor', async () => {
    const fetcher = vi.fn(async (input: RequestInfo | URL) => {
      const url = new URL(String(input));
      if (url.pathname === '/v1/agents') return Response.json([]);
      if (url.pathname === '/v1/printers' || url.pathname === '/v1/jobs') {
        const missingCursor =
          url.pathname === '/v1/jobs' &&
          url.searchParams.get('state') === 'delivery_uncertain';
        return Response.json({ data: [], next_cursor: null, has_more: missingCursor });
      }
      throw new Error(`Unexpected request: ${url}`);
    });

    await expect(
      createLiveApi(fetcher as typeof fetch, 'https://api.example.test').overview()
    ).rejects.toThrow('reported more results without a cursor');
  });

  it('bounds pagination even when every response supplies a unique cursor', async () => {
    let uncertainPage = 0;
    const fetcher = vi.fn(async (input: RequestInfo | URL) => {
      const url = new URL(String(input));
      if (url.pathname === '/v1/agents') return Response.json([]);
      if (url.pathname === '/v1/printers') {
        return Response.json({ data: [], next_cursor: null, has_more: false });
      }
      if (url.pathname === '/v1/jobs' && url.searchParams.get('state') === 'delivery_uncertain') {
        uncertainPage += 1;
        return Response.json({
          data: [],
          next_cursor: `page_${uncertainPage + 1}`,
          has_more: true
        });
      }
      if (url.pathname === '/v1/jobs') {
        return Response.json({ data: [], next_cursor: null, has_more: false });
      }
      throw new Error(`Unexpected request: ${url}`);
    });

    await expect(
      createLiveApi(fetcher as typeof fetch, 'https://api.example.test').overview()
    ).rejects.toThrow('exceeded its pagination bound');
    expect(uncertainPage).toBe(100);
  });
});
