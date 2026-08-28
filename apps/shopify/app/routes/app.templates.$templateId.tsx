import type { ActionFunctionArgs, LoaderFunctionArgs } from "react-router";
import { Form, redirect, useActionData, useLoaderData } from "react-router";
import { useEffect, useState } from "react";
import shopify from "../shopify.server";
import {
  bounded,
  newWorkflowId,
  validateDocumentSource,
  workflows,
  WorkflowConflictError,
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
  validatePrintPacket,
  type PrintPacket,
  type TemplateEditorMode,
} from "../core/template-model";
import { syncTemplateIndex } from "../core/template-index.server";
import { publishCanonicalTemplate } from "../core/template-publisher.server";
import { createProductionServices } from "../services.server";
import { shopifyCustomDocumentFields } from "../core/shopify-document-fields";
import { importOrderPrinterProTemplate } from "../core/order-printer-pro-import.server";
import { loadShopifyPrintTargets } from "../core/shopify-print-targets.server";
import {
  targetSupportsDocument,
  selectTargetDestination,
  type ShopifyPrintTarget,
} from "../core/shopify-print-targets";
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
        draftRevision: 1,
        designTargetId: null,
        designSpecificationRevision: null,
        published: null,
        updatedAt: new Date(0).toISOString(),
      };
  }
  const services = createProductionServices();
  const link = await services.repository.get(session.shop);
  let printTargets: ShopifyPrintTarget[] = [];
  let printTargetError = "";
  if (link) {
    try {
      const loaded = await loadShopifyPrintTargets(
        services.clientForLink(link),
      );
      printTargets = loaded.targets;
      if (loaded.partial)
        printTargetError = "Some print targets are temporarily unavailable";
    } catch {
      printTargetError = "Print targets are temporarily unavailable";
    }
  }
  return {
    template,
    initialTemplate,
    printTargets,
    printTargetError,
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
    const expectedDraftRevisionValue = bounded(
      form,
      "expectedDraftRevision",
      20,
    );
    const expectedDraftRevision = expectedDraftRevisionValue
      ? Number(expectedDraftRevisionValue)
      : null;
    if (
      expectedDraftRevision !== null &&
      (!Number.isInteger(expectedDraftRevision) || expectedDraftRevision < 1)
    )
      throw new Error("Document revision is invalid");
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
    if (
      ![
        "invoice",
        "packing_slip",
        "receipt",
        "returns",
        "credit_note",
        "label",
        "custom",
      ].includes(kind)
    )
      throw new Error("Template format is invalid");
    // Starter documents remain pristine. Editing one transparently creates a
    // merchant-owned copy so the first edit feels like editing a normal file.
    if (savingFromStarter) existing = null;
    if (existing && expectedDraftRevision === null)
      throw new Error("Document revision is required; reload before saving");
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
    validatePrintPacket(envelope.document);
    const pageSize = pageSizeForDocument(envelope.document);
    const designTargetId = bounded(form, "designTargetId", 128);
    let designSpecificationRevision: string | null = null;
    if (designTargetId) {
      const services = createProductionServices();
      const link = await services.repository.get(session.shop);
      if (!link)
        throw new Error("Connect Piqae before assigning a print target");
      const specification = await services
        .clientForLink(link)
        .targets.designSpecification(designTargetId);
      const loaded = await loadShopifyPrintTargets({
        targets: {
          list: async () => [specification.target],
          designSpecification: async () => specification,
        },
      });
      const [target] = loaded.targets;
      if (!target || !targetSupportsDocument(target, envelope.document))
        throw new Error(
          "The selected print target does not support this document media",
        );
      designSpecificationRevision = specification.specification_revision;
    }
    let source = serializeTemplateEnvelope(envelope);
    const id = existing?.id ?? newWorkflowId();
    let staged = null;
    let saved: MerchantTemplate | undefined;
    if (intent === "publish") {
      // Persist the editable draft through CAS before creating a remote Piqae
      // revision. An already-stale browser is rejected before that external
      // side effect, and a remote outage still leaves the merchant's draft safe.
      staged = await workflows().saveTemplate(session.shop, {
        id,
        name: bounded(form, "name", 200, true),
        kind: kind as MerchantTemplate["kind"],
        pageSize,
        state: "draft",
        source,
        revision: existing?.revision ?? 1,
        designTargetId: designTargetId || null,
        designSpecificationRevision,
        expectedDraftRevision: existing ? expectedDraftRevision : null,
      });
      const services = createProductionServices();
      source = await publishCanonicalTemplate({
        shop: session.shop,
        name: bounded(form, "name", 200, true),
        source,
        shops: services.repository,
        vault: services.vault,
        baseUrl: services.baseUrl,
        managedClientFactory: (link) => services.managedAccounts.client(link),
        activate: async (publishedSource) => {
          saved = await workflows().saveTemplate(session.shop, {
            id,
            name: bounded(form, "name", 200, true),
            kind: kind as MerchantTemplate["kind"],
            pageSize,
            state: "published",
            source: publishedSource,
            revision: existing?.revision ?? 1,
            designTargetId: designTargetId || null,
            designSpecificationRevision,
            expectedDraftRevision: staged!.draftRevision,
          });
        },
      });
    } else {
      saved = await workflows().saveTemplate(session.shop, {
        id,
        name: bounded(form, "name", 200, true),
        kind: kind as MerchantTemplate["kind"],
        pageSize,
        state: "draft",
        source,
        revision: existing?.revision ?? 1,
        designTargetId: designTargetId || null,
        designSpecificationRevision,
        expectedDraftRevision: existing ? expectedDraftRevision : null,
      });
    }
    if (!saved) throw new Error("Published document activation failed");
    await syncTemplateIndex(admin, workflows(), session.shop);
    if (savingFromStarter || !params.templateId || params.templateId === "new")
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
      { status: error instanceof WorkflowConflictError ? 409 : 400 },
    );
  }
}
const WORKSPACES = [
  ["visual", "Design", "design"],
  ["liquid", "Code", "code"],
  ["preview", "Preview", "preview"],
] as const;

