# Secrets, signing, and low-cost builds

**Decision:** use local machines for development feedback, standard GitHub-hosted
runners for CI and release builds, 1Password as the human-controlled recovery
source of truth, GitHub environments for release-only copies, and Railway
environment variables for runtime-only copies.

Piqae is a public repository, so standard GitHub-hosted Linux, macOS, and
Windows runners do not consume paid Actions minutes. Do not add a paid runner
provider merely to reduce cost. Retain the runner-variable switches documented
in [CI and release operations](ci-and-release.md) for a future speed or capacity
trial.

## Build and publication boundary

Use local compute aggressively:

```console
cargo xtask doctor
cargo xtask test changed
```

Local macOS and Windows packages are development fixtures. They may be
installed only on controlled test machines and are never uploaded into a public
release channel. A published package is rebuilt from a reviewed tag on the
matching hosted operating system so the native signature, updater signature,
SBOM, checksum, and provenance all cover the same bytes.

Keep ordinary CI path-filtered and cancel superseded pull-request runs. Keep
development artifacts for three days, release candidates for fourteen days,
and durable release evidence in the release bucket rather than indefinite
Actions storage.

## Secret ownership model

The 1Password vault **Piqae Production** is the recovery source of truth. Use
separate items for each credential family and record its issuer, owner,
environment, creation date, expiry, rotation procedure, revocation procedure,
and last recovery test.

| Destination | Purpose | Rule |
| --- | --- | --- |
| 1Password | encrypted recovery and human handover | retain the original or an encrypted recovery copy; require production access |
| GitHub `native-signing` | native and updater signing during candidate builds | signing material only; restrict to reviewed `main` and protected `v*` refs |
| GitHub `native-release` | immutable release-bucket publication | release publisher credentials only; protected tags and reviewer approval |
| Railway Staging | synthetic hosted-auth and deployment tests | staging credentials only; never copy production WorkOS or cookie secrets |
| Railway Production | production runtime | runtime credentials only; no native signing keys or release publisher key |
| Node operating-system store | device identity and confidential-print recipient keys | non-exportable/OS-protected where available; never place WorkOS sessions on nodes |

Never paste secrets into chat, workflow inputs, command arguments, logs, source
control, appcasts, manifests, crash reports, or `PUBLIC_` web variables. Do not
use GitHub or Railway as the only copy of an updater private key: their secret
values cannot be read back for disaster recovery.

Prefer manual, reviewed synchronization from 1Password initially. Do not create
a broad unattended vault token merely to avoid entering a small number of
release secrets. If automation becomes necessary, use a dedicated read-only
service account that can access only the exact item fields required by one
GitHub environment, and rotate it independently.

## Credential families

### macOS

Retain encrypted recovery copies of the Developer ID certificate and Sparkle
Ed25519 private key. GitHub `native-signing` currently expects:

```text
MACOS_CERTIFICATE_P12_BASE64
MACOS_CERTIFICATE_PASSWORD
MACOS_APPLICATION_IDENTITY
MACOS_INSTALLER_CERTIFICATE_P12_BASE64
MACOS_INSTALLER_CERTIFICATE_PASSWORD
MACOS_INSTALLER_IDENTITY
APPLE_ID
APPLE_TEAM_ID
APPLE_APP_PASSWORD
SPARKLE_PRIVATE_KEY_BASE64
SPARKLE_PUBLIC_ED_KEY
```

Prefer an App Store Connect team API key for future notarisation rotation where
the workflow supports it. Apple private keys are downloadable only once. Keep
the `.p8`, issuer ID, and key ID together in 1Password, but in separate fields.
Keep the Developer ID Application and Developer ID Installer identities in
separate encrypted PKCS#12 bundles. The latter signs the standard macOS
Installer package and can rotate independently.
The public Sparkle key belongs in repository or environment configuration; its
private key remains recoverable only through the protected vault and signing
environment.

### Windows

Use one of these mutually exclusive public-trust paths:

1. **Microsoft Artifact Signing (preferred):** managed public-trust signing with GitHub OIDC
   and no exportable certificate. At the time of this decision, Microsoft
   accepts Public Trust organizations in New Zealand. Confirm that the Piqae
   publisher's Azure billing profile exactly matches its legal identity and
   recheck the official availability list during setup. Store the Azure
   tenant, client, subscription, signing-account endpoint, and
   certificate-profile identifiers as GitHub environment variables. Grant the
   federated release identity only the Artifact Signing Certificate Profile
   Signer role. No client secret is required when OIDC is used.
