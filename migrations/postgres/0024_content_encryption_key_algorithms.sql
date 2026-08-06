ALTER TABLE node_content_encryption_keys
    DROP CONSTRAINT node_content_encryption_keys_algorithm_check;

ALTER TABLE node_content_encryption_keys
    ADD CONSTRAINT node_content_encryption_keys_algorithm_check
    CHECK (algorithm IN ('RSA-OAEP-256', 'ECDH-P256-HKDF-SHA256'));

ALTER TABLE node_content_encryption_keys
    DROP CONSTRAINT node_content_encryption_keys_public_key_spki_check;

ALTER TABLE node_content_encryption_keys
    ADD CONSTRAINT node_content_encryption_keys_public_key_spki_check
    CHECK (length(public_key_spki) BETWEEN 80 AND 4096);
