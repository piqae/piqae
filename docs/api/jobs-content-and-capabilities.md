# Jobs, content, options, and printer capabilities

**Status:** the native V1 job, upload, printer, profile, stock, target, and
event contracts are implemented. Their current release tiers are defined by
the [support matrix](../../release/support-matrix.yaml). In particular,
durable delivery and routing remain Preview, and content-confidential printing
is Disabled as a production support claim.

This is the narrative reference for constructing a native Piqae print job. The
[OpenAPI contract](../../contracts/openapi/piqae-v1.yaml) remains authoritative
for machine validation. The TypeScript names below match `@piqae/sdk`.

## What creating a job means

`POST /v1/jobs` registers one durable print attempt. A `201` response means
Piqae recorded the attempt and accepted responsibility for progressing it. A
`200` response means the same idempotent request already created the returned
job. Neither response proves that a node downloaded the content, an operating
system accepted it, or paper was produced.

Every job belongs to exactly one workspace and one Test or Live environment.
Its destination, content descriptor, options, quantity, expiry, and metadata
are immutable after registration. Cancellation is a state transition request,
not an edit.

## Complete job request

```ts
type CreateJob = {
  // Supply exactly one destination.
  printer_id: string;
  target_id?: never;

  title: string;
  source?: string | null;
  content_type: 'pdf';
  printer_native?: never;
  content: JobContent;
  options?: JobOptions;
  deliveries?: number;
  expire_after_seconds?: number;
  metadata?: Record<string, string>;
} | {
  target_id: string;
  printer_id?: never;
  title: string;
  source?: string | null;
  content_type: 'raw';
  printer_native: {
    output_profile_id: string;
    language_profile_id: string;
  };
  content: JobContent;
  options?: JobOptions;
  deliveries?: number;
  expire_after_seconds?: number;
  metadata?: Record<string, string>;
};
```

Top-level validation:

| Field | Required | Validation and meaning |
| --- | --- | --- |
| `printer_id` | Conditional | Exactly one of `printer_id` and `target_id` must be a non-empty ID. It addresses one discovered physical queue directly. |
| `target_id` | Conditional | Exactly one destination. It addresses a logical primary/standby target pinned to an immutable profile revision. Prefer this for product integrations. |
| `title` | Yes | Non-blank and at most 255 UTF-8 bytes. It is operator-facing; do not put secrets or document content in it. |
| `source` | No | Nullable, at most 255 characters in the OpenAPI contract. Use a stable, non-secret application label when useful. |
| `content_type` | Yes | Exactly `pdf` or `raw`. It must agree with the upload media type and encrypted binding. |
| `printer_native` | RAW only | Required for RAW and forbidden for PDF. It selects an exact output/language profile currently advertised for the selected physical printer; it is not a generic MIME hint. |
| `content` | Yes | Exactly one of the four discriminated structures described below. Unknown fields are rejected. |
| `options` | No | Portable/native print choices. Omit or use `{}` for driver defaults. RAW jobs must use no options. |
| `deliveries` | No | Integer from 1 through 100; defaults to 1. This is the requested delivery count, distinct from `options.copies`. Integrators should normally keep one quantity mechanism. |
| `expire_after_seconds` | No | Integer from 1 through 1,209,600 (14 days); defaults to 14 days. Encrypted jobs use the authenticated envelope expiry. |
| `metadata` | No | String-to-string application correlation fields. Do not place credentials, private document data, or unbounded user input here. Piqae adds reserved routing metadata internally. |

JSON request objects reject unknown fields. TypeScript types catch many shape
errors at build time, but the server remains the security boundary and repeats
validation.

### Direct printer or logical target

Use `printer_id` when the application deliberately targets one queue and owns
the consequences of that choice. Use `target_id` when users think in business
destinations such as “Packing label” or “A4 invoice”. A target can bind stock
and exact native profile revisions to a primary and standbys.

Target creation still selects one concrete printer before offering the job to
a node. Automatic reassignment is safe only before local acceptance. Piqae
does not fail over after local acceptance or `spool_intent`, because another
handoff could create a duplicate physical output.

Do not send both IDs, omit both IDs, or silently replace a target with an
arbitrary online printer when readiness fails.

## Content structures

`content_type` describes what the operating-system print path receives;
`content.type` describes how Piqae obtains those bytes.

```ts
type JobContent =
  | { type: 'upload'; upload_id: string }
  | { type: 'encrypted_upload'; upload_id: string; manifest: EncryptedJobManifest }
  | { type: 'base64'; data: string }
  | { type: 'uri'; uri: string };
```

