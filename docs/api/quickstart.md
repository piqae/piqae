# API quickstart

**Status:** native V1 job API and PrintNode-shaped compatibility API
implemented.

Set a server-side API origin/key and list printers:

```sh
curl --fail-with-body \
  --header "Authorization: Bearer $SPOOL_API_KEY" \
  "$SPOOL_API_ORIGIN/v1/printers?limit=25"
```

Register a PDF job with a stable idempotency key:

```sh
curl --fail-with-body --request POST \
  --header "Authorization: Bearer $SPOOL_API_KEY" \
  --header "Content-Type: application/json" \
  --header "Idempotency-Key: order-10428-label-v1" \
  --data '{
    "printer_id": "ptr_01J...",
    "title": "Order 10428",
    "content_type": "pdf",
    "content": {"type":"uri","uri":"https://objects.example/10428.pdf"},
    "options": {"copies":1,"paper":"A4"},
    "expire_after_seconds": 600,
    "metadata": {"order_id":"10428"}
  }' \
  "$SPOOL_API_ORIGIN/v1/jobs"
```

Then fetch `/v1/jobs/{job_id}` and `/v1/jobs/{job_id}/events`. A successful
create means durable registration, not physical delivery. Use webhooks or event
polling until a terminal state, and handle `delivery_uncertain` manually.

URI sources must remain available until the node downloads them. Prefer upload
objects for private or short-lived content. Never put keys in query strings.

The machine-readable contract is
[`contracts/openapi/spool-v1.yaml`](../../contracts/openapi/spool-v1.yaml).
