# Document format adapters

Piqae treats an already-rendered PDF as the universal print interchange. An
application may use pdfme, HTML, Liquid, Typst, or another renderer locally and
submit the PDF through the normal print SDK without converting its template.

When an application wants Piqae's controlled renderer, the optional
[`@piqae/document-adapters`](../../sdk/document-adapters/README.md) package
can convert a supported source subset to `piqae.document/v1`. The first adapter
targets pdfme JSON templates. It performs data conversion only: it never loads
pdfme, plugins, JavaScript, files, or network assets.

## Fidelity and compatibility

Every conversion returns `exact`, `lossy`, or `incompatible`, plus structured
warnings and errors containing stable codes and JSON paths. Strict conversion
is the default, so a known loss such as reducing pdfme absolute boxes to Piqae's
flow layout produces no document. A caller must explicitly select non-strict
conversion and surface its warnings before using a lossy result.

The versioned, machine-readable
[`pdfme-v1.json`](../../sdk/document-adapters/compatibility/pdfme-v1.json)
matrix is the support authority for each node and feature. "Reviewed source
versions" does not imply complete compatibility with those pdfme releases.

## Full fidelity versus native distributed rendering

Use local pdfme generation followed by ordinary PDF submission when exact
pdfme output matters. Use adapter conversion when the declared subset is enough
and deterministic rendering inside Piqae is more important. These modes can
coexist in one application and do not change the existing PDF/RAW APIs.

Piqae must not claim that converting a template makes printer-native raw output
possible. A converted document currently produces PDF. Future raw/receipt
compilers require explicit printer capability profiles and equivalence tests;
they must fall back visibly to PDF rather than silently changing output.

## Server API boundary

Conversion can remain explicit and client-side. The optional hosted boundary is
`POST /v1/document-conversions`, followed by
`GET /v1/document-conversions/{conversion_id}`. It accepts only the exact
`pdfme@1.0.0` data-only subset and requires an idempotency key. It never loads
pdfme, plugins, JavaScript, local files, background PDFs, or remote assets.

Each successful conversion atomically stores the tenant, exact adapter ID and
version, adapter API version, source format and SHA-256 digest, strict decision,
fidelity result, renderer version, and encrypted converted document with its
diagnostics. Source JSON is not retained. An idempotency-key replay with a
different adapter, strict decision, or canonical source is rejected. The stored
renderer and adapter versions are evidence, not instructions to run whatever
version happens to be installed during a later retry.

Incompatible conversions return an error and are not persisted. The normal
render API continues to accept only a versioned Piqae document, so hosted
conversion remains fully optional.
