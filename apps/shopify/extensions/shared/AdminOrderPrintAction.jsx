/** @jsxImportSource preact */
import { Component } from "preact";
import { useEffect, useMemo, useRef, useState } from "preact/hooks";

import { authorizedJson } from "./print-urls.js";

export const ADMIN_OPTIONS_TIMEOUT_MS = 8_000;
export const PRINT_PLACEHOLDER_PATH = "/api/public/print-placeholder";

let interactionSequence = 0;

export function newInteractionId(orderIds) {
  interactionSequence += 1;
  const resource = orderIds
    .join("-")
    .replaceAll(/[^A-Za-z0-9_-]/g, "-")
    .slice(-96);
  return `${resource || "orders"}-${Date.now().toString(36)}-${interactionSequence.toString(36)}`;
}

export function stableOptionKey(value) {
  let hash = 2166136261;
  for (let index = 0; index < value.length; index += 1) {
    hash ^= value.charCodeAt(index);
    hash = Math.imul(hash, 16777619);
  }
  return (hash >>> 0).toString(36);
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
  if (
    error instanceof TypeError &&
    ["Load failed", "Failed to fetch"].includes(error.message)
  )
    return "The preview could not reach Piqae. Try again, or close and reopen this print action.";
  return error instanceof Error
    ? error.message
    : "Printing options could not be loaded.";
}

export function chooseDefault(items) {
  // An entry without an id cannot be previewed or printed, so it must never
  // win the default selection: doing so leaves an empty picker next to a
  // failed preview with nothing the merchant can act on.
  const usable = items.filter((item) => item?.id);
  return (
    usable.find((item) => item.isDefault) ??
    usable.find((item) => item.eligible) ??
    usable[0]
  );
}

export function chooseDefaultDocument(items) {
  const usable = items.filter((item) => item?.id);
  const label = (item) => `${item.kind ?? ""} ${item.name ?? ""}`.toLowerCase();
  return (
    usable.find((item) => item.isDefault) ??
    usable.find((item) => item.kind === "packing_slip") ??
    usable.find((item) => label(item).includes("packing slip")) ??
    usable.find((item) => label(item).includes("invoice")) ??
    usable[0]
  );
}

export function targetForDocument(document, targets) {
  if (!canUsePublishedBinding(document)) return undefined;
  const allowed = targets.filter(
    (target) =>
      target.eligible &&
      (!document?.compatibilityKnown ||
        document.compatibleTargetIds.includes(target.id)),
  );
  if (document?.designTargetId)
    return allowed.find(({ id }) => id === document.designTargetId);
  return undefined;
}

export function printerOptionsForDocument(document, targets, printers) {
  const selectedTarget = canUsePublishedBinding(document)
    ? targets.find(
        (target) =>
          target.id === document.designTargetId &&
          target.eligible &&
          (!document.compatibilityKnown ||
            document.compatibleTargetIds.includes(target.id)),
      )
    : undefined;
  return (printers ?? []).map((printer) => {
    const destination = selectedTarget?.destinations?.find(
      (item) =>
        item.printerId === printer.id &&
        item.mediaCompatibility?.status === "ready",
    );
    return {
      id: printer.id,
      value: printer.id,
      label: printer.name,
      directTargetId: destination ? selectedTarget.id : null,
      compatible: Boolean(destination),
      isDefault: Boolean(printer.isDefault),
    };
  });
}

export function chooseDefaultPrinterOption(items) {
  return (
    items.find((item) => item.isDefault) ??
    items.find((item) => item.compatible) ??
    items[0]
  );
}

export function printerCompatibilityMessage(document, printer) {
  if (!document || !printer || printer.compatible) return "";
  const documentName = document.name || "This document";
  return `${printer.label} is connected, but ${documentName} does not have compatible published stock settings for it. Browser printing is still available.`;
}

export function previewPlaceholderUrl(baseUrl, status) {
  if (!baseUrl) return undefined;
  const url = new URL(baseUrl);
  url.searchParams.set("state", status);
  return url.toString();
}

export function canUsePublishedBinding(document) {
  return Boolean(
    document?.targetBindingStatus === "ready" &&
    document.designTargetId &&
    document.designSpecificationRevision,
  );
}

export function renderPolicySummary(policy) {
  if (policy === "cloud_only")
    return "Cloud rendering is required. The exact preview PDF is sent to the printer.";
  if (policy === "prefer_node")
    return "A ready compatible node is preferred. Piqae safely falls back to the exact preview PDF.";
  if (policy === "require_node")
    return "Node rendering is required. Piqae checks every compatible target binding at submission and fails closed if none can accept the exact render.";
  return "Piqae automatically selects the fastest compatible path for this document and destination.";
}