| Structure | Appropriate use | Validation and operational behavior | Current limitations |
| --- | --- | --- | --- |
| `upload` | Recommended for private PDFs and stable production behavior | Upload must belong to the same workspace/environment, be complete, and have `application/pdf` for PDF or `application/octet-stream` for RAW. Declared length and SHA-256 must match exactly. | Maximum 50 MiB. An incomplete, expired, cross-tenant, or media-mismatched upload is rejected. |
| `encrypted_upload` | Controlled evaluation of node-only content decryption | Requires `target_id`, a complete octet-stream upload, and a v3 manifest whose tenant, target, selected printer/profile revision, content type, options, quantity, expiry, digest, and active recipient key all match. | Preview implementation; Disabled production claim. Not independently reviewed, not hardware-key backed, and not a claim that the ordinary API is zero knowledge. |
| `base64` | Small compatibility payloads | Must be standard Base64 decoding to 1 byte through 50 MiB. Piqae persists it as a tenant upload before delivery. | Base64 expands the request and materializes bytes in JSON and memory. Avoid it for normal documents. |
| `uri` | Public or deliberately reachable short-lived content | Must be an absolute URI. The node fetcher accepts HTTP/HTTPS, refuses redirects and embedded credentials, pins DNS, blocks cloud metadata and non-public destinations by default, enforces bounded timeouts, and streams into bounded local storage. | The API does not persist URI credentials. The URI must remain available until pickup; expiry or URL rotation can strand the job. Prefer uploads for private content. |

### Upload lifecycle

For an upload-backed job:

1. Compute the exact binary byte length and hexadecimal SHA-256 digest.
2. Create the upload with media type, length, and digest.
3. PUT the unmodified binary body to the returned URL.
4. If `requires_completion` is true, call the completion endpoint with the
   same digest and byte length.
5. Confirm the upload is `complete`.
6. Create the job with its `upload_id` and a stable idempotency key.

```ts
const upload = await piqae.uploads.createAndPut(
  {
    media_type: 'application/pdf',
    byte_length: bytes.byteLength,
    sha256: await sha256Hex(bytes)
  },
  bytes
);

const job = await piqae.jobs.create(
  {
    target_id: 'tgt_...',
    title: 'Order 10428 label',
    content_type: 'pdf',
    content: { type: 'upload', upload_id: upload.id },
    metadata: { order_id: '10428', print_attempt_id: 'pa_123' }
  },
  'print-attempt-pa_123-v1'
);
```

A relative `upload_url` is a Piqae API route and uses normal authorization. An
absolute upload URL is a time-limited object-store capability: send only the
returned `upload_headers`. Never forward a Piqae credential to an absolute
signed URL.

Upload state is `pending`, `complete`, or `expired`. Retain the upload ID until
job creation succeeds so an interrupted client can retrieve and reconcile it
instead of generating unbounded replacement uploads.

### PDF content

Piqae transports PDF bytes and hands them to the selected node/profile. It
does not guarantee that a PDF is visually correct for a product design. The
integrator must define and preflight its own contract for:

- page count and page selection;
- page boxes and physical page size;
- portrait/landscape treatment and rotation;
- bleed and safe area;
- scaling or explicit no-scaling;
- font embedding and substitution;
- image resolution and colour expectations; and
- whether mixed page sizes are allowed.

`fit_to_page` is a print option, not a design correction mechanism. Never
silently enable it to make incompatible artwork appear printable.

### RAW content

RAW sends vendor/printer-language bytes through the native raw path. The
request must use `content_type: 'raw'`, an octet-stream upload or compatible
content source, an exact `printer_native` descriptor, and no `options`. Piqae
does not reinterpret RAW bytes or apply portable driver options. Instead it
resolves the descriptor against the authenticated node's exact printer-scoped
language report and persists the resolved language, language version, support
pack profile version, media type, driver fingerprint, support-pack digest, and
printer ID with the immutable job. The server intersects that complete binding
again before every offer, and the node compares it with its latest locally
derived trusted support-pack/driver binding before downloading job content.

```ts
await piqae.jobs.create({
  printer_id: 'ptr_...',
  title: 'Order 10428 shipping label',
  content_type: 'raw',
  printer_native: {
    output_profile_id: 'zpl.acme-model/v1',
    language_profile_id: 'zpl.acme-model/v1'
  },
  content: { type: 'upload', upload_id: 'upl_...' }
}, 'order-10428-zpl-v1');
```

If a driver, firmware, replay-tested support pack, language version, or profile
fingerprint changes after registration, the binding becomes update-required
and the bytes are withheld. Reusing a profile ID never authorizes changed
semantics. A support pack with only discovered or mapped evidence cannot
activate RAW language output.

Only enable RAW in a trusted server-side workflow that knows the exact printer
family, command language, document provenance, and authorization policy. Never
accept arbitrary browser-provided RAW data as a generic printing escape hatch.

## Job options

