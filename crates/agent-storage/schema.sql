CREATE TABLE IF NOT EXISTS schema_migrations (
  version INTEGER PRIMARY KEY,
  applied_unix_ms INTEGER NOT NULL
);

INSERT OR IGNORE INTO schema_migrations (version, applied_unix_ms)
VALUES (1, CAST(unixepoch('subsec') * 1000 AS INTEGER));

INSERT OR IGNORE INTO schema_migrations (version, applied_unix_ms)
VALUES (2, CAST(unixepoch('subsec') * 1000 AS INTEGER));

INSERT OR IGNORE INTO schema_migrations (version, applied_unix_ms)
VALUES (3, CAST(unixepoch('subsec') * 1000 AS INTEGER));

INSERT OR IGNORE INTO schema_migrations (version, applied_unix_ms)
VALUES (4, CAST(unixepoch('subsec') * 1000 AS INTEGER));

INSERT OR IGNORE INTO schema_migrations (version, applied_unix_ms)
VALUES (5, CAST(unixepoch('subsec') * 1000 AS INTEGER));

CREATE TABLE IF NOT EXISTS document_resources (
  digest TEXT PRIMARY KEY CHECK (
    length(digest) = 64 AND digest NOT GLOB '*[^0-9a-f]*'
  ),
  media_type TEXT NOT NULL CHECK (length(trim(media_type)) > 0),
  byte_length INTEGER NOT NULL CHECK (byte_length >= 0),
  relative_path TEXT NOT NULL UNIQUE CHECK (length(trim(relative_path)) > 0),
  verified_unix_ms INTEGER NOT NULL,
  last_accessed_unix_ms INTEGER NOT NULL,
  reference_count INTEGER NOT NULL DEFAULT 0 CHECK (reference_count >= 0),
  evicting INTEGER NOT NULL DEFAULT 0 CHECK (evicting IN (0, 1))
);

CREATE INDEX IF NOT EXISTS document_resources_lru
  ON document_resources (reference_count, last_accessed_unix_ms, digest);

CREATE TABLE IF NOT EXISTS identity (
  key TEXT PRIMARY KEY,
  value BLOB NOT NULL,
  updated_unix_ms INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS settings (
  key TEXT PRIMARY KEY,
  value_json TEXT NOT NULL CHECK (json_valid(value_json)),
  updated_unix_ms INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS printers (
  printer_id TEXT PRIMARY KEY,
  native_id TEXT NOT NULL UNIQUE,
  name TEXT NOT NULL,
  state TEXT NOT NULL,
  is_default INTEGER NOT NULL DEFAULT 0 CHECK (is_default IN (0, 1)),
  present INTEGER NOT NULL DEFAULT 1 CHECK (present IN (0, 1)),
  observed_unix_ms INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS printer_capabilities (
  printer_id TEXT NOT NULL REFERENCES printers(printer_id) ON DELETE CASCADE,
  revision TEXT NOT NULL,
  capabilities_json TEXT NOT NULL CHECK (json_valid(capabilities_json)),
  observed_unix_ms INTEGER NOT NULL,
  PRIMARY KEY (printer_id, revision)
);

CREATE TABLE IF NOT EXISTS printer_sequences (
  printer_id TEXT PRIMARY KEY,
  next_sequence INTEGER NOT NULL CHECK (next_sequence > 0)
);

CREATE TABLE IF NOT EXISTS printer_exposure (
  printer_id TEXT PRIMARY KEY REFERENCES printers(printer_id) ON DELETE CASCADE,
  exposed INTEGER NOT NULL DEFAULT 0 CHECK (exposed IN (0, 1)),
  updated_unix_ms INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS printer_capability_snapshots (
  printer_id TEXT NOT NULL REFERENCES printers(printer_id) ON DELETE CASCADE,
  revision INTEGER NOT NULL CHECK (revision > 0),
  schema_version INTEGER NOT NULL,
  portable_json TEXT NOT NULL CHECK (json_valid(portable_json)),
  native_options_json TEXT NOT NULL CHECK (json_valid(native_options_json)),
  observed_unix_ms INTEGER NOT NULL,
  PRIMARY KEY (printer_id, revision)
);

CREATE TABLE IF NOT EXISTS printer_profiles (
  profile_id TEXT NOT NULL,
  printer_id TEXT NOT NULL REFERENCES printers(printer_id) ON DELETE CASCADE,
  revision INTEGER NOT NULL CHECK (revision > 0),
  name TEXT NOT NULL CHECK (length(trim(name)) > 0),
  is_default INTEGER NOT NULL DEFAULT 0 CHECK (is_default IN (0, 1)),
  options_json TEXT NOT NULL CHECK (json_valid(options_json)),
  status TEXT NOT NULL DEFAULT 'needs_test',
  native_kind TEXT NOT NULL DEFAULT 'portable_options',
  native_blob_id TEXT,
  native_digest TEXT,
  driver_fingerprint_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(driver_fingerprint_json)),
  summary_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(summary_json)),
  stock_id TEXT,
  safe_overrides_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(safe_overrides_json)),
  last_validated_unix_ms INTEGER,
  last_test_job_id TEXT,
  published INTEGER NOT NULL DEFAULT 0 CHECK (published IN (0, 1)),
  deleted INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1)),
  updated_unix_ms INTEGER NOT NULL,
  PRIMARY KEY (profile_id, revision)
);

