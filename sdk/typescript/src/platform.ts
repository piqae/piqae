import { PiqaeClient, PiqaeError } from './client.js';
import type {
  CreateJob,
  ErrorEnvelope,
  Job,
  JobOptions,
  PlatformAccount,
  UpsertPlatformAccount
} from './types.js';

export interface PiqaePlatformOptions {
  platformKey: string;
  baseUrl?: string;
  fetch?: typeof globalThis.fetch;
  headers?: Record<string, string>;
}

export type PrintPdfInput = {
  title: string;
  pdf: Blob | ArrayBuffer | Uint8Array;
  options?: JobOptions;
  metadata?: Record<string, string>;
  idempotencyKey?: string;
} & (
  | { printerId: string; targetId?: never }
  | { targetId: string; printerId?: never }
);

interface AccountClientOptions {
  platformKey: string;
  baseUrl: string;
  fetch: typeof globalThis.fetch;
  headers: Record<string, string>;
}

export class PiqaeAccountEnvironment extends PiqaeClient {
  async printPdf(input: PrintPdfInput): Promise<Job> {
    const { body, bytes } = await pdfContent(input.pdf);
    const sha256 = await sha256Hex(bytes);
    const upload = await this.uploads.createAndPut(
      {
        media_type: 'application/pdf',
        byte_length: bytes.byteLength,
        sha256
      },
      body
    );
    const destination =
      input.printerId !== undefined
        ? { printer_id: input.printerId as string }
        : { target_id: input.targetId as string };
    const job: CreateJob = {
      ...destination,
      title: input.title,
      content_type: 'pdf',
      content: { type: 'upload', upload_id: upload.id },
      ...(input.options === undefined ? {} : { options: input.options }),
      ...(input.metadata === undefined ? {} : { metadata: input.metadata })
    };
    return this.jobs.create(job, input.idempotencyKey);
  }
}

export class PiqaeAccount extends PiqaeAccountEnvironment {
  readonly account: PlatformAccount;
  readonly test: PiqaeAccountEnvironment;
  readonly live: PiqaeAccountEnvironment;

  constructor(account: PlatformAccount, options: AccountClientOptions) {
    super({
      platformKey: options.platformKey,
      platformContext: {
        workspaceId: account.id,
        environmentId: account.environments.live.id
      },
      baseUrl: options.baseUrl,
      fetch: options.fetch,
      headers: options.headers
    });
    this.account = account;
    this.live = this;
    this.test = new PiqaeAccountEnvironment({
      platformKey: options.platformKey,
      platformContext: {
        workspaceId: account.id,
        environmentId: account.environments.test.id
      },
      baseUrl: options.baseUrl,
      fetch: options.fetch,
      headers: options.headers
    });
  }

  get id(): string {
    return this.account.id;
  }

  get externalId(): string {
    return this.account.external_id;
  }

  get name(): string {
    return this.account.name;
  }

  get status(): PlatformAccount['status'] {
    return this.account.status;
  }

  get metadata(): Record<string, string> {
    return this.account.metadata;
  }

  get environments(): PlatformAccount['environments'] {
    return this.account.environments;
  }

  get createdAt(): string {
    return this.account.created_at;
  }

  get updatedAt(): string {
    return this.account.updated_at;
  }
}

export class PiqaePlatform {
  readonly baseUrl: string;
  private readonly platformKey: string;
  private readonly fetcher: typeof globalThis.fetch;
  private readonly defaultHeaders: Record<string, string>;

  constructor(options: PiqaePlatformOptions) {
    if (!options.platformKey.startsWith('piq_platform_')) {
      throw new TypeError('PiqaePlatform requires a piq_platform_ service-account key');
    }
    this.baseUrl = (options.baseUrl ?? 'https://api.piqae.com').replace(/\/+$/, '');
    this.platformKey = options.platformKey;
    this.fetcher = options.fetch ?? globalThis.fetch;
    this.defaultHeaders = withoutTenantSelection(options.headers ?? {});
  }

  readonly accounts = {
    getOrCreate: async (externalId: string, input: UpsertPlatformAccount) =>
      this.account(
        await this.request<PlatformAccount>(
          'PUT',
          `/v1/platform/accounts/${encodeURIComponent(externalId)}`,
          input
        )
      ),
    retrieve: async (externalId: string) =>
      this.account(
        await this.request<PlatformAccount>(
          'GET',
          `/v1/platform/accounts/${encodeURIComponent(externalId)}`
        )
      ),
    list: async () =>
      Promise.all(
        (await this.request<PlatformAccount[]>('GET', '/v1/platform/accounts')).map(
          (account) => this.account(account)
        )
      ),
    archive: (externalId: string) =>
      this.request<void>(
        'DELETE',
        `/v1/platform/accounts/${encodeURIComponent(externalId)}`
      )
  };

  private account(account: PlatformAccount): PiqaeAccount {
    return new PiqaeAccount(account, {
      platformKey: this.platformKey,
      baseUrl: this.baseUrl,
      fetch: this.fetcher,
      headers: this.defaultHeaders
    });
  }

  private async request<T>(method: string, path: string, body?: unknown): Promise<T> {
    const headers: Record<string, string> = {
      accept: 'application/json',
      ...this.defaultHeaders,
      authorization: `Bearer ${this.platformKey}`
    };
    if (body !== undefined) headers['content-type'] = 'application/json';
    const init: RequestInit = { method, headers };
    if (body !== undefined) init.body = JSON.stringify(body);
    const response = await this.fetcher(`${this.baseUrl}${path}`, init);
    if (!response.ok) {
      let responseBody: ErrorEnvelope | undefined;
      try {
        responseBody = (await response.json()) as ErrorEnvelope;
      } catch {
        // Preserve one stable error shape for proxy or edge responses.
      }
      throw new PiqaeError(
        response.status,
        responseBody?.error ?? {
          code: 'unexpected_response',
          message: response.statusText || 'Piqae platform request failed',
          retryable: response.status >= 500
        }
      );
    }
    if (response.status === 204) return undefined as T;
    const text = await response.text();
    return text === '' ? (undefined as T) : (JSON.parse(text) as T);
  }
}

async function pdfContent(
  pdf: Blob | ArrayBuffer | Uint8Array
): Promise<{ body: BodyInit; bytes: ArrayBuffer }> {
  if (pdf instanceof Blob) {
    return { body: pdf, bytes: await pdf.arrayBuffer() };
  }
  if (pdf instanceof ArrayBuffer) {
    return { body: pdf, bytes: pdf };
  }
  const copy = Uint8Array.from(pdf);
  return { body: copy, bytes: copy.buffer };
}

async function sha256Hex(content: ArrayBuffer): Promise<string> {
  const cryptoApi = globalThis.crypto;
  if (!cryptoApi?.subtle) {
    throw new Error('SHA-256 requires Web Crypto support');
  }
  const digest = await cryptoApi.subtle.digest('SHA-256', content);
  return Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, '0')).join('');
}

const TENANT_SELECTION_HEADERS = new Set([
  'x-piqae-workspace-id',
  'x-piqae-environment-id'
]);

function withoutTenantSelection(headers: Record<string, string>): Record<string, string> {
  return Object.fromEntries(
    Object.entries(headers).filter(
      ([name]) => !TENANT_SELECTION_HEADERS.has(name.toLowerCase())
    )
  );
}
