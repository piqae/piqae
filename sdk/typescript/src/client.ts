import type {
  Agent,
  AgentEnrolment,
  ApiKey,
  BillingSummary,
  CapabilityDocument,
  BootstrappedLocalOwner,
  CreateApiKey,
  CreateDeviceAuthorization,
  CreateJob,
  CreateNodeConnectSession,
  CreatePrintWorkflow,
  CreateStock,
  CreateTarget,
  CreateTargetBinding,
  CreateUpload,
  CreatedUpload,
  CreatedDeviceAuthorization,
  DeploymentMeta,
  DeviceAuthorizationExchange,
  DeviceAuthorizationReview,
  DeviceAuthorizationStatus,
  DesignSpecification,
  CreatedApiKey,
  CurrentIdentity,
  ErrorEnvelope,
  Health,
  Job,
  JobEvent,
  JobListOptions,
  LoadedMediaObservation,
  ListOptions,
  LocalOwnerSession,
  NodeConnector,
  NodeContentEncryptionKey,
  NodeUpdate,
  NodeConnectSession,
  NodeUpdatePolicy,
  PatchStock,
  PatchTarget,
  Page,
  PlatformContext,
  Printer,
  PrintIntent,
  PrintIntentValidation,
  PrintWorkflow,
  ResolvedPrintTicket,
  Stock,
  Target,
  TargetBinding,
  TargetReadiness,
  Upload,
  UsageSummary,
  UpsertLoadedMediaObservation,
  Webhook,
  WebhookDelivery,
  Workspace,
  WorkspaceMember
} from './types.js';
import { canonicalJobOptions, encryptedJobCiphertext, encryptedJobManifest, type EncryptedJobEnvelope } from './encrypted-jobs.js';

interface CommonPiqaeClientOptions {
  baseUrl?: string;
  fetch?: typeof globalThis.fetch;
  eventSource?: typeof EventSource;
  headers?: Record<string, string>;
}

interface TenantPiqaeClientOptions {
  apiKey?: string;
  accessToken?: () => string | Promise<string | undefined> | undefined;
  platformKey?: never;
  platformContext?: never;
}

interface PlatformPiqaeClientOptions {
  /** Distinct operator-issued service-account credential. Never use a tenant API key. */
  platformKey: string;
  /** Explicit grant boundary applied to every tenant API request. */
  platformContext: PlatformContext;
  apiKey?: never;
  accessToken?: never;
}

export type PiqaeClientOptions = CommonPiqaeClientOptions &
  (TenantPiqaeClientOptions | PlatformPiqaeClientOptions);

type AccessTokenProvider = TenantPiqaeClientOptions['accessToken'];

export class PiqaeError extends Error {
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
    this.name = 'PiqaeError';
    this.code = error.code;
    this.status = status;
    this.requestId = error.request_id;
    this.retryable = error.retryable ?? false;
    this.details = error.details;
  }
}

export class PiqaeClient {
  readonly baseUrl: string;
  private readonly apiKey: string | undefined;
  private readonly platformKey: string | undefined;
  private readonly platformContext: PlatformContext | undefined;
  private readonly fetcher: typeof globalThis.fetch;
  private readonly eventSourceConstructor: typeof EventSource | undefined;
  private readonly defaultHeaders: Record<string, string>;
  private readonly accessToken: AccessTokenProvider;

