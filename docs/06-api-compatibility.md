# Drop-in API compatibility

## Compatibility strategy

Expose two contracts:

- `/` or a dedicated compatibility hostname implements the documented
  legacy API behavior needed by existing SDKs.
- `/v1/...` is the native API with modern authentication, streaming uploads,
  richer states, and administrative functions.

Do not add native fields to compatibility responses by default. Extra fields
can break strict consumers even when JSON normally permits them.

Compatibility is versioned by a published matrix:

- endpoint implemented;
- request fields and validation;
- response body and headers;
- status/error behavior;
- pagination/ordering;
- side-effect semantics;
- official SDKs tested;
- known differences.

## Authentication

### Compatibility

Support an API key as the HTTP Basic username with an empty password, as the
legacy-provider examples do. Return compatible `401` behavior and relevant
authentication/account headers.

Store only a versioned password hash of each API key. A key has scopes even
though the compatibility API may present unrestricted behavior:

- printers read;
- jobs read/create/cancel;
- RAW print;
- scales read;
- webhooks manage;
- accounts manage.

### Native

Support:

- scoped API keys;
- short-lived OAuth/OIDC user sessions for the UI;
- service accounts;
- tenant/workspace selection;
- device certificates or proof-of-possession credentials;
- audit events for key creation, use, rotation, and revocation.

## Resource identifiers

legacy identifiers are positive integers. The native model should use
sortable globally unique IDs internally, with a per-deployment integer mapping
for compatibility resources.

Requirements:

- mappings never change or reuse deleted IDs;
- integer generation is transactional;
- comma-separated sets are parsed with deduplication;
- authorization is checked after resolution without revealing foreign-tenant
  existence.

## Endpoint implementation plan

### Milestone 1: printing subset

- `GET /whoami`
- `GET /computers[/{set}]`
- `GET /printers[/{set}]`
- `GET /computers/{set}/printers[/{set}]`
- `POST /printjobs`
- `GET /printjobs[/{set}]`
- printer-filtered job reads;
- compatible pre-delivery cancellation routes;
- `GET /printjobs/states`
- `GET /printjobs/{set}/states`

### Milestone 2: webhooks and management

- view/create/modify/delete webhooks;
- computer deletion;
- account API keys and tags where publicly specified;
- account state.

### Milestone 3: scale parity

- list/read scales;
- virtual scale testing;
- WebSocket subscription behavior.

### Milestone 4: integrator parity

- child account CRUD;
- child selection headers by ID/email/creator reference;
- child account state, tags, and keys;
- client enrolment keys;
- delegated authentication.

### Milestone 5: distribution and utility parity

- client download listings and enable/disable management;
- operating-system-specific latest-client lookup;
- `ping` and `noop`;
- remaining account controllability queries.

The exact public reference must be captured as machine-readable contract tests
before implementing each milestone.

## Print job create contract

Compatibility accepts JSON and form-encoded bodies.

Validation sequence:

1. Authenticate and resolve child account headers.
2. Parse request and allocate/request ID.
3. Validate idempotency key scope and existing result.
4. Resolve printer and authorise access.
5. Validate required fields and content type.
6. Validate declared options against the compatibility schema; capability
   validation may occur at the agent because printer state can change.
7. For base64, decode to a streaming temporary object with size limit and
   digest.
8. Transactionally register job, compatibility ID, initial state, content
   reference, idempotency result, and routing outbox.
9. Return HTTP 201 and the integer job ID only after the configured durable
   stores commit.

An identical idempotency key and request should return the recorded result in
the native API. Compatibility mode should reproduce the legacy service's documented 409
on reuse.

## Option schema

Compatibility recognises exactly:

```json
{
  "bin": "Tray 1",
  "collate": true,
  "color": false,
  "copies": 2,
  "dpi": "300x300",
  "duplex": "long-edge",
  "fit_to_page": true,
  "media": "Labels",
  "nup": 1,
  "pages": "1,3-5",
  "paper": "A4",
  "rotate": 90
}
```

RAW jobs ignore printing options in compatibility mode. The native API should
reject options on RAW by default because silently ignoring them hides mistakes;
an explicit compatibility flag can preserve the old behavior.

## Pagination, filtering, and ordering

Implement:

- default `limit=100`;
- `dir=asc|desc`;
- exclusive `after={id}`;
- stable ordering by compatibility integer ID;
- record count/limit/offset headers where the reference returns them;
- comma-separated set filters;
- printer/computer nesting.

