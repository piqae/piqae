import { Component } from "preact";
import { useEffect, useMemo, useRef, useState } from "preact/hooks";

import { authorizedJson } from "./print-urls.js";

export const ADMIN_OPTIONS_TIMEOUT_MS = 8_000;

let interactionSequence = 0;

export function newInteractionId(orderIds) {
  interactionSequence += 1;
  const resource = orderIds
    .join("-")
    .replaceAll(/[^A-Za-z0-9_-]/g, "-")
    .slice(-96);
  return `${resource || "orders"}-${Date.now().toString(36)}-${interactionSequence.toString(36)}`;
}

class PrintActionErrorBoundary extends Component {
  state = { error: "" };

  componentDidCatch(error) {
    this.setState({
      error:
        error instanceof Error
          ? error.message
          : "The print action could not be opened.",
    });
  }

  render() {
    if (this.state.error)
      return (
        <s-admin-print-action>
          <s-banner tone="critical">
            Piqae Order Printing could not start: {this.state.error}
          </s-banner>
        </s-admin-print-action>
      );
    return this.props.children;
  }
}

export function messageForLoadError(error) {
  if (error?.name === "AbortError" || error?.name === "TimeoutError")
    return "Piqae took too long to respond. Check the connection and try again.";
  return error instanceof Error
    ? error.message
    : "Printing options could not be loaded.";
}

export function chooseDefault(items) {
  return (
    items.find((item) => item.isDefault) ??
    items.find((item) => item.eligible) ??
    items[0]
  );
}

export async function loadWithTimeout(
  load,
  timeoutMs = ADMIN_OPTIONS_TIMEOUT_MS,
) {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), timeoutMs);
  try {
    return await load(controller.signal);
  } finally {
    clearTimeout(timeout);
  }
}

export function AdminOrderPrintAction(props) {
  return (
    <PrintActionErrorBoundary>
      <AdminOrderPrintActionContent {...props} />
    </PrintActionErrorBoundary>
  );
}

