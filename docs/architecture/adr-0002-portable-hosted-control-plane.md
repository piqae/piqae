# ADR-0002: Portable hosted control plane

Status: accepted

## Decision

The hosted service uses:

- Vercel for SvelteKit UI and documentation.
- Cloud Run in Sydney for the Rust API and agent gateway.
- Neon PostgreSQL in Sydney.
- Cloudflare R2 for S3-compatible content storage.
- WorkOS for hosted human identity.

The authoritative Rust service remains an ordinary OCI image. Database,
object-store, and human-identity integrations sit behind narrow interfaces.
Self-hosting uses the same image with PostgreSQL, MinIO or another S3 service,
and generic OIDC or a one-time local owner bootstrap.

## Consequences

The managed service can scale without defining a separate proprietary
architecture. Customers can run Piqae inside their own infrastructure, and a
future white-labelled deployment does not fork the printing domain.