  constructor(options: PiqaeClientOptions = {}) {
    if (options.platformContext && !options.platformKey) {
      throw new TypeError('platformContext requires a distinct platformKey');
    }
    if (options.platformKey && (options.apiKey || options.accessToken)) {
      throw new TypeError('platformKey cannot be combined with apiKey or accessToken');
    }
    this.baseUrl = (options.baseUrl ?? 'https://api.piqae.com').replace(/\/+$/, '');
    this.apiKey = options.apiKey;
    this.platformKey = options.platformKey;
    this.platformContext = options.platformContext;
    this.fetcher = options.fetch ?? globalThis.fetch;
    this.eventSourceConstructor = options.eventSource ?? globalThis.EventSource;
    this.defaultHeaders = withoutPlatformSelectionHeaders(options.headers ?? {});
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
        headers: { 'x-piqae-bootstrap-token': bootstrapToken }
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

  readonly billing = {
    summary: () => this.request<BillingSummary>('GET', '/v1/billing/summary')
  };

  readonly usage = {
    retrieve: (month?: string) =>
      this.request<UsageSummary>(
        'GET',
        '/v1/usage',
        month === undefined ? {} : { query: { month } }
      )
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
    // The device code is a bearer secret for the pairing exchange, so it is
    // sent in the request body. In a URL it would be recorded by every proxy
    // and CDN access log between the caller and the control plane.
    status: (deviceCode: string) =>
      this.request<DeviceAuthorizationStatus>('POST', '/v1/device-authorizations/status', {
        body: { device_code: deviceCode }
      }),
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
      this.request<DeviceAuthorizationExchange>('POST', '/v1/device-authorizations/exchange', {
        body: { device_code: deviceCode }
      })
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
    listDiagnostics: (id: string) =>
      this.request<import('./types.js').NodeDiagnostic[]>(
        'GET',
        `/v1/nodes/${encodeURIComponent(id)}/diagnostics`
      ),
    retrieveDiagnostic: (id: string, requestId: string) =>
      this.request<import('./types.js').NodeDiagnostic>(
        'GET',
        `/v1/nodes/${encodeURIComponent(id)}/diagnostics/${encodeURIComponent(requestId)}`
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
      }),
    connectors: (id: string) =>
      this.request<NodeConnector[]>(
        'GET',
        `/v1/nodes/${encodeURIComponent(id)}/connectors`
      ),
    revokeConnector: (nodeId: string, connectorId: string) =>
      this.request<void>(
        'DELETE',
        `/v1/nodes/${encodeURIComponent(nodeId)}/connectors/${encodeURIComponent(connectorId)}`
      )
  };

  readonly connectSessions = {
    create: (input: CreateNodeConnectSession) =>
      this.request<NodeConnectSession>('POST', '/v1/node-connect-sessions', { body: input }),
    retrieve: (id: string) =>
      this.request<NodeConnectSession>('GET', `/v1/node-connect-sessions/${encodeURIComponent(id)}`)
  };

  readonly printers = {
    list: (options?: ListOptions) =>
      this.request<Page<Printer>>('GET', '/v1/printers', options ? { query: options } : {}),
    retrieve: (id: string) =>
      this.request<Printer>('GET', `/v1/printers/${encodeURIComponent(id)}`),
    contentEncryptionKey: (id: string) =>
      this.request<NodeContentEncryptionKey>('GET', `/v1/printers/${encodeURIComponent(id)}/content-encryption-key`),
    capabilities: (id: string) =>
      this.request<CapabilityDocument>('GET', `/v1/printers/${encodeURIComponent(id)}/capabilities`),
    loadedMedia: (id: string) =>
      this.request<LoadedMediaObservation[]>('GET', `/v1/printers/${encodeURIComponent(id)}/loaded-media`),
    confirmLoadedMedia: (id: string, input: UpsertLoadedMediaObservation) =>
      this.request<LoadedMediaObservation>('PUT', `/v1/printers/${encodeURIComponent(id)}/loaded-media`, { body: input })
  };

  readonly printIntents = {
    validate: (intent: PrintIntent) =>
      this.request<PrintIntentValidation>('POST', '/v1/print-intents/validate', { body: { intent } }),
    resolve: (intent: PrintIntent) =>
      this.request<ResolvedPrintTicket>('POST', '/v1/print-intents/resolve', { body: { intent } })
  };

  readonly printWorkflows = {
    list: () => this.request<PrintWorkflow[]>('GET', '/v1/print-workflows'),
    create: (input: CreatePrintWorkflow) =>
      this.request<PrintWorkflow>('POST', '/v1/print-workflows', { body: input })
  };

  readonly stocks = {
    list: () => this.request<Stock[]>('GET', '/v1/stocks'),
    create: (input: CreateStock) =>
      this.request<Stock>('POST', '/v1/stocks', { body: input }),
    update: (id: string, input: PatchStock) =>
      this.request<Stock>('PATCH', `/v1/stocks/${encodeURIComponent(id)}`, { body: input })
  };

