/** @jsxImportSource preact */
import { Component } from "preact";
import { useEffect, useMemo, useRef, useState } from "preact/hooks";

import {
  chooseDefaultDocument,
  loadWithTimeout,
  messageForLoadError,
  newInteractionId,
  previewDownloadUrl,
} from "./AdminOrderPrintAction.jsx";
import { authorizedJson } from "./print-urls.js";

class BrowserPrintErrorBoundary extends Component {
  state = { error: "" };

  componentDidCatch(error) {
    this.setState({
      error:
        error instanceof Error
          ? error.message
          : "The Shopify print action could not be opened.",
    });
  }

  render() {
    if (this.state.error)
      return (
        <s-admin-print-action>
          <s-banner tone="critical">{this.state.error}</s-banner>
        </s-admin-print-action>
      );
    return this.props.children;
  }
}

export function AdminOrderBrowserPrintAction(props) {
  return (
    <BrowserPrintErrorBoundary>
      <AdminOrderBrowserPrintActionContent {...props} />
    </BrowserPrintErrorBoundary>
  );
}

export function AdminProductBrowserPrintAction(props) {
  return <AdminOrderBrowserPrintAction {...props} resourceType="products" />;
}

function AdminOrderBrowserPrintActionContent({
  bulk = false,
  resourceType = "orders",
}) {
  const resourceIds = useMemo(
    () => (shopify.data.selected ?? []).map(({ id }) => id),
    [],
  );
  const productMode = resourceType === "products";
  const optionsPath = productMode
    ? "/api/print/admin/product-options"
    : "/api/print/admin/options";
  const previewsPath = productMode
    ? "/api/print/admin/product-previews"
    : "/api/print/admin/previews";
  const [options, setOptions] = useState(null);
  const [documentId, setDocumentId] = useState("");
  const [state, setState] = useState("loading");
  const [error, setError] = useState("");
  const [preview, setPreview] = useState(null);
  const [previewState, setPreviewState] = useState("idle");
  const [requestAttempt, setRequestAttempt] = useState(0);
  const [previewAttempt, setPreviewAttempt] = useState(0);
  const requestSequence = useRef(0);
  const previewSequence = useRef(0);
  const currentPreview = useRef(null);
  const interactionId = useRef(newInteractionId(resourceIds));

  useEffect(() => {
    const sequence = ++requestSequence.current;
    setState("loading");
    setError("");
    loadWithTimeout((signal) => authorizedJson(optionsPath, { signal }))
      .then((value) => {
        if (sequence !== requestSequence.current) return;
        const document = chooseDefaultDocument(value.documents ?? []);
        setOptions(value);
        setDocumentId(document?.id ?? "");
        setState("ready");
      })
      .catch((cause) => {
        if (sequence !== requestSequence.current) return;
        setError(messageForLoadError(cause));
        setState("failed");
      });
    return () => {
      requestSequence.current += 1;
    };
  }, [requestAttempt]);

  const selectedDocument = documentId
    ? options?.documents?.find(({ id }) => id === documentId)
    : undefined;

  useEffect(() => {
    if (!options?.linked || !selectedDocument || resourceIds.length === 0) {
      setPreview(null);
      setPreviewState("idle");
      return;
    }
    const sequence = ++previewSequence.current;
    const superseded = currentPreview.current;
    currentPreview.current = null;
    if (superseded)
      authorizedJson(
        `/api/print/previews/${encodeURIComponent(superseded.previewId)}/cancel`,
        {
          method: "POST",
          headers: {
            "content-type": "application/json",
            "idempotency-key": `shopify-browser-cancel-${interactionId.current}-${superseded.previewId}`,
          },
          body: JSON.stringify({ renderId: superseded.renderId }),
        },
      ).catch(() => {});
    setPreview(null);
    setError("");
    setPreviewState("loading");
    loadWithTimeout(
      (signal) =>
        authorizedJson(previewsPath, {
          method: "POST",
          headers: {
            "content-type": "application/json",
            "idempotency-key": `shopify-browser-preview-${interactionId.current}-${selectedDocument.id}`,
          },
          body: JSON.stringify({
            [productMode ? "productIds" : "orderIds"]: resourceIds,
            templateId: selectedDocument.id,
          }),
          signal,
        }),
      15_000,
    )
      .then((value) => {
        if (sequence !== previewSequence.current) return;
        currentPreview.current = value;
        setPreview(value);
        setPreviewState("ready");
      })
      .catch((cause) => {
        if (sequence !== previewSequence.current) return;
        setError(messageForLoadError(cause));
        setPreviewState("failed");
      });
    return () => {
      previewSequence.current += 1;
    };
  }, [
    options,
    selectedDocument?.id,
    resourceIds.join(","),
    previewAttempt,
    previewsPath,
    productMode,
  ]);

  // Shopify enables its native Print action as soon as `src` exists. Never
  // supply a loading/error placeholder here or it can be printed as if it were
  // the merchant's document.
  const src = previewState === "ready" ? preview?.artifactUrl : undefined;
  const downloadUrl = previewDownloadUrl(preview?.artifactUrl);

  return (
    <s-admin-print-action {...(src ? { src } : {})}>
      <s-stack direction="block" gap="base">
        {bulk ? (
          <s-text type="strong">
            {resourceIds.length} selected {productMode ? "products" : "orders"}
          </s-text>
        ) : null}
        {state === "loading" ? <s-text>Loading documents…</s-text> : null}
        {state === "failed" ? (
          <s-banner tone="critical">
            {error}
            <s-button onClick={() => setRequestAttempt((value) => value + 1)}>
              Try again
            </s-button>
          </s-banner>
        ) : null}
        {resourceIds.length === 0 ? (
          <s-banner tone="critical">
            No {productMode ? "product or variant" : "order"} was selected.
            Close this view and try again.
          </s-banner>
        ) : null}
        {options && !options.linked ? (
          <s-banner tone="info">
            Connect Piqae to generate Shopify print-ready documents.
            <s-button href={options.setupDestinationUrl}>Set up Piqae</s-button>
          </s-banner>
        ) : null}
        {options?.linked && options.documents?.length ? (
          <s-select
            label="Document"
            value={documentId}
            onChange={(event) => setDocumentId(event.currentTarget.value)}
          >
            {options.documents.map((document) => (
              <s-option key={document.id} value={document.id}>
                {document.name}
              </s-option>
            ))}
          </s-select>
        ) : options?.linked ? (
          <s-banner tone="warning">
            Publish a document in Piqae before printing.
          </s-banner>
        ) : null}
        {previewState === "loading" ? (
          <s-stack direction="inline" gap="small" alignItems="center">
            <s-spinner accessibilityLabel="Generating printable PDF" />
            <s-text>Generating the printable PDF…</s-text>
          </s-stack>
        ) : null}
        {previewState === "failed" && error ? (
          <s-banner tone="critical">
            {error}
            <s-button onClick={() => setPreviewAttempt((value) => value + 1)}>
              Try again
            </s-button>
          </s-banner>
        ) : null}
        {previewState === "ready"
          ? (preview?.warnings ?? []).map((warning, index) => (
              <s-banner
                key={`${warning.code ?? "preview-warning"}-${index}`}
                tone="warning"
              >
                {warning.message}
              </s-banner>
            ))
          : null}
        {previewState === "ready" && downloadUrl ? (
          <s-button href={downloadUrl} target="_blank" icon="download">
            Download PDF
          </s-button>
        ) : null}
      </s-stack>
    </s-admin-print-action>
  );
}
