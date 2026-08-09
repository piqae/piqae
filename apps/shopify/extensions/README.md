# Shopify-native printing surfaces

These extensions use Shopify's native Admin and POS surfaces. They never print on mount and never access a printer during preview.

## Surfaces

- `admin-order-print`: order-details **Print** menu, using `admin.order-details.print-action.render`.
- `admin-bulk-print`: selected orders **Print** menu, using `admin.order-index.selection-print-action.render`.
- `pos-print`: completed-order and post-purchase action menu/modal pairs.

The Admin extensions provide a same-origin PDF URL to `s-admin-print-action`. Shopify owns preview and the browser print dialog. The POS extension targets API version `2026-07`: it discovers a connected receipt printer and sends HTML directly. When no connected receipt printer is available, it passes the PDF URL without a printer so Shopify opens the system print dialog. A PDF is never passed to a hardware `Printer`.

All sources are relative to `application_url`, allowing Shopify to attach the extension session token. The backend must return final printable content (or redirect to it); a `202` render handle is not printable. It must authenticate the Shopify token, derive the shop server-side, validate selected resource IDs, and return script-free HTML for direct receipt printing.

## Current validation boundary

Unit tests cover URL encoding, direct HTML printing, empty-printer fallback, and disconnect fallback. A real development store with Shopify POS 11.11.0 or later and a paired receipt printer is still required to certify discovery and physical output. No store or physical hardware is exercised by repository tests.

Official references:

- https://shopify.dev/docs/apps/build/admin/actions-blocks/build-admin-print-action
- https://shopify.dev/docs/api/admin-extensions/latest/target-apis/core-apis/action-extension-api
- https://shopify.dev/docs/api/pos-ui-extensions/2026-07/target-apis/platform-apis/printing-api
- https://shopify.dev/changelog/pos-ui-extensions-can-now-print-directly-to-hardware-receipt-printers
