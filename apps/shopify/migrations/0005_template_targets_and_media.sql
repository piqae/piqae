ALTER TABLE shopify_workflow_templates
  ADD COLUMN IF NOT EXISTS design_target_id text,
  ADD COLUMN IF NOT EXISTS design_specification_revision text;

ALTER TABLE shopify_workflow_templates
  DROP CONSTRAINT IF EXISTS shopify_workflow_templates_kind_check,
  DROP CONSTRAINT IF EXISTS shopify_workflow_templates_page_size_check,
  DROP CONSTRAINT IF EXISTS shopify_workflow_templates_design_target_check;

ALTER TABLE shopify_workflow_templates
  ADD CONSTRAINT shopify_workflow_templates_kind_check
    CHECK(kind IN ('invoice','packing_slip','receipt','returns','credit_note','label','custom')),
  ADD CONSTRAINT shopify_workflow_templates_page_size_check
    CHECK(length(page_size) BETWEEN 1 AND 32 AND page_size ~ '^[A-Za-z0-9 .x-]+$'),
  ADD CONSTRAINT shopify_workflow_templates_design_target_check CHECK (
    (design_target_id IS NULL AND design_specification_revision IS NULL)
    OR
    (design_target_id IS NOT NULL
      AND design_specification_revision IS NOT NULL
      AND length(design_target_id) BETWEEN 1 AND 128
      AND length(design_specification_revision) BETWEEN 1 AND 128)
  );

ALTER TABLE shopify_workflow_template_revisions
  ADD COLUMN IF NOT EXISTS design_target_id text,
  ADD COLUMN IF NOT EXISTS design_specification_revision text;

ALTER TABLE shopify_workflow_template_revisions
  DROP CONSTRAINT IF EXISTS shopify_workflow_template_revisions_design_target_check;

ALTER TABLE shopify_workflow_template_revisions
  ADD CONSTRAINT shopify_workflow_template_revisions_design_target_check CHECK (
    (design_target_id IS NULL AND design_specification_revision IS NULL)
    OR
    (design_target_id IS NOT NULL
      AND design_specification_revision IS NOT NULL
      AND length(design_target_id) BETWEEN 1 AND 128
      AND length(design_specification_revision) BETWEEN 1 AND 128)
  );
