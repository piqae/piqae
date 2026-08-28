ALTER TABLE shopify_workflow_template_revisions
  ADD COLUMN IF NOT EXISTS media jsonb;

UPDATE shopify_workflow_template_revisions
SET media = CASE
  WHEN pg_input_is_valid(source, 'jsonb')
    THEN COALESCE(source::jsonb #> '{document,media}', '{"kind":"paged","size":"a4"}'::jsonb)
  ELSE '{"kind":"paged","size":"a4"}'::jsonb
END
WHERE media IS NULL;

ALTER TABLE shopify_workflow_template_revisions
  ALTER COLUMN media SET NOT NULL;

ALTER TABLE shopify_workflow_template_revisions
  DROP CONSTRAINT IF EXISTS shopify_workflow_template_revisions_source_check;

ALTER TABLE shopify_workflow_template_revisions
  ADD CONSTRAINT shopify_workflow_template_revisions_source_check
    CHECK(octet_length(source) <= 262144);

ALTER TABLE shopify_workflow_template_revisions
  DROP CONSTRAINT IF EXISTS shopify_workflow_template_revisions_media_check;

ALTER TABLE shopify_workflow_template_revisions
  ADD CONSTRAINT shopify_workflow_template_revisions_media_check
    CHECK(jsonb_typeof(media) = 'object');

ALTER TABLE shopify_workflow_templates
  ADD COLUMN IF NOT EXISTS draft_source text,
  ADD COLUMN IF NOT EXISTS draft_revision integer,
  ADD COLUMN IF NOT EXISTS published_revision integer;

DO $$
BEGIN
  IF EXISTS (
    SELECT 1 FROM information_schema.columns
    WHERE table_schema = current_schema()
      AND table_name = 'shopify_workflow_templates'
      AND column_name = 'source'
  ) THEN
    EXECUTE $migration$
      UPDATE shopify_workflow_templates
      SET draft_source = source,
          draft_revision = GREATEST(COALESCE(revision, 1), 1)
      WHERE draft_source IS NULL OR draft_revision IS NULL
    $migration$;
    EXECUTE $migration$
      INSERT INTO shopify_workflow_template_revisions(
        template_id,shop,revision,name,kind,page_size,source,
        design_target_id,design_specification_revision,media
      )
      SELECT
        id,shop,revision,name,kind,page_size,source,
        design_target_id,design_specification_revision,
        CASE
          WHEN pg_input_is_valid(source, 'jsonb')
            THEN COALESCE(source::jsonb #> '{document,media}', '{"kind":"paged","size":"a4"}'::jsonb)
          ELSE '{"kind":"paged","size":"a4"}'::jsonb
        END
      FROM shopify_workflow_templates
      WHERE state = 'published'
      ON CONFLICT(template_id,shop,revision) DO NOTHING
    $migration$;
    EXECUTE $migration$
      UPDATE shopify_workflow_templates
      SET published_revision = revision
      WHERE state = 'published' AND published_revision IS NULL
    $migration$;
  END IF;
END $$;

ALTER TABLE shopify_workflow_templates
  ALTER COLUMN draft_source SET NOT NULL,
  ALTER COLUMN draft_revision SET NOT NULL;

ALTER TABLE shopify_workflow_templates
  DROP CONSTRAINT IF EXISTS shopify_workflow_templates_draft_source_check,
  DROP CONSTRAINT IF EXISTS shopify_workflow_templates_draft_revision_check,
  DROP CONSTRAINT IF EXISTS shopify_workflow_templates_published_revision_fkey;

ALTER TABLE shopify_workflow_templates
  ADD CONSTRAINT shopify_workflow_templates_draft_source_check
    CHECK(octet_length(draft_source) <= 262144),
  ADD CONSTRAINT shopify_workflow_templates_draft_revision_check
    CHECK(draft_revision > 0),
  ADD CONSTRAINT shopify_workflow_templates_published_revision_fkey
    FOREIGN KEY(id,shop,published_revision)
    REFERENCES shopify_workflow_template_revisions(template_id,shop,revision)
    DEFERRABLE INITIALLY DEFERRED;
