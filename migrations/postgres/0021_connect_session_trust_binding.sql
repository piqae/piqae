ALTER TABLE enrolment_tokens
    ADD COLUMN return_url text,
    ADD COLUMN requesting_service_account_id text
        REFERENCES platform_service_accounts(id) ON DELETE SET NULL,
    ADD COLUMN requesting_service_name text;

ALTER TABLE enrolment_tokens
    ADD CONSTRAINT enrolment_tokens_return_url_length
        CHECK (return_url IS NULL OR char_length(return_url) <= 2048),
    ADD CONSTRAINT enrolment_tokens_requesting_service_name_length
        CHECK (requesting_service_name IS NULL
               OR char_length(requesting_service_name) BETWEEN 1 AND 120);
