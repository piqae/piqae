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
export const templates = [
  ["Invoice", "Orders · A4", "Published"],
  ["Packing slip", "Fulfillment · A4", "Published"],
  ["Receipt", "Orders · 80 mm", "Draft"],
  ["Returns form", "Orders · A4", "Published"],
  ["Quote / pro forma", "Draft orders · A4", "Published"],
  ["Refund / credit note", "Refunds · A4", "Published"],
  ["Gift receipt", "Orders · A5", "Published"],
  ["Delivery note", "Fulfillment · A4", "Published"],
] as const;
export async function loader({ request }: LoaderFunctionArgs) {
  const { session } = await shopify.authenticate.admin(request);
  return { templates: await workflows().listTemplates(session.shop) };
}
export async function action({ request }: ActionFunctionArgs) {
  const { session } = await shopify.authenticate.admin(request);
  const form = await request.formData();
  try {
    const raw = bounded(form, "import", 65536, true);
    const parsed = JSON.parse(raw) as Partial<MerchantTemplate>;
    if (
      parsed.source === undefined ||
      typeof parsed.source !== "string" ||
      parsed.source.length > 65536
    )
      throw new Error("Imported template source is invalid");
    const source = validateDocumentSource(parsed.source);
    const saved = await workflows().saveTemplate(session.shop, {
      id: newWorkflowId(),
      name:
        typeof parsed.name === "string"
          ? parsed.name.slice(0, 200)
          : "Imported template",
      kind: "custom",
      pageSize: ["A4", "A5", "Letter", "80mm"].includes(String(parsed.pageSize))
        ? String(parsed.pageSize)
        : "A4",
      state: "draft",
      source,
      revision: 1,
    } as Omit<MerchantTemplate, "updatedAt">);
    return { ok: true, error: "", id: saved.id };
  } catch (error) {
    return Response.json(
      {
        ok: false,
        error: error instanceof Error ? error.message : "Import failed",
      },
      { status: 400 },
    );
  }
}
export default function Templates() {
  const data = useLoaderData<typeof loader>();
  const result = useActionData<typeof action>();
  return (
    <s-page heading="Templates">
      <s-button
        slot="primary-action"
        href="/app/templates/new"
        variant="primary"
      >
        Create template
      </s-button>
      <s-section>
        <s-stack direction="block" gap="base">
          {result?.ok ? (
            <s-banner tone="success">Template imported as a draft.</s-banner>
          ) : result?.error ? (
            <s-banner tone="critical">{result.error}</s-banner>
          ) : null}
          <s-paragraph>
            Published revisions are immutable for queued jobs. Editing and
            publishing creates a new revision.
          </s-paragraph>
          <div className="piqae-grid">
            {data.templates.map((t) => (
              <div className="piqae-card" key={t.id}>
                <div className="piqae-actions">
                  <s-heading>{t.name}</s-heading>
                  <s-badge tone={t.state === "published" ? "success" : "info"}>
                    {t.state}
                  </s-badge>
                </div>
                <s-paragraph>
                  {t.kind.replaceAll("_", " ")} · {t.pageSize} · revision{" "}
                  {t.revision}
                </s-paragraph>
                <div className="piqae-actions">
                  <s-button href={`/app/templates/${t.id}`}>Edit</s-button>
                  <a
                    className="piqae-link-button"
                    download={`${t.name}.piqae-template.json`}
                    href={`data:application/json;charset=utf-8,${encodeURIComponent(JSON.stringify(t))}`}
                  >
                    Export
                  </a>
                </div>
              </div>
            ))}
            {data.templates.length === 0 ? (
              <div className="piqae-card">
                <s-heading>No templates yet</s-heading>
                <s-paragraph>
                  Create one or import a portable template JSON document.
                </s-paragraph>
              </div>
            ) : null}
          </div>
          <details className="piqae-card">
            <summary>Import template JSON</summary>
            <Form method="post">
              <label>
                Portable template
                <textarea
                  className="piqae-code piqae-code-short"
                  name="import"
                  required
                  maxLength={65536}
                />
              </label>
              <s-button type="submit">Import as draft</s-button>
            </Form>
          </details>
        </s-stack>
      </s-section>
    </s-page>
  );
}
