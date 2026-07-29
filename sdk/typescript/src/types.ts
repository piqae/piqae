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

export interface DeploymentMeta {
  deployment: 'cloud' | 'self_hosted' | 'local';
  version: string;
  auth: {
    provider: 'workos' | 'local_owner' | 'oidc' | 'hybrid' | 'none';
    workspace_switching: boolean;
    invitations: boolean;
  };
  billing: { enabled: boolean };
  updates: { official_feed: boolean; custom_feed: boolean };
}

export type UploadMediaType = 'application/pdf' | 'application/octet-stream';

export interface CreateUpload {
  media_type: UploadMediaType;
  /** Exact byte length. The server rejects content that differs. */
  byte_length: number;
  /** Lower- or upper-case hexadecimal SHA-256 digest. */
  sha256: string;
}

export interface Upload {
  id: SpoolId;
  /** Storage implementation detail. Do not persist or construct this value. */
  object_key: string;
  media_type: UploadMediaType;
  expected_sha256: string;
  expected_bytes: number;
  state: 'pending' | 'complete' | 'expired';
  expires_at: string;
}

export interface CreatedUpload extends Upload {
  /** Relative proxy URL or absolute, time-limited object-store URL. */
  upload_url: string;
  upload_method?: 'PUT';
  upload_headers?: Record<string, string>;
  /**
   * Direct object-store uploads require an explicit completion call. Proxy
   * uploads are verified and completed by their PUT response.
   */
  requires_completion?: boolean;
}

export interface Workspace {
  id: SpoolId;
  name: string;
  slug: string;
  status: 'active' | 'suspended' | 'cancelled';
  created_at: string;
  updated_at: string;
}

export interface WorkspaceMember {
  id: SpoolId;
  email: string;
  name: string | null;
  role: 'owner' | 'admin' | 'developer' | 'operator' | 'viewer' | 'billing';
  status: 'pending' | 'active' | 'inactive';
  created_at: string;
  updated_at: string;
}

export interface PlatformContext {
  workspaceId: SpoolId;
  environmentId: SpoolId;
}

export interface PlatformAccountEnvironment {
  id: SpoolId;
  kind: 'test' | 'live';
}

export interface PlatformAccountEnvironments {
  test: PlatformAccountEnvironment;
  live: PlatformAccountEnvironment;
}

export interface PlatformAccount {
  id: SpoolId;
  external_id: string;
  name: string;
  status: 'active' | 'suspended' | 'cancelled';
  metadata: Record<string, string>;
  environments: PlatformAccountEnvironments;
  created_at: string;
  updated_at: string;
}

export interface UpsertPlatformAccount {
  name: string;
  metadata?: Record<string, string>;
}

/**
 * Server-side grant projected from operator-managed platform configuration.
 * Platform grant provisioning is intentionally not part of the tenant API.
 */
export interface PlatformGrant {
  id: SpoolId;
  service_account_id: SpoolId;
  workspace_id: SpoolId;
  environment_id: SpoolId;
  scopes: ApiKeyScope[];
  expires_at: string | null;
  revoked_at: string | null;
  created_at: string;
}

export interface PlatformServiceAccount {
  id: SpoolId;
  name: string;
  grants: PlatformGrant[];
  created_at: string;
  revoked_at: string | null;
}

export interface CurrentIdentity {
  id: SpoolId;
  email: string;
  name: string | null;
  workspace_id: SpoolId;
  environment_id: SpoolId;
  roles: WorkspaceMember['role'][];
}

export interface LocalOwnerSession {
  /** Opaque secret returned only by exchange or rotation. */
  token: string;
  expires_at: string;
}

