# Platform service-account release evidence

**Status:** release policy implemented; the feature remains Disabled until its
implementation and automated evidence exist.

Platform service accounts are intended for a trusted SaaS or integration
platform that operates explicitly granted customer workspaces. They are not
ordinary workspace API keys with broader scope.

The authoritative gate set is
[`release/platform-service-account-gates.json`](../../release/platform-service-account-gates.json).
Validate it with:

```console
python3 release/tools/check_platform_service_account_policy.py
python3 release/tools/audit_platform_service_account_coverage.py
python3 release/tools/test_platform_service_account_policy.py -v
```

The code-backed coverage audit is intentionally separate from release
evidence. It records implemented controls, currently passing focused tests, and
the remaining assertions for each gate. It cannot mark a partial scenario as
passed, and it blocks Preview while any scenario remains partial or missing.

The current audit keeps this feature Disabled:

| Gate | Current evidence | Smallest remaining change |
| --- | --- | --- |
| Tenant isolation | PostgreSQL and HTTP tests cover granted, ungranted, unknown and scope-limited tenants | Add cross-resource and concurrent multi-tenant HTTP tests |
| Grant revocation | Passed: next-request denial, unaffected second grant, account revoke, rotation and expiry are database-tested | Retain the non-skipped release database gate |
| Auditability | Lifecycle mutations and verified or scope-denied requests write tenant-scoped audit events transactionally | Add durable operator identity attribution and audit-export redaction evidence |
| Ordinary-key selection | Paired platform headers are rejected for an ordinary key | Cover partial headers, query/JSON selector attempts, and the ordinary tenant control case |
| Secret redaction | Platform keys are distinct Argon2-backed credentials; database tests scan account and audit rows for plaintext | Add synthetic canary scans for logs, traces, errors, metrics, process arguments and support bundles |

Policy validation describes the required scenarios but does not prove the
PostgreSQL authorization boundary. A release candidate must also run:

```console
PIQAE_TEST_DATABASE_URL=postgres://postgres:password@127.0.0.1:5432/piqae_test \
  python3 release/tools/check_postgres_release_tests.py
```

That wrapper requires the exact non-skipped storage and HTTP tests:

- `postgres_platform_grants_are_exact_scoped_and_revocable`
- `postgres_platform_http_auth_is_tenant_scoped_audited_and_revocable`
- `postgres_http_platform_accounts_are_owned_idempotent_and_archive_safely`

It rejects a missing integration-test target, zero matched tests, a skip
message, or a failing command. This requirement applies even while the feature
is Disabled, so a future tier change cannot accidentally rely only on policy
fixtures or in-memory authorization tests.

## Security invariants

- A platform credential has no implicit workspace access. Every workspace
  requires an active grant.
- Revoking one grant takes effect on the next authorization decision without
  disabling unrelated grants.
- Credential and grant mutations, workspace selection, action, result, and
  request ID are auditable without recording secrets or document content.
- An ordinary workspace key cannot select another workspace by adding a
  platform-only header, query parameter, path component, or body field.
- Platform secrets are shown once, stored only as verifiers, and redacted from
  logs, traces, metrics, errors, support bundles, process arguments, and audit
  payloads.

Workspace selection must be an explicit authenticated input covered by the
authorization decision. It must never be inferred from a printer, job, target,
or profile identifier supplied by the caller.

## Promotion rules

Disabled requires the policy to remain complete and visible in the support
matrix. Preview additionally requires a release- and commit-bound evidence file
with one passing automated result for every required scenario. Evidence must
use synthetic credentials and repository-relative report references.

Supported additionally requires:

1. an independent authorization review;
2. review of a production-shaped audit export; and
3. a credential/grant revocation soak.

Passing unit tests alone is not enough to call cross-workspace access
Supported. Production cache behavior, concurrent workspace requests, incident
exports, and revocation latency must be observed.

## Required negative tests

The implementation suite must include:

- an ungranted workspace and a nonexistent workspace returning
  indistinguishable responses;
- cross-tenant resource IDs under both granted and ungranted selectors;
- concurrent requests using one credential and different workspace grants;
- grant revocation between two otherwise identical requests;
- replay after revocation;
- an ordinary workspace key supplying every supported workspace-selector
  location;
- secret-shaped canaries checked against logs, traces, audit records, support
  bundles, metrics, and error responses.

Evidence must never contain a real customer API key or reusable platform
secret.
