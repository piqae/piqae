import type {
  Agent,
  AgentEnrolment,
  ApiKey,
  BootstrappedLocalOwner,
  CreateApiKey,
  CreateDeviceAuthorization,
  CreateJob,
  CreatedDeviceAuthorization,
  DeploymentMeta,
  DeviceAuthorizationExchange,
  DeviceAuthorizationReview,
  DeviceAuthorizationStatus,
  CreatedApiKey,
  CurrentIdentity,
  ErrorEnvelope,
  Health,
  Job,
  JobEvent,
  ListOptions,
  LocalOwnerSession,
  NodeUpdate,
  NodeUpdatePolicy,
  Page,
  Printer,
  Webhook,
  Workspace,
  WorkspaceMember
} from './types.js';

export interface SpoolClientOptions {
  apiKey?: string;
  baseUrl?: string;
  fetch?: typeof globalThis.fetch;
  headers?: Record<string, string>;
  accessToken?: () => string | Promise<string | undefined> | undefined;
}

export class SpoolError extends Error {
  readonly code: string;
  readonly status: number;
  readonly requestId: string | undefined;
  readonly retryable: boolean;
  readonly details: Record<string, unknown> | undefined;

  constructor(
    status: number,
    error: {
      code: string;
      message: string;
      request_id?: string;
      retryable?: boolean;
      details?: Record<string, unknown>;
    }
  ) {
    super(error.message);
    this.name = 'SpoolError';
    this.code = error.code;
    this.status = status;
    this.requestId = error.request_id;
    this.retryable = error.retryable ?? false;
    this.details = error.details;
  }
}

export class SpoolClient {
  readonly baseUrl: string;
  private readonly apiKey: string | undefined;
  private readonly fetcher: typeof globalThis.fetch;
  private readonly defaultHeaders: Record<string, string>;
  private readonly accessToken: SpoolClientOptions['accessToken'];

  constructor(options: SpoolClientOptions = {}) {
    this.baseUrl = (options.baseUrl ?? 'https://api.spool.dev').replace(/\/+$/, '');
    this.apiKey = options.apiKey;
    this.fetcher = options.fetch ?? globalThis.fetch;
    this.defaultHeaders = options.headers ?? {};
    this.accessToken = options.accessToken;
  }

  health = () => this.request<Health>('GET', '/v1/health');
  ready = () => this.request<Health>('GET', '/v1/ready');
  meta = () => this.request<DeploymentMeta>('GET', '/v1/meta');

  readonly identity = {
    bootstrap: (
      input: { workspace_name: string; email: string; display_name?: string | null },
      bootstrapToken: string
    ) =>
      this.request<BootstrappedLocalOwner>('POST', '/v1/identity/local/bootstrap', {
        body: input,
        headers: { 'x-spool-bootstrap-token': bootstrapToken }
      }),
    exchange: (credential: string) =>
      this.request<LocalOwnerSession>('POST', '/v1/identity/local/exchange', {
        body: { credential }
      }),
    rotate: () =>
      this.request<LocalOwnerSession>('POST', '/v1/identity/local/sessions/rotate'),
    revoke: () => this.request<void>('POST', '/v1/identity/local/sessions/revoke'),
    me: () => this.request<CurrentIdentity>('GET', '/v1/identity/me')
  };

  readonly workspaces = {
    current: () => this.request<Workspace>('GET', '/v1/workspaces/current'),
    members: () =>
      this.request<WorkspaceMember[]>('GET', '/v1/workspaces/current/members')
  };

  readonly apiKeys = {
    list: () => this.request<ApiKey[]>('GET', '/v1/api-keys'),
    create: (input: CreateApiKey) =>
      this.request<CreatedApiKey>('POST', '/v1/api-keys', { body: input }),
    revoke: (id: string) =>
      this.request<ApiKey>('DELETE', `/v1/api-keys/${encodeURIComponent(id)}`)
  };

  readonly agents = {
    list: () => this.request<Agent[]>('GET', '/v1/agents'),
    createEnrolment: (input: { name: string; expires_in_seconds?: number }) =>
      this.request<AgentEnrolment>('POST', '/v1/agent-enrolments', { body: input })
  };

  readonly pairing = {
    create: (input: CreateDeviceAuthorization) =>
      this.request<CreatedDeviceAuthorization>('POST', '/v1/device-authorizations', {
        body: input
      }),
    status: (deviceCode: string) =>
      this.request<DeviceAuthorizationStatus>(
        'GET',
        `/v1/device-authorizations/${encodeURIComponent(deviceCode)}`
      ),
    review: (authorizationId: string) =>
      this.request<DeviceAuthorizationReview>(
        'GET',
        `/v1/device-authorizations/${encodeURIComponent(authorizationId)}/review`
      ),
    approve: (authorizationId: string, userCode: string) =>
      this.request<DeviceAuthorizationStatus>(
        'POST',
        `/v1/device-authorizations/${encodeURIComponent(authorizationId)}/approve`,
        { body: { user_code: userCode } }
      ),
    deny: (authorizationId: string, userCode: string) =>
      this.request<DeviceAuthorizationStatus>(
        'POST',
        `/v1/device-authorizations/${encodeURIComponent(authorizationId)}/deny`,
        { body: { user_code: userCode } }
      ),
    exchange: (deviceCode: string) =>
      this.request<DeviceAuthorizationExchange>(
        'POST',
        `/v1/device-authorizations/${encodeURIComponent(deviceCode)}/exchange`
      )
  };

