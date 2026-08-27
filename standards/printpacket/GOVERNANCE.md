# PrintPacket governance

PrintPacket is developed in public under Apache-2.0. Piqae currently maintains
the reference implementation, schema, and conformance suite, but Piqae-specific
accounts, routes, queues, APIs, and product metadata are not part of the
standard.

Changes should be proposed with:

1. the user/developer use case and a transport-neutral example;
2. an updated exact schema and feature identifier;
3. valid, invalid, limit, and security fixtures;
4. renderer golden outputs for every affected output profile;
5. compatibility behavior for an older parser/renderer;
6. independent implementation feedback before declaring a stable profile.

Editorial corrections may update the text without changing accepted packets or
output. A schema addition or semantic change creates a new exact format/profile
identifier. A renderer byte change creates a new renderer profile and golden
suite. Existing identifiers and fixtures are immutable once marked Stable.

The checked-in v1 contract is Preview while the independent package/repository,
media-type registration strategy, Unicode/font profile, GS1 validation, and
physical printer-language certification are still being completed. Preview
does not weaken compatibility inside a released Piqae build: existing stored
`piqae.business-document/v1` revisions remain readable through the frozen
adapter.

Issues and proposals must not include customer print data, credentials, device
keys, printer addresses, or production logs.