export interface BootstrappedLocalOwner {
  /** Long-lived owner credential returned exactly once. */
  credential: string;
  workspace: Workspace;
  member: WorkspaceMember;
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

export interface CreateDeviceAuthorization {
  public_key: string;
  installation_id: string;
  proposed_name: string;
  hostname: string;
  platform: string;
  architecture: string;
  installation_mode: 'user' | 'machine' | 'local';
  agent_version: string;
  protocol_version: number;
}

export interface CreatedDeviceAuthorization {
  id: SpoolId;
  device_code: string;
  user_code: string;
  verification_uri: string;
  expires_in: number;
  interval: number;
}

export interface DeviceAuthorizationStatus {
  id: SpoolId;
  state: 'pending' | 'approved' | 'denied' | 'consumed' | 'expired';
  expires_at: string;
}

export interface DeviceAuthorizationReview {
  id: SpoolId;
  proposed_name: string;
  hostname: string;
  platform: string;
  architecture: string;
  state: 'pending' | 'approved' | 'denied' | 'consumed' | 'expired';
  expires_at: string;
}

export interface DeviceAuthorizationExchange {
  node_id: SpoolId;
  workspace_id: SpoolId;
  environment_id: SpoolId;
  server_time: string;
  sync_after_ms: number;
}

export interface NodeUpdatePolicy {
  channel: 'stable' | 'canary' | 'pinned';
  mode: 'automatic' | 'prompt' | 'disabled';
  pinned_version: string | null;
  maintenance_window: Record<string, unknown> | null;
}

export interface NodeUpdateState {
  current_version: string;
  available_version: string | null;
  state: string;
  download_percent: number | null;
  deferred_reason: string | null;
  last_checked_at: string | null;
  last_success_at: string | null;
  last_error_code: string | null;
  rollback_version: string | null;
}

export interface NodeUpdate {
  node_id: SpoolId;
  policy: NodeUpdatePolicy;
  status: NodeUpdateState;
}

export interface Printer {
  id: SpoolId;
  agent_id: SpoolId;
  name: string;
  state: 'online' | 'offline' | 'paused' | 'busy' | 'paper_out' | 'error' | 'unknown';
  capabilities: PrinterCapabilities;
  /** Monotonic revision of the synced driver capability snapshot. */
  capability_revision: number;
  /** Driver-native option definitions keyed by the stable driver option name. */
  native_options: Record<string, NativePrinterOption>;
  /** Named printer option snapshots synced from the agent. */
  profiles: PrinterProfileSnapshot[];
  updated_at: string;
}

export type PrintRateUnit = 'ppm' | 'ipm' | 'lmp' | 'cpm';

export interface PrintRate {
  unit: PrintRateUnit;
  rate: number;
}

export type PrinterExtent = [number, number];
export type PrinterPaperDimensions = [number | null, number | null];

export interface PrinterCapabilities {
  bins: string[];
  collate: boolean;
  color: boolean;
  copies: number;
  dpis: string[];
  duplex: boolean;
  extent: PrinterExtent[];
  medias: string[];
  nup: number[];
  papers: Record<string, PrinterPaperDimensions>;
  printrate: PrintRate | null;
  supports_custom_paper_size: boolean;
}

export interface NativePrinterChoice {
  value: string;
  display_name: string;
}

export interface NativePrinterOption {
  display_name: string;
  default_choice: string | null;
  selected_choice: string | null;
  choices: NativePrinterChoice[];
}

export interface PrinterProfileSnapshot {
  profile_id: string;
  revision: number;
  name: string;
  is_default: boolean;
  options: JobOptions;
  /** Readiness of the exact immutable native profile revision. */
  status?:
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
  /** Platform replay backend for the opaque native configuration. */
  native_kind?:
    | 'portable_options'
    | 'cups_options'
    | 'cups_instance'
    | 'macos_printcore'
    | 'windows_devmode'
    | 'windows_printticket';
  /** Digest of the node-local opaque native configuration; the blob is never returned. */
  native_digest?: string | null;
  driver_fingerprint?: DriverFingerprint | null;
  summary?: PrintProfileSummary;
  stock_id?: string | null;
  safe_overrides?: string[];
  last_validated_at?: string | null;
  last_test_job_id?: string | null;
  published?: boolean;
}

export interface DriverFingerprint {
  platform: 'windows' | 'macos' | 'linux' | string;
  driver_name: string;
  driver_version: string | null;
  architecture: string | null;
  native_queue_id: string;
  device_fingerprint: string | null;
}

export interface PrintProfileSummary {
  paper?: string | null;
  dimensions_mm?: [number, number] | null;
  source?: string | null;
  media?: string | null;
  color?: string | null;
  duplex?: string | null;
  resolution?: string | null;
  copies?: number | null;
  native?: Record<string, string>;
}

export interface StockSafeAreaMm {
  top: number;
  right: number;
  bottom: number;
  left: number;
}

/**
 * Portable, design-facing stock facts. Vendor-native controls remain opaque
 * inside the node profile and cannot be edited through this object.
 */
export interface StockAttributes {
  kind?: 'sheet' | 'label' | 'roll' | 'continuous' | 'envelope' | 'card' | string;
  width_mm?: number;
  height_mm?: number;
  length_mm?: number;
  orientation?: 'portrait' | 'landscape' | 'either';
  gap_mm?: number;
  mark_interval_mm?: number;
  bleed_mm?: number;
  safe_area_mm?: StockSafeAreaMm;
  source?: string;
  media?: string;
  [key: string]: unknown;
}

export interface Stock {
  id: SpoolId;
  name: string;
  sku: string | null;
  description: string | null;
  attributes: StockAttributes;
  archived: boolean;
  created_at: string;
  updated_at: string;
}

export interface CreateStock {
  name: string;
  sku?: string;
  description?: string;
  attributes?: StockAttributes;
}

export interface PatchStock {
  name?: string;
  sku?: string;
  description?: string;
  attributes?: StockAttributes;
  archived?: boolean;
}

export interface Target {
  id: SpoolId;
  name: string;
  description: string | null;
  stock_id: SpoolId | null;
  enabled: boolean;
  routing_policy: 'primary_then_standby';
  created_at: string;
  updated_at: string;
}

export interface CreateTarget {
  name: string;
  description?: string;
  stock_id?: SpoolId;
  enabled?: boolean;
  routing_policy?: 'primary_then_standby';
}

export interface PatchTarget {
  name?: string;
  description?: string;
  stock_id?: SpoolId;
  clear_stock?: boolean;
  enabled?: boolean;
  routing_policy?: 'primary_then_standby';
}

export interface TargetBinding {
  id: SpoolId;
  target_id: SpoolId;
  printer_id: SpoolId;
  agent_id: SpoolId;
  profile_id: SpoolId;
  profile_revision: number;
  role: 'primary' | 'standby';
  enabled: boolean;
  created_at: string;
  updated_at: string;
}

export interface CreateTargetBinding {
  printer_id: SpoolId;
  profile_id: SpoolId;
  profile_revision: number;
  role: 'primary' | 'standby';
  enabled?: boolean;
}

export type BindingReadinessStatus =
  | 'ready'
  | 'disabled'
  | 'node_offline'
  | 'destination_offline'
  | 'destination_missing'
  | 'needs_operator'
  | 'profile_stale'
  | 'driver_mismatch'
  | 'dependency_missing'
  | 'busy';

export interface BindingReadiness {
  binding: TargetBinding;
  status: BindingReadinessStatus;
  reasons: string[];
}

export interface TargetReadiness {
  target_id: SpoolId;
  status: 'ready' | 'target_has_no_ready_binding';
  selected_binding_id: SpoolId | null;
  bindings: BindingReadiness[];
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
  /** Validated driver-native choices captured by a versioned printer profile. */
  native_options?: Record<string, string>;
}

export type JobContent =
  | { type: 'upload'; upload_id: SpoolId }
  | { type: 'base64'; data: string }
  | { type: 'uri'; uri: string };

export interface CreateJobBase {
  title: string;
  source?: string | null;
  content_type: 'pdf' | 'raw';
  content: JobContent;
  options?: JobOptions;
  deliveries?: number;
  expire_after_seconds?: number;
  metadata?: Record<string, string>;
}

export type CreateJob = CreateJobBase &
  (
    | { printer_id: SpoolId; target_id?: never }
    | { target_id: SpoolId; printer_id?: never }
  );

export interface Job {
  id: SpoolId;
  printer_id: SpoolId;
  title: string;
  source?: string | null;
  content_type: 'pdf' | 'raw';
  deliveries: number;
  state: JobState;
  metadata?: Record<string, string>;
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
