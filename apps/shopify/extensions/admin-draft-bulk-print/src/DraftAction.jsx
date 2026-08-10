/** @jsxImportSource preact */
import { render } from "preact";
import { buildDraftPrintUrl } from "../../shared/print-urls.js";

export default async () => {
  render(<DraftBulkAction />, document.body);
};

export function DraftBulkAction() {
  const ids = (shopify.data.selected ?? []).map(({ id }) => id);
  const href = buildDraftPrintUrl({ draftOrderIds: ids });
  return (
    <s-admin-action>
      <s-stack direction="block" gap="base">
        <s-text>{ids.length} selected draft orders</s-text>
        <s-text>
          Generate quote / pro forma PDFs without issuing invoices.
        </s-text>
        {!href && <s-banner tone="critical">Select a draft order.</s-banner>}
      </s-stack>
      <s-button slot="primary-action" href={href ?? undefined} disabled={!href}>
        Download PDFs
      </s-button>
      <s-button slot="secondary-actions" onClick={() => shopify.close()}>
        Cancel
      </s-button>
    </s-admin-action>
  );
}
