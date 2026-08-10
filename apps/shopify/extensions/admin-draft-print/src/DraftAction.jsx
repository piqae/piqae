/** @jsxImportSource preact */
import { render } from "preact";
import { buildDraftPrintUrl } from "../../shared/print-urls.js";

export default async () => {
  render(<DraftAction />, document.body);
};

export function DraftAction() {
  const ids = (shopify.data.selected ?? []).map(({ id }) => id);
  const href = buildDraftPrintUrl({ draftOrderIds: ids });
  return (
    <s-admin-action>
      <s-stack direction="block" gap="base">
        <s-text>Generate a quote / pro forma PDF for this draft order.</s-text>
        {!href && (
          <s-banner tone="critical">No draft order was selected.</s-banner>
        )}
      </s-stack>
      <s-button slot="primary-action" href={href ?? undefined} disabled={!href}>
        Download PDF
      </s-button>
      <s-button slot="secondary-actions" onClick={() => shopify.close()}>
        Cancel
      </s-button>
    </s-admin-action>
  );
}
