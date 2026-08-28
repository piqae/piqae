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
import {
  PrintPacketEditor,
  PrintPacketPreview,
  DocumentSettingsFields,
  Icon,
} from "../components/PrintPacketEditor";
import { starterTemplates } from "../core/starter-templates";
import {
  parseTemplateEnvelope,
  removeSystemOwnership,
  serializeTemplateEnvelope,
  type PrintPacket,
  type TemplateEditorMode,
} from "../core/template-model";
import { syncTemplateIndex } from "../core/template-index.server";
import { publishCanonicalTemplate } from "../core/template-publisher.server";
import { createProductionServices } from "../services.server";
import { shopifyCustomDocumentFields } from "../core/shopify-document-fields";
import { importOrderPrinterProTemplate } from "../core/order-printer-pro-import.server";
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
    if (intent === "import_order_printer") {
      const imported = importOrderPrinterProTemplate(
        bounded(form, "orderPrinterSource", 65536, true),
      );
      return imported.ok
        ? { ok: true, error: "", deleted: false, imported }
        : Response.json(
            {
              ok: false,
              error: "The template needs changes before it can be imported.",
              deleted: false,
              imported,
            },
            { status: 400 },
          );
    }
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
      ) as PrintPacket;
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
const WORKSPACES = [
  ["visual", "Design", "design"],
  ["liquid", "Code", "code"],
  ["preview", "Preview", "preview"],
] as const;