```ts
interface JobOptions {
  bin?: string;
  collate?: boolean;
  color?: boolean;
  copies?: number;
  dpi?: string;
  duplex?: 'one-sided' | 'long-edge' | 'short-edge';
  fit_to_page?: boolean;
  media?: string;
  nup?: number;
  pages?: string;
  paper?: string;
  rotate?: 0 | 90 | 180 | 270;
  native_options?: Record<string, string>;
}
```

Unknown option names are rejected. `copies` and `nup` must be at least 1;
`rotate` and `duplex` accept only the values shown. String fields use the
driver-facing names reported by printer discovery and the selected profile.

| Option | Capability/profile source | Integration guidance |
| --- | --- | --- |
| `bin` | `capabilities.bins`, profile summary/source | Use an advertised value or a published profile; a named bin does not prove stock is loaded. |
| `collate` | `capabilities.collate` | Relevant to multiple copies/pages; false capability means do not offer it. |
| `color` | `capabilities.color`, profile summary | A capability means the driver exposes colour, not that consumables are present. |
| `copies` | `capabilities.copies` and profile safe overrides | Prefer one application quantity model. Verify it is safe to override on a pinned profile. |
| `dpi` | `capabilities.dpis`, profile summary | Use the exact advertised string; do not invent normalized DPI labels. |
| `duplex` | `capabilities.duplex`, profile summary | Do not expose duplex when unsupported; edge semantics depend on orientation. |
| `fit_to_page` | profile safe overrides | Make scaling explicit in product UX. It cannot repair bleed or aspect-ratio errors. |
| `media` | `capabilities.medias`, profile summary | A driver media name is not proof of the physical substrate. |
| `nup` | `capabilities.nup` | Values describe pages per sheet. Treat `1` as normal single-up output. |
| `pages` | profile safe overrides | Driver-specific page-range string. Validate in the product before submission. |
| `paper` | `capabilities.papers`, profile summary | Use the stable driver key, while displaying the associated dimensions where known. |
| `rotate` | profile safe overrides | Only 0, 90, 180, or 270 degrees. Prefer correctly oriented source artwork. |
| `native_options` | `printer.native_options` plus profile `safe_overrides` | Keys and choice values must come from captured driver definitions. Never let a browser invent arbitrary native settings. |

The installed OS driver remains authoritative. Capability discovery is a
snapshot and has a monotonic `capability_revision`; a driver update, queue
replacement, or device change can make a saved choice stale.

## Capability and profile model

There are four distinct layers. Do not collapse them into one “supported” flag.

1. **Printer capabilities** are the latest portable facts reported by the
   installed driver: bins, paper keys/dimensions, media, DPI, colour, duplex,
   collation, N-up, custom-size support, extents, and optional print rate.
2. **Native options** are driver-specific choice definitions. Their stable keys
   and values may be used only through a validated profile/safe override path.
3. **Profile snapshots** pin portable options and an opaque node-local native
   configuration to an immutable revision and driver fingerprint.
4. **Stocks and targets** express business geometry and route it to one or more
   exact printer/profile revisions.

Profile status must be handled explicitly:

| Status | Meaning for an integrator |
| --- | --- |
| `ready` | Exact revision is available and currently considered usable. This still does not prove physical stock. |
| `draft`, `capturing` | Operator setup is incomplete; do not submit production jobs. |
| `needs_test` | Configuration needs the required validation fixture before promotion. |
| `stale`, `driver_mismatch` | Installed driver facts no longer match the captured revision; require recapture/revalidation. |
| `destination_missing`, `dependency_missing` | Required queue, driver asset, font, or other local dependency is absent. |
| `interactive_only` | Replay cannot be safely automated through the current executor. |
| `invalid`, `retired` | Do not use for new jobs. |

Use `targets.designSpecification(id)` when building a design or print UI. It
atomically returns stock geometry, readiness, bindings, printers, exact profile
revisions, and a `specification_revision`. Save that revision with artwork and
re-fetch before printing. A changed revision requires a fresh comparison and,
when production constraints differ, an explicit user decision.

Target readiness is either `ready` or `target_has_no_ready_binding`. Each
binding can report `ready`, `disabled`, `node_offline`, `destination_offline`,
`destination_missing`, `needs_operator`, `profile_stale`, `driver_mismatch`,
`dependency_missing`, or `busy`, with human-readable reasons. Present the
specific corrective action rather than replacing all failures with “offline”.

## Idempotency, retries, and copies

Send `Idempotency-Key` on every job creation. It must be 8–255 bytes and should
identify one intended print attempt, for example `print-attempt-pa_123-v1`.

- Retry an ambiguous HTTP result with the identical body and key.
- The same normalized request returns the original job.
- A different request under the same key returns `409 idempotency_conflict`.
- A user-authorized replacement or reprint is a new attempt with a new key and
  a recorded relationship to the original.
- Never randomize keys merely to bypass a conflict.

