ALTER TABLE printers
    ADD COLUMN native_options jsonb NOT NULL DEFAULT '{}'::jsonb
        CHECK (jsonb_typeof(native_options) = 'object'),
    ADD COLUMN profiles jsonb NOT NULL DEFAULT '[]'::jsonb
        CHECK (jsonb_typeof(profiles) = 'array');
