import { render } from "preact";
import { useMemo, useState } from "preact/hooks";

import { buildAdminPrintUrl } from "../../shared/print-urls.js";

export default async () => {
  render(<BulkPrintAction />, document.body);
};

export function BulkPrintAction() {
  const [invoice, setInvoice] = useState(true);
  const [packingSlip, setPackingSlip] = useState(false);
  const orderIds = (shopify.data.selected ?? []).map(({ id }) => id);
  const documents = [
    invoice && "invoice",
    packingSlip && "packing_slip",
  ].filter(Boolean);
  const src = useMemo(
    () => buildAdminPrintUrl({ orderIds, documents }),
    [orderIds.join(","), documents.join(",")],
  );

  return (
    <s-admin-print-action src={src ?? undefined}>
      <s-stack direction="block" gap="base">
        <s-text type="strong">{orderIds.length} selected orders</s-text>
        <s-checkbox
          label="Invoices"
          checked={invoice}
          onChange={(event) => setInvoice(event.currentTarget.checked)}
        />
        <s-checkbox
          label="Packing slips"
          checked={packingSlip}
          onChange={(event) => setPackingSlip(event.currentTarget.checked)}
        />
        {documents.length === 0 && (
          <s-banner tone="warning">
            Select at least one document to continue.
          </s-banner>
        )}
      </s-stack>
    </s-admin-print-action>
  );
}
