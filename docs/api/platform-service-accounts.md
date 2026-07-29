# Platform service accounts

**Status:** private-preview authentication contract. Grant creation and
revocation use the database-backed operator CLI; no public grant-management API
is supported in V1.

Platform service accounts let a trusted SaaS backend operate Spool resources
for multiple customer workspaces. They are intentionally different from
ordinary Spool API keys:

- the credential is issued as a platform service-account key;
- every grant pins one workspace, one environment, and a bounded scope set;
- every tenant request names that exact workspace and environment;
- the server authenticates the credential before considering selection headers;
- an ordinary API key cannot use selection headers to cross tenants;
- revoking one grant does not revoke unrelated customer grants.
- every successful credential verification writes a tenant-scoped audit event
  before the request is allowed to proceed.

## SDK

Construct a client explicitly for one grant:

```ts
import { SpoolClient } from '@spool/sdk';

const customerSpool = new SpoolClient({
  platformKey: process.env.SPOOL_PLATFORM_KEY!,
  platformContext: {
    workspaceId: customer.spoolWorkspaceId,
    environmentId: customer.spoolLiveEnvironmentId
  }
});

const printers = await customerSpool.printers.list();
```

The SDK emits the platform bearer plus `X-Spool-Workspace-Id` and
`X-Spool-Environment-Id`. `platformContext` is a constructor-only trust
decision. Resolve it from the SaaS application's authenticated, server-owned
organisation mapping—not from browser-supplied resource IDs.

The SDK strips both selection headers from ordinary custom headers. It rejects
`platformContext` unless a distinct `platformKey` is present, and rejects
mixing `platformKey` with a tenant `apiKey` or interactive access-token
provider.

## Grant model

A grant contains:

- service-account identity;
- workspace ID;
- Test or Live environment ID;
- API scopes such as `printers_read`, `jobs_write`, and `webhooks_write`;
- optional expiry;
- revocation state and audit timestamps.

Authorization is the intersection of the verified credential, selected
workspace/environment, and endpoint scope. Possession of a platform key alone
grants no tenant access.

Use separate service accounts for production, previews, migrations, and
support tooling. Grant support tooling read-only access by default. Platform
credentials and grant details must be redacted from logs, traces, support
bundles and error analytics.

## Operator provisioning

Set `SPOOL_DATABASE_URL` in the operator shell; never put a database password
or platform credential in command arguments.

```console
spoolctl platform create \
  --name fulfilment-production \
  --workspace wrk_... \
  --environment env_... \
  --scopes printers_read,jobs_read,jobs_write
```

The credential is printed once. Store it in the SaaS secret manager. The
database retains only its Argon2 hash.

Grant the same identity another explicitly approved tenant:

```console
spoolctl platform grant \
  --service-account 019... \
  --workspace wrk_... \
  --environment env_... \
  --scopes printers_read,jobs_read,jobs_write
```

Revoke one tenant without affecting its other grants:

```console
spoolctl platform revoke-grant \
  --service-account 019... \
  --workspace wrk_... \
  --environment env_...
```

Rotate a credential without changing its grants:

```console
spoolctl platform rotate --service-account 019...
```

The old credential stops working as soon as the transaction commits. The
replacement is printed once.

Revoke the whole account immediately:

```console
spoolctl platform revoke --service-account 019...
```

Deleting an account is intentionally harder than revocation: the account must
already be revoked, and the command requires repeating its exact ID:

```console
spoolctl platform delete \
  --service-account 019... \
  --confirm 019...
```

Creation, grant changes, rotation, account revocation, deletion, and verified
request use write tenant-scoped audit records. Those records contain IDs,
scopes, outcomes, and request IDs—not credentials or document content. If the
request audit write fails, platform authentication fails closed.
