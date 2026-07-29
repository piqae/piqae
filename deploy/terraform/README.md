# Hosted infrastructure

This module deploys the Spool control plane to Cloud Run in Sydney. Neon,
Cloudflare R2, WorkOS, and Vercel remain separately managed services; their
credentials are passed to this module as secrets.

`webhook_master_key_secret` must be a base64-encoded 32-byte random key. The
module stores it, the Neon connection URL, and both R2 credentials in Secret
Manager; none are emitted as Terraform outputs.

Production uses three always-allocated instances so API, agent long-polling,
outbox workers, and webhook delivery continue without a cold start. PostgreSQL
leases and transactional outboxes make every replica safe to run the combined
`all` role.

Hosted authentication defaults to OIDC. For WorkOS AuthKit, set
`oidc_jwks_url` to the application's HTTPS signing JWKS endpoint and
`oidc_binding_value` to its client ID; leave `oidc_audience` empty. Providers
that issue a standard audience can set `oidc_audience` and leave
`oidc_binding_value` empty. The module refuses an OIDC deployment without
exactly one application-binding mechanism. OIDC permissions are mapped from
the verified `permissions` array and unrestricted OIDC access is always
disabled in this hosted module.

The control plane currently verifies 50 MiB object digests from bounded
in-memory buffers. Cloud Run concurrency is therefore capped at eight and
instances use 1 GiB of memory. Increase neither limit independently; migrate
the object-store boundary to streaming before raising transfer concurrency.

Apply staging and production from different GCP projects and separate Terraform
state. Always provide the server image by digest, never by a mutable tag.
