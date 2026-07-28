import type { JobState } from '@spool/sdk';

export type ResourceState = 'online' | 'offline' | 'degraded' | 'paused' | 'unknown';

export interface DashboardPage<T> {
  data: T[];
  nextCursor: string | null;
}

export interface DashboardAgent {
  id: string;
  name: string;
  state: ResourceState;
  os: 'windows' | 'macos' | 'linux';
  architecture: string;
  version: string;
  protocolVersion: string;
  lastSeenAt: string;
  queueDepth: number;
  printerCount: number;
  labels: string[];
}

export interface DashboardPrinter {
  id: string;
  agentId: string;
  name: string;
  description: string | null;
  location: string | null;
  state: ResourceState;
  stateReasons: string[];
  isDefault: boolean;
  queueDepth: number;
  lastSeenAt: string;
  capabilities: {
    color: boolean;
    duplex: boolean;
    copies: number;
    papers: string[];
    dpis: string[];
    source: string;
    revision: string;
    observedAt: string;
  };
}

export interface DashboardJob {
  id: string;
  printerId: string;
  agentId: string;
  title: string;
  source: string | null;
  contentFormat: 'pdf' | 'raw';
  state: JobState;
  reasonCode: string | null;
  message: string | null;
  authority: 'service' | 'agent' | 'renderer' | 'os_queue' | 'device';
  nativeJobId: string | null;
  createdAt: string;
  updatedAt: string;
  expiresAt: string | null;
  contentRetained: boolean;
}

export interface DashboardJobEvent {
  id: string;
  jobId: string;
  sequence: number;
  type: string;
  state: JobState;
  observer: string;
  authority: DashboardJob['authority'];
  reasonCode: string | null;
  message: string;
  occurredAt: string;
  receivedAt: string;
  details: Record<string, unknown>;
}

export interface DashboardWebhook {
  id: string;
  url: string;
  description: string | null;
  events: string[];
  enabled: boolean;
  status: 'healthy' | 'failing' | 'disabled';
  lastDeliveryAt: string | null;
  createdAt: string;
}

export interface DashboardApiKey {
  id: string;
  name: string;
  prefix: string;
  environment: 'test' | 'live';
  scopes: string[];
  lastUsedAt: string | null;
  createdAt: string;
}

export interface DashboardOverview {
  agents: { total: number; online: number; degraded: number };
  printers: { total: number; online: number; attention: number };
  jobs: { today: number; active: number; failed: number; uncertain: number };
  pickupLatencyP95Ms: number;
}