  readonly targets = {
    list: () => this.request<Target[]>('GET', '/v1/targets'),
    create: (input: CreateTarget) =>
      this.request<Target>('POST', '/v1/targets', { body: input }),
    update: (id: string, input: PatchTarget) =>
      this.request<Target>('PATCH', `/v1/targets/${encodeURIComponent(id)}`, { body: input }),
    bindings: (id: string) =>
      this.request<TargetBinding[]>(
        'GET',
        `/v1/targets/${encodeURIComponent(id)}/bindings`
      ),
    bind: (id: string, input: CreateTargetBinding) =>
      this.request<TargetBinding>(
        'POST',
        `/v1/targets/${encodeURIComponent(id)}/bindings`,
        { body: input }
      ),
    unbind: (targetId: string, bindingId: string) =>
      this.request<void>(
        'DELETE',
        `/v1/targets/${encodeURIComponent(targetId)}/bindings/${encodeURIComponent(bindingId)}`
      ),
    readiness: (id: string) =>
      this.request<TargetReadiness>(
        'GET',
        `/v1/targets/${encodeURIComponent(id)}/readiness`
      ),
    designSpecification: (id: string) =>
      this.request<DesignSpecification>(
        'GET',
        `/v1/targets/${encodeURIComponent(id)}/design-specification`
      )
  };

  readonly uploads = {
    create: (input: CreateUpload) =>
      this.request<CreatedUpload>('POST', '/v1/uploads', { body: input }),
    retrieve: (id: string) =>
      this.request<Upload>('GET', `/v1/uploads/${encodeURIComponent(id)}`),
    put: (upload: CreatedUpload, content: BodyInit) => this.putUpload(upload, content),
    complete: (id: string, sha256: string, byteLength: number) =>
      this.request<Upload>('POST', `/v1/uploads/${encodeURIComponent(id)}/complete`, {
        body: { sha256, byte_length: byteLength }
      }),
    createAndPut: async (input: CreateUpload, content: BodyInit) => {
      const upload = await this.uploads.create(input);
      const stored = await this.uploads.put(upload, content);
      if (upload.requires_completion) {
        return this.uploads.complete(upload.id, input.sha256, input.byte_length);
      }
      return stored ?? this.uploads.retrieve(upload.id);
    }
  };

