# Content-confidential printing foundation

Status: **implemented Preview path; Disabled as a production support claim**.
Piqae does not claim a security-audited zero-knowledge or end-to-end encrypted
service. Existing PDF and RAW job paths remain plaintext-capable and unchanged.

`@piqae/sdk` builds a versioned `piqae-encrypted-job-v3` envelope before a
document leaves the integrator-controlled browser or backend:

```ts
import { encryptJobContent } from '@piqae/sdk';

const envelope = await encryptJobContent(pdfBytes, {
  workspace_id: account.workspaceId,
  environment_id: account.environmentId,
  content_type: 'pdf',
  printer_id: destination.printer.id,
  target_id: target.id,
  profile_revision: `${destination.binding.profile_id}:${destination.binding.profile_revision}`,
  options: {},
  deliveries: 1,
  expires_at: '2026-09-01T00:00:00Z',
  raw_authorized: false,
  recipients: nodeEncryptionKeys
});

const job = await piqae.jobs.createEncrypted({
  target_id: target.id,
  title: 'Private design',
  content_type: 'pdf'
}, envelope, crypto.randomUUID());
```

`createEncrypted` uploads only the ciphertext as `application/octet-stream`,
then submits a manifest that contains the wrapped per-node key and authenticated
production binding. The control plane validates the exact target/profile,
active recipient, digest and expiry before queueing it.

V3 also authenticates a random one-time envelope ID, tenant, selected printer,
normalized complete job options, delivery count, and RAW authorization.
Envelope IDs are consumed transactionally per tenant. Replaying the identical
manifest returns the original job; reusing an ID with any changed manifest is
rejected even under a new idempotency key.

## Envelope properties

V3 uses these exact, case-sensitive identifiers from the OpenAPI contract:

- Envelope suite: `ECDH-ES-P256+HKDF-SHA256+A256GCMKW+A256GCM`
- Wrapped-key recipient algorithm: `ECDH-ES-P256+HKDF-SHA256+A256GCMKW`
- Registered node-key capability: `ECDH-P256-HKDF-SHA256`

The node-key capability describes the reusable public key; it is intentionally
distinct from the per-recipient wrapping algorithm. SDKs and nodes must reject
aliases rather than silently reinterpret them.

- A fresh 256-bit AES-GCM content key and 96-bit IV are generated per job.
- PDF or RAW bytes are encrypted once with AES-256-GCM.
- Tenant, envelope ID, printer, target, profile revision, complete options,
  delivery count, content type, expiry, and RAW authorization are canonical
  authenticated additional data. Changing any of them makes decryption fail.
- The ciphertext and authentication tag have a SHA-256 digest for validation
  before transport and decryption.
- The content key is encrypted separately to every permitted recipient using
  ephemeral P-256 ECDH, HKDF-SHA-256 and AES-256-GCM key wrapping, allowing
  bounded failover without a cloud master key.
- Each recipient uses a fresh ephemeral P-256 key, 256-bit HKDF salt and 96-bit
  key-wrap IV. HKDF info binds the envelope and key IDs; the canonical document
  binding is also authenticated by the AES-GCM key wrap.
- Key IDs and an explicit suite identifier allow rotation and future algorithm
  versions.

The initial suite uses Web Crypto algorithms available in supported browsers
and Node.js. It is a deliberately versioned interoperability foundation, not a
new cryptographic primitive. Before enabling production transport, commission
an independent protocol and implementation review and consider adopting a
standards-track HPKE/JWE profile when runtime support is consistent.

## Separate node keys are mandatory

The node authentication key is Ed25519 and is never reused or converted for
document encryption. Each connector creates a separate P-256 ECDH content key
and registers its SPKI public key through a device-signed request. macOS stores
the private key in the login Keychain and Windows stores it in Windows
Credential Manager, protected for the current operating-system user. Existing
owner-only key files are imported, read back and verified before the plaintext
file is removed. Migration and startup fail closed if the OS store is
unavailable or returns different bytes. Linux and other headless Unix nodes
retain an owner-only file because a desktop secret service cannot be assumed;
encrypted storage volumes are recommended there. Rotation makes the previous
recipient decrypt-only while the node retains bounded older
generations for decryption. The keyring manifest contains identifiers and
lifecycle only, is bound to the stable node identity, and fails closed if prior
key material is missing or corrupt. Rotation refuses to discard an old key when
the generation cap is reached.

The control plane retains each encrypted job's tenant-scoped recipient-key
reference for as long as the encrypted job row exists. The database removes
that reference transactionally only when the exact job row is actually deleted
after retention, through its foreign-key cascade. The current source does not
yet implement the planned metadata-retention deletion worker, so references
remain conservatively durable and a referenced key cannot be revoked or
destroyed. Operators must drain retained encrypted-job records before the
node's bounded keyring reaches its generation cap; rotation fails closed at the
cap rather than discarding a potentially required private key.

The v3 upgrade does not reinterpret or convert an existing RSA private key.
RSA envelopes and RSA-sized stored key material fail closed. Before upgrading
an enrolled Preview node, drain or cancel its v2 encrypted jobs, deploy v3 to
both clients and control plane, then re-enrol or rotate the connector so the
node registers a P-256 recipient. Keep any retired RSA key only in an approved
encrypted recovery archive until the v2 retention window has closed; the v3
runtime never loads it.

These stores protect an exportable P-256 scalar at rest and enforce the local user
boundary. They are not evidence of Secure Enclave, TPM, or other non-exportable
hardware key storage, which remains future hardening.

## Implemented transport boundary

The current source and virtual-test path provides:

1. Tenant-scoped recipient registration, rotation, lookup and revocation.
2. Ciphertext-only object storage and lease download.
3. Digest verification before ECDH/HKDF/AES-GCM key unwrap and document decryption.
4. Local validation of the authenticated target/profile pin.
5. Bounded decryption into the existing owner-restricted content store and
   immediate ciphertext staging cleanup, including failure paths. Decrypted
   files have a durable SQLite association and remain available across restart,
   offline-printer and retry periods. Startup and periodic cleanup remove them
   only after a truthful terminal state (`completed_reported`,
   `delivery_uncertain`, `failed_terminal`, `cancelled`, or `expired`). A
   spooler handoff alone does not authorize plaintext deletion.

Before production promotion this still requires hardware-backed non-exportable key options,
lost-key/node-replacement UX, fleet soak and crash-injection evidence, physical
PDF/RAW fixtures, and an independent cryptographic/protocol review.

TLS remains mandatory. Payload encryption protects content from cloud storage,
proxies and Piqae operators, but traffic analysis still exposes timing, size and
routing metadata. The destination operating system, spooler, driver and printer
must see plaintext to print and remain inside the trusted execution boundary.

RAW additionally requires a separately granted `raw_print` permission and
printer-language policy. Encryption does not make arbitrary printer commands
safe.
