import type { ActionFunctionArgs, LoaderFunctionArgs } from "react-router";
import { Form, useActionData, useLoaderData } from "react-router";
import shopify from "../shopify.server";
import {
  bounded,
  newWorkflowId,
  validateDocumentSource,
  workflows,
  type MerchantTemplate,
} from "../core/workflows.server";
import { TemplatePreview, editorDocument } from "../components/shopify-ui";
export type EditorMode = "visual" | "liquid";
export function liquidCompatibilityNotice(mode: EditorMode) {
  return mode === "liquid"
    ? "Advanced Liquid is compatibility-gated. Unsupported tags or filters must be resolved before publishing."
    : null;
}
export async function loader({ request, params }: LoaderFunctionArgs) {
  const { session } = await shopify.authenticate.admin(request);
  const id = params.templateId;
  if (!id || id === "new") return { template: null };
  return { template: await workflows().getTemplate(session.shop, id) };
}
export async function action({ request, params }: ActionFunctionArgs) {
  const { session } = await shopify.authenticate.admin(request);
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
    const source = validateDocumentSource(bounded(form, "source", 65536, true));
    const saved = await workflows().saveTemplate(session.shop, {
      id: existing?.id ?? newWorkflowId(),
      name: bounded(form, "name", 200, true),
      kind: kind as MerchantTemplate["kind"],
      pageSize,
      state: intent === "publish" ? "published" : "draft",
      source,
      revision: existing?.revision ?? 1,
    });
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
  const source = template?.source ?? JSON.stringify(editorDocument, null, 2);
  return (
    <s-page heading={template?.name ?? "New template"}>
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
              Publishing pins an immutable document revision for queued jobs.
              Unsupported Liquid and arbitrary HTML are not executed.
            </s-banner>
            <div className="piqae-split">
              <div className="piqae-card">
                <s-stack direction="block" gap="base">
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
                    Piqae document JSON
                    <textarea
                      className="piqae-code"
                      name="source"
                      required
                      maxLength={65536}
                      defaultValue={source}
                    />
                  </label>
                  <div className="piqae-actions">
                    <button
                      className="piqae-link-button"
                      type="submit"
                      name="intent"
                      value="draft"
                    >
                      Save draft
                    </button>
                    <button
                      className="piqae-link-button"
                      type="submit"
                      name="intent"
                      value="publish"
                    >
                      Publish revision
                    </button>
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
              <TemplatePreview />
            </div>
          </s-stack>
        </Form>
      </s-section>
    </s-page>
  );
}
