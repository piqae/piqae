# Runtime availability, wake hints, and authority boundaries

Piqae schedules work from authenticated observations, not from a device's
platform name or from the existence of a connection. Embedded and mobile hosts
report a bounded runtime observation on agent sync. It contains the host mode,
availability class, lifecycle state, whether the runtime can currently accept
cloud work, its remaining execution budget when applicable, and a freshness
deadline no more than ten minutes after observation.

The accepted host modes are `machine_service`, `user_agent`,
`embedded_application`, and `attached_client`. Availability classes are
`continuous_while_awake`, `foreground_only`, `background_opportunistic`,
`managed_kiosk`, and `wake_relay_capable`. These values are scheduling facts,
not inferred support claims. An opportunistic background runtime needs at
least 30 seconds of reported execution budget before it may accept work. An
attached client never accepts cloud work independently of the runtime it uses.

## Admission and wake

A route must have fresh, authenticated printer telemetry and be accepting work
before the control plane acquires a physical-destination fence and creates an
offer. A runtime observation must also be live and eligible during the signed
sync which requests work. Suspended, suspending, unavailable, stale, or
budget-constrained embedded hosts receive no offers.

Wake hints are deliberately separate resources. They contain an opaque ID, a
bounded reason, timestamps, and delivery state only. They never contain or own
a job ID, lease, reservation, fence token, document location, or document
metadata. Observing a wake hint does not accept a job. The server waits for a
new authenticated availability and printer observation before normal routing.
Hints expire after at most 15 minutes and are idempotent per node and tenant.
The current durable channel is `connected_session`: a node which is already
awake observes pending hints on signed sync. This is never described as remote
wake. `external_push`, `local_relay`, and `manual` are reserved delivery-channel
values, but the server does not select them until a provider-neutral dispatcher
has a tenant-bound verified endpoint. No APNs token or provider credential is
accepted by this API, and provider credentials must never be shipped in an SDK.

During an N/N-1 rolling upgrade, a legacy desktop node without a runtime field
may continue to request one job from its current authenticated sync. It still
needs a fresh accepting route observation before a fence is acquired. A new
embedded runtime must send its explicit runtime observation; it cannot claim a
stronger availability class by omission.

## Queue privacy

Native queue telemetry contains counts only. The public preferred view is:

- `piqae_owned_jobs`: work attributable to this authenticated connector;
- `external_jobs`: opaque work known not to belong to this connector and whose
  ownership class is known;
- `unknown_jobs`: opaque work whose ownership cannot be safely classified.

The N-1 fields `connector_jobs` and `other_piqae_or_external_jobs` remain in the
additive response for compatibility. The latter includes the unknown count;
the preferred view subtracts unknown work so its three categories partition
the total without double counting. No category contains titles, usernames,
paths, native document data, another tenant's job identifiers, or content.
External and unknown counts improve occupancy estimates but cannot promise
ordering relative to software outside this scheduling authority.

## Tenant and authority isolation

Runtime observations and wake hints have composite tenant keys and composite
foreign keys to the tenant's node. Every read and write includes workspace and
environment. A node ID from another tenant returns no state, and idempotency
keys are tenant- and node-scoped.

This implementation does not coordinate routes across tenants. Doing so would
require a separately reviewed internal envelope that proves authorization,
ordering, and privacy without exposing destination or job data. Until then,
each tenant is an independent scheduling domain even when physical identity
evidence suggests the same printer.

A SaaS control plane and a self-hosted control plane are always independent
scheduling authorities. Neither can see, wake, lease, fence, or reroute the
other authority's work. Automatic cross-authority failover is disabled because
there is no shared authenticated fence. Local nodes may serialize native
handoffs from multiple connectors, but a control plane never interprets that
local serialization as global exactly-once delivery.

## Operator and SDK endpoints

Tenant-scoped clients can inspect the latest runtime observation with
`GET /v1/nodes/{node_id}/runtime`, list advisory wake state with
`GET /v1/nodes/{node_id}/wake-hints`, and request a bounded hint with
`POST /v1/nodes/{node_id}/wake-hints` plus an `Idempotency-Key`. These fields
are suitable for route and node detail views: UIs should show freshness,
lifecycle, execution budget, and the privacy-safe occupancy categories rather
than claiming that a wake hint made a node ready.

Operational tables should use the bounded, node-ID cursor route
`GET /v1/nodes/runtime-observations` instead of issuing one request per node.
Platform `GET /v1/platform/operations` rows include the same latest runtime
projection inside each immutable managed-customer row. Runtime state is never
joined or moved across those customer boundaries.
