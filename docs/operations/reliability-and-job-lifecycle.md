# Reliability and job lifecycle

**Status:** the durable control-plane queue, leased node pickup, target-based
primary/standby selection, pre-acceptance reassignment, durable local queue,
native handoff intent, and spooler reconciliation are implemented.
Destination-wide ordering, route reservation/acceptance fences, and the
installation coordinator have disposable PostgreSQL and non-physical evidence.
Physical-printer redundancy, production regional failover, rolling-fleet and
the release soak gates in this document are not yet proven.

Piqae has two different reliability responsibilities:

1. keep the control plane available;
2. never lose or blindly duplicate a job that it has durably accepted.

An HTTP uptime percentage alone does not prove the second property. A 99.0%
monthly availability target permits about 7 hours 18 minutes of downtime. The
private-beta design target remains 99.95%, about 21 minutes 55 seconds per
30-day month, while print correctness has a separate zero-silent-loss
objective.

## The durable chain

```text
client
  -> API + idempotency record
  -> PostgreSQL job/event history + object storage document
  -> short server lease
  -> node SQLite inbox + content-addressed file
  -> persisted spool intent
  -> OS spooler native job ID
  -> observed spooler state
```

Each arrow has an acknowledgement boundary. The sender retries only when the
receiver can prove that replay is idempotent.

| Boundary | Durable evidence | Safe recovery |
| --- | --- | --- |
| Client to API | Job ID, request hash, idempotency key, PostgreSQL row | Return the original job for the same key and body |
| Document storage | Expected digest/length and completed upload row | Retry upload before registering the job |
| Server to node | Expiring lease ID and secret bound to job/node | Re-offer after lease expiry only if no durable node acceptance exists |
| Destination route choice | Attempt generation plus destination-wide reservation whose fencing token is stored only as a digest | Release or reroute only before native handoff; a superseded generation cannot resume |
| Node acceptance | SQLite job, inbox receipt, content digest, pending acceptance intent | Replay the same acceptance until the server confirms it |
| Node to spooler | SQLite `spool_intent` written before the native call | Never blindly resubmit after an ambiguous native call |
| Spooler observation | Native job ID and append-only observations | Reconcile to queued/printing/completed/failed/cancelled or uncertain |

PostgreSQL and object storage are the server durability boundary. SQLite and
the content-addressed local file are the node durability boundary. Memory,
WebSockets, tray state, and logs are never authoritative queues.

## Lifecycle and retry rules

### Before node acceptance

`registered`, `content_pending`, `waiting_for_agent`, and
`agent_downloading` have not crossed the local acceptance boundary.

- API retries require the same idempotency key and request body.
- A short node lease prevents two nodes from concurrently accepting one job.
- The node renews the lease while downloading and validating content.
- Lease loss before local persistence aborts the acceptance attempt.
- An offline target remains visible as `waiting_for_agent`; it is not reported
  as printed or failed merely because a live connection disappeared.
- A route without an acknowledged installation-stable route key is held as
  `node_upgrade_required`; it receives no unfenced compatibility offer.
- Expiry is explicit and produces an event.

Rerouting to another node is safe only in this phase and only when the selected
printer/profile/stock contract is equivalent. A target-based job chooses the
first ready primary or standby binding at registration and pins its concrete
node, printer, profile revision, and stock metadata. If every node is offline
but the target has a valid configured binding, the API durably registers the
job as `waiting_for_agent` instead of rejecting or discarding it.

On a later node sync, Piqae examines a bounded oldest-first batch of waiting
target jobs. Reassignment uses a PostgreSQL row lock and a final conditional
update requiring the correct tenant, a waiting/retryable state, no unexpired
lease, and no durable acceptance. The target, binding, immutable profile
revision, and stock contract are revalidated in the same transaction. A
successful change writes exactly one `job_routing_attempts` row and a routing
outbox event. If the original node returns and remains the selected binding,
the job needs no reassignment and is offered normally.

### Durable node acceptance

The node downloads and verifies content, validates the printer and immutable
profile revision, allocates a per-printer sequence, and writes the job plus a
cloud-acceptance intent in one local transaction. It then confirms acceptance
to the server and activates the local job.

If either process crashes between those operations, the pending intent is
replayed. The server binds acceptance to the job, node, lease, digest, and
local sequence. A later node must not receive the same accepted job.

The server orders eligible work for one physical destination by stable
`(created_at, job_id)` order across its routes. The node retains a route-local
acceptance sequence and an installation-wide coordinator serializes final
handoffs from every connector using the same locally identified physical group.
Separate physical destinations can progress concurrently.

