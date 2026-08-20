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
import { BusinessDocumentEditor } from "../components/BusinessDocumentEditor";
import { starterTemplates } from "../core/starter-templates";
import {
  parseTemplateEnvelope,
  removeSystemOwnership,
  serializeTemplateEnvelope,
  type BusinessDocument,
  type TemplateEditorMode,
} from "../core/template-model";
import { syncTemplateIndex } from "../core/template-index.server";
import { publishCanonicalTemplate } from "../core/template-publisher.server";
import { createProductionServices } from "../services.server";
import { shopifyCustomDocumentFields } from "../core/shopify-document-fields";
import {
  canonicalToLiquid,
  liquidToCanonical,
} from "../core/liquid-document-adapter";
export type EditorMode = TemplateEditorMode;
export const liquidCompatibilityNotice = (mode: EditorMode) =>
  mode === "liquid"
    ? "Advanced Liquid is compatibility-gated. Unsupported constructs must be resolved before publishing."
    : null;
export const canSubmitTemplateMode = (
  _mode: TemplateEditorMode,
  document: unknown,
) => document != null;
export const customizedTemplateName = (name: string) =>
  `${name} — customized`.slice(0, 200);
export const editorLiquidForMode = (
  _mode: TemplateEditorMode,
  liquid: string,
) => liquid;
export async function loader({ request, params }: LoaderFunctionArgs) {
  const { session } = await shopify.authenticate.admin(request);
  const template =
    !params.templateId || params.templateId === "new"
      ? null
      : await workflows().getTemplate(session.shop, params.templateId);
  let initialTemplate: MerchantTemplate | null = template;
  if (!template && params.templateId === "new") {
    const sourceId = new URL(request.url).searchParams.get("from");
    const customSource = sourceId
      ? await workflows().getTemplate(session.shop, sourceId)
      : null;
    const starterSource = starterTemplates.find(
      (candidate) => candidate.id === sourceId,
    );
    if (customSource) initialTemplate = customSource;
    else if (starterSource)
      initialTemplate = {
        id: starterSource.id,
        name: starterSource.name,
        kind: starterSource.kind,
        pageSize: starterSource.pageSize,
        state: "draft",
        source: starterSource.source,
        revision: 1,
        updatedAt: new Date(0).toISOString(),
      };
  }
  return {
    template,
    initialTemplate,
    customFields: shopifyCustomDocumentFields(
      (await workflows().getSettings(session.shop)).metafieldAllowlist,
    ),
  };
}
export async function action({ request, params }: ActionFunctionArgs) {
  const { session, admin } = await shopify.authenticate.admin(request);
  const form = await request.formData();
  try {
    const intent = String(form.get("intent") ?? "draft");
    let existing =
      params.templateId && params.templateId !== "new"
        ? await workflows().getTemplate(session.shop, params.templateId)
        : null;
    const savingFromStarter = Boolean(
      existing && parseTemplateEnvelope(existing.source).system?.immutable,
    );
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
        throw new Error("Only system documents can be customized");
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
    // Starter documents remain pristine. Editing one transparently creates a
    // merchant-owned copy so the first edit feels like editing a normal file.
    if (savingFromStarter) existing = null;
    const envelope = parseTemplateEnvelope(
      validateDocumentSource(bounded(form, "source", 262144, true)),
    );
    removeSystemOwnership(envelope);
    const mode = bounded(form, "mode", 10) as TemplateEditorMode;
    if (!["visual", "liquid", "source"].includes(mode))
      throw new Error("Editor mode is invalid");
    envelope.editor.mode = mode;
    if (mode === "liquid") {
      const liquid = bounded(form, "liquid", 65536, true);
      const conversion = liquidToCanonical(liquid, envelope.document);
      if (!conversion.ok) {
        const d = conversion.diagnostics[0]!;
        throw new Error(
          `Liquid ${d.code} at ${d.line}:${d.column}: ${d.message}`,
        );
      }
      envelope.document = conversion.document;
      envelope.editor.liquid = conversion.normalizedSource;
    } else {
      const document = JSON.parse(
        bounded(form, "document", 196608, true),
      ) as BusinessDocument;
      envelope.document = document;
      envelope.editor.liquid = canonicalToLiquid(document).source;
    }
    let source = serializeTemplateEnvelope(envelope);
    if (intent === "publish") {
      const services = createProductionServices();
      source = await publishCanonicalTemplate({
        shop: session.shop,
        name: bounded(form, "name", 200, true),
        source,
        shops: services.repository,
        vault: services.vault,
        baseUrl: services.baseUrl,
        managedClientFactory: (link) => services.managedAccounts.client(link),
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
    if (savingFromStarter)
      return redirect(`/app/templates/${encodeURIComponent(saved.id)}`, 303);
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
  const { template, initialTemplate, customFields } =
    useLoaderData<typeof loader>();
  const result = useActionData<typeof action>();
  const initial = parseTemplateEnvelope(
    initialTemplate?.source ?? starterTemplates[0]!.source,
  );
  if (!template) removeSystemOwnership(initial);
  const [document, setDocument] = useState(initial.document);
  const [mode, setMode] = useState(initial.editor.mode);
  const [liquid, setLiquid] = useState(initial.editor.liquid);
  const [view, setView] = useState<"edit" | "preview">("edit");
  const [error, setError] = useState("");
  const starter = Boolean(initial.system?.immutable);
  useEffect(() => {
    setDocument(initial.document);
    setMode(initial.editor.mode);
    setLiquid(initial.editor.liquid);
  }, [initialTemplate?.source]);
  const switchMode = (next: TemplateEditorMode) => {
    if (mode === "liquid" && next !== "liquid") {
      const conversion = liquidToCanonical(liquid, document);
      if (!conversion.ok) {
        const d = conversion.diagnostics[0]!;
        setError(`${d.message} (${d.line}:${d.column})`);
        return;
      }
      setDocument(conversion.document);
    } else if (mode !== "liquid" && next === "liquid")
      setLiquid(canonicalToLiquid(document).source);
    setError("");
    setMode(next);
  };
  const source = serializeTemplateEnvelope({
    ...initial,
    document,
    editor: { mode, liquid, roundTrip: "lossless", warnings: [] },
  });
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
              This structured document reflows line items, tables and text
              automatically. Preview, download and print use the same published
              Piqae revision.
            </s-banner>
            {starter ? (
              <s-banner tone="info">
                You are editing a starter. Saving creates your own copy and
                keeps the original available for future documents.
              </s-banner>
            ) : null}
            <s-button-group accessibilityLabel="Document view">
              <s-button
                type="button"
                variant={view === "edit" ? "primary" : "secondary"}
                onClick={() => setView("edit")}
              >
                Edit
              </s-button>
              <s-button
                type="button"
                variant={view === "preview" ? "primary" : "secondary"}
                onClick={() => setView("preview")}
              >
                Preview
              </s-button>
            </s-button-group>
            {view === "preview" ? (
              <TemplatePreview />
            ) : (
              <div className="piqae-editor-surface">
                <div className="piqae-editor-settings">
                  <label>
                    Name
                    <input
                      className="piqae-input"
                      name="name"
                      required
                      maxLength={200}
                      defaultValue={
                        template?.name ??
                        (initialTemplate
                          ? `${initialTemplate.name} — copy`.slice(0, 200)
                          : "Untitled document")
                      }
                    />
                  </label>
                  <label>
                    Document type
                    <select
                      className="piqae-input"
                      name="kind"
                      defaultValue={initialTemplate?.kind ?? "invoice"}
                    >
                      <option value="invoice">Invoice</option>
                      <option value="packing_slip">Packing slip</option>
                      <option value="receipt">Receipt</option>
                      <option value="credit_note">Credit note</option>
                      <option value="returns">Returns form</option>
                      <option value="custom">Custom</option>
                    </select>
                  </label>
                  <label>
                    Media
                    <select
                      className="piqae-input"
                      name="pageSize"
                      defaultValue={initialTemplate?.pageSize ?? "A4"}
                    >
                      <option>A4</option>
                      <option>A5</option>
                      <option>Letter</option>
                      <option value="80mm">80 mm receipt</option>
                    </select>
                  </label>
                  <label>
                    Editor
                    <select
                      className="piqae-input"
                      name="mode"
                      value={mode}
                      onChange={(event) =>
                        switchMode(
                          event.currentTarget.value as TemplateEditorMode,
                        )
                      }
                      disabled={false}
                    >
                      <option value="visual">Document editor</option>
                      <option value="liquid">Advanced Liquid</option>
                      <option value="source">Piqae source</option>
                    </select>
                  </label>
                </div>
                {error ? <s-banner tone="critical">{error}</s-banner> : null}
                {mode === "visual" ? (
                  <BusinessDocumentEditor
                    value={document}
                    disabled={false}
                    customFields={customFields}
                    onChange={setDocument}
                  />
                ) : mode === "liquid" ? (
                  <label>
                    Shopify Liquid
                    <textarea
                      className="piqae-code"
                      name="liquid"
                      maxLength={65536}
                      value={liquid}
                      onChange={(e) => setLiquid(e.currentTarget.value)}
                      disabled={false}
                    />
                  </label>
                ) : (
                  <label>
                    Piqae business-document source
                    <textarea
                      className="piqae-code"
                      name="document"
                      maxLength={196608}
                      value={JSON.stringify(document, null, 2)}
                      onChange={(e) => {
                        try {
                          setDocument(JSON.parse(e.currentTarget.value));
                          setError("");
                        } catch {
                          setError("Source must be valid JSON");
                        }
                      }}
                      disabled={false}
                    />
                  </label>
                )}
                {mode !== "source" ? (
                  <input
                    type="hidden"
                    name="document"
                    value={JSON.stringify(document)}
                  />
                ) : null}
                <input type="hidden" name="source" value={source} />
                {mode !== "liquid" ? (
                  <input type="hidden" name="liquid" value={liquid} />
                ) : null}
                <div className="piqae-actions">
                  <>
                    <button
                      className="piqae-link-button"
                      name="intent"
                      value="draft"
                    >
                      {starter ? "Save as new template" : "Save draft"}
                    </button>
                    <button
                      className="piqae-link-button"
                      name="intent"
                      value="publish"
                    >
                      Publish revision
                    </button>
                  </>
                  {template?.state === "draft" ? (
                    <button
                      className="piqae-link-button"
                      name="intent"
                      value="delete"
                    >
                      Delete draft
                    </button>
                  ) : null}
                </div>
              </div>
            )}
          </s-stack>
        </Form>
      </s-section>
    </s-page>
  );
}