Run differential tests for missing, empty, repeated, unordered, malformed, and
unauthorised ID sets. Error details often reveal compatibility gaps before the
happy path does.

## Errors and request IDs

All API responses carry `X-Request-Id`; compatibility error `uid` is always
the exact same value. Unsafe or oversized caller identifiers are replaced with
`req_<ULID>`. Compatibility errors project the internal error to:

```json
{
  "uid": "request-id-when-applicable",
  "code": "StableCode",
  "message": "Human-readable detail"
}
```

The native error additionally includes:

- documentation URL;
- retryability;
- structured field violations;
- trace ID;
- causal stage (`api`, `content`, `agent`, `renderer`, `spooler`, `device`).

Never expose native stack traces, database errors, URI credentials, document
content, or cross-tenant identifiers.

## Printers and capabilities

The compatibility projection must preserve:

- object nesting and nullable values;
- paper dimensions in tenths of a millimetre;
- capability names exactly as the driver reports them;
- computer connected/disconnected state;
- printer online/offline state;
- default flag behavior.

Native capability responses add:

- source and source revision;
- timestamp and staleness;
- stable native keys separate from display names;
- imageable area;
- explicit duplex/color variants;
- vendor extensions;
- capability query warnings.

## Job states

Compatibility preserves the five stable states and their meaning. In
particular, `done` is emitted at OS spooler acceptance, not after the richer
native completion state.

State age is calculated from the initial `new` event. Client-reported
timestamps, server receipt time, and estimated clock offset are stored
separately so clock skew cannot reorder events.

## Webhooks

Compatibility mode:

- accepts URL, secret, and message selection;
- emits arrays of documented computer/job events;
- expects the legacy acknowledgement response;
- reproduces the stable event types and state projection.

Native mode:

- signs raw body with HMAC-SHA256 and timestamp;
- supplies unique delivery/event IDs;
- has configurable exponential retry over hours/days;
- preserves events in a dead-letter view;
- supports replay without changing event IDs;
- allows more than five endpoints subject to deployment policy;
- supports printer, agent health, rich job, and audit events.

Webhook receivers must be protected against DNS rebinding and access to
internal/cloud metadata addresses. Self-hosted administrators may explicitly
allow private targets.

## WebSocket APIs

There are two distinct sockets:

1. The private, documented open-source agent protocol.
2. The customer/browser live API.

Do not expose the agent socket to browser clients. Browser connections use
short-lived tokens, per-workspace authorization, subscription limits, and
backpressure.

The legacy-compatible browser socket can be added when scale parity is
implemented. Before claiming compatibility, test the public JavaScript SDK's
authentication, hierarchical subscription filters, initial snapshot, live
measurements, unsubscription, and connection tracking against both services.

## Official SDK migration findings

Public legacy-provider SDK inspection found:

- Python accepts a configurable gateway URL.
- Ruby accepts an API URL constructor argument.
- Java has an API URL setter.
- JavaScript has a configurable server option, though fallback logic contains
  legacy API host assumptions.
- PHP contains an overridable endpoint/host, but migration ergonomics require
  testing by version.

Create first-party SDKs for TypeScript, Python, PHP, Ruby, Java, Go, Rust, and
.NET only as demand warrants. The native API should begin with TypeScript and
one server-side language used internally. A concise cURL workflow remains the
canonical contract.

## Compatibility testing method

For each endpoint:

1. Generate valid and invalid fixture requests.
2. Send them to a dedicated legacy-provider test account and the clone.
3. Normalise nondeterministic IDs, timestamps, and request IDs.
4. Compare status, headers, JSON shape, ordering, and durable side effects.
5. Record intentional differences in the matrix.

Never send sensitive production documents into differential testing. Use
generated PDFs, virtual/file printers, and RAW fixtures.

## Migration experience

Publish:

- a base-URL change example for each supported legacy-provider SDK;
- a legacy API key/resource import tool that imports metadata only where lawful
  and available;
- an agent installation and printer remapping workflow;
- a dry-run endpoint that validates printer/options without output;
- parallel shadow status observation without printing twice;
- a cutover checklist and rollback plan.

There is no safe transparent proxy that can mirror a live create-print request
to both services. Shadow testing must not duplicate the side effect.
