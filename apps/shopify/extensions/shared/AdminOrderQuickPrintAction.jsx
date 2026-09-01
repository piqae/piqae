/** @jsxImportSource preact */
import { Component } from "preact";
import { useEffect, useMemo, useRef, useState } from "preact/hooks";

import {
  approvalForDocumentPrinter,
  loadWithTimeout,
  messageForLoadError,
  newInteractionId,
  printerOptionsForDocument,
  stableOptionKey,
} from "./AdminOrderPrintAction.jsx";
import { authorizedJson } from "./print-urls.js";

class QuickPrintErrorBoundary extends Component {
  state = { error: "" };

  componentDidCatch(error) {
    this.setState({
      error:
        error instanceof Error
          ? error.message
          : "Quick print could not be opened.",
    });
  }

  render() {
    if (this.state.error)
      return (
        <s-admin-action heading="Quick print with Piqae">
          <s-banner tone="critical">{this.state.error}</s-banner>
          <s-button slot="primary-action" onClick={() => shopify.close()}>
            Close
          </s-button>
        </s-admin-action>
      );
    return this.props.children;
  }
}

export function AdminOrderQuickPrintAction(props) {
  return (
    <QuickPrintErrorBoundary>
      <AdminOrderQuickPrintActionContent {...props} />
    </QuickPrintErrorBoundary>
  );
}

function AdminOrderQuickPrintActionContent({ bulk = false }) {
  const orderIds = useMemo(
    () => (shopify.data.selected ?? []).map(({ id }) => id),
    [],
  );
  const interactionId = useRef(newInteractionId(orderIds));
  const [attempt, setAttempt] = useState(0);
  const [state, setState] = useState("printing");
  const [error, setError] = useState("");
  const [setupUrl, setSetupUrl] = useState("/app/templates");
  const [summary, setSummary] = useState("");
  const [jobId, setJobId] = useState("");

  useEffect(() => {
    let active = true;
    setState("printing");
    setError("");
    setSetupUrl("/app/templates");
    setSummary("");
    setJobId("");

    async function submit() {
      if (orderIds.length === 0)
        throw new Error(
          "No order was selected. Close this view and try again.",
        );
      const options = await loadWithTimeout((signal) =>
        authorizedJson("/api/print/admin/options", { signal }),
      );
      if (!active) return;
      if (!options.linked) {
        setState("setup");
        setSetupUrl(options.setupDestinationUrl ?? "/app/printers");
        setError("Connect a Piqae Node before using quick print.");
        return;
      }
      const document = (options.documents ?? []).find(
        (item) => item.id && item.isDefault,
      );
      if (!document) {
        setState("setup");
        setError(
          "Choose a published quick-print document in Piqae Templates first.",
        );
        return;
      }
      const printer = printerOptionsForDocument(
        document,
        options.targets ?? [],
        options.printers ?? [],
      ).find((item) => item.isDefault && item.eligible);
      if (!printer) {
        setState("setup");
        setSetupUrl(options.setupDestinationUrl ?? "/app/printers");
        setError(
          "Choose an available default printer in Piqae Nodes & printers first.",
        );
        return;
      }
      const approval = approvalForDocumentPrinter(document, printer);
      if (!approval)
        throw new Error(
          "The configured quick-print document cannot use the default printer.",
        );
      setSummary(`${document.name} → ${printer.label}`);
      const preview = await loadWithTimeout(
        (signal) =>
          authorizedJson("/api/print/admin/previews", {
            method: "POST",
            headers: {
              "content-type": "application/json",
              "idempotency-key": `shopify-quick-preview-${interactionId.current}-${document.id}`,
            },
            body: JSON.stringify({ orderIds, templateId: document.id }),
            signal,
          }),
        15_000,
      );
      if (!active) {
        authorizedJson(
          `/api/print/previews/${encodeURIComponent(preview.previewId)}/cancel`,
          {
            method: "POST",
            headers: {
              "content-type": "application/json",
              "idempotency-key": `shopify-quick-cancel-${interactionId.current}-${preview.previewId}`,
            },
            body: JSON.stringify({ renderId: preview.renderId }),
          },
        ).catch(() => {});
        return;
      }
      const approvalKey =
        approval.mode === "target"
          ? `${printer.id}:${approval.targetId}:${approval.specificationRevision}`
          : `${approval.printerId}:current-defaults`;
      const destination =
        approval.mode === "target"
          ? {
              targetId: approval.targetId,
              specificationRevision: approval.specificationRevision,
              templateId: document.id,
            }
          : { printerId: approval.printerId, templateId: document.id };
      const result = await loadWithTimeout(
        (signal) =>
          authorizedJson(
            `/api/print/previews/${encodeURIComponent(preview.previewId)}/approve`,
            {
              method: "POST",
              headers: {
                "content-type": "application/json",
                "idempotency-key": `shopify-quick-${interactionId.current}-${stableOptionKey(approvalKey)}`,
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
      if (!active) return;
      setJobId(result.jobId);
      setState("success");
    }

    submit().catch((cause) => {
      if (!active || cause?.name === "AbortError") return;
      setError(messageForLoadError(cause));
      setState("failed");
    });
    return () => {
      active = false;
      // A timed-out approval has uncertain delivery. Keep the exact preview
      // alive and retry with the same idempotency key so a completed handoff is
      // replayed rather than duplicated. Unused previews expire server-side.
    };
  }, [attempt, orderIds.join(",")]);

  return (
    <s-admin-action
      heading="Quick print with Piqae"
      loading={state === "printing"}
    >
      <s-stack direction="block" gap="base">
        {bulk ? (
          <s-text type="strong">{orderIds.length} selected orders</s-text>
        ) : null}
        {state === "printing" ? (
          <s-stack direction="inline" gap="small" alignItems="center">
            <s-spinner accessibilityLabel="Sending the quick-print job" />
            <s-text>
              {summary
                ? `Sending ${summary}…`
                : "Loading quick-print settings…"}
            </s-text>
          </s-stack>
        ) : null}
        {state === "success" ? (
          <s-banner tone="success">
            {summary} was accepted as print job {jobId}.
          </s-banner>
        ) : null}
        {state === "failed" ? (
          <s-banner tone="critical">
            {error}
            <s-button onClick={() => setAttempt((value) => value + 1)}>
              Try again
            </s-button>
          </s-banner>
        ) : null}
        {state === "setup" ? (
          <s-banner tone="info">
            {error}
            <s-button href={setupUrl}>Open Piqae settings</s-button>
          </s-banner>
        ) : null}
      </s-stack>
      <s-button
        slot="primary-action"
        variant="primary"
        disabled={state === "printing"}
        onClick={() => shopify.close()}
      >
        {state === "success" ? "Done" : "Close"}
      </s-button>
    </s-admin-action>
  );
}
