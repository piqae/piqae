/** @jsxImportSource preact */
import { render } from "preact";
import { useEffect, useState } from "preact/hooks";

import { buildPosPrintUrl, printPosReceipt } from "../../shared/print-urls.js";

export default async () => {
  render(<PrintModal />, document.body);
};

export function PrintModal() {
  const [state, setState] = useState("ready");
  const [printers, setPrinters] = useState([]);
  const [printerId, setPrinterId] = useState("");
  const orderId = shopify.order.id;

  useEffect(() => {
    let active = true;
    shopify.printing
      .getPrinters()
      .then((available) => {
        if (active) setPrinters(available.filter(({ connected }) => connected));
      })
      .catch(() => {
        if (active) setPrinters([]);
      });
    return () => {
      active = false;
    };
  }, []);

  async function printReceipt() {
    setState("printing");
    try {
      const result = await printPosReceipt({
        printing: shopify.printing,
        orderId,
        printer: printers.find(({ id }) => id === printerId),
      });
      shopify.toast.show(
        result.mode === "receipt-printer"
          ? `Sent to ${result.printer.name}`
          : "Opened the system print dialog",
      );
      setState("complete");
    } catch {
      setState("failed");
    }
  }

  async function printPdf() {
    const src = buildPosPrintUrl({ orderId, format: "pdf" });
    if (!src) return setState("failed");
    setState("printing");
    try {
      await shopify.printing.print(src);
      setState("complete");
    } catch {
      setState("failed");
    }
  }

  return (
    <s-page heading="Print receipt">
      <s-scroll-box>
        <s-stack direction="block" gap="base">
          <s-text>{shopify.order.name}</s-text>
          <s-text>
            Choose a receipt printer for direct printing, or explicitly open the
            PDF system dialog.
          </s-text>
          <s-select
            label="Receipt printer"
            value={printerId}
            onChange={(event) => setPrinterId(event.currentTarget.value)}
          >
            <s-option value="">Select a connected printer</s-option>
            {printers.map((printer) => (
              <s-option key={printer.id} value={printer.id}>
                {printer.name}
              </s-option>
            ))}
          </s-select>
          {printers.length === 0 && (
            <s-banner tone="info">
              No connected receipt printers were found. You can still open the
              PDF system dialog.
            </s-banner>
          )}
          {state === "failed" && (
            <s-banner tone="critical">
              Unable to print. Check the connection and try again.
            </s-banner>
          )}
          <s-button
            variant="primary"
            disabled={state === "printing" || !printerId}
            onClick={printReceipt}
          >
            Print to selected printer
          </s-button>
          <s-button disabled={state === "printing"} onClick={printPdf}>
            Open PDF dialog
          </s-button>
        </s-stack>
      </s-scroll-box>
    </s-page>
  );
}
