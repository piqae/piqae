import type {
  Agent,
  AgentEnrolment,
  ApiKey,
  CreateApiKey,
  CreateJob,
  CreatedApiKey,
  ErrorEnvelope,
  Health,
  Job,
  JobEvent,
  ListOptions,
  Page,
  Printer,
  Webhook
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
      ...this.defaultHeaders
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
    return (await response.json()) as T;
  }
}
