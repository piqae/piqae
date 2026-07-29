# Webhooks

**Status:** endpoint management, encrypted signing secrets, transactional
outbox, retry worker, delivery history, and replay implemented.

Create endpoints through `/v1/webhooks`. The signing secret is returned once;
store it immediately. Spool sends:

```text
Spool-Timestamp: 1750000000
Spool-Signature: v1=<base64-hmac-sha256>
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
Webhook destinations are validated and private-network targets are blocked by
default.

Monitor pending age, attempts, non-2xx responses, and secret-decryption errors.
Never log the secret, signature input body, or document metadata unnecessarily.