### Before native handoff

`queued_local`, `preparing`, `rendering`, and a retryable failure before
`spool_intent` can be retried locally with bounded exponential backoff. Invalid
documents, unsupported options, stale profiles, and missing driver
dependencies become explicit operator or terminal failures rather than
infinite retries.

The exact immutable native profile revision is pinned to the job. Editing a
profile creates a new revision and cannot mutate an already accepted job.

### Native handoff

The node persists `spool_intent` before calling CUPS or Winspool. There are
three outcomes:

- a native job ID is returned: record `accepted_by_spooler` and reconcile;
- the call definitely failed before handoff: retry only when classified safe;
- the process or driver fails at an ambiguous point: record
  `delivery_uncertain` and do not automatically print again.

This is deliberate. Blind retry after an ambiguous handoff can produce a
duplicate physical label, invoice, or dispatch document. An operator may
choose to reprint as a new job with a recorded relationship to the uncertain
job. The alerting and decision procedure for a single uncertain job is
[`uncertain-delivery-response.md`](uncertain-delivery-response.md).

### After native handoff

CUPS or Winspool observations may move the job through `spooling`, `printing`,
`blocked`, `completed_reported`, `cancelled`, or `failed_terminal`. If the
native job disappears or cannot be observed by the uncertainty deadline, the
result is `delivery_uncertain`.

`completed_reported` means the operating-system spooler reported completion.
It does not prove that ink reached stock. Piqae must not relabel this state as
“physically printed” unless future hardware supplies stronger evidence.

Cancellation before spooler handoff can be definitive. Cancellation after
handoff is a request that must be reconciled; failure to prove the result is
uncertain, not cancelled.

## Multiple nodes and control planes

One installation can project a route into several isolated hosted, managed
child, or self-hosted connectors. A tenant-scoped physical destination can also
contain routes on several nodes. Matching model, display name, driver, or
capabilities alone never establishes that two routes reach the same printer.
Only unambiguous same-kind strong/verified identity evidence can group a new
route automatically; conflicts require a reversible operator decision.

For a resilient destination, a **target** should bind:

- one primary node/printer/profile revision;
- one or more standby node/printer/profile revisions;
- the same logical stock requirement;
- a declared `primary_only` or `primary_then_standby` policy.

A binding is ready only when the node is connected, the printer is usable, the
profile revision is present and published, and its stock/dependencies match.
The scheduler also requires an authenticated route observation that has not
passed `fresh_until`. The current control plane emits a 90-second freshness
window; expired observations are displayable as `recent` for at most five
minutes and are never eligible for new work.

Cross-node failover must obey this rule:

```text
no local acceptance exists
  -> atomically select and pin one ready binding
  -> increment routing attempt
  -> offer only to that node

local acceptance exists OR spool_intent exists
  -> never fail over automatically
  -> recover from that node's SQLite queue or require an operator decision
```

Round-robin routing is not a reliability strategy for stateful printer queues.
V1 should prefer deterministic primary/standby routing, with an explicit
weighted or least-queue policy added only after queue-depth staleness and stock
semantics are proven.

Several connectors on the same installation share a local final-handoff
coordinator even though their credentials, content, queues, and cursors remain
isolated. Independent hosted and self-hosted control planes do not share a
reservation ledger. Piqae therefore serializes their local handoffs but does
not claim global FIFO, automatic cross-server failover, or exactly-once
delivery between those authorities. Direct OS-spooler jobs are reported only
as aggregate occupancy and remain outside Piqae ordering and idempotency.

## If a job does not reach a node

The dashboard and alerting distinguish:

| Condition | Interpretation | Action |
| --- | --- | --- |
| No eligible online node at registration | Valid configured target is temporarily unavailable | Register durably against its configured binding and retain as waiting |
| Node offline after registration | Job is durable but not picked up | Alert on pickup age; reroute only to a fresh compatible route before acceptance |
| Route projection missing | The node can report presence but cannot prove the new fence alias | Upgrade the node and wait for `projection_health=current`; do not bypass the hold |
| Repeated lease expiry | Node cannot finish download/validation | Surface reason and retry count; quarantine unhealthy node |
| Local acceptance not confirmed | Node has durable intent; server is uncertain | Let the same node replay acceptance; do not offer elsewhere |
| Node offline after local acceptance | Job survives in node SQLite | Wait for that node; operator-controlled recovery if hardware is lost |
| No spooler job ID after intent | Native handoff is ambiguous | Mark uncertain; never automatic duplicate |
| Native job blocked | Printer/stock/driver needs attention | Preserve order and show the native reason |

