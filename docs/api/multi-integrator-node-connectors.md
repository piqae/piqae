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
proof by the immutable physical-installation key and persists the exact local
printer selection; an empty selection never means all printers.

This remains a **Preview implementation**, not a Supported fleet claim. macOS
has the Piqae-branded application-link and local consent flow in source and
tested unsigned packaging. Windows and Linux have the shared headless
stdin-only preview/accept transaction but do not register an application link
or provide an equivalent native consent shell. No platform has the signed,
clean-install, physical-printer, concurrent-fleet and revocation-soak evidence
required for production promotion.

Each connector's discovery projection and queue are tenant-scoped. The local
operator grants concrete printer IDs; another connector cannot infer that a
printer or service exists. Revocation, key loss and partial restart recovery
still require release evidence across every advertised operating system.