Idempotency prevents duplicate registration. It cannot prevent a duplicate
created by retrying after an ambiguous native/spooler handoff.

`deliveries`, `options.copies`, and repeated jobs can all increase physical
output. Define one product quantity model and test the resulting driver
behavior. For auditability, a deliberate reprint is generally clearest as a
new linked job.

## Lifecycle and state handling

Preserve the exact state and event sequence even when the host application
groups states for display.

| State | Meaning and required handling |
| --- | --- |
| `registered` | Durable registration exists. |
| `content_pending` | Required content is not yet ready for delivery. |
| `waiting_for_agent` | No eligible node has accepted the job yet; it may remain durable while offline. |
| `agent_downloading` | A leased node is obtaining and verifying bytes. |
| `agent_accepted` | The node acknowledged durable local ownership. Do not fail over automatically. |
| `queued_local` | Stored in the node's local durable queue. |
| `preparing` | Native preparation has started. |
| `rendering` | PDF/native rendering is in progress. |
| `spool_intent` | The node durably recorded intent immediately before OS handoff. Duplicate risk begins here. |
| `accepted_by_spooler` | The OS accepted the request. Physical output is not proven. |
| `spooling`, `printing` | Native observation reports progress. These remain observations, not independent paper sensors. |
| `blocked` | Native queue needs attention; retain the reason and preserve order. |
| `completed_reported` | Strongest available spooler/driver completion report, not proof that ink reached stock. |
| `delivery_uncertain` | Piqae cannot prove whether output occurred. Never automatically reprint. Require operator reconciliation. |
| `cancel_requested` | Cancellation was requested but prevention of output is not yet confirmed. |
| `cancelled` | This attempt is confirmed cancelled within available native evidence. |
| `expired` | It did not progress before its expiry. |
| `failed_retryable` | A bounded retry or operator correction may allow progress. Follow `error.retryable` and state-specific policy. |
| `failed_terminal` | This attempt will not progress; replacement requires a new authorized attempt. |

Use signed `job.updated` webhooks for the durable integration record and poll
`jobs.retrieve()`/`jobs.events()` to reconcile gaps. Webhooks are at least once:
verify the exact raw body, persist and deduplicate the event ID, then return
2xx. A browser subscription is useful for display but must not be the system of
record.

## Cancellation and uncertain delivery

Cancellation before native handoff can be definitive. After `spool_intent`,
the OS or printer may already possess the job, so cancellation remains a
request until reconciled.

For `delivery_uncertain`:

1. stop automatic retries and failover;
2. show the original job, printer, timestamps, and native reason to an
   authorized operator;
3. inspect the physical printer, queue, stock, and downstream business state;
4. call `POST /v1/jobs/{id}/resolve-uncertain` with a stable
   `Idempotency-Key`, a required operator note, and one of
   `acknowledge_printed`, `acknowledge_missing`, `cancelled`, or `reprint`;
5. treat HTTP 202 as pending until the exact node command cursor is
   acknowledged; and
6. if `reprint` is selected, use the separately linked cloud job created after
   acknowledgement. Retained Base64/upload content can be cloned; URI or
   encrypted content requires a fresh authorized submission.

The original uncertain attempt remains immutable and terminal. Its resolution,
actor, note, node acknowledgement, and optional replacement-job link are audit
records; they do not rewrite the physical evidence. No choice automatically
releases the old attempt for retry or proves that paper was produced.

## Known limits and non-claims

- Public npm installation is not available until the first SDK release is
  published; repository examples can use the workspace package meanwhile.
- The request limit for uploaded or decoded Base64 content is 50 MiB.
- Only PDF and RAW are native V1 content types. Images, HTML, office documents,
  ZPL, EPL, and other languages are not separate declared types; an integrator
  must render to PDF or deliberately use the authorized RAW path.
- Piqae does not virus-scan, visually preflight, repair, paginate, or prove the
  business correctness of a supplied document.
- URI redirects and credentials are not supported by the node fetch policy;
  private destinations are blocked by default.
- Capability snapshots and profiles do not prove that physical media,
  consumables, finishing hardware, or an attentive operator are present.
- Automatic reassignment is bounded to the pre-acceptance phase; post-handoff
  ambiguity requires operator action.
- A spooler report is not physical-delivery evidence.
- Platform service accounts, multi-integrator connectors, content encryption,
  native packages, and operating systems must be described at their exact
  current support-matrix tier.

## Related guides

- [Web design platform integration](web-design-platform-integration.md)
- [Uploads and design applications](uploads-and-design-apps.md)
- [Idempotency](idempotency.md)
- [Webhooks](webhooks.md)
- [Jobs and statuses](../printing/jobs-and-statuses.md)
- [Reliability and lifecycle](../operations/reliability-and-job-lifecycle.md)
- [Content-confidential printing](content-confidential-printing.md)
