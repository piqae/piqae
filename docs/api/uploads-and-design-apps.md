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
or standby binding and why other bindings are unavailable.

Before presenting a printable template, a design application should:

1. load the target and stock;
2. check target readiness;
3. load the selected binding's printer/profile revision;
4. use stock geometry as the intended design size;
5. compare it with the profile `summary.dimensions_mm`;
6. block or warn when geometry is missing or materially different.

Dimensions are facts, not proof that the correct roll or tray is physically
loaded.

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
