/** @jsxImportSource preact */
import { render } from "preact";
import { useEffect, useMemo, useRef, useState } from "preact/hooks";

import {
  approvalForDocumentPrinter,
  chooseDefaultDocument,
  chooseDefaultPrinterOption,
  loadWithTimeout,
  messageForLoadError,
  newInteractionId,
  previewDownloadUrl,
  printerCompatibilityMessage,
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
      : Number.isSafeInteger(productId) && productId > 0
        ? `gid://shopify/Product/${productId}`
        : "";
  const [options, setOptions] = useState(null);
  const [documentId, setDocumentId] = useState("");
  const [printerId, setPrinterId] = useState("");
  const [preview, setPreview] = useState(null);
  const [state, setState] = useState("loading");
  const [error, setError] = useState("");
  const [errorStage, setErrorStage] = useState("");
  const [optionsAttempt, setOptionsAttempt] = useState(0);
  const [previewAttempt, setPreviewAttempt] = useState(0);
  const interactionId = useRef(newInteractionId([resourceId]));
  const approvedPreview = useRef("");
  const previewRequest = useRef(0);

  useEffect(() => {
    let active = true;
    setState("loading");
    setError("");
    setErrorStage("");
    loadWithTimeout((signal) =>
      authorizedJson("/api/print/admin/product-options", { signal }),
    )
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
        setError(messageForLoadError(cause));
        setErrorStage("options");
        setState("failed");
      });
    return () => {
      active = false;
    };
  }, [optionsAttempt]);

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
    const requestNumber = ++previewRequest.current;
    setPreview(null);
    setError("");
    setErrorStage("");
    setState("previewing");
    loadWithTimeout(
      (signal) =>
        authorizedJson("/api/print/admin/product-previews", {
          method: "POST",
          headers: {
            "content-type": "application/json",
            "idempotency-key": `shopify-pos-product-${interactionId.current}-${documentId}-${requestNumber}`,
          },
          body: JSON.stringify({
            productIds: [resourceId],
            templateId: documentId,
          }),
          signal,
        }),
      15_000,
    )
      .then((value) => {
        if (!active) return;
        setPreview(value);
        setState("ready");
      })
      .catch((cause) => {
        if (!active) return;
        setError(messageForLoadError(cause));
        setErrorStage("preview");
        setState("failed");
      });
    return () => {
      active = false;
    };
  }, [documentId, resourceId, previewAttempt]);

  useEffect(() => {
    if (!preview) return;
    return () => {
      if (approvedPreview.current === preview.previewId) return;
      authorizedJson(
        `/api/print/previews/${encodeURIComponent(preview.previewId)}/cancel`,
        {
          method: "POST",
          headers: {
            "content-type": "application/json",
            "idempotency-key": `shopify-pos-product-cancel-${interactionId.current}-${preview.previewId}`,
          },
          body: JSON.stringify({ renderId: preview.renderId }),
        },
      ).catch(() => {});
    };
  }, [preview?.previewId]);

  async function printToNode() {
    if (!preview || !approval || !document) return;
    setState("printing");
    setError("");
    setErrorStage("");
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
      await loadWithTimeout(
        (signal) =>
          authorizedJson(
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
              signal,
            },
          ),
        15_000,
      );
      approvedPreview.current = preview.previewId;
      shopify.toast.show("Product label sent to Piqae Node");
      setState("complete");
      shopify.action.dismissModal();
    } catch (cause) {
      setError(messageForLoadError(cause) || "The label could not be printed");
      setErrorStage("print");
      setState("failed");
    }
  }

  const compatibilityMessage = printerCompatibilityMessage(document, printer);
  const downloadUrl = previewDownloadUrl(preview?.artifactUrl);

  return (
    <s-page heading="Print product label">
      <s-scroll-box>
        <s-stack direction="block" gap="base">
          {state === "loading" || state === "previewing" ? (
            <s-stack direction="inline" gap="small" alignItems="center">
              <s-spinner accessibilityLabel="Preparing product label" />
              <s-text>Preparing label…</s-text>
            </s-stack>
          ) : null}
          {!resourceId ? (
            <s-banner tone="critical">
              Shopify did not provide a product or variant for this action.
            </s-banner>
          ) : null}
          {error ? (
            <s-banner tone="critical">
              {error}
              {errorStage === "options" ? (
                <s-button
                  onClick={() => setOptionsAttempt((value) => value + 1)}
                >
                  Try again
                </s-button>
              ) : null}
              {errorStage === "preview" ? (
                <s-button
                  onClick={() => setPreviewAttempt((value) => value + 1)}
                >
                  Try again
                </s-button>
              ) : null}
            </s-banner>
          ) : null}
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
          {compatibilityMessage ? (
            <s-banner tone="warning">{compatibilityMessage}</s-banner>
          ) : null}
          {(preview?.warnings ?? []).map((warning, index) => (
            <s-banner
              key={`${warning.code ?? "preview-warning"}-${index}`}
              tone="warning"
            >
              {warning.message}
            </s-banner>
          ))}
          {preview?.previewImageUrl ? (
            <s-image
              src={preview.previewImageUrl}
              alt="Product label preview"
            />
          ) : null}
          <s-button
            variant="primary"
            disabled={
              !preview || !approval || state === "printing" || !resourceId
            }
            onClick={printToNode}
          >
            Print to Node
          </s-button>
          {downloadUrl ? (
            <s-button href={downloadUrl} target="_blank" icon="download">
              Download PDF
            </s-button>
          ) : (
            <s-button icon="download" disabled>
              Download PDF
            </s-button>
          )}
          <s-button onClick={() => shopify.action.dismissModal()}>
            Cancel
          </s-button>
        </s-stack>
      </s-scroll-box>
    </s-page>
  );
}
