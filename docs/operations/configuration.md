# Configuration

**Status:** environment-based server/agent configuration implemented; secret
manager integration is deployment-specific.

## Control plane

Required production values include PostgreSQL URL, webhook master key, object
store selection/credentials, authentication mode, and bind address. OIDC also
requires an exact issuer, JWKS URL, application binding, organization claim,
and permissions claim.

For local-owner identity set `SPOOL_IDENTITY_PROVIDER=local_owner`, provide
`SPOOL_LOCAL_OWNER_BOOTSTRAP_TOKEN` only for the one-time first-owner request,
and use `SPOOL_LOCAL_OWNER_SESSION_SECONDS` for the 15-minute–24-hour session
lifetime. The dashboard uses `SPOOL_AUTH_MODE=local`; hosted WorkOS deployments
continue using `SPOOL_AUTH_MODE=workos`. See
[`API authentication`](../api/authentication.md) for the exchange and removal
procedure.

Use S3-compatible storage in replicated deployments. Filesystem storage is for
single-process development only. Keep `SPOOL_OIDC_ALLOW_UNRESTRICTED=false` in
hosted environments and reject plain HTTP object endpoints outside isolated
development.

## Node

Configure mode, private data directory, loopback bind, executor path, control
plane URL, enrolled agent ID, and device-key file. Preserve identity and SQLite
state across upgrades. The loopback API refuses non-loopback addresses.

## Rules

- Secrets come from files/secret stores, not images, Git, URLs, or logs.
- Pin images and native bundles to immutable versions.
- Separate staging and production credentials/state.
- Validate IDs and origins exactly; do not silently infer tenants.
- Restart one replica/node at a time and verify readiness.

Examples live in
[`deploy/self-host/.env.example`](../../deploy/self-host/.env.example),
[`packaging/linux/spool-agent.env.example`](../../packaging/linux/spool-agent.env.example),
and the Helm/Terraform values. Examples are templates, not safe secrets.
