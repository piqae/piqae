ALTER TABLE document_renders
    ADD COLUMN warnings TEXT[] NOT NULL DEFAULT '{}',
    ADD CONSTRAINT document_renders_warnings_bounded
        CHECK (
            cardinality(warnings) <= 20
            AND array_position(warnings, NULL) IS NULL
        );
