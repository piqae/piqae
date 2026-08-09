import { render } from "preact";
import { useMemo, useState } from "preact/hooks";

import { buildAdminPrintUrl } from "../../shared/print-urls.js";

export default async () => {
  render(<PrintAction />, document.body);
};

export function PrintAction() {
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
        {orderIds.length === 0 && (
          <s-banner tone="critical">
            The order could not be identified. Close this view and try again.
          </s-banner>
        )}
        <s-text type="strong">Documents</s-text>
        <s-checkbox
          label="Invoice"
          checked={invoice}
          onChange={(event) => setInvoice(event.currentTarget.checked)}
        />
        <s-checkbox
          label="Packing slip"
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
