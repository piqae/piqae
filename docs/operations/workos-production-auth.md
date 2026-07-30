# WorkOS production authentication

Piqae Cloud uses WorkOS AuthKit for human browser identity. Native nodes and
tray/menu applications continue to use enrolled, durable device credentials;
they must never receive or persist a WorkOS access token, refresh token, or
sealed browser session.

Self-hosted deployments do not require WorkOS. `local_owner` remains the
default identity provider, and generic `oidc` remains available for an
operator-selected provider.

## Production application contract

The production AuthKit application is `Piqae Admin`:

- application and post-logout origin: `https://app.piqae.com`;
- callback: `https://app.piqae.com/auth/callback`;
- roles: `owner`, `admin`, `developer`, `operator`, `viewer`, and `billing`;
- organization claim: `org_id`;
- permission claim: `permissions`;
- access tokens: 10 minutes;
- inactivity timeout: 14 days;
- maximum session lifetime: 30 days.

The application uses the WorkOS-hosted AuthKit domain until a production custom
domain is supported and its DNS ownership is verified. Do not publish
`auth.piqae.com` before WorkOS and Cloudflare both report the domain verified.

Every role includes `usage_read`. The other permission slugs are
`api_keys_read`, `api_keys_write`, `agents_read`, `agents_write`,
`printers_read`, `printers_write`, `jobs_read`, `jobs_write`,
`webhooks_read`, `webhooks_write`, and `audit_read`. Authorization in the Rust
control plane remains authoritative; hiding a dashboard control is not an
authorization boundary.

## Web and control-plane secrets

The SvelteKit production environment requires:

```text
PIQAE_AUTH_MODE=workos
WORKOS_CLIENT_ID=<production client ID>
WORKOS_API_KEY=<production secret key>
WORKOS_REDIRECT_URI=https://app.piqae.com/auth/callback
WORKOS_COOKIE_PASSWORD=<independent random value of at least 32 bytes>
```

Store these in the hosting provider's encrypted Production environment. Never
reuse the WorkOS API key as the cookie password, copy either value into a
preview environment, or expose either through a `PUBLIC_` variable.

The production Rust API uses:

```text
PIQAE_AUTH_MODE=oidc
PIQAE_IDENTITY_PROVIDER=workos
PIQAE_OIDC_ISSUER=https://api.workos.com/user_management/<production-client-id>
PIQAE_OIDC_JWKS_URL=https://api.workos.com/sso/jwks/<production-client-id>
PIQAE_OIDC_CLIENT_ID=<production-client-id>
PIQAE_OIDC_ORGANIZATION_CLAIM=org_id
PIQAE_OIDC_PERMISSIONS_CLAIM=permissions
PIQAE_OIDC_ENVIRONMENT=live
PIQAE_OIDC_ALLOW_UNRESTRICTED=false
WORKOS_WEBHOOK_SECRET=<endpoint signing secret>
```

The combined authenticator retains scoped Piqae API keys and durable node
authentication while `oidc` enables WorkOS human access tokens. It does not
put a human WorkOS session into a node.

## Identity event projection

Register `https://api.piqae.com/v1/integrations/workos/webhook` for:

```text
organization.created
organization.updated
organization.deleted
user.created
user.updated
user.deleted
organization_membership.created
organization_membership.updated
organization_membership.deleted
invitation.created
invitation.accepted
invitation.revoked
```

The endpoint verifies `WorkOS-Signature` against the exact raw body with a
five-minute tolerance and a 1 MiB body limit. Event IDs are stored with their
payload hashes. An exact replay is acknowledged, reuse of an event ID with
different content is rejected, and entity timestamps prevent an older event
from replacing a newer organization, user, role, or membership state.

Membership deactivation and user deletion are projected as inactive access.
For WorkOS only, the Rust JWT path checks a known projection after verifying
the signature, issuer, application binding, `sub`, `org_id`, and permissions.
A projected inactive member is rejected even if an otherwise valid
10-minute token has not expired. A previously unseen membership is allowed
only because the signed WorkOS organization-bound token is initial proof while
the creation webhook is in flight.

## Required live acceptance evidence

Run with synthetic accounts and empty workspaces only:

1. sign up, sign in, and sign out;
2. create a workspace;
3. place one user in two organizations;
4. switch between them and verify the access token `org_id` changes;
5. invite and accept a second user;
6. exercise every production role and its permission boundaries;
7. change a role, refresh the session, and verify the new permissions;
8. remove access and verify both dashboard and API denial;
9. probe known resource IDs from the other workspace and receive denial or
   not-found without disclosing resource details;
10. run the self-hosted local-owner login/session/logout test unchanged.

Record the immutable commit, deployment IDs, WorkOS environment ID, webhook
endpoint ID, test timestamps, and redacted pass/fail evidence. Do not record
cookies, tokens, invitation URLs, API keys, signing secrets, or customer data.
