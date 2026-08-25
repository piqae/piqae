-- A physical node can be authorized into several isolated workspaces. Its
-- stable printer IDs therefore identify a printer within a tenant, not across
-- the whole control plane.

ALTER TABLE jobs
    DROP CONSTRAINT jobs_printer_id_fkey;

ALTER TABLE target_bindings
    DROP CONSTRAINT target_bindings_printer_id_fkey;

ALTER TABLE printers
    DROP CONSTRAINT printers_pkey;

ALTER TABLE printers
    ADD CONSTRAINT printers_pkey
    PRIMARY KEY (workspace_id, environment_id, id);

ALTER TABLE jobs
    ADD CONSTRAINT jobs_printer_tenant_fkey
    FOREIGN KEY (workspace_id, environment_id, printer_id)
    REFERENCES printers(workspace_id, environment_id, id);

ALTER TABLE target_bindings
    ADD CONSTRAINT target_bindings_printer_tenant_fkey
    FOREIGN KEY (workspace_id, environment_id, printer_id)
    REFERENCES printers(workspace_id, environment_id, id);
