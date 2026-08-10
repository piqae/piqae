-- Durable document rendering claims. A lease is deliberately independent of
-- encryption key selection: queued ciphertext retains its key identifier and
-- can be decrypted throughout a key rotation.
ALTER TABLE document_renders
    ADD COLUMN attempt integer NOT NULL DEFAULT 0 CHECK (attempt BETWEEN 0 AND 20),
    ADD COLUMN max_attempts integer NOT NULL DEFAULT 5 CHECK (max_attempts BETWEEN 1 AND 20),
    ADD COLUMN available_at timestamptz NOT NULL DEFAULT now(),
    ADD COLUMN lease_owner text CHECK (lease_owner IS NULL OR char_length(lease_owner) BETWEEN 1 AND 120),
    ADD COLUMN lease_token uuid,
    ADD COLUMN lease_expires_at timestamptz,
    ADD COLUMN completed_at timestamptz,
    ADD COLUMN expires_at timestamptz NOT NULL DEFAULT (now() + interval '30 days'),
    ADD COLUMN last_failure_code text CHECK (last_failure_code IS NULL OR char_length(last_failure_code) BETWEEN 1 AND 80),
    ADD CONSTRAINT document_render_lease_shape CHECK (
        (lease_owner IS NULL AND lease_token IS NULL AND lease_expires_at IS NULL)
        OR (lease_owner IS NOT NULL AND lease_token IS NOT NULL AND lease_expires_at IS NOT NULL)
    );

ALTER TABLE document_renders DROP CONSTRAINT document_renders_state_check;
ALTER TABLE document_renders ADD CONSTRAINT document_renders_state_check
    CHECK (state IN ('registered', 'rendering', 'completed', 'failed_terminal', 'expiring', 'expired'));

DROP INDEX document_renders_pending_idx;
CREATE INDEX document_renders_claimable_idx
    ON document_renders (available_at, created_at, id)
    WHERE state IN ('registered', 'rendering');

CREATE INDEX document_renders_expiry_idx
    ON document_renders (expires_at, id)
    WHERE state IN ('completed', 'failed_terminal', 'expiring');
