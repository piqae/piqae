# Declarative documents

Piqae Documents is an optional, provider-neutral path for creating a bounded
document specification, publishing an immutable revision, rendering JSON data
to PDF, and registering that completed render as an ordinary durable PDF print
job. Existing PDF and RAW job requests do not require templates and are
unchanged.

See [document render performance evidence](../testing/document-render-performance.md)
for the reproducible renderer probe, production soak requirements and the safe
artifact-reuse prerequisite.

The first contract is `piqae.document/v1`. It supports A4, A5, Letter, 4-by-6,
58 mm roll, and 80 mm roll pages; text, stack, row, spacer, line, page break,
condition, bounded repeat, bounded flow tables, and vector QR nodes. Table
columns have deterministic relative weights and cells use item-relative JSON
Pointers. Tables do not currently wrap, clip, span cells, repeat headers after
automatic page breaks, or provide the absolute box model of HTML. It deliberately does not run
Liquid, HTML, JavaScript, Typst source, filesystem reads, remote URLs, or CDN
font requests. Unsupported fields and node types are rejected.

## Lifecycle

1. `POST /v1/document-templates` creates an encrypted draft.
2. `POST /v1/document-templates/{id}/publish` freezes an encrypted immutable
   revision.
3. `POST /v1/document-renders` combines a published revision with encrypted
   JSON input and produces a deterministic PDF artifact.
4. `POST /v1/document-renders/{id}/print` reuses that artifact through the
   normal PDF job pipeline. Its response reports durable job registration, not
   proof of physical delivery.

### Zero-copy print ownership

Printing a completed render does not download and upload the PDF again. The
control plane registers a completed upload alias pointing at the immutable
artifact object; the job transaction then creates a tenant-fenced
`document_artifact_job_references` edge. The artifact-acquisition record stores
only a SHA-256 digest of the idempotency key, so replay selects the same alias;
the ordinary job idempotency policy remains unchanged.

The reference extends render retention through job expiry and is released when
the job becomes terminal. Artifact cleanup cannot claim a render with a live,
unexpired reference. Conversely, if cleanup has already leased the render, the
job-insert trigger rejects printing atomically. No artifact object is deleted
while a job can still download it, and the alias stores no duplicate PDF bytes.

All mutating calls require `Idempotency-Key`. Template specifications, render
inputs, and artifact references are encrypted with ChaCha20-Poly1305 using a
distinct `PIQAE_DOCUMENT_MASTER_KEY` and tenant/resource-bound authenticated
data. Deployments must provide a base64-encoded 32-byte key separate from the
webhook key. PDF artifacts inherit the configured object store's at-rest
protection and access controls.

### Encryption-key rotation

Ciphertexts written by the current service carry a non-secret key identifier.
`PIQAE_DOCUMENT_ACTIVE_KEY_ID` selects the only key permitted to encrypt and
`PIQAE_DOCUMENT_MASTER_KEY` contains its base64-encoded 32-byte material.
`PIQAE_DOCUMENT_DECRYPTION_KEYS` is a JSON object containing prior key ids and
base64 keys; those keys are decrypt-only. Ciphertexts written before key ids
were introduced use the reserved `legacy-v1` id.

Rotate without losing registered, rendering, completed, or queued work:

1. Generate a new key and unique id. Keep the prior key in
   `PIQAE_DOCUMENT_DECRYPTION_KEYS` (use `legacy-v1` for the original key).
2. Deploy the complete keyring to every API and worker instance before changing
   the active id. Mixed versions can decrypt both generations.
3. Deploy the new active id/key. New ciphertext uses it while retained records
   continue to decrypt with the prior key.
4. Dry-run a bounded batch with
   `piqae-server document-key-rewrap OLD_KEY_ID 100 --dry-run`, then run
   the same command without `--dry-run` repeatedly until
   `references_after=0`. The operation decrypts with the record's exact tenant
   and resource AAD, encrypts with the active generation, and compare-and-swaps
   the old ciphertext. It is restartable, and skips renders with live worker
   leases. Malformed, tampered, or unavailable-key records increment the
   `unreadable` counter while the rest of the batch continues. Their ciphertext
   stays untouched and `references_after` remains non-zero; investigate the
   retained record and key configuration rather than forcing retirement.
   Database retirement or deletion is rejected while a template,
   immutable revision, render input, artifact reference, or hosted adapter
   conversion result remains.
5. Set the old database key lifecycle to `retired`, then remove its material
   from all instances. Never remove key material before this succeeds.

The database stores key ids and lifecycle state, never key material. Startup
serializes active-key changes and registers every configured id as active or
decrypt-only. An unavailable referenced key fails closed rather than discarding
or regenerating a print job.

The renderer enforces limits for JSON size and structure, nodes, nesting,
repeat items, pages, text bytes, QR payloads, and PDF output bytes. The current
font is built-in Helvetica/WinAnsi, so full Unicode shaping, embedded fonts,
images, non-QR barcodes, Liquid, and Shopify-specific mappings remain later
profiles rather than implied support. Text and QR payloads share the byte
budget; table cells additionally consume node, repeat-item, page, text, and
output limits. Non-finite programmatic dimensions, over-height elements,
invalid pointers, empty/oversized tables, and invalid column weights fail
closed. PDF text escaping prevents template values from injecting content
stream operators.

The authoritative support tier is `declarative_document_generation` in
`release/support-matrix.yaml`. It remains Disabled for public production claims
until database fault injection, independent security review, load soak, and
physical-printer release gates have evidence.