Required timers are workspace-configurable within safe limits:

- online-node pickup p95 target: under 2 seconds;
- pickup warning: 30 seconds;
- pickup critical: 2 minutes for an otherwise ready online target;
- node heartbeat stale: 90 seconds in V1 (idle polls are at most 60 seconds and
  lease renewal also refreshes presence);
- native reconciliation: every few seconds with bounded backoff;
- uncertainty deadline: driver/platform specific and never silently terminal.

## Release without interrupting printing

### Server releases

1. Build one immutable image and SBOM from a signed tag.
2. Run migrations as a separate forward-compatible job.
3. Verify the release's declared server/node compatibility matrix. For the
   physical-destination cutover, an N-1 node may remain visible but is held from
   new work until it upgrades and acknowledges its route projection.
4. Deploy a canary revision with no exclusive schema dependency.
5. Exercise virtual registration, route projection, destination ordering,
   lease/reservation fencing, acceptance, event, and webhook synthetics.
6. Shift a small traffic cohort, watching error rate, queue age, pickup latency,
   lease churn, and event lag.
7. Shift the primary region, then verify secondary-region readiness.
8. Roll application traffic back immediately on threshold breach. Database
   rollback uses a tested forward repair; destructive down-migrations are not
   part of an emergency rollback.

Long-lived node sync uses short requests and durable cursors, so reconnecting
to another stateless API instance does not lose queue state. Cloud Run
instances receive readiness removal and graceful termination time. Workers
claim PostgreSQL work with leases so an interrupted process can be replaced.

### Node releases

1. Publish signed metadata and packages from isolated keys.
2. Roll internal nodes, then 5%, 25%, and 100% cohorts.
3. A node update is eligible only when no render, profile capture, spooler
   handoff, or local accepted work is active.
4. Persist and integrity-check SQLite before replacement.
5. Retain the prior runtime.
6. Require SQLite open, executor handshake, local health, printer discovery,
   and control-plane reconnect after restart.
7. Restore the prior runtime when the health deadline fails.

The current macOS release path polls for idle state but does not yet establish
an atomic queue-admission barrier. Windows updater runtime integration is also
not complete. Until those gates close, native automatic updates remain Preview.

## Availability architecture

The Cloud target is two stateless compute regions behind a global load
balancer, a regional-HA PostgreSQL primary with a cross-region DR replica, and
dual-region object storage. Cross-region compute alone is insufficient: a
writable database and the referenced document must both be available.

Regional database promotion is an incident operation with fencing:

1. stop or reject writes in the failed primary region;
2. confirm replication position and declared RPO;
3. promote the DR replica;
4. rotate the database endpoint/secret;
5. restore API and worker traffic;
6. reconcile leases, outboxes, and every job created near the cutover;
7. rebuild DR protection before closing the incident.

The beta must not promise automatic zero-RPO regional database failover when
the chosen data service cannot provide it.

## SLOs and alerts

Measure at least:

- API availability and durable registration latency;
- object write/digest failure rate;
- oldest `waiting_for_agent` age;
- online-node pickup latency;
- lease expiry and acceptance-replay rate;
- oldest node SQLite job age and queue depth;
- time from local event to dashboard visibility;
- jobs stuck at every non-terminal state;
- `delivery_uncertain` and potential duplicate-handoff rate;
- database saturation, replication lag, and recovery point;
- webhook backlog;
- native update rollback rate.

The release stops when any of these occur:

- one durably registered job disappears from event history;
- the same job is handed to two node identities;
- a spool intent is retried without proof that the prior handoff failed;
- tenant data crosses a workspace boundary;
- queue age or status propagation exceeds the declared critical threshold;
- rollback cannot restore a healthy compatible server or node.

## Evidence required before the paid beta

- fault tests at download, acceptance, render, spool intent, and event upload;
- node power loss and restart with queued work;
- server instance and whole-region loss;
- planned database switchover and rehearsed DR promotion;
- object-store denial and recovery;
- Kubernetes pod deletion during active leases;
- N/N-1 server/node matrix;
- macOS HP and Windows HP/OKI physical matrices;
- at least one long soak with no silent loss or duplicate handoff;
- dashboards and alerts observed firing, acknowledging, and clearing;
- a support runbook for uncertain delivery and node hardware loss.

Until this evidence exists, the repository must say **Implemented** or
**Preview**, never **Supported** or “99.95% achieved.”
