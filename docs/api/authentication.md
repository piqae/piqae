# API authentication

**Status:** scoped API keys, bootstrap keys, OIDC/hybrid authentication, and
tenant isolation implemented.

Native API clients send:

```text
Authorization: Bearer spl_...
```

Keep keys in a server-side secret manager. Never embed them in browser
JavaScript, native print content, URLs, logs, or support bundles. Create one key
per integration/environment with the minimum scopes and revoke without deleting
the audit record.

Bootstrap authentication is for initial self-host setup. Rotate or remove the
bootstrap key after creating durable keys. Nodes use their enrolled device
identity, not an integration key.

OIDC deployments verify issuer, JWKS, application binding, organization claim,
and permission claim. Hosted configurations keep unrestricted OIDC disabled.
The dashboard's server session forwards verified access tokens; it must not
fall back to exposing a bootstrap key.

PrintNode compatibility routes use HTTP Basic with the compatibility key as
username and an empty password. That convention applies only to compatibility
routes.

Read [`07-security-observability-and-operations.md`](../07-security-observability-and-operations.md)
before assigning scopes.
