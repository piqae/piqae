# Hosted infrastructure

This module deploys the Spool control plane to Cloud Run in Sydney. Neon,
Cloudflare R2, WorkOS, and Vercel remain separately managed services; their
credentials are passed to this module as secrets.

Production uses three always-allocated instances so API, agent long-polling,
outbox workers, and webhook delivery continue without a cold start. PostgreSQL
leases and transactional outboxes make every replica safe to run the combined
`all` role.

Apply staging and production from different GCP projects and separate Terraform
state. Always provide the server image by digest, never by a mutable tag.
