# Migrating a PrintNode printing integration

Spool V1 implements the PrintNode printing surface at the API origin root. The
native Spool API remains under `/v1`.

## Minimal migration

1. Create a live Spool API key.
2. Enrol an agent on each machine that currently runs the PrintNode client.
3. Confirm the same installed OS queues appear in Spool.
4. Change the integration's API base URL to the Spool origin.
5. Replace the PrintNode API key with the Spool compatibility key.
6. Run a PDF and RAW canary through each printer class.

Compatibility authentication keeps the PrintNode convention: use the key as
the HTTP Basic username and an empty password.

```sh
curl --user "$SPOOL_API_KEY:" "$SPOOL_API_ORIGIN/whoami"
```

## Create a compatible print job

```sh
curl --request POST \
  --user "$SPOOL_API_KEY:" \
  --header "Content-Type: application/json" \
  --header "X-Idempotency-Key: order-10428-label" \
  --data '{
    "printerId": 34,
    "title": "Order 10428",
    "contentType": "pdf_uri",
    "content": "https://example.invalid/labels/10428.pdf",
    "source": "warehouse",
    "expireAfter": 600,
    "options": {
      "copies": 1,
      "paper": "A4",
      "fit_to_page": true
    }
  }' \
  "$SPOOL_API_ORIGIN/printjobs"
```

The response is a numeric compatibility job ID. Spool also retains its native
typed ID internally.

## Status semantics

The stable compatibility states are `new`, `sent_to_client`, `done`, `error`,
and `expired`. As with PrintNode, `done` means the local client successfully
handed the document to the operating-system print queue. It does not prove
that paper physically exited the printer.

Spool's native API exposes more precise events including `queued_local`,
`spool_intent`, `accepted_by_spooler`, `blocked`, and
`delivery_uncertain`.

## Deliberate V1 differences

- Scales are deferred to V1.1.
- Integrator and child-account headers are deferred to V1.1.
- Compatibility jobs cannot be cancelled after durable local acceptance,
  matching PrintNode's documented control boundary.
- Private-network URI sources require an administrator to enable the
  `allow_private_uri_sources` workspace policy.
- Webhook signing is stronger in the native API. Compatibility webhooks retain
  the expected PrintNode body and secret behavior.

An unsupported endpoint returns a stable error; it never silently reports
success.
