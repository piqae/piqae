/** @jsxImportSource preact */
import { Component } from "preact";
import { useEffect, useMemo, useRef, useState } from "preact/hooks";

import { authorizedJson } from "./print-urls.js";

export const ADMIN_OPTIONS_TIMEOUT_MS = 8_000;
export const PRINT_PLACEHOLDER_URL = "/api/public/print-placeholder";

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
  const [destinationId, setDestinationId] = useState("");
  const [targetSearch, setTargetSearch] = useState("");
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
        (value.targets ?? []).filter((item) => item.eligible),
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

  const selectedDocument = documentId
    ? options?.documents?.find(({ id }) => id === documentId)
    : undefined;
  const allEligibleTargets =
    options?.targets?.filter((item) => item.eligible) ?? [];
  const compatibleTargets = allEligibleTargets.filter(
    (target) =>
      (!selectedDocument?.compatibilityKnown ||
        selectedDocument.compatibleTargetIds.includes(target.id)) &&
      `${target.name} ${target.stock?.name ?? ""} ${target.destinations?.map(({ printerName }) => printerName).join(" ") ?? ""}`
        .toLowerCase()
        .includes(targetSearch.toLowerCase()),
  );
  const selectedTarget = options?.targets?.find(
    ({ id }) => id === destinationId,
  );
  useEffect(() => {
    if (!selectedDocument || !options?.targets) return;
    setDestinationId(
      targetForDocument(selectedDocument, options.targets)?.id ?? "",
    );
  }, [selectedDocument?.id, options]);
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

  // Shopify's print host can suppress the extension body while `src` is
  // absent. Keep a safe same-origin document attached from the first paint,
  // then atomically replace it with the signed preview artifact.
  const src = preview?.artifactUrl ?? PRINT_PLACEHOLDER_URL;

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
      !destinationId ||
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
            "idempotency-key": `shopify-admin-${interactionId.current}-${stableOptionKey(`${destinationId}:${selectedDocument.designSpecificationRevision}`)}`,
          },
          body: JSON.stringify({
            renderId: preview.renderId,
            targetId: destinationId,
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

  const eligible = compatibleTargets;
  const selectedDestination = selectedTarget;
  const policy = options?.renderExecutionPolicy ?? "automatic";
  const canPrint = Boolean(
    preview &&
    destinationId &&
    canUsePublishedBinding(selectedDocument) &&
    state === "ready" &&
    canUseDestinationForPolicy(selectedDestination, policy),
  );

  return (
    <s-admin-print-action src={src}>
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

            <s-text type="strong">Print target</s-text>
            {selectedDocument?.targetBindingStatus === "revision_changed" && (
              <s-banner tone="critical">
                This target changed after the document was published. Review its
                printer, profile, and stock in the editor, then publish the
                document again.
                <s-button href={options.manageDocumentsUrl}>
                  Review document
                </s-button>
              </s-banner>
            )}
            {selectedDocument?.targetBindingStatus === "target_missing" && (
              <s-banner tone="critical">
                This document's published target is no longer available. Choose
                a replacement in the editor and publish again.
              </s-banner>
            )}
            {selectedDocument?.targetBindingStatus === "unknown" && (
              <s-banner tone="info">
                Printer status is temporarily unavailable, so direct printing
                stays paused. The PDF preview remains available; try again when
                the connection recovers.
              </s-banner>
            )}
            {selectedDocument?.targetBindingStatus === "document_invalid" && (
              <s-banner tone="critical">
                This published document is damaged and cannot be matched to a
                print target. Open it in Documents and publish a valid revision.
              </s-banner>
            )}
            {selectedDocument?.targetBindingStatus === "media_incompatible" && (
              <s-banner tone="critical">
                This document no longer matches its published target stock or
                profile. Review the media settings and publish again.
              </s-banner>
            )}
            {selectedDocument?.targetBindingStatus === "unbound" && (
              <s-banner tone="warning">
                Choose a print target in the document editor and publish before
                direct printing. PDF preview and browser printing remain
                available.
              </s-banner>
            )}
            <s-banner tone={policy === "require_node" ? "warning" : "info"}>
              {renderPolicySummary(policy)}
            </s-banner>
            {options.destinationError && (
              <s-banner tone="warning">{options.destinationError}</s-banner>
            )}
            <s-text-field
              label="Search targets"
              value={targetSearch}
              onInput={(event) => setTargetSearch(event.currentTarget.value)}
              placeholder="Printer, target, or stock"
            />
            {eligible.length > 0 ? (
              <s-select
                label="Target"
                value={destinationId}
                onChange={(event) =>
                  setDestinationId(event.currentTarget.value)
                }
              >
                {eligible.map((destination) => (
                  <option key={destination.id} value={destination.id}>
                    {destination.name} ·{" "}
                    {destination.stock?.name ?? "stock not configured"}
                  </option>
                ))}
              </s-select>
            ) : (
              <s-banner tone="info">
                No compatible target has a ready printer, pinned profile, and
                reported matching stock. PDF preview and browser printing remain
                available.
                <s-button href={options.setupDestinationUrl}>
                  Set up a printer
                </s-button>
              </s-banner>
            )}
            {selectedDestination && (
              <>
                <s-text>
                  Advisory destination:{" "}
                  {selectedDocument?.advisoryDestination?.printerName ??
                    "selected by Piqae at handoff"}{" "}
                  · Profile:{" "}
                  {selectedDocument?.advisoryDestination?.profileName ??
                    "selected by Piqae"}{" "}
                  · Stock: {selectedDestination.stock?.name ?? "not configured"}
                </s-text>
                <s-text>
                  Loaded media:{" "}
                  {selectedDocument?.advisoryDestination?.mediaStatus?.replaceAll(
                    "_",
                    " ",
                  ) ?? "not reported"}
                </s-text>
                <s-text>
                  Last reported destination state:{" "}
                  {selectedDocument?.advisoryDestination?.readinessStatus?.replaceAll(
                    "_",
                    " ",
                  ) ?? "unknown"}
                </s-text>
              </>
            )}
            {selectedDestination && policy !== "cloud_only" && (
              <s-text>
                Piqae validates the exact target binding, route, renderer, and
                resources when you submit. A compatible standby is used when the
                primary cannot satisfy this document.
              </s-text>
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
            {preview && <s-button href={src}>Download PDF</s-button>}
          </>
        )}
      </s-stack>
    </s-admin-print-action>
  );
}
