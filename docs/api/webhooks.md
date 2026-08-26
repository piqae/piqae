# Webhooks

**Status:** endpoint management, encrypted signing secrets, transactional
outbox, retry worker, delivery history, and replay implemented.

Create endpoints through `/v1/webhooks`. The signing secret is returned once;
store it immediately. Subscriptions may name one exact event or a trailing
family such as `job.*` or `node.*`; no other wildcard position is expanded.
Endpoints are scoped to one workspace and environment. Piqae sends:

```text
Piqae-Timestamp: 1750000000
Piqae-Signature: v1=<base64-hmac-sha256>
```

Verify HMAC-SHA256 over:

```text
<decimal timestamp>.<exact raw request body>
```

Use the exact returned `whsec_...` UTF-8 bytes as the HMAC key, constant-time
comparison, a small timestamp tolerance, and durable event-ID deduplication.
Verify before JSON parsing or side effects. Reject missing/duplicate headers
and stale timestamps.

Delivery is at least once. Return a 2xx only after durable acceptance. Replays
create another delivery attempt and must not duplicate business actions.
Webhook destinations are validated at creation and again at delivery:
`localhost`, loopback, unspecified, private, link-local, and unique-local
targets are refused and there is no configuration override. Consume
`/v1/events/stream` when the receiver cannot be publicly resolvable.

Monitor pending age, attempts, non-2xx responses, and secret-decryption errors.
Never log the secret, signature input body, or document metadata unnecessarily.

`node.wake_hint.requested` is a provider-neutral mobile/embedded wake request.
It contains only the opaque hint and node IDs, bounded reason, channel, status,
and timestamps. It never includes a job ID, title, content, document metadata,
lease, or printer settings. A tenant backend may translate an `external_push`
event into APNs or a vendor notification. Deduplicate repeats by hint ID, keep
provider credentials outside Piqae and client SDKs, and require the node to
report fresh authenticated runtime and printer state before expecting a lease.

For the uncertain-delivery events specifically, see
[`operations/uncertain-delivery-response.md`](../operations/uncertain-delivery-response.md).