function AdminOrderPrintActionContent({ bulk = false }) {
  const orderIds = useMemo(
    () => (shopify.data.selected ?? []).map(({ id }) => id),
    [],
  );
  const [options, setOptions] = useState(null);
  const [documentId, setDocumentId] = useState("");
  const [destinationId, setDestinationId] = useState("");
  const [state, setState] = useState("loading");
  const [error, setError] = useState("");
  const [result, setResult] = useState("");
  const [preview, setPreview] = useState(null);
  const requestSequence = useRef(0);
  const previewSequence = useRef(0);
  const interactionId = useRef(newInteractionId(orderIds));
  const approvedPreview = useRef("");

  async function loadOptions() {
    const sequence = ++requestSequence.current;
    setState("loading");
    setError("");
    try {
      const value = await loadWithTimeout((signal) =>
        authorizedJson("/api/print/admin/options", { signal }),
      );
      if (sequence !== requestSequence.current) return;
      const defaultDocument = chooseDefault(value.documents ?? []);
      const defaultDestination = chooseDefault(
        (value.destinations ?? []).filter((item) => item.eligible),
      );
      setOptions(value);
      setDocumentId(defaultDocument?.id ?? "");
      setDestinationId(defaultDestination?.id ?? "");
      setState("ready");
    } catch (cause) {
      if (sequence !== requestSequence.current) return;
      setError(messageForLoadError(cause));
      setState("failed");
    }
  }

  useEffect(() => {
    loadOptions();
    return () => {
      requestSequence.current += 1;
    };
  }, []);

  const selectedDocument = options?.documents?.find(
    ({ id }) => id === documentId,
  );
  useEffect(() => {
    if (!options?.linked || !selectedDocument || orderIds.length === 0) return;
    const sequence = ++previewSequence.current;
    setPreview(null);
    setError("");
    setState("rendering");
    loadWithTimeout(
      (signal) =>
        authorizedJson("/api/print/admin/previews", {
          method: "POST",
          headers: {
            "content-type": "application/json",
            "idempotency-key": `shopify-preview-${interactionId.current}-${selectedDocument.id}`,
          },
          body: JSON.stringify({ orderIds, templateId: selectedDocument.id }),
          signal,
        }),
      15_000,
    )
      .then((value) => {
        if (sequence !== previewSequence.current) return;
        setPreview(value);
        setState("ready");
      })
      .catch((cause) => {
        if (sequence !== previewSequence.current) return;
        setError(messageForLoadError(cause));
        setState("ready");
      });
    return () => {
      previewSequence.current += 1;
    };
  }, [options, selectedDocument?.id, orderIds.join(",")]);

  const src = preview?.artifactUrl ?? null;

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
            "idempotency-key": `shopify-cancel-${interactionId.current}-${preview.previewId}`,
          },
          body: JSON.stringify({ renderId: preview.renderId }),
        },
      ).catch(() => {});
    };
  }, [preview?.previewId]);

  async function printDirect() {
    if (!preview || !destinationId || state === "printing") return;
    setState("printing");
    setError("");
    setResult("");
    try {
      const value = await authorizedJson(
        `/api/print/previews/${encodeURIComponent(preview.previewId)}/approve`,
        {
          method: "POST",
          headers: {
            "content-type": "application/json",
            "idempotency-key": `shopify-admin-${interactionId.current}`,
          },
          body: JSON.stringify({
            renderId: preview.renderId,
            printerId: destinationId,
          }),
        },
      );
      approvedPreview.current = preview.previewId;
      setResult(`Print job ${value.jobId} was accepted.`);
      setState("ready");
    } catch (cause) {
      setError(
        cause instanceof Error
          ? cause.message
          : "The print job could not be submitted.",
      );
      setState("ready");
    }
  }

  const eligible = options?.destinations?.filter((item) => item.eligible) ?? [];
  const canPrint = Boolean(src && destinationId && state === "ready");

  return (
    <s-admin-print-action src={src ?? undefined}>
      <s-stack direction="block" gap="base">
        {bulk && (
          <s-text type="strong">{orderIds.length} selected orders</s-text>
        )}
        {orderIds.length === 0 && (
          <s-banner tone="critical">
            No order was selected. Close this view and try again.
          </s-banner>
        )}
        {state === "loading" && (
          <s-text>Loading documents and destinations…</s-text>
        )}
        {state === "rendering" && <s-text>Generating preview…</s-text>}
        {state === "failed" && (
          <s-banner tone="critical">
            {error}
            <s-button onClick={loadOptions}>Try again</s-button>
          </s-banner>
        )}
        {options && !options.linked && (
          <s-banner tone="info">
            Connect Piqae to preview, download, or print order documents.
            <s-button href={options.setupDestinationUrl}>Set up Piqae</s-button>
          </s-banner>
        )}
        {options?.linked && (
          <>
            <s-text type="strong">Documents</s-text>
            <s-text>Select one published document for this print.</s-text>
            {options.documents.length > 0 ? (
              <s-select
                label="Document"
                value={documentId}
                onChange={(event) => {
                  setDocumentId(event.currentTarget.value);
                  setResult("");
                }}
              >
                {options.documents.map((document) => (
                  <option key={document.id} value={document.id}>
                    {document.name}
                  </option>
                ))}
              </s-select>
            ) : (
              <s-banner tone="warning">
                Publish a document before printing.
              </s-banner>
            )}
            <s-button href={options.manageDocumentsUrl}>
              Manage documents
            </s-button>

            <s-text type="strong">Destination</s-text>
            {options.destinationError && (
              <s-banner tone="warning">{options.destinationError}</s-banner>
            )}
            {eligible.length > 0 ? (
              <s-select
                label="Printer"
                value={destinationId}
                onChange={(event) =>
                  setDestinationId(event.currentTarget.value)
                }
              >
                {eligible.map((destination) => (
                  <option key={destination.id} value={destination.id}>
                    {destination.name} · Ready
                  </option>
                ))}
              </s-select>
            ) : (
              <s-banner tone="info">
                No connected printer is ready. PDF preview and browser printing
                remain available.
                <s-button href={options.setupDestinationUrl}>
                  Set up a printer
                </s-button>
              </s-banner>
            )}
            {error && <s-banner tone="critical">{error}</s-banner>}
            {result && <s-banner tone="success">{result}</s-banner>}
            <s-button
              variant="primary"
              disabled={!canPrint}
              onClick={printDirect}
            >
              {state === "printing" ? "Sending…" : "Print with Piqae"}
            </s-button>
            {src && <s-button href={src}>Download PDF</s-button>}
          </>
        )}
      </s-stack>
    </s-admin-print-action>
  );
}
