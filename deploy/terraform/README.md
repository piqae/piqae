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

Apply staging and production from different GCP projects and separate Terraform
state. Always provide the server image by digest, never by a mutable tag.
