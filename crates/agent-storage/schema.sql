CREATE TABLE IF NOT EXISTS schema_migrations (
  version INTEGER PRIMARY KEY,
  applied_unix_ms INTEGER NOT NULL
);

INSERT OR IGNORE INTO schema_migrations (version, applied_unix_ms)
VALUES (1, CAST(unixepoch('subsec') * 1000 AS INTEGER));

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

CREATE TABLE IF NOT EXISTS content_files (
  sha256 TEXT PRIMARY KEY,
  path TEXT NOT NULL,
  reference_count INTEGER NOT NULL CHECK (reference_count >= 0),
  verified_unix_ms INTEGER NOT NULL
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
  prepared_unix_ms INTEGER NOT NULL
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
