import type { ActionFunctionArgs, LoaderFunctionArgs } from "react-router";
import { Form, redirect, useActionData, useLoaderData } from "react-router";
import { useEffect, useState } from "react";
import shopify from "../shopify.server";
import {
  bounded,
  newWorkflowId,
  validateDocumentSource,
  workflows,
  type MerchantTemplate,
} from "../core/workflows.server";
import { TemplatePreview } from "../components/shopify-ui";
import { PdfmeDesigner } from "../components/PdfmeDesigner";
import { starterTemplates } from "../core/starter-templates";
import {
  parseTemplateEnvelope,
  serializeTemplateEnvelope,
  canonicalToVisual,
  visualCompatibility,
  visualToCanonical,
  type PdfmeVisualModel,
  type TemplateEnvelope,
  type TemplateEditorMode,
} from "../core/template-model";
import { syncTemplateIndex } from "../core/template-index.server";
import { publishCanonicalTemplate } from "../core/template-publisher.server";
import { createProductionServices } from "../services.server";
import {
  canonicalToLiquid,
  liquidToCanonical,
} from "../core/liquid-document-adapter";
export type EditorMode = TemplateEditorMode;
export function liquidCompatibilityNotice(mode: EditorMode) {
  return mode === "liquid"
    ? "Advanced Liquid is compatibility-gated. Unsupported tags or filters must be resolved before publishing."
    : null;
}
export function canSubmitTemplateMode(
  mode: TemplateEditorMode,
  visual: unknown,
) {
  return mode !== "visual" || visual != null;
}
export function customizedTemplateName(name: string) {
  return `${name} — customized`.slice(0, 200);
}
export function editorLiquidForMode(mode: TemplateEditorMode, liquid: string) {
  void mode;
  return liquid;
}
export function removeSystemOwnership(envelope: TemplateEnvelope) {
  delete envelope.system;
  return envelope;
}
export function parseVisualEditorSource(source: string): PdfmeVisualModel {
  let parsed: unknown;
  try {
    parsed = JSON.parse(source);
  } catch {
    throw new Error("Visual source must be valid JSON");
  }
  if (
    !parsed ||
    typeof parsed !== "object" ||
    (parsed as { schema?: unknown }).schema !== "pdfme-compatible/v1" ||
    !["A4", "A5", "Letter", "80mm"].includes(
      String((parsed as { page?: unknown }).page ?? ""),
    ) ||
    !Array.isArray((parsed as { fields?: unknown }).fields) ||
    ((parsed as { template?: { schemas?: unknown } }).template != null &&
      !Array.isArray(
        (parsed as { template?: { schemas?: unknown } }).template?.schemas,
      ))
  )
    throw new Error(
      "Visual source must use the supported pdfme-compatible/v1 shape",
    );
  return parsed as PdfmeVisualModel;
}
export async function loader({ request, params }: LoaderFunctionArgs) {
  const { session } = await shopify.authenticate.admin(request);
  const id = params.templateId;
  if (!id || id === "new") return { template: null };
  return { template: await workflows().getTemplate(session.shop, id) };
}
export async function action({ request, params }: ActionFunctionArgs) {
  const { session, admin } = await shopify.authenticate.admin(request);
  const form = await request.formData();
  try {
    const intent = String(form.get("intent") ?? "draft");
    const existing =
      params.templateId && params.templateId !== "new"
        ? await workflows().getTemplate(session.shop, params.templateId)
        : null;
    if (intent === "delete") {
      if (
        !existing ||
        !(await workflows().deleteTemplate(session.shop, existing.id))
      )
        throw new Error("Only draft templates can be deleted");
      return { ok: true, error: "", deleted: true };
    }
    if (intent === "customize") {
      if (!existing) throw new Error("System document was not found");
      const envelope = parseTemplateEnvelope(existing.source);
      if (!envelope.system?.immutable)
        throw new Error("Only system documents are customized this way");
      removeSystemOwnership(envelope);
      const saved = await workflows().saveTemplate(session.shop, {
        id: newWorkflowId(),
        name: customizedTemplateName(existing.name),
        kind: existing.kind,
        pageSize: existing.pageSize,
        state: "draft",
        source: serializeTemplateEnvelope(envelope),
        revision: 1,
      });
      await syncTemplateIndex(admin, workflows(), session.shop);
      return redirect(`/app/templates/${encodeURIComponent(saved.id)}`, 303);
    }
    const kind = bounded(form, "kind", 30, true);
    const pageSize = bounded(form, "pageSize", 10, true);
    if (
      ![
        "invoice",
        "packing_slip",
        "receipt",
        "returns",
        "credit_note",
        "custom",
      ].includes(kind) ||
      !["A4", "A5", "Letter", "80mm"].includes(pageSize)
    )
      throw new Error("Template format is invalid");
    if (existing && parseTemplateEnvelope(existing.source).system?.immutable)
      throw new Error("System documents are immutable; choose Customize first");
    let source = validateDocumentSource(bounded(form, "source", 65536, true));
    const envelope = parseTemplateEnvelope(source);
    removeSystemOwnership(envelope);
    const mode = bounded(form, "mode", 10) as TemplateEditorMode;
    if (!["visual", "liquid", "native"].includes(mode))
      throw new Error("Editor mode is invalid");
    envelope.editor.mode = mode;
    envelope.editor.liquid = editorLiquidForMode(
      mode,
      bounded(form, "liquid", 32768),
    );
    if (mode === "liquid") {
      if (!envelope.editor.liquid.trim())
        throw new Error(
          "Liquid source is required in the Liquid editing view.",
        );
      const conversion = liquidToCanonical(
        envelope.editor.liquid,
        envelope.canonical.page,
      );
      if (!conversion.ok) {
        const diagnostic = conversion.diagnostics[0]!;
        throw new Error(
          `Liquid ${diagnostic.code} on line ${diagnostic.line}: ${diagnostic.message}`,
        );
      }
      envelope.canonical = conversion.document;
      envelope.editor.liquid = conversion.normalizedSource;
      try {
        envelope.editor.pdfme = canonicalToVisual(conversion.document);
        envelope.editor.roundTrip = "lossless";
        envelope.editor.warnings = [];
      } catch (error) {
        envelope.editor.roundTrip = "unsupported";
        envelope.editor.warnings = [
          error instanceof Error ? error.message : "Visual conversion failed.",
        ];
      }
    }
    if (mode === "visual") {
      const visual = parseVisualEditorSource(
        bounded(form, "visual", 32768, true),
      );
      envelope.editor.pdfme = visual;
      envelope.canonical = visualToCanonical(visual);
      Object.assign(envelope.editor, visualCompatibility(visual));
      const converted = canonicalToLiquid(envelope.canonical);
      if (converted.source) envelope.editor.liquid = converted.source;
      else {
        envelope.editor.roundTrip = "unsupported";
        envelope.editor.warnings.push(
          converted.diagnostics[0]?.message ?? "Liquid conversion failed.",
        );
      }
    }
    source = serializeTemplateEnvelope(envelope);
    if (intent === "publish") {
      const services = createProductionServices();
      source = await publishCanonicalTemplate({
        shop: session.shop,
        name: bounded(form, "name", 200, true),
        source,
        shops: services.repository,
        vault: services.vault,
        baseUrl: services.baseUrl,
      });
    }
    const saved = await workflows().saveTemplate(session.shop, {
      id: existing?.id ?? newWorkflowId(),
      name: bounded(form, "name", 200, true),
      kind: kind as MerchantTemplate["kind"],
      pageSize,
      state: intent === "publish" ? "published" : "draft",
      source,
      revision: existing?.revision ?? 1,
    });
    await syncTemplateIndex(admin, workflows(), session.shop);
    return { ok: true, error: "", deleted: false, id: saved.id };
  } catch (error) {
    return Response.json(
      {
        ok: false,
        error:
          error instanceof Error
            ? error.message
            : "Template could not be saved",
      },
      { status: 400 },
    );
  }
}
export default function TemplateEditor() {
  const { template } = useLoaderData<typeof loader>();
  const result = useActionData<typeof action>();
  const initialEnvelope = parseTemplateEnvelope(
    template?.source ?? starterTemplates[0]!.source,
  );
  if (!template) delete initialEnvelope.system;
  const source = template?.source ?? serializeTemplateEnvelope(initialEnvelope);
  const envelope = parseTemplateEnvelope(source);
  const [mode, setMode] = useState<TemplateEditorMode>(envelope.editor.mode);
  const [visual, setVisual] = useState(envelope.editor.pdfme);
  const generatedLiquid = canonicalToLiquid(envelope.canonical);
  const [liquid, setLiquid] = useState(
    envelope.editor.liquid ?? generatedLiquid.source ?? "",
  );
  const [workspaceView, setWorkspaceView] = useState<"editor" | "preview">(
    "editor",
  );
  const [switchError, setSwitchError] = useState("");
  useEffect(() => {
    setMode(envelope.editor.mode);
    setVisual(envelope.editor.pdfme);
    setLiquid(envelope.editor.liquid ?? generatedLiquid.source ?? "");
  }, [source]);
  const immutable = Boolean(envelope.system?.immutable);
  const switchMode = (next: TemplateEditorMode) => {
    try {
      if (mode === "visual" && visual && next === "liquid") {
        const converted = canonicalToLiquid(visualToCanonical(visual));
        if (!converted.source)
          throw new Error(
            converted.diagnostics[0]?.message ?? "Liquid conversion failed.",
          );
        setLiquid(converted.source);
      } else if (mode === "liquid" && next === "visual") {
        const converted = liquidToCanonical(liquid, envelope.canonical.page);
        if (!converted.ok) throw new Error(converted.diagnostics[0]!.message);
        setVisual(canonicalToVisual(converted.document));
      }
      setSwitchError("");
      setMode(next);
    } catch (error) {
      setSwitchError(
        error instanceof Error
          ? error.message
          : "Views could not be synchronized.",
      );
    }
  };
  return (
    <s-page heading={template?.name ?? "New template"} inlineSize="large">
      <s-section>
        <Form method="post">
          <s-stack direction="block" gap="base">
            {result?.ok ? (
              <s-banner tone="success">
                {result.deleted ? "Draft deleted." : "Template saved."}
              </s-banner>
            ) : result?.error ? (
              <s-banner tone="critical">{result.error}</s-banner>
            ) : null}
            <s-banner tone="info">
              Publishing converts and pins the canonical Piqae document revision
              used by preview, download and print. Arbitrary Liquid, PDFme
              plugins and HTML are not executed.
            </s-banner>
            {immutable ? (
              <s-banner tone="info">
                This is a read-only system document. Customize it to create a
                merchant-owned draft.
              </s-banner>
            ) : null}
            <s-button-group accessibilityLabel="Template workspace view">
              <s-button
                type="button"
                variant={workspaceView === "editor" ? "primary" : "secondary"}
                onClick={() => setWorkspaceView("editor")}
              >
                Editor
              </s-button>
              <s-button
                type="button"
                variant={workspaceView === "preview" ? "primary" : "secondary"}
                onClick={() => setWorkspaceView("preview")}
              >
                Preview
              </s-button>
            </s-button-group>
            {workspaceView === "editor" ? (
              <div className="piqae-editor-surface">
                <s-stack direction="block" gap="base">
                  <div className="piqae-editor-settings">
                    <label>
                      Name
                      <input
                        className="piqae-input"
                        name="name"
                        required
                        maxLength={200}
                        defaultValue={template?.name ?? "Invoice"}
                      />
                    </label>
                    <label>
                      Document type
                      <select
                        className="piqae-input"
                        name="kind"
                        defaultValue={template?.kind ?? "invoice"}
                      >
                        <option value="invoice">Invoice</option>
                        <option value="packing_slip">Packing slip</option>
                        <option value="receipt">Receipt</option>
                        <option value="returns">Returns form</option>
                        <option value="credit_note">Credit note</option>
                        <option value="custom">Custom</option>
                      </select>
                    </label>
                    <label>
                      Page size
                      <select
                        className="piqae-input"
                        name="pageSize"
                        defaultValue={template?.pageSize ?? "A4"}
                      >
                        <option>A4</option>
                        <option>A5</option>
                        <option>Letter</option>
                        <option value="80mm">80 mm receipt</option>
                      </select>
                    </label>
                    <label>
                      Template editor
                      <select
                        className="piqae-input"
                        name="mode"
                        value={mode}
                        onChange={(event) =>
                          switchMode(
                            event.currentTarget.value as TemplateEditorMode,
                          )
                        }
                        disabled={immutable}
                      >
                        <option value="visual">Visual</option>
                        <option value="liquid">Liquid code</option>
                        <option value="native">Canonical JSON</option>
                      </select>
                    </label>
                  </div>
                  {switchError ? (
                    <s-banner tone="critical">{switchError}</s-banner>
                  ) : null}
                  {envelope.editor.roundTrip !== "lossless" ? (
                    <s-banner tone="warning">
                      Switching views is {envelope.editor.roundTrip}.{" "}
                      {envelope.editor.warnings.join(" ") ||
                        "This source cannot be represented by the visual editor without losing unsupported constructs."}
                    </s-banner>
                  ) : null}
                  {mode === "liquid" ? (
                    generatedLiquid.diagnostics.length ? (
                      <s-banner tone="warning">
                        This canonical document cannot switch losslessly to the
                        bounded Liquid view:{" "}
                        {generatedLiquid.diagnostics[0]!.message}
                      </s-banner>
                    ) : (
                      <s-banner tone="info">
                        This safe Liquid subset maps directly to the canonical
                        document. Whole-line variables, for/if blocks, QR,
                        lines, spacers and page breaks are supported. HTML,
                        filters, includes and plugins are never executed.
                      </s-banner>
                    )
                  ) : null}
                  {mode === "visual" ? (
                    <div className="piqae-card">
                      <s-heading>Visual layout</s-heading>
                      <s-paragraph>
                        This PDFme canvas and the Liquid code view edit the same
                        document. Text and QR fields may contain bounded Liquid
                        expressions such as {"{{ orders.0.name }}"}. The
                        canonical preview is authoritative.
                      </s-paragraph>
                      {visual ? (
                        <input
                          type="hidden"
                          name="visual"
                          value={JSON.stringify(visual)}
                        />
                      ) : null}
                      {visual ? (
                        <PdfmeDesigner
                          value={visual}
                          disabled={immutable}
                          onChange={setVisual}
                        />
                      ) : (
                        <s-banner tone="critical">
                          This template has no visual source. Continue in the
                          canonical or Liquid editor.
                        </s-banner>
                      )}
                    </div>
                  ) : null}
                  {mode === "liquid" ? (
                    <label>
                      Bounded Liquid source
                      <textarea
                        className="piqae-code"
                        name="liquid"
                        maxLength={32768}
                        value={liquid}
                        onChange={(event) =>
                          setLiquid(event.currentTarget.value)
                        }
                        disabled={immutable}
                      />
                    </label>
                  ) : (
                    <input type="hidden" name="liquid" value={liquid} />
                  )}
                  <label>
                    Canonical Piqae document envelope
                    <textarea
                      key={source}
                      className="piqae-code"
                      name="source"
                      required
                      maxLength={65536}
                      defaultValue={source}
                      readOnly={immutable || mode !== "native"}
                    />
                  </label>
                  <div className="piqae-actions">
                    {immutable ? (
                      <button
                        className="piqae-link-button"
                        type="submit"
                        name="intent"
                        value="customize"
                      >
                        Customize
                      </button>
                    ) : (
                      <>
                        <button
                          className="piqae-link-button"
                          type="submit"
                          name="intent"
                          value="draft"
                          disabled={!canSubmitTemplateMode(mode, visual)}
                        >
                          Save draft
                        </button>
                        <button
                          className="piqae-link-button"
                          type="submit"
                          name="intent"
                          value="publish"
                          disabled={!canSubmitTemplateMode(mode, visual)}
                        >
                          Publish revision
                        </button>
                      </>
                    )}
                    {template?.state === "draft" ? (
                      <button
                        className="piqae-link-button"
                        type="submit"
                        name="intent"
                        value="delete"
                      >
                        Delete draft
                      </button>
                    ) : null}
                  </div>
                </s-stack>
              </div>
            ) : (
              <TemplatePreview />
            )}
          </s-stack>
        </Form>
      </s-section>
    </s-page>
  );
}