export function canUseDestinationForPolicy(destination, _policy) {
  // Target print is authoritative: core evaluates every exact binding against
  // renderer capability and topology at handoff. A printer-only preflight here
  // would check the wrong binding when a compatible standby is available.
  return Boolean(destination?.eligible);
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
  const [printerId, setPrinterId] = useState("");
  const [state, setState] = useState("loading");
  const [error, setError] = useState("");
  const [result, setResult] = useState("");
  const [preview, setPreview] = useState(null);
  const [previewState, setPreviewState] = useState("idle");
  const [previewError, setPreviewError] = useState("");
  const [previewAttempt, setPreviewAttempt] = useState(0);
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
      const defaultDocument = chooseDefaultDocument(value.documents ?? []);
      const defaultPrinter = chooseDefaultPrinterOption(
        printerOptionsForDocument(
          defaultDocument,
          value.targets ?? [],
          value.printers ?? [],
        ),
      );
      setOptions(value);
      setDocumentId(defaultDocument?.id ?? "");
      setPrinterId(defaultPrinter?.value ?? "");
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

  const selectedDocument = documentId
    ? options?.documents?.find(({ id }) => id === documentId)
    : undefined;
  const printerOptions = printerOptionsForDocument(
    selectedDocument,
    options?.targets ?? [],
    options?.printers ?? [],
  );
  const selectedPrinter = printerOptions.find(({ id }) => id === printerId);
  const selectedTarget = options?.targets?.find(
    ({ id }) => id === selectedPrinter?.directTargetId,
  );
  useEffect(() => {
    if (!selectedDocument || !options?.targets) return;
    const next = chooseDefaultPrinterOption(
      printerOptionsForDocument(
        selectedDocument,
        options.targets,
        options.printers ?? [],
      ),
    );
    setPrinterId(next?.value ?? "");
  }, [selectedDocument?.id, options]);
  useEffect(() => {
    if (!options?.linked || !selectedDocument || orderIds.length === 0) {
      setPreview(null);
      setPreviewState("idle");
      setPreviewError("");
      return;
    }
    const sequence = ++previewSequence.current;
    setPreview(null);
    setPreviewError("");
    setPreviewState("loading");
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
        setPreviewState("ready");
      })
      .catch((cause) => {
        if (sequence !== previewSequence.current) return;
        setPreviewError(messageForLoadError(cause));
        setPreviewState("failed");
      });
    return () => {
      previewSequence.current += 1;
    };
  }, [options, selectedDocument?.id, orderIds.join(","), previewAttempt]);

  const src =
    preview?.artifactUrl ??
    previewPlaceholderUrl(
      options?.previewPlaceholderUrl,
      previewState === "failed"
        ? "error"
        : previewState === "loading"
          ? "loading"
          : "empty",
    );

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
    if (
      !preview ||
      !selectedTarget ||
      !canUsePublishedBinding(selectedDocument) ||
      state === "printing"
    )
      return;
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
            "idempotency-key": `shopify-admin-${interactionId.current}-${stableOptionKey(`${printerId}:${selectedTarget.id}:${selectedDocument.designSpecificationRevision}`)}`,
          },
          body: JSON.stringify({
            renderId: preview.renderId,
            targetId: selectedTarget.id,
            specificationRevision: selectedDocument.designSpecificationRevision,
            templateId: selectedDocument.id,
            renderCost: preview.renderCost,
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

  const selectedDestination = selectedTarget;
  const policy = options?.renderExecutionPolicy ?? "automatic";
  const canPrint = Boolean(
    preview &&
    selectedPrinter &&
    canUsePublishedBinding(selectedDocument) &&
    previewState === "ready" &&
    state === "ready" &&
    canUseDestinationForPolicy(selectedDestination, policy),
  );

  return (
    <s-admin-print-action {...(src ? { src } : {})}>
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
                  <s-option key={document.id} value={document.id}>
                    {document.name}
                  </s-option>
                ))}
              </s-select>
            ) : (
              <s-banner tone="warning">
                Publish a document before printing.
              </s-banner>
            )}
            {printerOptions.length > 0 ? (
              <s-select
                label="Printer"
                value={printerId}
                onChange={(event) => setPrinterId(event.currentTarget.value)}
              >
                <s-option value="" disabled>
                  Choose a printer
                </s-option>
                {printerOptions.map((printer) => (
                  <s-option key={printer.id} value={printer.value}>
                    {printer.label}
                  </s-option>
                ))}
              </s-select>
            ) : (
              <s-text>No connected printers are available.</s-text>
            )}
            {printerCompatibilityMessage(selectedDocument, selectedPrinter) && (
              <s-banner tone="warning">
                {printerCompatibilityMessage(selectedDocument, selectedPrinter)}
              </s-banner>
            )}
            {previewState === "loading" && (
              <s-stack direction="inline" gap="small" alignItems="center">
                <s-spinner accessibilityLabel="Generating document preview" />
                <s-text>Generating a preview with the selected orders…</s-text>
              </s-stack>
            )}
            {previewState === "failed" && previewError && (
              <s-banner tone="critical">
                {previewError}
                <s-button
                  onClick={() => setPreviewAttempt((value) => value + 1)}
                >
                  Try again
                </s-button>
              </s-banner>
            )}
            {error && <s-banner tone="critical">{error}</s-banner>}
            {result && <s-banner tone="success">{result}</s-banner>}
            {canPrint ? (
              <s-button variant="primary" onClick={printDirect}>
                {state === "printing" ? "Sending…" : "Print directly"}
              </s-button>
            ) : null}
            {preview ? <s-link href={src}>Download PDF</s-link> : null}
          </>
        )}
      </s-stack>
    </s-admin-print-action>
  );
}
