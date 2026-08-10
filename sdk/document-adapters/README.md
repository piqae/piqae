# Piqae document adapters

`@piqae/document-adapters` converts a deliberately limited subset of external
JSON template formats into `piqae.document/v1`. It is optional: Piqae printing
continues to accept already-rendered PDF or raw jobs without an adapter.

Adapters are conversion code, not third-party renderer runtimes. They do not
load plugins, execute template code, read files, or fetch remote fonts/images.
Every result contains stable diagnostics and a fidelity classification. The
default strict mode fails if a mapping is knowingly lossy.

```ts
import { pdfmeAdapter } from "@piqae/document-adapters";

const converted = pdfmeAdapter.convert(pdfmeTemplate); // strict by default
if (!converted.document) {
  // Show converted.errors and either adjust the template or render with pdfme.
}

const preview = pdfmeAdapter.convert(pdfmeTemplate, { strict: false });
// Only submit preview.document after the user accepts every warning.
```

## pdfme modes

There are two intentionally distinct integration modes:

1. **Full fidelity:** run pdfme in the application/browser, then submit its PDF
   bytes through the ordinary Piqae SDK. This preserves pdfme behavior and is
   the recommended path for existing or complex templates.
2. **Piqae-native render:** convert the supported subset with this package,
   submit the resulting `piqae.document/v1` plus the input JSON, and let Piqae
   render and print it. This enables controlled distributed rendering but only
   has the fidelity declared by the compatibility manifest.

The current adapter maps blank preset pages, pages, text, font size and QR data.
Named pdfme fields become RFC 6901 JSON Pointers; `~` is escaped as `~0` and
`/` as `~1`, so names remain unambiguous.
Because document/v1 is flow-based, pdfme absolute positioning is currently a
lossy mapping. Background PDFs, images, pdfme table schemas, non-QR barcodes, custom fonts,
plugins and network assets are rejected. See
[`compatibility/pdfme-v1.json`](compatibility/pdfme-v1.json) for the
machine-readable matrix.

Source versions in the matrix mean reviewed schema families, not a guarantee
that every feature from those versions is supported. New mappings must add a
fixture and update both the manifest and tests. Unsupported inputs must never be
silently discarded.

## Versioning contract

- Adapter interface: `piqae.adapter/v1`
- Compatibility manifest: `piqae.adapter-compatibility/v1`
- Target document: `piqae.document/v1`
- Adapter behavior is pinned by its own semantic version.

A server-side adapter API is intentionally not part of this first package. The
client converts explicitly, which prevents an unannounced adapter upgrade from
changing an in-flight or retried render. A future API should store the adapter
ID, exact adapter version, source digest and converted document together.
