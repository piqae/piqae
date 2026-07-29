export type SpoolId = string;

export type JobState =
  | 'registered'
  | 'content_pending'
  | 'waiting_for_agent'
  | 'agent_downloading'
  | 'agent_accepted'
  | 'queued_local'
  | 'preparing'
  | 'rendering'
  | 'spool_intent'
  | 'accepted_by_spooler'
  | 'spooling'
  | 'printing'
  | 'blocked'
  | 'completed_reported'
  | 'delivery_uncertain'
  | 'cancel_requested'
  | 'cancelled'
  | 'expired'
  | 'failed_retryable'
  | 'failed_terminal';

export interface Page<T> {
  data: T[];
  next_cursor?: string | null;
  has_more: boolean;
}

export interface Health {
  status: 'ok';
  version: string;
}

export type ApiKeyScope =
  | 'api_keys_read'
  | 'api_keys_write'
  | 'agents_read'
  | 'agents_write'
  | 'printers_read'
  | 'printers_write'
  | 'jobs_read'
  | 'jobs_write'
  | 'webhooks_read'
  | 'webhooks_write'
  | 'usage_read'
  | 'audit_read';

export interface ApiKey {
  id: string;
  name: string;
  lookup_prefix: string;
  scopes: ApiKeyScope[];
  expires_at: string | null;
  last_used_at: string | null;
  revoked_at: string | null;
  created_at: string;
}

export interface CreateApiKey {
  name: string;
  scopes: ApiKeyScope[];
  expires_at?: string | null;
}

export interface CreatedApiKey extends ApiKey {
  /** Plaintext secret returned only by the create operation. */
  secret: string;
}

export interface Agent {
  id: SpoolId;
  name: string;
  platform: string;
  state: 'connected' | 'disconnected' | 'paused' | 'degraded';
  version: string;
  last_seen_at: string;
}

export interface Printer {
  id: SpoolId;
  agent_id: SpoolId;
  name: string;
  state: 'online' | 'offline' | 'paused' | 'busy' | 'paper_out' | 'error' | 'unknown';
  capabilities: Record<string, unknown>;
  updated_at: string;
}

export interface JobOptions {
  bin?: string;
  collate?: boolean;
  color?: boolean;
  copies?: number;
  dpi?: string;
  duplex?: 'one-sided' | 'long-edge' | 'short-edge';
  fit_to_page?: boolean;
  media?: string;
  nup?: number;
  pages?: string;
  paper?: string;
  rotate?: 0 | 90 | 180 | 270;
}

export type JobContent =
  | { type: 'upload'; upload_id: SpoolId }
  | { type: 'base64'; data: string }
  | { type: 'uri'; uri: string };

export interface CreateJob {
  printer_id: SpoolId;
  title: string;
  source?: string | null;
  content_type: 'pdf' | 'raw';
  content: JobContent;
  options?: JobOptions;
  deliveries?: number;
  expire_after_seconds?: number;
  metadata?: Record<string, string>;
}

export interface Job {
  id: SpoolId;
  printer_id: SpoolId;
  title: string;
  source?: string | null;
  content_type: 'pdf' | 'raw';
  deliveries: number;
  state: JobState;
  created_at: string;
  expires_at: string;
}

export interface JobEvent {
  id: SpoolId;
  job_id: SpoolId;
  sequence: number;
  state: JobState;
  reason?: string | null;
  message?: string | null;
  occurred_at: string;
}

export interface Webhook {
  id: SpoolId;
  url: string;
  events: string[];
  enabled: boolean;
  created_at: string;
}

export interface AgentEnrolment {
  id: SpoolId;
  token: string;
  expires_at: string;
}

export interface ErrorEnvelope {
  error: {
    code: string;
    message: string;
    request_id: string;
    retryable: boolean;
    details?: Record<string, unknown>;
  };
}

export interface ListOptions {
  limit?: number;
  after?: string;
}
