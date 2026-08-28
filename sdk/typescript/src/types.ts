import type { components } from './generated/schema.js';

export type PiqaeId = string;
export type CapabilityDocument = components['schemas']['CapabilityDocument'];
export type CapabilityFacet = components['schemas']['CapabilityFacet'];
export type DocumentManifest = components['schemas']['DocumentManifest'];
export type PrintIntent = components['schemas']['PrintIntent'];
export type PrintIntentFinding = components['schemas']['PrintIntentFinding'];
export type PrintIntentValidation = components['schemas']['PrintIntentValidation'];
export type ResolvedPrintTicket = components['schemas']['ResolvedPrintTicket'];
export type LoadedMediaObservation = components['schemas']['LoadedMediaObservation'];
export type UpsertLoadedMediaObservation = components['schemas']['UpsertLoadedMediaObservation'];
export type PrintWorkflow = components['schemas']['PrintWorkflow'];
export type CreatePrintWorkflow = components['schemas']['CreatePrintWorkflow'];
export type PhysicalDestination = components['schemas']['PhysicalDestination'];
export type PrinterRoute = components['schemas']['PrinterRoute'];
export type DestinationIdentityEvidence = components['schemas']['DestinationIdentityEvidence'];
export type DestinationIdentityDecision = components['schemas']['DestinationIdentityDecision'];
export type CreateDestinationIdentityDecision = components['schemas']['CreateDestinationIdentityDecision'];
export type RouteObservation = components['schemas']['RouteObservation'];
export type PrivacySafeQueueOccupancy = components['schemas']['PrivacySafeQueueOccupancy'];
export type RouteReservation = components['schemas']['RouteReservation'];
export type DeliveryAttempt = components['schemas']['DeliveryAttempt'];
export type ResolveUncertainDelivery = components['schemas']['ResolveUncertainDelivery'];
export type UncertainDeliveryResolution = components['schemas']['UncertainDeliveryResolution'];
export type NodeRuntimeObservation = components['schemas']['NodeRuntimeObservation'];
export type NodeRuntimeObservationPage = components['schemas']['NodeRuntimeObservationPage'];
export type NodeWakeHint = components['schemas']['NodeWakeHint'];
export type CreateNodeWakeHint = components['schemas']['CreateNodeWakeHint'];

export type PrintPacketV1 = components['schemas']['PrintPacketV1'];
export type PrintPacketNode = components['schemas']['PrintPacketNode'];
export type PrintPacketInline = components['schemas']['PrintPacketInline'];
export type PrintPacketExpression = components['schemas']['PrintPacketExpression'];
export type CreatePrintPacketTemplate = components['schemas']['CreatePrintPacketTemplate'];
export type PrintPacketTemplate = components['schemas']['PrintPacketTemplate'];
export type PrintPacketTemplateRevision = components['schemas']['PrintPacketTemplateRevision'];
export type CreatePrintPacketRender = components['schemas']['CreatePrintPacketRender'];
export type PrintPacketRender = components['schemas']['PrintPacketRender'];
export type PrintPacketRenderPolicy = components['schemas']['PrintPacketRenderPolicy'];
export type PrintPacketRenderCost = components['schemas']['PrintPacketRenderCost'];
export type EvaluatePrintPacketRenderReadiness = components['schemas']['EvaluatePrintPacketRenderReadiness'];
export type PrintPacketRenderReadiness = components['schemas']['PrintPacketRenderReadiness'];
export type PrintPacketPrintRequest =
  & { title: string; options?: JobOptions; deliveries?: number; render_policy?: PrintPacketRenderPolicy; render_cost?: PrintPacketRenderCost }
  & (
    | { printer_id: PiqaeId; target_id?: never; specification_revision?: never }
    | { target_id: PiqaeId; printer_id?: never; specification_revision: string }
  );
