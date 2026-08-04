# API authentication

**Status:** scoped API keys, bootstrap keys, self-hosted local-owner sessions,
OIDC/hybrid authentication, and tenant isolation implemented.

Native API clients send:

```text
Authorization: Bearer piq_...
```

Keep keys in a server-side secret manager. Never embed them in browser
JavaScript, native print content, URLs, logs, or support bundles. Create one key
per integration/environment with the minimum scopes and revoke without deleting
the audit record.

Ordinary API keys can never select another workspace or environment. Piqae
derives their tenant from the verified key and ignores or rejects
`X-Piqae-Workspace-Id` and `X-Piqae-Environment-Id` as tenant-escalation
attempts.

Multi-workspace SaaS backends use a separate platform service-account
credential with explicit workspace/environment/scope grants. See
[platform service accounts](platform-service-accounts.md). Platform
credentials are never suitable for browser JavaScript or native nodes.

Bootstrap authentication is for initial self-host setup. Rotate or remove the
bootstrap key after creating durable keys. Nodes use their enrolled device
identity, not an integration key.

OIDC deployments verify issuer, JWKS, application binding, organization claim,
and permission claim. Hosted configurations keep unrestricted OIDC disabled.
The dashboard's server session forwards verified access tokens; it must not
fall back to exposing a bootstrap key.

Piqae Cloud additionally projects signed WorkOS organization, user, and
membership events. A projected inactive member or user is denied after JWT
verification, so removal does not rely only on the access token's expiry. See
[WorkOS production authentication](../operations/workos-production-auth.md)
for the exact claims, event subscriptions, and live acceptance matrix.

## Self-hosted local owner

Set `PIQAE_IDENTITY_PROVIDER=local_owner` on the control plane and
`PIQAE_AUTH_MODE=local` on the SvelteKit dashboard. Configure a high-entropy
`PIQAE_LOCAL_OWNER_BOOTSTRAP_TOKEN`, then create the first owner once:

```console
curl --fail-with-body \
  -H 'Content-Type: application/json' \
  -H 'X-Piqae-Bootstrap-Token: replace-with-your-bootstrap-token' \
  -d '{"workspace_name":"My workspace","email":"owner@example.com"}' \
  http://127.0.0.1:8080/v1/identity/local/bootstrap
```

The response contains one `piq_owner_...` credential. Store it in a password
manager: Piqae stores only its Argon2id hash and cannot show it again. Remove
`PIQAE_LOCAL_OWNER_BOOTSTRAP_TOKEN` and restart the control plane after this
request. A deployment-wide advisory lock and database constraint reject a
second bootstrap.

Enter that owner credential at `/login`. SvelteKit exchanges it server-side
for a short-lived `piq_session_...` token and stores that token in an
`HttpOnly`, `SameSite=Strict` cookie. The browser never receives either secret
in page data or JavaScript. The session-inspection endpoint rotates sessions
when less than one hour remains, and logout revokes the server-side record
before deleting the cookie. Configure the lifetime with
`PIQAE_LOCAL_OWNER_SESSION_SECONDS` (bounded to 15 minutes–24 hours; 12 hours
by default). Set `PIQAE_COOKIE_SECURE=true` when TLS is terminated in front of
an internally HTTP SvelteKit process.

The tenant endpoints `GET /v1/identity/me`, `GET /v1/workspaces/current`, and
`GET /v1/workspaces/current/members` always derive the workspace from the
verified bearer; callers cannot provide a workspace ID.

Current limitations are intentional and explicit:

- local-owner V1 creates one owner, one workspace, and one live environment;
- it does not yet provide credential recovery, owner-credential rotation,
  invitations, workspace switching, password/email login, MFA, or member
  mutations;
- `GET /v1/identity/me` projects the first active member in the authenticated
  workspace, so hosted deployments should continue using WorkOS session data
  for exact user identity;
- session revocation is database-backed, but there is no all-sessions UI yet;
- the bootstrap endpoint is disabled when its environment token is absent.

legacy compatibility routes use HTTP Basic with the compatibility key as
username and an empty password. That convention applies only to compatibility
routes.

Read [`07-security-observability-and-operations.md`](../07-security-observability-and-operations.md)
before assigning scopes.