2. **Public-CA cloud HSM provider:** a separately billed fallback. The checked-in
   workflow supports DigiCert KeyLocker through DigiCert's current pinned
   GitHub integration. Set `WINDOWS_SIGNING_PROVIDER=digicert-keylocker`; keep
   its API key, client-authentication certificate/password, and keypair alias in
   the `native-signing` environment. The Authenticode private key remains in
   DigiCert's HSM. Other providers require a separately reviewed adapter before
   credentials are configured.

The existing PFX path remains a compatibility fallback and expects
`WINDOWS_AUTHENTICODE_PFX_BASE64` and
`WINDOWS_AUTHENTICODE_PFX_PASSWORD`. Do not generate these placeholders until
an actual certificate has been issued. WinSparkle update signing is separate
from Authenticode and always needs its own
`WINSPARKLE_ED25519_PRIVATE_KEY_BASE64` recovery copy and
`WINSPARKLE_ED25519_PUBLIC_KEY` configuration.

Do not set the remote provider variable before all provider, signer-policy, and
WinSparkle fields exist: a partial signing configuration deliberately fails
rather than producing an ambiguously signed candidate.

For Artifact Signing, set `WINDOWS_SIGNING_PROVIDER=artifact-signing` and
configure these `native-signing` environment variables:

```text
AZURE_ARTIFACT_SIGNING_CLIENT_ID
AZURE_ARTIFACT_SIGNING_TENANT_ID
AZURE_ARTIFACT_SIGNING_SUBSCRIPTION_ID
AZURE_ARTIFACT_SIGNING_ENDPOINT
AZURE_ARTIFACT_SIGNING_ACCOUNT_NAME
AZURE_ARTIFACT_SIGNING_CERTIFICATE_PROFILE_NAME
```

The Entra application needs a federated credential restricted to the
`native-signing` GitHub environment in `piqae/piqae`, and its service principal
needs only `Artifact Signing Certificate Profile Signer` on the selected
certificate profile. The workflow uses GitHub OIDC and deliberately has no
Azure client secret. Artifact Signing certificates rotate frequently, so the
workflow binds signing authority to the OIDC identity, account, profile, and
exact certificate subject instead of pinning a transient leaf thumbprint.
Use the account's documented regional endpoint and Microsoft's RFC 3161
timestamp endpoint, `http://timestamp.acs.microsoft.com`.

See Microsoft's current [Artifact Signing eligibility, setup, and regional availability](https://learn.microsoft.com/azure/artifact-signing/quickstart)
and [GitHub OIDC setup](https://learn.microsoft.com/azure/developer/github/connect-from-azure-openid-connect).

### WorkOS and web sessions

Use different WorkOS projects/environments and independent cookie passwords for
Staging and Production. Railway web services receive `WORKOS_CLIENT_ID`,
`WORKOS_API_KEY`, `WORKOS_REDIRECT_URI`, and a random
`WORKOS_COOKIE_PASSWORD` of at least 32 bytes. The Rust API receives WorkOS
OIDC metadata and a separately issued webhook secret. Never copy WorkOS API
keys, refresh tokens, browser sessions, or webhook secrets onto a print node.

Rotate the cookie password as a session-invalidating operation. Rotate the
WorkOS API key and webhook secret independently, using an overlap window where
the provider supports it. Test signup, login, logout, organization switching,
role changes, revocation, and webhook replay protection in Staging before a
Production change.

### Release storage and deployment

Use separate release-bucket reader and publisher credentials. The web service
gets the reader; only GitHub `native-release` gets the publisher. Customer print
documents use a different bucket and credentials. Railway production and
staging variables must remain isolated even when they refer to services in the
same project.

## Confidential printing keys

Release signing, hosted authentication, tenant API keys, node identity, and
document-encryption keys are separate trust domains and must never reuse key
material.

For confidential jobs, the integrator encrypts document bytes before upload.
The service stores and transports ciphertext plus the bound envelope; only the
authorized node unwraps the content key using its OS-protected recipient key.
The service must not log plaintext, decrypted content keys, or printable
documents. Recovery must be explicit: losing a non-exportable node recipient
key makes jobs encrypted only to that key unrecoverable, which is safer than a
server-side decryption backdoor.

## Rotation and recovery cadence

- Review access and secret inventory quarterly.
- Alert on certificate and API-key expiry at 90, 30, 14, and 7 days.
- Test updater-key recovery offline at least twice a year without publishing.
- Rotate runtime API credentials at least annually and immediately after staff,
  vendor, or incident changes.
- Record rotations as audit events without recording secret values.
- Revoke compromised credentials before replacing channel pointers or resuming
  release publication.