export type CreatePrintPacketPreview = components['schemas']['CreatePrintPacketPreview'];
export type PrintPacketPreview = components['schemas']['PrintPacketPreview'];
export type ApprovedPrintPacketPreview = components['schemas']['ApprovedPrintPacketPreview'];

export interface NodeConnector {
  id: string;
  node_id: string;
  permissions: Record<string, unknown>;
  revoked_at: string | null;
  created_at: string;
}

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
  platform: { accounts: boolean };
}

export interface UsageSummary {
  /** Inclusive start of the requested UTC or Stripe subscription period. */
  period_start: string;
  /** Exclusive end of the requested UTC or Stripe subscription period. */
  period_end: string;
  /** Live jobs counted exactly once when the node reports completion. */
  reported_complete_live_jobs: number;
  active_nodes: number;
}

export interface BillingEntitlement {
  included_live_jobs: number;
  node_limit: number;
  metadata_retention_days: number;
  document_retention_hours: number;
  overage_job_unit: number | null;
  overage_price_cents: number | null;
}

export interface BillingSummary {
  /** False for self-hosted and local-only deployments. */
  enabled: boolean;
  /** True when the owning platform workspace holds this workspace's subscription. */
  managed_by_platform: boolean;
  plan: 'free' | 'pro' | null;
  billing_interval: 'monthly' | 'annual' | null;
  subscription_status:
    | 'active'
    | 'trialing'
    | 'past_due'
    | 'unpaid'
    | 'paused'
    | 'cancelled'
    | null;
  grace_ends_at: string | null;
  accept_new_cloud_jobs: boolean;
  entitlement: BillingEntitlement | null;
  usage: UsageSummary;
  overage_live_jobs: number;
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
  id: PiqaeId;
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
  id: PiqaeId;
  name: string;
  slug: string;
  status: 'active' | 'suspended' | 'cancelled';
  created_at: string;
  updated_at: string;
}

export interface WorkspaceMember {
  id: PiqaeId;
  email: string;
  name: string | null;
  role: 'owner' | 'admin' | 'developer' | 'operator' | 'viewer' | 'billing';
  status: 'pending' | 'active' | 'inactive';
  created_at: string;
  updated_at: string;
}

export interface PlatformContext {
  workspaceId: PiqaeId;
  environmentId: PiqaeId;
}

export interface PlatformAccountEnvironment {
  id: PiqaeId;
  kind: 'test' | 'live';
}

export interface PlatformAccountEnvironments {
  test: PlatformAccountEnvironment;
  live: PlatformAccountEnvironment;
}

export interface PlatformAccount {
  id: PiqaeId;
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
  id: PiqaeId;
  service_account_id: PiqaeId;
  workspace_id: PiqaeId;
  environment_id: PiqaeId;
  scopes: ApiKeyScope[];
  expires_at: string | null;
  revoked_at: string | null;
  created_at: string;
}

export interface PlatformServiceAccount {
  id: PiqaeId;
  name: string;
  grants: PlatformGrant[];
  created_at: string;
  revoked_at: string | null;
}

export interface CurrentIdentity {
  id: PiqaeId;
  email: string;
  name: string | null;
  workspace_id: PiqaeId;
  environment_id: PiqaeId;
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
  id: PiqaeId;
  name: string;
  site?: string | null;
  location?: string | null;
  labels?: string[];
  platform: string;
  state: 'connected' | 'disconnected' | 'paused' | 'degraded';
  version: string;
  last_seen_at: string;
  health_started_at?: string | null;
  health_observed_at?: string | null;
  sqlite_integrity_ok?: boolean | null;
  executor_crashes?: number;
  last_error_code?: string | null;
  document_render?: components['schemas']['DocumentRenderCapabilities'];
}

export interface UpdateNodeDetails {
  name?: string;
  site?: string | null;
  location?: string | null;
  labels?: string[];
}

