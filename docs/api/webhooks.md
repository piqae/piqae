# Webhooks

**Status:** endpoint management, encrypted signing secrets, transactional
outbox, retry worker, delivery history, and replay implemented.

Create endpoints through `/v1/webhooks`. The signing secret is returned once;
store it immediately. Subscribed event names are matched exactly: there is no
wildcard expansion, so `job.*` matches nothing and every event you want must be
named in full. Endpoints are scoped to one workspace and environment. Piqae
sends:

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

For the uncertain-delivery events specifically, see
[`operations/uncertain-delivery-response.md`](../operations/uncertain-delivery-response.md).
