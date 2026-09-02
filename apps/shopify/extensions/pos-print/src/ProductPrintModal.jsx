/** @jsxImportSource preact */
import { render } from "preact";
import { useEffect, useMemo, useRef, useState } from "preact/hooks";

import {
  approvalForDocumentPrinter,
  chooseDefaultDocument,
  chooseDefaultPrinterOption,
  newInteractionId,
  printerOptionsForDocument,
  stableOptionKey,
} from "../../shared/AdminOrderPrintAction.jsx";
import { authorizedJson } from "../../shared/print-urls.js";

export default async () => {
  render(<ProductPrintModal />, document.body);
};

export function ProductPrintModal() {
  const variantId = Number(shopify.product.variantId);
  const productId = Number(shopify.product.id);
  const resourceId =
    Number.isSafeInteger(variantId) && variantId > 0
      ? `gid://shopify/ProductVariant/${variantId}`
      : `gid://shopify/Product/${productId}`;
  const [options, setOptions] = useState(null);
  const [documentId, setDocumentId] = useState("");
  const [printerId, setPrinterId] = useState("");
  const [preview, setPreview] = useState(null);
  const [state, setState] = useState("loading");
  const [error, setError] = useState("");
  const interactionId = useRef(newInteractionId([resourceId]));

  useEffect(() => {
    let active = true;
    authorizedJson("/api/print/admin/product-options")
      .then((value) => {
        if (!active) return;
        const document = chooseDefaultDocument(value.documents ?? []);
        const printer = chooseDefaultPrinterOption(
          printerOptionsForDocument(
            document,
            value.targets ?? [],
            value.printers ?? [],
          ),
        );
        setOptions(value);
        setDocumentId(document?.id ?? "");
        setPrinterId(printer?.id ?? "");
        setState("ready");
      })
      .catch((cause) => {
        if (!active) return;
        setError(
          cause instanceof Error ? cause.message : "Labels could not be loaded",
        );
        setState("failed");
      });
    return () => {
      active = false;
    };
  }, []);

  const document = options?.documents?.find(({ id }) => id === documentId);
  const printers = useMemo(
    () =>
      printerOptionsForDocument(
        document,
        options?.targets ?? [],
        options?.printers ?? [],
      ),
    [document, options],
  );
  const printer = printers.find(({ id }) => id === printerId);
  const approval = approvalForDocumentPrinter(document, printer);

  useEffect(() => {
    if (!document || !options) return;
    const next = chooseDefaultPrinterOption(
      printerOptionsForDocument(
        document,
        options.targets ?? [],
        options.printers ?? [],
      ),
    );
    setPrinterId(next?.id ?? "");
  }, [document?.id, options]);

  useEffect(() => {
    if (!documentId || !resourceId) return;
    let active = true;
    setPreview(null);
    setError("");
    setState("previewing");
    authorizedJson("/api/print/admin/product-previews", {
      method: "POST",
      headers: {
        "content-type": "application/json",
        "idempotency-key": `shopify-pos-product-${interactionId.current}-${documentId}`,
      },
      body: JSON.stringify({
        productIds: [resourceId],
        templateId: documentId,
      }),
    })
      .then((value) => {
        if (!active) return;
        setPreview(value);
        setState("ready");
      })
      .catch((cause) => {
        if (!active) return;
        setError(
          cause instanceof Error
            ? cause.message
            : "The label preview could not be generated",
        );
        setState("failed");
      });
    return () => {
      active = false;
    };
  }, [documentId, resourceId]);

  async function printToNode() {
    if (!preview || !approval || !document) return;
    setState("printing");
    setError("");
    try {
      const destination =
        approval.mode === "target"
          ? {
              targetId: approval.targetId,
              specificationRevision: approval.specificationRevision,
              templateId: document.id,
            }
          : { printerId: approval.printerId, templateId: document.id };
      const key =
        approval.mode === "target"
          ? `${printerId}:${approval.targetId}:${approval.specificationRevision}`
          : `${approval.printerId}:current-defaults`;
      await authorizedJson(
        `/api/print/previews/${encodeURIComponent(preview.previewId)}/approve`,
        {
          method: "POST",
          headers: {
            "content-type": "application/json",
            "idempotency-key": `shopify-pos-product-print-${interactionId.current}-${stableOptionKey(key)}`,
          },
          body: JSON.stringify({
            renderId: preview.renderId,
            ...destination,
            renderCost: preview.renderCost,
          }),
        },
      );
      shopify.toast.show("Product label sent to Piqae Node");
      setState("complete");
      shopify.action.dismissModal();
    } catch (cause) {
      setError(
        cause instanceof Error
          ? cause.message
          : "The label could not be printed",
      );
      setState("failed");
    }
  }

  return (
    <s-page heading="Print product label">
      <s-scroll-box>
        <s-stack direction="block" gap="base">
          {state === "loading" || state === "previewing" ? (
            <s-text>Preparing label…</s-text>
          ) : null}
          {error ? <s-banner tone="critical">{error}</s-banner> : null}
          {options && !options.linked ? (
            <s-banner tone="info">
              Connect Piqae before printing product labels.
            </s-banner>
          ) : null}
          {options?.linked && !options.documents?.length ? (
            <s-banner tone="warning">
              Publish a label template in Piqae before printing.
            </s-banner>
          ) : null}
          {options?.documents?.length ? (
            <s-select
              label="Label template"
              value={documentId}
              onChange={(event) => setDocumentId(event.currentTarget.value)}
            >
              {options.documents.map((item) => (
                <s-option key={item.id} value={item.id}>
                  {item.name}
                </s-option>
              ))}
            </s-select>
          ) : null}
          {printers.length ? (
            <s-select
              label="Printer"
              value={printerId}
              onChange={(event) => setPrinterId(event.currentTarget.value)}
            >
              {printers.map((item) => (
                <s-option key={item.id} value={item.id}>
                  {item.label}
                </s-option>
              ))}
            </s-select>
          ) : options?.linked ? (
            <s-banner tone="info">
              No connected Node printers are available.
            </s-banner>
          ) : null}
          {preview?.previewImageUrl ? (
            <s-image
              src={preview.previewImageUrl}
              alt="Product label preview"
            />
          ) : null}
          <s-button
            variant="primary"
            disabled={!preview || !approval || state === "printing"}
            onClick={printToNode}
          >
            Print to Node
          </s-button>
          <s-button onClick={() => shopify.action.dismissModal()}>
            Cancel
          </s-button>
        </s-stack>
      </s-scroll-box>
    </s-page>
  );
}