export interface DiagnosticReport {
  request_id: string;
  observed_at: string;
  state: 'complete' | 'failed';
  agent_version: string;
  platform: string;
  architecture: string;
  queued_jobs: number;
  active_jobs: number;
  sqlite_integrity_ok: boolean;
  executor_crashes: number;
  last_error_code: string | null;
  collection_error_code: string | null;
}

export interface NodeDiagnostic {
  request_id: string;
  node_id: PiqaeId;
  state: 'requested' | 'complete' | 'failed';
  report: DiagnosticReport | null;
  requested_at: string;
  received_at: string | null;
  expires_at: string;
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
  id: PiqaeId;
  device_code: string;
  user_code: string;
  verification_uri: string;
  expires_in: number;
  interval: number;
}

export interface DeviceAuthorizationStatus {
  id: PiqaeId;
  state: 'pending' | 'approved' | 'denied' | 'consumed' | 'expired';
  expires_at: string;
}

export interface DeviceAuthorizationReview {
  id: PiqaeId;
  proposed_name: string;
  hostname: string;
  platform: string;
  architecture: string;
  state: 'pending' | 'approved' | 'denied' | 'consumed' | 'expired';
  expires_at: string;
  /**
   * The node whose device key this approval would replace, when the request
   * comes from an installation already paired to this workspace. Null when
   * approving admits a new node.
   */
  replaces_node_id?: PiqaeId | null;
}

export interface DeviceAuthorizationExchange {
  node_id: PiqaeId;
  workspace_id: PiqaeId;
  environment_id: PiqaeId;
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
  node_id: PiqaeId;
  policy: NodeUpdatePolicy;
  status: NodeUpdateState;
}

export interface CreateNodeConnectSession {
  /** Optional operator label; the node uses its computer name when omitted. */
  name?: string;
  return_url?: string;
  expires_in_seconds?: number;
}

export interface NodeConnectSession {
  id: string;
  state: 'pending' | 'connected' | 'expired';
  expires_at: string;
  node_id: PiqaeId | null;
  connect_url?: string | null;
  native_connect_url?: string | null;
  return_url?: string | null;
  downloads: Array<{ platform: 'macos' | 'windows' | 'linux'; url: string }>;
}

