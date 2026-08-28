# Uploads and design applications

**Status:** tenant-scoped proxy uploads, typed stock/target discovery, and
profile geometry are implemented. Signed direct object-store upload generation
is an adapter extension point and is not yet a supported Cloud capability.

## Documents

Do not embed large or private documents as Base64 JSON. The V1 flow is:

1. calculate the exact byte length and SHA-256 digest;
2. `POST /v1/uploads`;
3. `PUT` the binary body to the returned `upload_url`;
4. for a future direct object-store URL, call the completion endpoint when
   `requires_completion` is true;
5. create the job with `content.type = upload`.

The current server advertises and enforces a 50 MiB maximum. The upload expires
after one hour if it is not completed. Length and digest mismatches fail closed.
`GET /v1/uploads/{upload_id}` can reconcile an interrupted client request.

The proxy request and node download are streamed in bounded chunks. Base64 job
content remains supported for compatibility, but necessarily expands and
materializes the document and should be reserved for small payloads.

An `upload_url` beginning with `/` is a Piqae proxy URL and needs the normal
API-key header. An absolute URL is a time-limited storage capability: send only
the returned `upload_headers` and never forward a Piqae API key to it.

## Printer and profile discovery

`GET /v1/printers` returns:

- driver-reported paper names and dimensions;
- bins, media, DPI, colour and duplex capabilities;
- immutable profile revisions;
- portable profile summaries such as `dimensions_mm`, source and media;
- publication, validation and driver-fingerprint state;
- the names of options that are safe for per-job override.

Opaque PrintCore, DEVMODE, PrintTicket, PostScript and manufacturer settings are
never returned. A design application can display the portable summary, but it
must not attempt to reproduce or edit the native driver UI.

## Stocks and logical targets

Stocks hold stable business identities and optional portable design facts:

```json
{
  "name": "62 × 29 chilled label",
  "sku": "LABEL-62-29-CHILL",
  "attributes": {
    "kind": "label",
    "width_mm": 62,
    "height_mm": 29,
    "gap_mm": 3,
    "bleed_mm": 1.5,
    "safe_area_mm": {"top": 2, "right": 2, "bottom": 2, "left": 2}
  }
}
```

A target binds that stock to one or more exact printer/profile revisions.
`GET /v1/targets/{target_id}/readiness` reports the currently selected primary
or standby binding and why other bindings are unavailable. This is generic
route readiness: a stockless target may carry PDF or an independently validated
printer-native job. PrintPacket stock safety is the separate per-destination
`media_compatibility` projection.

`GET /v1/targets/{target_id}/design-specification` performs these joins in one
tenant-scoped read and returns a `specification_revision` that changes only
with target routing constraints, the stock revision/attributes, or immutable
binding identities. Heartbeats, printer timestamps, current loaded-media
evidence, and temporary availability do not churn it. Save that revision with
artwork and re-fetch before printing to detect production setup changes.
Unavailable binding printer/profile snapshots are skipped from `destinations`
but remain visible with reasons under `readiness.bindings`.

Before presenting a printable template, a design application should:

1. load the target and stock;
2. check target readiness;
3. load the selected binding's printer/profile revision;
4. use stock geometry as the intended design size;
5. require the selected destination's `media_compatibility.status` to be
   `ready`; and
6. use `media_compatibility.profile_dimensions_mm` to compare the immutable
   profile with the stock, while failing closed when the stock itself omits
   the required kind, width, or height.

Sheet width/height describe physical stock and therefore match portrait or
landscape page geometry. An explicit stock `orientation` of `portrait` or
`landscape` restricts output; `either` or omission allows either. Label width and
height are ordered and cannot rotate unless stock explicitly declares
`rotatable: true`.

`media_compatibility` reports `ready`, `not_reported`, `stale`, `untrusted`, or
`incompatible`, with actionable `reasons`. Its optional `loaded_media` evidence
identifies the source, confidence, observation time, 15-minute `fresh_until`,
and exact loaded stock revision. Only `reported` or `operator_confirmed`
evidence with current calibration can authorize a new handoff. Missing,
inferred, unknown, or expired evidence never means that the expected stock is
loaded. Evidence is selected from the immutable profile `summary.source` or
profile `options.bin`. A per-job bin can change that source only when the exact
profile declares `bin` as a safe override; a correct roll in another tray does
not satisfy the pinned source.

Printing a rendered PrintPacket to a target requires the current
`specification_revision`. Registration validates the document media against
the target stock and immutable profile dimensions, then pins the binding,
profile, stock, and specification revisions. Piqae repeats those checks after
the server lease is claimed and immediately before an offer can transfer
native responsibility. A changed target/profile/driver, stale loaded-stock
evidence, or incompatible document media ends that unaccepted attempt with a
durable actionable event. Correct the setup and create a new print attempt;
never assume that the failed attempt printed. Direct concrete-printer and
printer-native jobs retain their distinct contracts and do not silently fall
back into this target-media path.

The lower-level `POST /v1/print-intents/validate` path also fails closed when
an intent names a stock revision. It compares every declared document page box
with the explicit stock kind and geometry, checks an exact workflow profile
when one is pinned, and requires fresh trusted evidence for the selected media
source. A loaded-media-only problem returns `operator_action_required`; an
invalid document, stock, workflow, profile, or capability returns `invalid`.

## Tenant boundary

Normal API keys belong to exactly one workspace and one Test or Live
environment. Resource IDs from another tenant return the same not-found
response as unknown IDs. Do not add workspace-selection headers to print API
calls.

A single-workspace integrator should create a separate environment-scoped key
for each customer workspace. A multi-tenant SaaS backend can instead use the
implemented-preview platform service-account grant and account-scoped SDK
facade described in [the web design platform guide](web-design-platform-integration.md).
That feature remains Disabled as a production support claim until its release
evidence passes. Ordinary print keys must never become cross-tenant
administration credentials.
