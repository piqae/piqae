# PrintPacket v1 specification

Status: Preview specification, implemented by the checked-in conformance suite.

Normative terms such as MUST, MUST NOT, SHOULD, and MAY are used in their usual
standards sense.

## 1. Model

A print operation has four separable values:

1. an immutable `printpacket/v1` template;
2. a bounded JSON data object;
3. zero or more immutable, content-addressed resources;
4. an explicit output target and an ordinary printer job ticket.

Keeping data out of the template permits a host to cache template validation,
layout planning, resources, or final output without copying source-language
logic into the renderer. Source languages and editors are adapters: Liquid,
visual editors, POS form builders, and application code compile to PrintPacket.
They are not executed by a PrintPacket renderer.

## 2. Security and determinism

A conforming v1 renderer MUST:

- perform no ambient network, file, plugin, script, clock, locale, or font lookup;
- enforce declared limits before allocation or rendering;
- resolve expressions only from the supplied data object and repeat scope;
- verify resource media type, byte length, and lowercase SHA-256 digest;
- reject missing paths, unsupported characters/features, invalid dimensions,
  and output overflow instead of substituting or silently degrading;
- produce identical bytes for the same canonical template, data, resources,
  output profile, and conformance suite.

Templates and input may contain sensitive customer data. Cache keys, telemetry,
and capability reports MUST NOT contain their plaintext. Durable content stores
SHOULD encrypt the values and MUST scope them to the owning application or
tenant.

## 3. Media profiles

`paged` represents A4, A5, or Letter documents. `continuous` represents a
receipt/ticket roll with a fixed width and bounded computed height. `label`
represents one fixed-size label. Dimensions and margins use millimetres; font
sizes use points.

The media declaration describes layout, not printer discovery or loaded stock.
A queueing system must separately match it to truthful printer/media capability
evidence and an installed-driver profile.

## 4. Layout and data

V1 supplies flow sections, boxes, paragraphs/headings, rows, weighted grids,
tables, repeats/data lists, conditions, spacers/dividers, page breaks,
keep-together groups, JPEG images, QR, and Code 128. Expressions provide typed
paths, current repeat scope, coalesce/concat, comparisons, booleans, existence,
membership, arithmetic, bounded number/money/date formatting, and ASCII string
normalisation.

V1 is not an arbitrary coordinate canvas. A future feature that changes the
schema or could make an old v1 validator misinterpret a packet requires a new
format identifier. Clarifications that preserve the same accepted documents
and bytes may update this text and conformance suite metadata.

Rows and grids contain at most 32 children; a grid has exactly one positive
column weight for each child. Layout gaps are finite and between 0 and 2,000 mm.
Adjacent inline nodes concatenate without an implicit separator. Horizontal
whitespace in resolved content collapses to one breakable space. A `line_break`
node, or LF, CRLF, or CR in a resolved value, starts a new line; leading
whitespace on that line or an automatically wrapped line is discarded.
Image dimensions are greater than 0 and at most 2,000 mm; v1 supports
`contain`, `fill`, and `scale_down`, with `scale_down` never enlarging an image.
QR size is 8 through 2,000 mm, Code 128 is at least 20 by 8 mm and at most 2,000
by 2,000 mm, divider width is 0.1 through 10 pt, and headings use levels 1
through 6. Continuous media rejects a `page_break` anywhere in its recursive
node tree. Header and footer variants are measured before body layout and each
region's maximum rendered height is 60 mm.

## 5. Typography and resources

The initial `printpacket.pdf-base14/v1` profile uses deterministic PDF Base-14
Helvetica faces and Windows-1252 text. Other scripts and downloadable fonts
require a new explicit output profile containing a fixed font bundle digest.

Renderers measure the selected Helvetica face using its Base-14 glyph widths
for wrapping, alignment, and decoration. `printpacket.conformance/core-v2` is
the first public suite that enforces these metrics and the inline whitespace
rules above; `core-v1` remains a historical compatibility identifier only.

V1 image resources are JPEG and are referenced by an application-chosen local
resource key plus their SHA-256, media type, and byte length. Remote URLs and
data URLs are not valid resource declarations. A host supplies verified bytes
out of band.

## 6. Compatibility and updates

A renderer advertises:

- exact format versions and conformance suites;
- semantic feature identifiers;
- exact output targets;
- input/output/page/resource limits;
- accepted resource media types;
- whether resource caching and direct offline rendering are available;
- an implementation version for diagnostics.

A job declares the exact version, required features, target, and limits. A host
may render only after all requirements match. Application-version comparison is
not compatibility evidence by itself.

An older node or embedded SDK that lacks a required version/feature reports
`node_update_required`, its implementation version, supported versions, and
missing feature identifiers. A queue may use an already-approved PDF fallback
when policy allows. `require_node` and direct-offline calls fail before enqueue.
The calling Shopify/POS/ERP integration can therefore show the affected node,
application, missing capability, safe fallback, and update action.

## 7. Caching

`printpacket.canonical-template/v1` serializes typed template fields in
declaration order and sorts declared resource-map keys. Template
canonicalization has its own version and never accepts or normalizes a format
alias.

`printpacket.canonical-data/v1` starts with that UTF-8 domain followed by a NUL
byte and encodes data without JSON number spelling ambiguity. Null, false, and
true are `n`, `f`, and `t`. A string is `s<byte_length>:` followed by its UTF-8
bytes. A number is `d` followed by the 16 lowercase hexadecimal digits of its
IEEE 754 binary64 bits; negative zero is normalized to positive zero, non-finite
values are rejected, and integral values with an absolute value greater than
2^53 - 1 are rejected. An array is `a<count>:` followed by its encoded values.
An object is `o<count>:` followed by each encoded key and value, with keys
ordered by Unicode scalar value and then UTF-8 bytes. Canonical data is bounded
to 4 MiB and 128 levels. This is a versioned typed encoding, not an RFC 8785
claim.

The render-cache identity is SHA-256 over a domain separator, canonical template
digest, conformance suite, explicit output target, and canonical data. Resource
digests are already inside the canonical template. Cache entries must also be
scoped by application/tenant where authorization differs. A conformance or
output-profile change creates a new key.

## 8. Output and handoff

PDF is the portable baseline because native drivers can apply their actual
media and device capabilities. PWG Raster, Apple Raster, or printer-native
command streams are output profiles, not template dialects.

Raw output requires an exact language/profile, DPI, printable width, and
reviewed printer support evidence. A raw renderer must fail before durable
enqueue when any requirement is absent. A spooler/native handoff remains only
evidence of acceptance, not proof of physical delivery.