export interface Printer {
  id: PiqaeId;
  agent_id: PiqaeId;
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

export interface NodeContentEncryptionKey {
  key_id: string;
  algorithm: 'ECDH-P256-HKDF-SHA256';
  public_key_spki: string;
  node_id: PiqaeId;
  lifecycle_state: 'active' | 'decrypt_only' | 'revoked';
  state_changed_at: string;
  created_at: string;
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
  kind?: 'sheet' | 'label' | 'roll' | 'continuous' | 'envelope' | 'card';
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
  id: PiqaeId;
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
  id: PiqaeId;
  name: string;
  description: string | null;
  stock_id: PiqaeId | null;
  enabled: boolean;
  routing_policy: 'primary_then_standby';
  created_at: string;
  updated_at: string;
}

export interface CreateTarget {
  name: string;
  description?: string;
  stock_id?: PiqaeId;
  enabled?: boolean;
  routing_policy?: 'primary_then_standby';
}

export interface PatchTarget {
  name?: string;
  description?: string;
  stock_id?: PiqaeId;
  clear_stock?: boolean;
  enabled?: boolean;
  routing_policy?: 'primary_then_standby';
}

export interface TargetBinding {
  id: PiqaeId;
  target_id: PiqaeId;
  printer_id: PiqaeId;
  agent_id: PiqaeId;
  profile_id: PiqaeId;
  profile_revision: number;
  destination_id: PiqaeId | null;
  route_id: PiqaeId | null;
  role: 'primary' | 'standby';
  enabled: boolean;
  created_at: string;
  updated_at: string;
}

export interface CreateTargetBinding {
  printer_id: PiqaeId;
  profile_id: PiqaeId;
  profile_revision: number;
  destination_id: PiqaeId;
  route_id: PiqaeId;
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
  target_id: PiqaeId;
  status: 'ready' | 'target_has_no_ready_binding';
  selected_binding_id: PiqaeId | null;
  bindings: BindingReadiness[];
}

export interface DesignSpecificationDestination {
  binding: TargetBinding;
  printer: Printer;
  /** Exact immutable profile revision selected by the binding. */
  profile: PrinterProfileSnapshot;
  media_compatibility: TargetMediaCompatibility;
}

export interface TargetMediaDimensions {
  width_mm: number;
  height_mm: number;
}

export interface TargetLoadedMediaEvidence {
  source: string;
  confidence: 'reported' | 'operator_confirmed' | 'inferred' | 'unknown';
  observed_at: string;
  /** After this instant the observation cannot authorize a new native handoff. */
  fresh_until: string;
  stock: { id: PiqaeId; revision: number } | null;
}

export interface TargetMediaCompatibility {
  status: 'ready' | 'not_reported' | 'stale' | 'untrusted' | 'incompatible';
  reasons: string[];
  profile_dimensions_mm: TargetMediaDimensions | null;
  loaded_media: TargetLoadedMediaEvidence | null;
}

/** One read model for sizing an editor canvas and checking production readiness. */
export interface DesignSpecification {
  target: Target;
  stock: Stock | null;
  readiness: TargetReadiness;
  destinations: DesignSpecificationDestination[];
  /** Changes when any design or production constraint in this projection changes. */
  specification_revision: string;
}

export interface JobListOptions extends ListOptions {
  state?: JobState;
  printer_id?: PiqaeId;
  target_id?: string;
  metadata_key?: string;
  metadata_value?: string;
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
  | { type: 'upload'; upload_id: PiqaeId }
  | {
      type: 'encrypted_upload';
      upload_id: PiqaeId;
      manifest: import('./encrypted-jobs.js').EncryptedJobManifest;
    }
  | { type: 'base64'; data: string }
  | { type: 'uri'; uri: string };

export type PrinterNativeJobContent = Exclude<JobContent, { type: 'encrypted_upload' }>;

export interface CreateJobBase {
  title: string;
  source?: string | null;
  content: JobContent;
  options?: JobOptions;
  deliveries?: number;
  expire_after_seconds?: number;
  metadata?: Record<string, string>;
  /** Short-lived digest returned by printIntents.resolve. */
  resolved_ticket_digest?: string;
}

export interface PrinterNativeJobDescriptor {
  output_profile_id: string;
  language_profile_id: string;
}

export type CreateJob = CreateJobBase &
  (
    | { printer_id: PiqaeId; target_id?: never }
    | { target_id: PiqaeId; printer_id?: never }
  ) &
  (
    | { content_type: 'pdf'; printer_native?: never; content: JobContent }
    | {
        content_type: 'raw';
        printer_native: PrinterNativeJobDescriptor;
        content: PrinterNativeJobContent;
        options?: never;
      }
  );

export interface Job {
  id: PiqaeId;
  printer_id: PiqaeId;
  title: string;
  source?: string | null;
  content_type: 'pdf' | 'raw';
  deliveries: number;
  state: JobState;
  metadata?: Record<string, string>;
  created_at: string;
  expires_at: string;
  /** Server timestamp for the transition into `delivery_uncertain`. */
  delivery_uncertain_since?: string;
}

export interface JobEvent {
  id: PiqaeId;
  job_id: PiqaeId;
  sequence: number;
  state: JobState;
  reason?: string | null;
  message?: string | null;
  occurred_at: string;
}

export interface Webhook {
  id: PiqaeId;
  url: string;
  events: string[];
  enabled: boolean;
  created_at: string;
}

export interface WebhookDelivery {
  id: PiqaeId;
  event_id: PiqaeId;
  attempt: number;
  next_attempt_at: string;
  response_status: number | null;
  delivered_at: string | null;
  dead_lettered_at: string | null;
}

export interface AgentEnrolment {
  id: PiqaeId;
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