export default function TemplateEditor() {
  const { template, initialTemplate, customFields } =
    useLoaderData<typeof loader>();
  const result = useActionData<typeof action>();
  const initial = parseTemplateEnvelope(
    initialTemplate?.source ?? starterTemplates[0]!.source,
  );
  if (!template) removeSystemOwnership(initial);
  const [document, setDocument] = useState(initial.document);
  const [name, setName] = useState(
    template?.name ??
      (initialTemplate
        ? `${initialTemplate.name} — copy`.slice(0, 200)
        : "Untitled document"),
  );
  const [mode, setMode] = useState<TemplateEditorMode>(
    initial.editor.mode === "liquid" ? "liquid" : "visual",
  );
  const [liquid, setLiquid] = useState(initial.editor.liquid);
  const [importMetadata, setImportMetadata] = useState(initial.editor.import);
  const [workspace, setWorkspace] = useState<"visual" | "preview" | "liquid">(
    initial.editor.mode === "liquid" ? "liquid" : "visual",
  );
  const [error, setError] = useState("");
  const starter = Boolean(initial.system?.immutable);
  useEffect(() => {
    setDocument(initial.document);
    setMode(initial.editor.mode === "liquid" ? "liquid" : "visual");
    setLiquid(initial.editor.liquid);
    setImportMetadata(initial.editor.import);
  }, [initialTemplate?.source]);
  useEffect(() => {
    if (result && "imported" in result && result.imported?.ok) {
      setDocument(result.imported.document);
      setLiquid(result.imported.normalizedLiquid);
      setImportMetadata({
        format: "order_printer_pro",
        originalSource: result.imported.originalSource,
        diagnostics: result.imported.diagnostics,
      });
      setMode("visual");
      setWorkspace("visual");
    }
  }, [result]);
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
    editor: {
      mode,
      liquid,
      roundTrip: "lossless",
      warnings: [],
      ...(importMetadata ? { import: importMetadata } : {}),
    },
  });
  const switchWorkspace = (next: "visual" | "preview" | "liquid") => {
    if (next !== "preview") switchMode(next);
    setWorkspace(next);
  };
  return (
    <s-page heading={template?.name ?? "New template"} inlineSize="large">
      <s-section>
        <Form method="post">
          <div className="piqae-actionbar">
            <div className="piqae-actionbar-lead">
              <input
                className="piqae-doc-title"
                name="name"
                required
                maxLength={200}
                aria-label="Document name"
                placeholder="Untitled document"
                value={name}
                onChange={(event) => setName(event.currentTarget.value)}
              />
              <span className="piqae-doc-state">
                {templateStateLabel(template, starter)}
              </span>
            </div>
            <div
              className="piqae-segmented"
              role="group"
              aria-label="Editor workspace"
            >
              {WORKSPACES.map(([key, label, icon]) => (
                <button
                  key={key}
                  type="button"
                  aria-pressed={workspace === key}
                  onClick={() => switchWorkspace(key)}
                >
                  <Icon name={icon} />
                  {label}
                </button>
              ))}
            </div>
            <div className="piqae-actionbar-trail">
              <details className="piqae-menu">
                <summary
                  className="piqae-button"
                  aria-label="Document settings"
                  title="Document settings"
                >
                  <Icon name="settings" />
                  Settings
                </summary>
                <div className="piqae-menu-panel piqae-settings-panel">
                  <label className="piqae-field">
                    <span>Document type</span>
                    <select
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
                  <label className="piqae-field">
                    <span>Media</span>
                    <select
                      name="pageSize"
                      defaultValue={initialTemplate?.pageSize ?? "A4"}
                    >
                      <option>A4</option>
                      <option>A5</option>
                      <option>Letter</option>
                      <option value="80mm">80 mm receipt</option>
                    </select>
                  </label>
                  <DocumentSettingsFields
                    value={document}
                    onChange={setDocument}
                  />
                  <p className="piqae-menu-note">
                    Content reflows across pages automatically.
                  </p>
                </div>
              </details>
              <button className="piqae-button" name="intent" value="draft">
                {starter ? "Save as copy" : "Save draft"}
              </button>
              <button
                className="piqae-button piqae-button-primary"
                name="intent"
                value="publish"
              >
                Publish
              </button>
              {template?.state === "draft" ? (
                <details className="piqae-menu piqae-menu-end">
                  <summary
                    className="piqae-button piqae-button-icon"
                    aria-label="More actions"
                    title="More actions"
                  >
                    <Icon name="more" />
                  </summary>
                  <div className="piqae-menu-panel">
                    <button
                      className="piqae-menu-item piqae-menu-critical"
                      name="intent"
                      value="delete"
                    >
                      <Icon name="trash" />
                      Delete draft
                    </button>
                  </div>
                </details>
              ) : null}
            </div>
          </div>
          <p className="piqae-actionbar-note">
            {templateFlowNote(template, starter)}
          </p>
          <s-stack direction="block" gap="base">
            {result?.ok ? (
              <s-banner tone="success">
                {"imported" in result
                  ? "Template imported into the visual editor. Review highlighted compatibility notes, then save it."
                  : result.deleted
                    ? "Draft deleted."
                    : "Template saved."}
              </s-banner>
            ) : result?.error ? (
              <s-banner tone="critical">{result.error}</s-banner>
            ) : null}
            {error ? <s-banner tone="critical">{error}</s-banner> : null}
            {result &&
            "imported" in result &&
            result.imported?.diagnostics.length ? (
              <div className="piqae-import-diagnostics">
                <strong>Import compatibility</strong>
                {result.imported.diagnostics.map((diagnostic, index) => (
                  <p key={`${diagnostic.code}-${index}`}>
                    <s-badge
                      tone={
                        diagnostic.fidelity === "unsupported"
                          ? "critical"
                          : diagnostic.fidelity === "lossy"
                            ? "warning"
                            : "info"
                      }
                    >
                      {diagnostic.fidelity}
                    </s-badge>{" "}
                    {diagnostic.message}
                  </p>
                ))}
              </div>
            ) : null}
            {workspace === "preview" ? (
              <PrintPacketPreview value={document} />
            ) : workspace === "visual" ? (
              <PrintPacketEditor
                value={document}
                disabled={false}
                customFields={customFields}
                onChange={setDocument}
              />
            ) : (
              <div className="piqae-code-workspace">
                <label>
                  Shopify Liquid / Order Printer template
                  <textarea
                    className="piqae-code"
                    name="liquid"
                    maxLength={65536}
                    value={liquid}
                    onChange={(event) => setLiquid(event.currentTarget.value)}
                  />
                </label>
                <input type="hidden" name="orderPrinterSource" value={liquid} />
                <button
                  className="piqae-button"
                  type="submit"
                  name="intent"
                  value="import_order_printer"
                >
                  Convert code to visual document
                </button>
              </div>
            )}
          </s-stack>
          <input type="hidden" name="mode" value={mode} />
          <input
            type="hidden"
            name="document"
            value={JSON.stringify(document)}
          />
          <input type="hidden" name="source" value={source} />
          {workspace === "liquid" ? null : (
            <input type="hidden" name="liquid" value={liquid} />
          )}
        </Form>
      </s-section>
    </s-page>
  );
}

export function templateStateLabel(
  template: MerchantTemplate | null,
  starter: boolean,
) {
  if (starter) return "Starter";
  if (!template) return "Not saved yet";
  return template.state === "published"
    ? `Published · revision ${template.revision}`
    : "Draft";
}

export function templateFlowNote(
  template: MerchantTemplate | null,
  starter: boolean,
) {
  if (starter)
    return "Starter document. Save as copy creates an editable document in your shop; the original stays untouched.";
  if (!template)
    return "New document. Save draft keeps your work private; Publish makes this revision available to printing and automations.";
  return template.state === "published"
    ? "Published. Save draft holds edits back from printing; Publish issues the next revision."
    : "Draft. Publish makes this revision available to printing and automations.";
}
