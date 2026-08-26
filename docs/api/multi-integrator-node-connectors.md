# Multi-integrator node connectors

Piqae separates a physical node installation from the tenant connector that is
visible through the API. One installation may therefore authorize multiple
workspace/environment pairs while every integrator continues to see only its
own node, printers, jobs, events, and connector grant.

The corresponding customer-facing information architecture and recommended
consent/Queue language are documented in
[White-label and integrator product UX](integrator-white-label-ux.md).

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

Locally discovered printers do not have a second cloud-exposure switch. The
connector consent grant is the authority: `all_local_printers` includes the
current inventory and printers discovered later, while `selected_printers`
remains limited to the durable IDs explicitly approved. Every authorized
printer publishes its generated **Current printer defaults** preset as the
initial job default. That live preset follows the installed driver's defaults;
captured native presets remain immutable, revision-pinned configurations.

This remains a **Preview implementation**, not a Supported fleet claim. macOS
has the Piqae-branded browser-to-app handoff and local consent flow in source
and unsigned packaging tests, but no signed/notarised release evidence is
recorded. Windows and Linux have the shared headless
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

## Connection and printer topology invariants

The connector registry is many-to-many. A durable connector is identified by
its immutable server origin/identity, workspace, environment and connector ID;
workspace names, customer names and future vanity domains are display data and
must not merge or replace it. This permits one installation to maintain any
combination of direct hosted workspaces, managed child workspaces and distinct
self-hosted control planes. Reauthentication replaces only the matching
connector identity and leaves every peer connector running.

One node-owned printer ID may be projected into every authorized tenant. The
PostgreSQL identity is consequently `(workspace_id, environment_id,
printer_id)`, not a globally unique printer ID. Each projection contains the
current state, driver/native options, semantic capabilities, profiles and
last-seen time allowed by that connector's grant. An inventory failure remains
dirty and retries on the next synchronization cycle; a later heartbeat must
not hide a missing projection. Additions, removals and driver changes wake all
connector workers, with periodic reconciliation as a recovery bound.

| Topology | Identity and isolation behaviour |
| --- | --- |
| One node, several hosted direct or child connectors | Each tenant receives an isolated projection of every granted local printer. |
| One node, hosted and self-hosted connectors | Each immutable server origin has independent credentials, queues, cursors and content roots. |
| Several nodes with disjoint printers | Each OS queue remains a separate printer route. |
| Several nodes exposing the same physical device | They are separate routes. The control plane may attach a new route to an existing physical destination only when same-kind strong or verified identity evidence matches unambiguously and no conflicting strong evidence exists. Names, drivers and queue labels never merge routes. Ambiguous evidence requires review. |
| Different connector subsets on different nodes | Only the explicit grant on each connector controls visibility and job admission. |
| Printer added, removed or reconfigured | Every connector is invalidated immediately and later reconciled; selected-printer grants never widen automatically. |

## Shared destinations and queue privacy

Within one installation, the local coordinator serializes Piqae handoffs for
routes grouped as one physical destination while preserving a tenant-private
durable queue for every connector. A tenant may receive privacy-safe aggregate
occupancy and work-ahead counts, but never another tenant's job title,
document, metadata or identifiers. Jobs submitted directly to the operating
system spooler are outside Piqae's ordering, reservation, and idempotency
boundary and may only appear as bounded native-queue observations.

Within one control plane, multiple compatible routes can serve a physical
destination. A destination reservation fences one job to one route and keeps a
stable destination order. The scheduler may choose another fresh compatible
route only before durable native acceptance. After spooler handoff or an
ambiguous crash, the attempt becomes `delivery_uncertain`; it must not print
automatically through a second node.

Hosted and self-hosted connectors on the same installation remain independent
authorities. They do not share a reservation ledger, global FIFO, job contents,
or an exactly-once physical-delivery guarantee. The local coordinator can
serialize the Piqae handoffs it sees, but one server cannot automatically fail
over a job registered on another server. Connector health and matching queue
names must never imply cross-authority ownership or redundancy.