export default function TemplateEditor() {
  const {
    template,
    initialTemplate,
    customFields,
    printTargets,
    printTargetError,
  } = useLoaderData<typeof loader>();
  const result = useActionData<typeof action>();
  const initial = parseTemplateEnvelope(
    initialTemplate?.source ?? starterTemplates[0]!.source,
  );
  if (!template) removeSystemOwnership(initial);
  const [document, setDocument] = useState(initial.document);
  const [kind, setKind] = useState(initialTemplate?.kind ?? "invoice");
  const [designTargetId, setDesignTargetId] = useState(
    initialTemplate?.designTargetId ?? "",
  );
  const [targetSearch, setTargetSearch] = useState("");
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
  const compatibleTargets = printTargets.filter(
    (target) =>
      targetSupportsDocument(target, document) &&
      `${target.name} ${target.stock?.name ?? ""} ${target.destinations.map(({ printerName }) => printerName).join(" ")}`
        .toLowerCase()
        .includes(targetSearch.toLowerCase()),
  );
  const selectedTarget = printTargets.find(
    (target) => target.id === designTargetId,
  );
  useEffect(() => {
    setDocument(initial.document);
    setMode(initial.editor.mode === "liquid" ? "liquid" : "visual");
    setLiquid(initial.editor.liquid);
    setImportMetadata(initial.editor.import);
    setKind(initialTemplate?.kind ?? "invoice");
    setDesignTargetId(initialTemplate?.designTargetId ?? "");
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
                      value={kind}
                      onChange={(event) => {
                        const nextKind = event.currentTarget.value;
                        setKind(nextKind);
                        if (nextKind === "receipt")
                          setDocument({
                            ...document,
                            media: mediaForPageSize("80mm"),
                          });
                        else if (nextKind === "label")
                          setDocument({
                            ...document,
                            media: mediaForPageSize("100x50mm"),
                          });
                        else if (
                          nextKind !== "custom" &&
                          document.media.kind !== "paged"
                        )
                          setDocument({
                            ...document,
                            media: mediaForPageSize("A4"),
                          });
                      }}
                    >
                      <option value="invoice">Invoice</option>
                      <option value="packing_slip">Packing slip</option>
                      <option value="receipt">Receipt</option>
                      <option value="credit_note">Credit note</option>
                      <option value="label">Label</option>
                      <option value="returns">Returns form</option>
                      <option value="custom">Custom</option>
                    </select>
                  </label>
                  <label className="piqae-field">
                    <span>Media</span>
                    <select
                      name="pageSize"
                      value={mediaPresetForDocument(document)}
                      onChange={(event) => {
                        const media = mediaForPageSize(
                          event.currentTarget.value,
                        );
                        setDocument({
                          ...document,
                          media,
                        });
                        if (media.kind === "continuous") setKind("receipt");
                        else if (media.kind === "label") setKind("label");
                        else if (kind === "receipt" || kind === "label")
                          setKind("custom");
                      }}
                    >
                      <option>A4</option>
                      <option>A5</option>
                      <option>Letter</option>
                      <option value="80mm">80 mm receipt</option>
                      <option value="custom-continuous">
                        Custom roll width
                      </option>
                      <option value="100x50mm">100 × 50 mm label</option>
                      <option value="custom-label">Custom fixed label</option>
                    </select>
                  </label>
                  {document.media.kind === "paged" ? (
                    <label className="piqae-field">
                      <span>Orientation</span>
                      <select
                        name="orientation"
                        value={document.media.orientation ?? "portrait"}
                        onChange={(event) => {
                          const orientation =
                            event.currentTarget.value === "landscape"
                              ? "landscape"
                              : "portrait";
                          setDocument((current) =>
                            current.media.kind !== "paged"
                              ? current
                              : {
                                  ...current,
                                  media: {
                                    ...current.media,
                                    orientation,
                                  },
                                },
                          );
                        }}
                      >
                        <option value="portrait">Portrait</option>
                        <option value="landscape">Landscape</option>
                      </select>
                    </label>
                  ) : null}
                  {document.media.kind !== "paged" ? (
                    <label className="piqae-field">
                      <span>Width (mm)</span>
                      <input
                        type="number"
                        min="10"
                        max="1000"
                        step="0.1"
                        value={document.media.width_mm}
                        onChange={(event) => {
                          const width = event.currentTarget.valueAsNumber;
                          if (Number.isFinite(width))
                            setDocument((current) =>
                              current.media.kind === "paged"
                                ? current
                                : {
                                    ...current,
                                    media: {
                                      ...current.media,
                                      width_mm: width,
                                    },
                                  },
                            );
                        }}
                      />
                    </label>
                  ) : null}
                  {document.media.kind === "label" ? (
                    <label className="piqae-field">
                      <span>Height (mm)</span>
                      <input
                        type="number"
                        min="5"
                        max="1000"
                        step="0.1"
                        value={document.media.height_mm}
                        onChange={(event) => {
                          const height = event.currentTarget.valueAsNumber;
                          if (Number.isFinite(height))
                            setDocument((current) =>
                              current.media.kind !== "label"
                                ? current
                                : {
                                    ...current,
                                    media: {
                                      ...current.media,
                                      height_mm: height,
                                    },
                                  },
                            );
                        }}
                      />
                    </label>
                  ) : null}
                  <label className="piqae-field">
                    <span>Find print target</span>
                    <input
                      type="search"
                      value={targetSearch}
                      placeholder="Printer, target, or stock"
                      onChange={(event) =>
                        setTargetSearch(event.currentTarget.value)
                      }
                    />
                  </label>
                  <label className="piqae-field">
                    <span>Print target</span>
                    <select
                      name="designTargetId"
                      value={designTargetId}
                      onChange={(event) =>
                        setDesignTargetId(event.currentTarget.value)
                      }
                    >
                      <option value="">Choose at print time</option>
                      {compatibleTargets.map((target) => (
                        <option key={target.id} value={target.id}>
                          {target.name} ·{" "}
                          {target.stock?.name ?? "stock not configured"}
                        </option>
                      ))}
                      {selectedTarget &&
                      !compatibleTargets.some(
                        ({ id }) => id === selectedTarget.id,
                      ) ? (
                        <option value={selectedTarget.id} disabled>
                          {selectedTarget.name} · incompatible with current
                          media
                        </option>
                      ) : null}
                    </select>
                  </label>
                  {printTargetError ? (
                    <p className="piqae-menu-note">{printTargetError}</p>
                  ) : null}
                  {selectedTarget ? (
                    <TargetStatus
                      target={selectedTarget}
                      document={document}
                      savedSpecificationRevision={
                        template?.designSpecificationRevision ?? null
                      }
                    />
                  ) : null}
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
              <PrintPacketPreview
                value={document}
                stock={selectedTarget?.stock}
              />
            ) : workspace === "visual" ? (
              <PrintPacketEditor
                value={document}
                disabled={false}
                customFields={customFields}
                stock={selectedTarget?.stock}
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
          <input
            type="hidden"
            name="expectedDraftRevision"
            value={template?.draftRevision ?? ""}
          />
          {workspace === "liquid" ? null : (
            <input type="hidden" name="liquid" value={liquid} />
          )}
        </Form>
      </s-section>
    </s-page>
  );
}

function TargetStatus({
  target,
  document,
  savedSpecificationRevision,
}: {
  target: ShopifyPrintTarget;
  document: PrintPacket;
  savedSpecificationRevision: string | null;
}) {
  const destination = selectTargetDestination(target, document);
  const media =
    destination?.mediaCompatibility ??
    target.destinations[0]?.mediaCompatibility;
  const changed =
    savedSpecificationRevision !== null &&
    savedSpecificationRevision !== target.specificationRevision;
  return (
    <div
      className="piqae-target-status"
      data-status={media?.status ?? "not_reported"}
    >
      <strong>{destination ? "Compatible" : "Needs attention"}</strong>
      <span>{destination?.printerName ?? "No compatible printer"}</span>
      <span>{destination?.profileName ?? "No compatible pinned profile"}</span>
      <span>{target.stock?.name ?? "No target stock configured"}</span>
      <span>
        Loaded media: {media?.status.replaceAll("_", " ") ?? "not reported"}
      </span>
      {media?.profileDimensionsMm ? (
        <small>
          Pinned profile: {media.profileDimensionsMm.widthMm} ×{" "}
          {media.profileDimensionsMm.heightMm} mm
        </small>
      ) : null}
      {(media?.reasons ?? ["loaded_media_not_reported"]).map((reason) => (
        <small key={reason}>{reason.replaceAll("_", " ")}</small>
      ))}
      {changed ? (
        <small>
          Target configuration changed since this template was saved. Saving
          revalidates and pins the current revision.
        </small>
      ) : null}
      {media?.observedAt ? (
        <small>Observed {new Date(media.observedAt).toLocaleString()}</small>
      ) : (
        <small>No loaded-media observation reported</small>
      )}
    </div>
  );
}

export function pageSizeForDocument(document: PrintPacket): string {
  if (document.media.kind === "paged")
    return document.media.size === "a4"
      ? "A4"
      : document.media.size === "a5"
        ? "A5"
        : "Letter";
  if (document.media.kind === "continuous")
    return `${mediaDimension(document.media.width_mm)}mm roll`;
  return `${mediaDimension(document.media.width_mm)}x${mediaDimension(document.media.height_mm)}mm label`;
}

export function mediaPresetForDocument(document: PrintPacket): string {
  if (document.media.kind === "paged") return pageSizeForDocument(document);
  if (document.media.kind === "continuous")
    return Math.abs(document.media.width_mm - 80) <= 0.05
      ? "80mm"
      : "custom-continuous";
  return Math.abs(document.media.width_mm - 100) <= 0.05 &&
    Math.abs(document.media.height_mm - 50) <= 0.05
    ? "100x50mm"
    : "custom-label";
}

export function mediaForPageSize(value: string): PrintPacket["media"] {
  const margins = {
    top_mm: 10,
    right_mm: 10,
    bottom_mm: 10,
    left_mm: 10,
  };
  if (value === "A4" || value === "A5" || value === "Letter")
    return {
      kind: "paged",
      size: value === "A4" ? "a4" : value === "A5" ? "a5" : "letter",
      margins,
    };
  if (value === "80mm")
    return {
      kind: "continuous",
      width_mm: 80,
      margins: { ...margins, right_mm: 4, left_mm: 4 },
    };
  if (value === "custom-continuous")
    return {
      kind: "continuous",
      width_mm: 76,
      margins: { ...margins, right_mm: 4, left_mm: 4 },
    };
  if (value === "100x50mm")
    return {
      kind: "label",
      width_mm: 100,
      height_mm: 50,
      margins: { top_mm: 3, right_mm: 3, bottom_mm: 3, left_mm: 3 },
    };
  if (value === "custom-label")
    return {
      kind: "label",
      width_mm: 62,
      height_mm: 29,
      margins: { top_mm: 2, right_mm: 2, bottom_mm: 2, left_mm: 2 },
    };
  throw new Error("Template media is invalid");
}

function mediaDimension(value: number): string {
  if (!Number.isFinite(value) || value < 5 || value > 1000)
    throw new Error("Document media dimensions must be between 5 and 1000 mm");
  return String(Math.round(value * 100) / 100);
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
