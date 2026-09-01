# Shopify-native printing surfaces

These extensions use Shopify's native Admin and POS surfaces. They never print on mount and never access a printer during preview.

## Surfaces

- `admin-order-print`: order-details **More actions** menu, using `admin.order-details.action.render`.
- `admin-bulk-print`: selected orders **More actions** menu, using `admin.order-index.selection-action.render`.
- `pos-print`: completed-order and post-purchase action menu/modal pairs.

The Admin extensions create a 15-minute Piqae preview from the selected orders. They render a signed first-page image of the exact PDF inside a standard Admin action, expose **Print to Node** as the primary modal action, and provide **Download PDF** as the alternate completion path. They do not hand the document to Shopify's browser-print flow. An unpinned published document can be sent directly to any connected printer using that computer's current operating-system/driver defaults; optional saved profiles do not replace this zero-configuration path. A document pinned to an immutable target/profile/stock binding remains fail-closed and can only approve that exact target and specification revision—Piqae never silently falls back to current defaults. Missing, stale, or untrusted loaded-media evidence is described as unverified rather than incompatible. Changing the document or closing the extension cancels the superseded preview on a best-effort basis, with server expiry as the durable fallback. The first release selects one published document per action; document bundling is not implied. The POS extension targets API version `2026-07`: its PDF/system-dialog path uses the published canonical 80 mm PrintPacket receipt. Shopify's connected receipt-printer API accepts HTML rather than a PrintPacket/PDF artifact, so that explicitly selected hardware path remains a bounded, script-free HTML projection until Shopify or the printer API accepts the canonical artifact. A PDF is never passed to a hardware `Printer`.

All sources are relative to `application_url`, allowing Shopify to attach the extension session token. The backend must return final printable content (or redirect to it); a `202` render handle is not printable. It must authenticate the Shopify token, derive the shop server-side, validate selected resource IDs, and return script-free HTML for direct receipt printing.

## Current validation boundary

Unit tests cover URL encoding, direct HTML printing, empty-printer fallback, and disconnect fallback. A real development store with Shopify POS 11.11.0 or later and a paired receipt printer is still required to certify discovery and physical output. No store or physical hardware is exercised by repository tests.

Official references:

- https://shopify.dev/docs/apps/build/admin/actions-blocks/build-admin-print-action
- https://shopify.dev/docs/api/admin-extensions/latest/target-apis/core-apis/action-extension-api
- https://shopify.dev/docs/api/pos-ui-extensions/2026-07/target-apis/platform-apis/printing-api
- https://shopify.dev/changelog/pos-ui-extensions-can-now-print-directly-to-hardware-receipt-printers
