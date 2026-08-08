DO $migration$
DECLARE
    owning_schema name := current_schema();
BEGIN
    EXECUTE format($function$
        CREATE OR REPLACE FUNCTION %1$I.guard_encrypted_job_key_reference()
        RETURNS trigger
        LANGUAGE plpgsql
        SET search_path = %1$I, pg_catalog, pg_temp
        AS $body$
        DECLARE
            key_state text;
        BEGIN
            SELECT lifecycle_state INTO key_state
            FROM %1$I.node_content_encryption_keys
            WHERE workspace_id = NEW.workspace_id
              AND environment_id = NEW.environment_id
              AND agent_id = NEW.agent_id
              AND key_id = NEW.key_id
            FOR UPDATE;

            IF key_state IS NULL OR key_state NOT IN ('active', 'decrypt_only') THEN
                RAISE EXCEPTION 'encrypted jobs require an active or decrypt-only recipient key';
            END IF;
            RETURN NEW;
        END;
        $body$
    $function$, owning_schema);
END;
$migration$;
