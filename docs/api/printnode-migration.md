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

## V1 compatibility matrix

| PrintNode-shaped route | Spool V1 | Notes |
| --- | --- | --- |
| `GET /computers` and `/computers/{set}` | Implemented | Comma-separated positive integer mappings are tenant scoped, sorted, and deduplicated. |
| `GET /printers` and `/printers/{set}` | Implemented, bounded | `limit`, `dir=asc|desc`, and exclusive `after={id}` use stable compatibility IDs inside the newest 500-printer hydration window. Set-qualified reads are exact inside that window. |
| `GET /computers/{set}/printers[/{set}]` | Implemented | Both sets are resolved inside the authenticated environment and intersected. |
| `GET /printjobs[/{set}]` | Implemented | Existing compatibility IDs and stable compatibility states are returned. |
| `GET /printers/{set}/printjobs[/{set}]` | Implemented, bounded | Printer and optional job filters are intersected after tenant-scoped resolution. Unqualified collections hydrate at most the newest 500 jobs; explicit job sets are exact. |
| `DELETE /printjobs[/{set}]` | Implemented, bounded | Returns the numeric IDs cancelled before durable agent acceptance. Unqualified deletion examines the newest 500 jobs; explicit job sets are exact. |
| `DELETE /printers/{set}/printjobs[/{set}]` | Implemented, bounded | Applies the same pre-delivery boundary after printer filtering. Unqualified job selection examines the newest 500 jobs. |
| `GET /printjobs[/\{set\}]/states` | Implemented | Projects native lifecycle states onto the stable compatibility states. |

Set routes retain Spool's documented integer-set behavior: invalid members
return a compatibility `400`, IDs are sorted and deduplicated, and a mapping
from another tenant is indistinguishable from a missing mapping. Collection
routes use stable compatibility-ID ordering, support exclusive `after={id}`,
and default to descending order.

Printer and job collection pagination is intentionally not claimed as complete
parity beyond 500 hydrated records in V1. An installation with more than 500
active printers or jobs must use explicit set-qualified reads/deletes or the
native cursor API until compatibility-ID paging is implemented directly in the
repository. Likewise, unqualified `DELETE /printjobs` and
`DELETE /printers/{set}/printjobs` do not claim to cancel eligible records
outside the newest 500-job window.

PrintNode documents that cancellation responses contain only jobs cancelled
before client delivery. Spool follows that boundary and returns a JSON array of
cancelled numeric IDs with `200`. Exact undocumented PrintNode behavior for
mixed missing, already-delivered, and racing sets has not been verified against
the hosted service; Spool returns missing requested mappings as `404`, omits
ineligible/racing jobs, and never reports them as cancelled.

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
