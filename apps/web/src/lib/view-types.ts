import type { JobOptions, JobState, NativePrinterOption } from '@piqae/sdk';

export type ResourceState = 'online' | 'offline' | 'degraded' | 'paused' | 'unknown';

export interface DashboardPage<T> {
  data: T[];
  nextCursor: string | null;
}

export interface DashboardResourceOwner {
  id: string;
  externalId: string;
  name: string;
}

export interface DashboardCustomerOperations {
  customer: DashboardResourceOwner;
  environment: { id: string; kind: 'live' };
  agents: DashboardAgent[];
  printers: DashboardPrinter[];
  jobs: DashboardJob[];
  destinations: DashboardDestination[];
  routes: DashboardPrinterRoute[];
  routeObservations: DashboardRouteObservation[];
}

export interface DashboardCustomerOperationsPage {
  data: DashboardCustomerOperations[];
  nextCursor: string | null;
  hasMore: boolean;
}

export interface DashboardAgent {
  customer?: DashboardResourceOwner | null;
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
  customer?: DashboardResourceOwner | null;
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
  capabilityRevision: number;
  nativeOptions: Record<string, NativePrinterOption>;
  profiles: DashboardPrinterProfile[];
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

export type DashboardIdentityConfidence =
  | 'verified'
  | 'high'
  | 'possible'
  | 'conflict'
  | 'unknown';

export interface DashboardDestination {
  customer?: DashboardResourceOwner | null;
  id: string;
  displayName: string;
  manufacturer: string | null;
  model: string | null;
  identityConfidence: DashboardIdentityConfidence;
  status: 'active' | 'needs_review' | 'split' | 'retired';
  routeCount: number;
  updatedAt: string;
}

export interface DashboardRouteObservation {
  customer?: DashboardResourceOwner | null;
  id: string;
  routeId: string;
  sequence: number;
  printerState: 'online' | 'busy' | 'paused' | 'paper_out' | 'error' | 'offline' | 'unknown';
  acceptingJobs: boolean;
  totalJobs: number;
  activeJobs: number;
  heldJobs: number;
  connectorJobs: number;
  otherPiqaeOrExternalJobs: number;
  unknownJobs: number;
  estimatedBusySeconds: number | null;
  observedAt: string;
  expiresAt: string;
}

export interface DashboardPrinterRoute {
  customer?: DashboardResourceOwner | null;
  id: string;
  physicalDestinationId: string;
  printerId: string;
  agentId: string;
  nativeQueueId: string;
  enabled: boolean;
  health: 'ready' | 'busy' | 'needs_operator' | 'offline' | 'stale' | 'unknown';
  telemetryFreshness: 'live' | 'recent' | 'stale' | 'never';
  projectionHealth: 'current' | 'pending' | 'failed' | 'unsupported';
  capabilityRevision: number;
  profileRevision: number;
  profileObservedAt: string | null;
  stockObservedAt: string | null;
  stockState: 'current' | 'stale' | 'unknown';
  schedulingAuthorityId: string | null;
  latestObservation: DashboardRouteObservation | null;
  updatedAt: string;
}

export interface DashboardPrinterProfile {
  profileId: string;
  revision: number;
  name: string;
  isDefault: boolean;
  options: JobOptions;
  status:
    | 'draft'
    | 'capturing'
    | 'ready'
    | 'needs_test'
    | 'stale'
    | 'driver_mismatch'
    | 'destination_missing'
    | 'dependency_missing'
    | 'interactive_only'
    | 'invalid'
    | 'retired';
  nativeKind: string;
  nativeDigest: string | null;
  driverName: string | null;
  driverVersion: string | null;
  summary: {
    paper: string | null;
    dimensionsMm: [number, number] | null;
    source: string | null;
    media: string | null;
    color: string | null;
    resolution: string | null;
  };
  stockId: string | null;
  safeOverrides: string[];
  lastValidatedAt: string | null;
  lastTestJobId: string | null;
  published: boolean;
}

export interface DashboardJob {
  customer?: DashboardResourceOwner | null;
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
  deliveryUncertainSince?: string | null;
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
  environment: 'test' | 'live' | 'platform';
  kind?: 'api_key' | 'platform';
  scopes: string[];
  lastUsedAt: string | null;
  createdAt: string;
}

export interface DashboardAccount {
  id: string;
  externalId: string;
  name: string;
  status: 'active' | 'suspended' | 'cancelled';
  metadata: Record<string, string>;
  environments: {
    testId: string;
    liveId: string;
  };
  createdAt: string;
  updatedAt: string;
}

export interface DashboardMeta {
  deployment: 'cloud' | 'self_hosted' | 'local';
  version: string;
  auth: {
    provider: 'workos' | 'local_owner' | 'oidc' | 'hybrid' | 'none';
    workspaceSwitching: boolean;
    invitations: boolean;
  };
  billing: { enabled: boolean };
  updates: { officialFeed: boolean; customFeed: boolean };
  platform: { accounts: boolean };
}

export interface DashboardOverview {
  agents: { total: number; online: number; degraded: number };
  printers: { total: number; online: number; attention: number };
  jobs: {
    recent: number;
    active: number;
    failed: number;
    uncertain: number;
    oldestUncertainSince?: string | null;
  };
}

export interface DashboardWorkspace {
  id: string;
  name: string;
  slug: string;
}

export interface DashboardNodeDiagnostic {
  requestId: string;
  state: 'requested' | 'complete' | 'failed';
  requestedAt: string;
  receivedAt: string | null;
  agentVersion: string | null;
  queuedJobs: number | null;
  activeJobs: number | null;
  storageHealthy: boolean | null;
  executorCrashes: number | null;
  lastErrorCode: string | null;
  collectionErrorCode: string | null;
}
