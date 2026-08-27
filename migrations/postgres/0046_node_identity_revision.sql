ALTER TABLE agents
    ADD COLUMN identity_site text,
    ADD COLUMN identity_location text,
    ADD COLUMN identity_labels jsonb NOT NULL DEFAULT '[]'::jsonb,
    ADD COLUMN identity_revision bigint NOT NULL DEFAULT 1;

CREATE FUNCTION piqae_valid_node_identity_labels_v1(labels jsonb)
RETURNS boolean
LANGUAGE sql
IMMUTABLE
STRICT
AS $$
    SELECT jsonb_typeof(labels) = 'array'
       AND jsonb_array_length(labels) <= 16
       AND NOT EXISTS (
           SELECT 1
           FROM jsonb_array_elements(labels) AS item(value)
           WHERE jsonb_typeof(value) <> 'string'
              OR octet_length(value #>> '{}') NOT BETWEEN 1 AND 64
              OR value #>> '{}' <> btrim(value #>> '{}')
              OR value #>> '{}' ~ '[[:cntrl:]]'
       )
       AND (
           SELECT count(*) = count(DISTINCT value #>> '{}')
           FROM jsonb_array_elements(labels) AS item(value)
       );
$$;

ALTER TABLE agents
    ADD CONSTRAINT agents_identity_site_bounded
        CHECK (identity_site IS NULL OR (
            octet_length(identity_site) BETWEEN 1 AND 120
            AND identity_site = btrim(identity_site)
            AND identity_site !~ '[[:cntrl:]]'
        )),
    ADD CONSTRAINT agents_identity_location_bounded
        CHECK (identity_location IS NULL OR (
            octet_length(identity_location) BETWEEN 1 AND 120
            AND identity_location = btrim(identity_location)
            AND identity_location !~ '[[:cntrl:]]'
        )),
    ADD CONSTRAINT agents_identity_labels_valid
        CHECK (piqae_valid_node_identity_labels_v1(identity_labels)),
    ADD CONSTRAINT agents_identity_revision_positive
        CHECK (identity_revision > 0);
