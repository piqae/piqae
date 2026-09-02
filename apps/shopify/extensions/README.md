# Shopify-native printing surfaces

These extensions use Shopify's native Admin and POS surfaces. They never print on mount and never access a printer during preview.

## Surfaces

- `admin-order-print`: order-details **More actions** menu for direct Node printing, using `admin.order-details.action.render`.
- `admin-bulk-print`: selected orders **More actions** menu for direct Node printing, using `admin.order-index.selection-action.render`.
- `admin-order-browser-print`: order-details **Print** menu for Shopify's standard PDF print flow, using `admin.order-details.print-action.render`.
- `admin-bulk-browser-print`: selected orders **Print** menu for Shopify's standard PDF print flow, using `admin.order-index.selection-print-action.render`.
- `admin-product-print`: product-details **More actions** entry for direct Node label printing.
- `admin-product-bulk-print`: selected-products **More actions** entry for direct Node label printing.
- `admin-variant-print`: product-variant-details **More actions** entry for direct Node label printing.
- `admin-product-browser-print`: Shopify-native product-details **Print** entry.
- `admin-product-bulk-browser-print`: Shopify-native selected-products **Print** entry.

Each product or variant Admin target has its own extension identity. Shopify validates and deploys these surfaces independently, so their configuration files intentionally declare exactly one target apiece.

- `pos-print`: completed-order/post-purchase actions plus a product-details action menu/modal for printing product or variant labels.

The Admin extensions create a 15-minute Piqae preview from the selected orders. The **Print** menu surfaces hand the exact PDF to Shopify's standard browser-print flow and also expose a PDF download link. The separate **More actions** surfaces render a signed first-page image of that PDF, expose **Print to Node** as the primary modal action, and keep **Download PDF** as the alternate completion path. They always show the document and printer choices before submission. An unpinned published document can be sent directly to any connected printer using that computer's current operating-system/driver defaults; optional saved profiles do not replace this zero-configuration path. A document pinned to an immutable target/profile/stock binding remains fail-closed and can only approve that exact target and specification revision—Piqae never silently falls back to current defaults. Missing, stale, or untrusted loaded-media evidence is described as unverified rather than incompatible. Changing the document or closing the extension cancels the superseded preview on a best-effort basis, with server expiry as the durable fallback. The first release selects one published document per action; document bundling is not implied. The POS extension targets API version `2026-07`: its PDF/system-dialog path uses the published canonical 80 mm PrintPacket receipt. Shopify's connected receipt-printer API accepts HTML rather than a PrintPacket/PDF artifact, so that explicitly selected hardware path remains a bounded, script-free HTML projection until Shopify or the printer API accepts the canonical artifact. A PDF is never passed to a hardware `Printer`.

Product actions use the same preview approval boundary. A product-details action expands the selected product into one label per variant; a variant-details action prints only that variant; a bulk product action expands each selected product. POS prefers the variant currently in context and otherwise expands the product. The default quantity is one label per resolved variant. Only published label documents are offered, and the printer still has to be selected or resolved from the merchant's default. Product and variant IDs are validated and re-fetched server-side before rendering, so extension input is never treated as printable data.

All sources are relative to `application_url`, allowing Shopify to attach the extension session token. The backend must return final printable content (or redirect to it); a `202` render handle is not printable. It must authenticate the Shopify token, derive the shop server-side, validate selected resource IDs, and return script-free HTML for direct receipt printing.

## Current validation boundary

Unit tests cover URL encoding, direct HTML printing, empty-printer fallback, and disconnect fallback. A real development store with Shopify POS 11.11.0 or later and a paired receipt printer is still required to certify discovery and physical output. No store or physical hardware is exercised by repository tests.

Official references:

- https://shopify.dev/docs/apps/build/admin/actions-blocks/build-admin-print-action
- https://shopify.dev/docs/api/admin-extensions/latest/targets/products
- https://shopify.dev/docs/api/admin-extensions/latest/targets/product-variants
- https://shopify.dev/docs/api/admin-extensions/latest/target-apis/core-apis/action-extension-api
- https://shopify.dev/docs/api/pos-ui-extensions/latest/target-apis/contextual-apis/product-api
- https://shopify.dev/docs/api/pos-ui-extensions/2026-07/target-apis/platform-apis/printing-api
- https://shopify.dev/changelog/pos-ui-extensions-can-now-print-directly-to-hardware-receipt-printers