CREATE INDEX IF NOT EXISTS printer_profiles_latest
  ON printer_profiles (printer_id, profile_id, revision DESC);

CREATE TABLE IF NOT EXISTS physical_devices (
  device_id TEXT PRIMARY KEY,
  display_name TEXT NOT NULL,
  hardware_fingerprint TEXT,
  identity_confidence TEXT NOT NULL DEFAULT 'unknown',
  metadata_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(metadata_json)),
  updated_unix_ms INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS printer_device_bindings (
  printer_id TEXT PRIMARY KEY REFERENCES printers(printer_id) ON DELETE CASCADE,
  device_id TEXT NOT NULL REFERENCES physical_devices(device_id) ON DELETE CASCADE,
  binding_confidence TEXT NOT NULL,
  confirmed_by TEXT,
  updated_unix_ms INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS profile_native_blobs (
  blob_id TEXT PRIMARY KEY,
  profile_id TEXT NOT NULL,
  profile_revision INTEGER NOT NULL CHECK (profile_revision > 0),
  native_kind TEXT NOT NULL,
  schema_version INTEGER NOT NULL CHECK (schema_version > 0),
  digest TEXT NOT NULL,
  native_blob BLOB NOT NULL,
  created_unix_ms INTEGER NOT NULL,
  UNIQUE (profile_id, profile_revision),
  FOREIGN KEY (profile_id, profile_revision)
    REFERENCES printer_profiles(profile_id, revision) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS profile_native_blobs_digest
  ON profile_native_blobs (digest);

CREATE TABLE IF NOT EXISTS profile_dependencies (
  profile_id TEXT NOT NULL,
  profile_revision INTEGER NOT NULL CHECK (profile_revision > 0),
  dependency_index INTEGER NOT NULL CHECK (dependency_index >= 0),
  kind TEXT NOT NULL,
  value TEXT NOT NULL,
  PRIMARY KEY (profile_id, profile_revision, dependency_index),
  FOREIGN KEY (profile_id, profile_revision)
    REFERENCES printer_profiles(profile_id, revision) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS stocks (
  stock_id TEXT PRIMARY KEY,
  name TEXT NOT NULL CHECK (length(trim(name)) > 0),
  sku TEXT,
  kind TEXT NOT NULL,
  definition_json TEXT NOT NULL CHECK (json_valid(definition_json)),
  retired INTEGER NOT NULL DEFAULT 0 CHECK (retired IN (0, 1)),
  updated_unix_ms INTEGER NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS stocks_sku_active
  ON stocks (sku) WHERE sku IS NOT NULL AND retired = 0;

CREATE TABLE IF NOT EXISTS loaded_media (
  device_id TEXT NOT NULL REFERENCES physical_devices(device_id) ON DELETE CASCADE,
  source TEXT NOT NULL,
  stock_id TEXT REFERENCES stocks(stock_id),
  confidence TEXT NOT NULL,
  confirmed_unix_ms INTEGER NOT NULL,
  confirmed_by TEXT,
  PRIMARY KEY (device_id, source)
);

CREATE TABLE IF NOT EXISTS targets (
  target_id TEXT PRIMARY KEY,
  name TEXT NOT NULL CHECK (length(trim(name)) > 0),
  stock_id TEXT REFERENCES stocks(stock_id),
  routing_policy TEXT NOT NULL DEFAULT 'primary_only',
  published INTEGER NOT NULL DEFAULT 0 CHECK (published IN (0, 1)),
  retired INTEGER NOT NULL DEFAULT 0 CHECK (retired IN (0, 1)),
  updated_unix_ms INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS target_bindings (
  binding_id TEXT PRIMARY KEY,
  target_id TEXT NOT NULL REFERENCES targets(target_id) ON DELETE CASCADE,
  agent_id TEXT NOT NULL,
  printer_id TEXT NOT NULL REFERENCES printers(printer_id) ON DELETE CASCADE,
  profile_id TEXT NOT NULL,
  profile_revision INTEGER NOT NULL CHECK (profile_revision > 0),
  role TEXT NOT NULL,
  priority INTEGER NOT NULL DEFAULT 0 CHECK (priority >= 0),
  enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
  created_unix_ms INTEGER NOT NULL,
  FOREIGN KEY (profile_id, profile_revision)
    REFERENCES printer_profiles(profile_id, revision)
);

CREATE INDEX IF NOT EXISTS target_bindings_route
  ON target_bindings (target_id, enabled DESC, priority, binding_id);

CREATE TABLE IF NOT EXISTS profile_validation_events (
  validation_id TEXT PRIMARY KEY,
  profile_id TEXT NOT NULL,
  profile_revision INTEGER NOT NULL CHECK (profile_revision > 0),
  status TEXT NOT NULL,
  code TEXT,
  summary_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(summary_json)),
  observed_unix_ms INTEGER NOT NULL,
  FOREIGN KEY (profile_id, profile_revision)
    REFERENCES printer_profiles(profile_id, revision) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS profile_capture_sessions (
  session_id TEXT PRIMARY KEY,
  token_digest TEXT NOT NULL,
  printer_id TEXT NOT NULL REFERENCES printers(printer_id) ON DELETE CASCADE,
  profile_id TEXT,
  expected_revision INTEGER,
  operation TEXT NOT NULL,
  status TEXT NOT NULL,
  peer_user_id TEXT NOT NULL,
  expires_unix_ms INTEGER NOT NULL,
  created_unix_ms INTEGER NOT NULL,
  completed_unix_ms INTEGER
);

CREATE INDEX IF NOT EXISTS profile_capture_sessions_expiry
  ON profile_capture_sessions (status, expires_unix_ms);

CREATE TABLE IF NOT EXISTS content_files (
  sha256 TEXT PRIMARY KEY,
  path TEXT NOT NULL,
  reference_count INTEGER NOT NULL CHECK (reference_count >= 0),
  verified_unix_ms INTEGER NOT NULL,
  reclaiming INTEGER NOT NULL DEFAULT 0 CHECK (reclaiming IN (0, 1))
);

CREATE TABLE IF NOT EXISTS jobs (
  job_id TEXT PRIMARY KEY,
  submission_id TEXT NOT NULL,
  printer_id TEXT NOT NULL,
  printer_native_id TEXT NOT NULL,
  printer_sequence INTEGER NOT NULL,
  title TEXT NOT NULL,
  content_sha256 TEXT NOT NULL REFERENCES content_files(sha256),
  content_path TEXT NOT NULL,
  content_kind TEXT NOT NULL CHECK (content_kind IN ('pdf', 'raw')),
  options_json TEXT NOT NULL CHECK (json_valid(options_json)),
  state TEXT NOT NULL,
  expires_unix_ms INTEGER,
  next_attempt_unix_ms INTEGER,
  attempt_count INTEGER NOT NULL DEFAULT 0,
  native_job_id TEXT,
  accepted_unix_ms INTEGER NOT NULL,
  updated_unix_ms INTEGER NOT NULL,
  cloud_managed INTEGER NOT NULL DEFAULT 0 CHECK (cloud_managed IN (0, 1)),
  confidential INTEGER NOT NULL DEFAULT 0 CHECK (confidential IN (0, 1)),
  confidential_delete_after_unix_ms INTEGER,
  target_id TEXT,
  binding_id TEXT,
  profile_id TEXT,
  profile_revision INTEGER,
  stock_id TEXT,
  loaded_media_snapshot_json TEXT CHECK (
    loaded_media_snapshot_json IS NULL OR json_valid(loaded_media_snapshot_json)
  ),
  UNIQUE (printer_id, printer_sequence)
);

CREATE INDEX IF NOT EXISTS jobs_runnable
  ON jobs (state, next_attempt_unix_ms, expires_unix_ms);
CREATE INDEX IF NOT EXISTS jobs_printer_order
  ON jobs (printer_id, printer_sequence);

CREATE TABLE IF NOT EXISTS inbox_receipts (
  receipt_id TEXT PRIMARY KEY,
  job_id TEXT NOT NULL UNIQUE REFERENCES jobs(job_id) DEFERRABLE INITIALLY DEFERRED,
  content_sha256 TEXT NOT NULL,
  received_unix_ms INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS cloud_accept_intents (
  job_id TEXT PRIMARY KEY REFERENCES jobs(job_id) ON DELETE CASCADE,
  lease_id TEXT NOT NULL,
  lease_token TEXT NOT NULL,
  lease_expires_unix_ms INTEGER NOT NULL,
  content_sha256 TEXT NOT NULL,
  local_sequence INTEGER NOT NULL CHECK (local_sequence > 0),
  route_reservation_id TEXT,
  route_generation INTEGER,
  route_fencing_token TEXT,
  acceptance_state TEXT NOT NULL DEFAULT 'prepared'
    CHECK (acceptance_state IN ('prepared', 'remote_accept_confirmed')),
  prepared_unix_ms INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS cloud_release_cleanups (
  job_id TEXT PRIMARY KEY REFERENCES jobs(job_id) ON DELETE CASCADE,
  lease_id TEXT NOT NULL,
  lease_token TEXT NOT NULL,
  reason TEXT NOT NULL,
  quarantined_unix_ms INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS job_submissions (
  submission_id TEXT PRIMARY KEY,
  job_id TEXT NOT NULL REFERENCES jobs(job_id) ON DELETE CASCADE,
  quantity_index INTEGER NOT NULL DEFAULT 0,
  native_job_id TEXT,
  state TEXT NOT NULL DEFAULT 'pending',
  UNIQUE (job_id, quantity_index)
);

CREATE TABLE IF NOT EXISTS job_events (
  event_id TEXT PRIMARY KEY,
  job_id TEXT NOT NULL REFERENCES jobs(job_id) ON DELETE CASCADE,
  job_sequence INTEGER NOT NULL,
  state TEXT NOT NULL,
  reason TEXT,
  message TEXT,
  details_json TEXT NOT NULL CHECK (json_valid(details_json)),
  observed_unix_ms INTEGER NOT NULL,
  UNIQUE (job_id, job_sequence)
);

CREATE TABLE IF NOT EXISTS event_outbox (
  outbox_sequence INTEGER PRIMARY KEY AUTOINCREMENT,
  event_id TEXT NOT NULL UNIQUE REFERENCES job_events(event_id) ON DELETE CASCADE,
  job_id TEXT NOT NULL,
  job_sequence INTEGER NOT NULL,
  state TEXT NOT NULL,
  reason TEXT,
  message TEXT,
  details_json TEXT NOT NULL CHECK (json_valid(details_json)),
  observed_unix_ms INTEGER NOT NULL,
  acknowledged_unix_ms INTEGER
);

CREATE INDEX IF NOT EXISTS event_outbox_pending
  ON event_outbox (acknowledged_unix_ms, outbox_sequence);

CREATE TABLE IF NOT EXISTS native_observations (
  observation_id TEXT PRIMARY KEY,
  job_id TEXT NOT NULL REFERENCES jobs(job_id) ON DELETE CASCADE,
  native_job_id TEXT,
  state TEXT NOT NULL,
  authority TEXT NOT NULL,
  details_json TEXT NOT NULL CHECK (json_valid(details_json)),
  observed_unix_ms INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS job_reconciliation (
  job_id TEXT PRIMARY KEY REFERENCES jobs(job_id) ON DELETE CASCADE,
  next_observe_unix_ms INTEGER NOT NULL,
  uncertainty_deadline_unix_ms INTEGER NOT NULL,
  attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
  last_native_state TEXT,
  last_error_code TEXT,
  cancel_requested INTEGER NOT NULL DEFAULT 0 CHECK (cancel_requested IN (0, 1))
);

CREATE INDEX IF NOT EXISTS job_reconciliation_due
  ON job_reconciliation (next_observe_unix_ms);
