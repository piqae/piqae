# Multi-integrator node connectors

Piqae separates a physical node installation from the tenant connector that is
visible through the API. One installation may therefore authorize multiple
workspace/environment pairs while every integrator continues to see only its
own node, printers, jobs, events, and connector grant.

`GET /v1/nodes/{node_id}/connectors` lists only connectors belonging to the
authenticated workspace and environment. It deliberately omits the physical
installation identifier and never discloses whether another service uses the
same computer. `DELETE /v1/nodes/{node_id}/connectors/{connector_id}` revokes
that tenant projection and leaves connectors for other tenants untouched.

Connector creation is transactional with token enrolment. Concurrent
enrolments for the same installation serialize on the installation key and
create at most one grant per workspace/environment. Legacy agents are
backfilled one-to-one: historical tenant-local identifiers are not trusted to
merge installations across tenants.

## Current implementation and release limits

The agent has a durable connector registry, separate signing and content keys,
SQLite/content roots, synchronization cursors and outboxes per connector, plus
bounded fair scheduling across enabled connectors. Connector enrolment requires
proof by the immutable physical-installation key and persists a durable local
printer policy. The operator can authorize every printer on the computer,
including printers added later, or only explicitly selected printer IDs. An
empty selected-printer list never means all printers. A local-first
installation activates only connectors accepted through the native consent
flow. Its menu aggregates their live health without disclosing one connector
to another.

This remains a **Preview implementation**, not a Supported fleet claim. macOS
has the Piqae-branded browser-to-app handoff and local consent flow in signed,
notarised packaging. Windows and Linux have the shared headless
stdin-only preview/accept transaction but do not register an application link
or provide an equivalent native consent shell. No platform yet has the
clean-install physical-printer, concurrent-fleet and revocation-soak evidence
required for Supported promotion.

Each connector's discovery projection and queue are tenant-scoped. For design
editors that need the complete printer, stock, and captured-profile inventory,
`all_local_printers` is the recommended policy. Sites requiring strict device
separation can choose `selected_printers`. Another connector cannot infer that
a printer or service exists. Revocation, key loss and partial restart recovery
still require release evidence across every advertised operating system.