  readonly jobs = {
    createEncrypted: async (input: Omit<CreateJob, 'content' | 'print_intent' | 'resolved_ticket_digest'> & { target_id: string; printer_id?: never; print_intent?: never; resolved_ticket_digest?: never }, envelope: EncryptedJobEnvelope, idempotencyKey: string) => {
      if ('print_intent' in input || 'resolved_ticket_digest' in input) throw new TypeError('Encrypted v3 jobs cannot attach unbound print intent or ticket provenance');
      const idempotencyKeyBytes = new TextEncoder().encode(idempotencyKey).byteLength;
      if (idempotencyKeyBytes < 8 || idempotencyKeyBytes > 255) throw new TypeError('Encrypted jobs require an Idempotency-Key between 8 and 255 bytes');
      if (envelope.binding.target_id !== input.target_id || envelope.binding.content_type !== input.content_type || envelope.binding.deliveries !== (input.deliveries ?? 1) || JSON.stringify(envelope.binding.options) !== JSON.stringify(canonicalJobOptions(input.options))) throw new TypeError('Encrypted envelope does not match the job request');
      const ciphertext = encryptedJobCiphertext(envelope);
      const digest = await globalThis.crypto.subtle.digest('SHA-256', ciphertext);
      const sha256 = Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, '0')).join('');
      const upload = await this.uploads.createAndPut({ media_type: 'application/octet-stream', byte_length: ciphertext.byteLength, sha256 }, ciphertext);
      return this.request<Job>('POST', '/v1/jobs', { body: { ...input, content: { type: 'encrypted_upload', upload_id: upload.id, manifest: encryptedJobManifest(envelope) } }, idempotencyKey });
    },
    /** Submit resolved options bound by v3 AAD; ticket provenance is intentionally not attached. */
    createEncryptedResolved: async (input: Omit<CreateJob, 'content' | 'print_intent' | 'resolved_ticket_digest'> & { target_id: string; printer_id?: never; print_intent?: never; resolved_ticket_digest?: never }, envelope: EncryptedJobEnvelope, ticket: ResolvedPrintTicket, idempotencyKey: string) => {
      if (ticket.printer_id !== envelope.binding.printer_id) throw new TypeError('Resolved ticket and encrypted envelope target different printers');
      if (JSON.stringify(canonicalJobOptions(ticket.resolved_options)) !== JSON.stringify(envelope.binding.options)) throw new TypeError('Resolved ticket options are not bound by the encrypted envelope');
      if (JSON.stringify(canonicalJobOptions(input.options)) !== JSON.stringify(envelope.binding.options)) throw new TypeError('Resolved ticket options do not match the job request');
      return this.jobs.createEncrypted(input, envelope, idempotencyKey);
    },
    list: (options?: JobListOptions) =>
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
      this.request<void>('DELETE', `/v1/webhooks/${encodeURIComponent(id)}`),
    deliveries: (id: string) =>
      this.request<WebhookDelivery[]>(
        'GET',
        `/v1/webhooks/${encodeURIComponent(id)}/deliveries`
      ),
    replay: (deliveryId: string) =>
      this.request<void>(
        'POST',
        `/v1/webhook-deliveries/${encodeURIComponent(deliveryId)}/replay`
      )
  };

  /**
   * Opens the canonical same-origin event stream. EventSource cannot set a
   * bearer header, so hosted dashboards should call a same-origin authenticated
   * BFF route and SDK users should prefer signed webhooks outside a browser.
   */
  events(path = '/v1/events/stream'): EventSource {
    if (!this.eventSourceConstructor) {
      throw new TypeError(
        'PiqaeClient.events requires a browser EventSource or the eventSource constructor option'
      );
    }
    return new this.eventSourceConstructor(new URL(path, `${this.baseUrl}/`).toString());
  }

  private async request<T>(
    method: string,
    path: string,
    options: {
      body?: unknown;
      idempotencyKey?: string;
      query?: ListOptions | Record<string, string | number | boolean | undefined>;
      headers?: Record<string, string>;
    } = {}
  ): Promise<T> {
    const url = new URL(`${this.baseUrl}${path}`);
    for (const [key, value] of Object.entries(options.query ?? {})) {
      if (value !== undefined) url.searchParams.set(key, String(value));
    }

    const dynamicToken = await this.accessToken?.();
    const authorization = this.platformKey ?? this.apiKey ?? dynamicToken;
    const headers: Record<string, string> = {
      accept: 'application/json',
      ...this.defaultHeaders,
      ...options.headers
    };
    if (this.platformContext) {
      headers['x-piqae-workspace-id'] = this.platformContext.workspaceId;
      headers['x-piqae-environment-id'] = this.platformContext.environmentId;
    }
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
      throw new PiqaeError(
        response.status,
        body?.error ?? {
          code: 'unexpected_response',
          message: response.statusText || 'Piqae request failed',
          retryable: response.status >= 500
        }
      );
    }

    if (response.status === 204) return undefined as T;
    const text = await response.text();
    if (text === '') return undefined as T;
    return JSON.parse(text) as T;
  }

  private async putUpload(upload: CreatedUpload, content: BodyInit): Promise<Upload | undefined> {
    const relative = !/^[a-z][a-z\d+\-.]*:/i.test(upload.upload_url);
    const url = new URL(upload.upload_url, `${this.baseUrl}/`);
    const headers: Record<string, string> = {
      ...(upload.upload_headers ?? {})
    };
    if (!headers['content-type']) headers['content-type'] = upload.media_type;
    if (relative) {
      Object.assign(headers, this.defaultHeaders);
      const dynamicToken = await this.accessToken?.();
      const authorization = this.platformKey ?? this.apiKey ?? dynamicToken;
      if (authorization) headers.authorization = `Bearer ${authorization}`;
      if (this.platformContext) {
        headers['x-piqae-workspace-id'] = this.platformContext.workspaceId;
        headers['x-piqae-environment-id'] = this.platformContext.environmentId;
      }
    }
    const response = await this.fetcher(url, {
      method: upload.upload_method ?? 'PUT',
      headers,
      body: content
    });
    if (!response.ok) {
      let body: ErrorEnvelope | undefined;
      try {
        body = (await response.json()) as ErrorEnvelope;
      } catch {
        // Signed storage URLs can return provider-specific text or XML.
      }
      throw new PiqaeError(
        response.status,
        body?.error ?? {
          code: 'upload_failed',
          message: response.statusText || 'Document upload failed',
          retryable: response.status >= 500
        }
      );
    }
    const text = await response.text();
    return text === '' ? undefined : (JSON.parse(text) as Upload);
  }
}

const PLATFORM_SELECTION_HEADERS = new Set([
  'x-piqae-workspace-id',
  'x-piqae-environment-id'
]);

function withoutPlatformSelectionHeaders(
  headers: Record<string, string>
): Record<string, string> {
  return Object.fromEntries(
    Object.entries(headers).filter(
      ([name]) => !PLATFORM_SELECTION_HEADERS.has(name.toLowerCase())
    )
  );
}