  readonly nodes = {
    list: () => this.request<Agent[]>('GET', '/v1/nodes'),
    retrieve: (id: string) =>
      this.request<Agent>('GET', `/v1/nodes/${encodeURIComponent(id)}`),
    rename: (id: string, name: string) =>
      this.request<Agent>('PATCH', `/v1/nodes/${encodeURIComponent(id)}`, {
        body: { name }
      }),
    revoke: (id: string) =>
      this.request<void>('DELETE', `/v1/nodes/${encodeURIComponent(id)}`),
    pause: (id: string) =>
      this.request<void>('POST', `/v1/nodes/${encodeURIComponent(id)}/pause`),
    resume: (id: string) =>
      this.request<void>('POST', `/v1/nodes/${encodeURIComponent(id)}/resume`),
    diagnostics: (id: string) =>
      this.request<{ request_id: string; state: 'requested' }>(
        'POST',
        `/v1/nodes/${encodeURIComponent(id)}/diagnostics`
      ),
    update: (id: string) =>
      this.request<NodeUpdate>('GET', `/v1/nodes/${encodeURIComponent(id)}/update`),
    updatePolicy: (id: string, policy: NodeUpdatePolicy) =>
      this.request<NodeUpdate>('PATCH', `/v1/nodes/${encodeURIComponent(id)}/update-policy`, {
        body: policy
      }),
    requestUpdate: (id: string, version: string, metadataUrl: string) =>
      this.request<NodeUpdate>('POST', `/v1/nodes/${encodeURIComponent(id)}/update`, {
        body: { version, metadata_url: metadataUrl }
      }),
    rollback: (id: string, metadataUrl: string) =>
      this.request<NodeUpdate>('POST', `/v1/nodes/${encodeURIComponent(id)}/rollback`, {
        body: { metadata_url: metadataUrl }
      })
  };

  readonly printers = {
    list: (options?: ListOptions) =>
      this.request<Page<Printer>>('GET', '/v1/printers', options ? { query: options } : {})
  };

  readonly jobs = {
    list: (options?: ListOptions) =>
      this.request<Page<Job>>('GET', '/v1/jobs', options ? { query: options } : {}),
    retrieve: (id: string) => this.request<Job>('GET', `/v1/jobs/${encodeURIComponent(id)}`),
    events: (id: string) =>
      this.request<JobEvent[]>('GET', `/v1/jobs/${encodeURIComponent(id)}/events`),
    create: (input: CreateJob, idempotencyKey?: string) =>
      this.request<Job>(
        'POST',
        '/v1/jobs',
        idempotencyKey ? { body: input, idempotencyKey } : { body: input }
      ),
    cancel: (id: string) =>
      this.request<Job>('POST', `/v1/jobs/${encodeURIComponent(id)}/cancel`)
  };

  readonly webhooks = {
    list: () => this.request<Webhook[]>('GET', '/v1/webhooks'),
    create: (input: { url: string; events: string[] }) =>
      this.request<Webhook & { secret: string }>('POST', '/v1/webhooks', { body: input }),
    remove: (id: string) =>
      this.request<void>('DELETE', `/v1/webhooks/${encodeURIComponent(id)}`)
  };

  /**
   * Opens the canonical same-origin event stream. EventSource cannot set a
   * bearer header, so hosted dashboards should call a same-origin authenticated
   * BFF route and SDK users should prefer signed webhooks outside a browser.
   */
  events(path = '/v1/events/stream'): EventSource {
    return new EventSource(new URL(path, `${this.baseUrl}/`).toString());
  }

  private async request<T>(
    method: string,
    path: string,
    options: {
      body?: unknown;
      idempotencyKey?: string;
      query?: ListOptions;
      headers?: Record<string, string>;
    } = {}
  ): Promise<T> {
    const url = new URL(`${this.baseUrl}${path}`);
    for (const [key, value] of Object.entries(options.query ?? {})) {
      if (value !== undefined) url.searchParams.set(key, String(value));
    }

    const dynamicToken = await this.accessToken?.();
    const authorization = this.apiKey ?? dynamicToken;
    const headers: Record<string, string> = {
      accept: 'application/json',
      ...this.defaultHeaders,
      ...options.headers
    };
    if (options.body !== undefined) headers['content-type'] = 'application/json';
    if (authorization) headers.authorization = `Bearer ${authorization}`;
    if (options.idempotencyKey) headers['idempotency-key'] = options.idempotencyKey;

    const init: RequestInit = { method, headers };
    if (options.body !== undefined) init.body = JSON.stringify(options.body);
    const response = await this.fetcher(url, init);

    if (!response.ok) {
      let body: ErrorEnvelope | undefined;
      try {
        body = (await response.json()) as ErrorEnvelope;
      } catch {
        // A proxy may return HTML or an empty response. Preserve a stable error.
      }
      throw new SpoolError(
        response.status,
        body?.error ?? {
          code: 'unexpected_response',
          message: response.statusText || 'Spool request failed',
          retryable: response.status >= 500
        }
      );
    }

    if (response.status === 204) return undefined as T;
    const text = await response.text();
    if (text === '') return undefined as T;
    return JSON.parse(text) as T;
  }
}
