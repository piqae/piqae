# Idempotency

**Status:** durable tenant/environment-scoped job idempotency implemented.

Send `Idempotency-Key` on every native job creation. Keys must contain 8–255
characters. Build them from a stable business operation and output version, for
example:

```text
order-10428-shipping-label-v2
```

Reusing the same key with the same normalized request returns the recorded job.
Reusing it with a different request returns `409 idempotency_conflict`. Do not
work around a conflict by appending random text; decide whether the intended
operation is a retry, a replacement, or an additional physical copy.

The PrintNode compatibility route uses `X-Idempotency-Key`. Native and
compatibility payloads have different shapes, so keep integration namespaces
distinct.

Idempotency prevents duplicate registration. It cannot prove whether paper
printed after an ambiguous spooler handoff. Treat `delivery_uncertain` as an
operator reconciliation state.

The transactional boundary is described in
[`06-api-compatibility.md`](../06-api-compatibility.md).
