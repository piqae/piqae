ALTER TABLE node_connectors
    ADD CONSTRAINT node_connectors_printer_grant_valid CHECK (
        (jsonb_typeof(permissions->'printers') = 'string'
            AND permissions->>'printers' = 'all')
        OR
        (jsonb_typeof(permissions->'printers') = 'array'
            AND jsonb_array_length(permissions->'printers') BETWEEN 1 AND 128)
    );

