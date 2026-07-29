ALTER TABLE workspaces
    ADD COLUMN platform_service_account_id text
        REFERENCES platform_service_accounts(id) ON DELETE RESTRICT,
    ADD COLUMN platform_external_id text,
    ADD COLUMN platform_metadata jsonb NOT NULL DEFAULT '{}'::jsonb
        CHECK (jsonb_typeof(platform_metadata) = 'object'),
    ADD CONSTRAINT workspaces_platform_owner_pair_check CHECK (
        (platform_service_account_id IS NULL) = (platform_external_id IS NULL)
    ),
    ADD CONSTRAINT workspaces_platform_external_id_check CHECK (
        platform_external_id IS NULL
        OR (
            char_length(platform_external_id) BETWEEN 1 AND 120
            AND platform_external_id ~ '^[A-Za-z0-9][A-Za-z0-9_.:-]*$'
        )
    );

ALTER TABLE platform_service_accounts
    ADD COLUMN owner_workspace_id text REFERENCES workspaces(id) ON DELETE RESTRICT;

UPDATE platform_service_accounts account
SET owner_workspace_id = (
    SELECT grant_row.workspace_id
    FROM platform_workspace_grants grant_row
    WHERE grant_row.service_account_id = account.id
    ORDER BY grant_row.created_at, grant_row.id
    LIMIT 1
);

ALTER TABLE platform_service_accounts
    ALTER COLUMN owner_workspace_id SET NOT NULL;

CREATE UNIQUE INDEX platform_service_accounts_owner_workspace_unique
    ON platform_service_accounts (owner_workspace_id)
    WHERE revoked_at IS NULL;

CREATE UNIQUE INDEX workspaces_platform_external_id_unique
    ON workspaces (platform_service_account_id, platform_external_id)
    WHERE platform_service_account_id IS NOT NULL;
