import type { ActionFunctionArgs, LoaderFunctionArgs } from "react-router";
import { Form, redirect, useActionData, useLoaderData } from "react-router";
import { useMemo, useState } from "react";
import shopify from "../shopify.server";
import {
  bounded,
  newWorkflowId,
  validateDocumentSource,
  workflows,
  type MerchantTemplate,
} from "../core/workflows.server";
import {
  seedStarterTemplates,
  syncTemplateIndex,
} from "../core/template-index.server";
import {
  parseTemplateEnvelope,
  serializeTemplateEnvelope,
} from "../core/template-model";
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
export function customizedSystemDraft(
  existing: MerchantTemplate,
  id: string,
): Omit<MerchantTemplate, "updatedAt"> {
  const envelope = parseTemplateEnvelope(existing.source);
  if (!envelope.system?.immutable)
    throw new Error("Only system documents can be customized this way");
  delete envelope.system;
  return {
    id,
    name: `${existing.name} — customized`.slice(0, 200),
    kind: existing.kind,
    pageSize: existing.pageSize,
    state: "draft",
    source: serializeTemplateEnvelope(envelope),
    revision: 1,
  };
}
export async function loader({ request }: LoaderFunctionArgs) {
  const { session } = await shopify.authenticate.admin(request);
  await seedStarterTemplates(workflows(), session.shop);
  return { templates: await workflows().listTemplates(session.shop) };
}
export async function action({ request }: ActionFunctionArgs) {
  const { session, admin } = await shopify.authenticate.admin(request);
  const form = await request.formData();
  try {
    if (form.get("intent") === "customize") {
      const templateId = bounded(form, "templateId", 200, true);
      const existing = await workflows().getTemplate(session.shop, templateId);
      if (!existing) throw new Error("System document was not found");
      const saved = await workflows().saveTemplate(
        session.shop,
        customizedSystemDraft(existing, newWorkflowId()),
      );
      await syncTemplateIndex(admin, workflows(), session.shop);
      return redirect(`/app/templates/${encodeURIComponent(saved.id)}`, 303);
    }
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
    await syncTemplateIndex(admin, workflows(), session.shop);
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
  const [query, setQuery] = useState("");
  const visibleTemplates = useMemo(() => {
    const normalized = query.trim().toLowerCase();
    if (!normalized) return data.templates;
    return data.templates.filter((template) =>
      [template.name, template.kind, template.pageSize, template.state].some(
        (value) => value.toLowerCase().includes(normalized),
      ),
    );
  }, [data.templates, query]);
  return (
    <s-page heading="Templates">
      <s-button
        slot="primary-action"
        href="/app/templates/new"
        variant="primary"
      >
        Create template
      </s-button>
      <s-section padding="none" accessibilityLabel="Templates table">
        <s-stack direction="block" gap="base">
          {result?.ok ? (
            <s-banner tone="success">Template imported as a draft.</s-banner>
          ) : result?.error ? (
            <s-banner tone="critical">{result.error}</s-banner>
          ) : null}
          <s-box padding="base">
            <s-stack direction="block" gap="small">
              <s-search-field
                label="Search templates"
                placeholder="Search by name, type, size, or status"
                value={query}
                onInput={(event) => setQuery(event.currentTarget.value)}
              />
              <s-text color="subdued">
                Published revisions are immutable. Customize a system document
                to create an editable merchant draft.
              </s-text>
            </s-stack>
          </s-box>
          {visibleTemplates.length ? (
            <s-table>
              <s-table-header-row>
                <s-table-header listSlot="primary">Document</s-table-header>
                <s-table-header>Type</s-table-header>
                <s-table-header>Page size</s-table-header>
                <s-table-header>Status</s-table-header>
                <s-table-header format="numeric">Revision</s-table-header>
                <s-table-header>Actions</s-table-header>
              </s-table-header-row>
              <s-table-body>
                {visibleTemplates.map((template) => {
                  const immutable =
                    template.source.includes('"immutable":true');
                  return (
                    <s-table-row key={template.id}>
                      <s-table-cell>
                        <s-link href={`/app/templates/${template.id}`}>
                          {template.name}
                        </s-link>
                      </s-table-cell>
                      <s-table-cell>
                        {template.kind.replaceAll("_", " ")}
                      </s-table-cell>
                      <s-table-cell>{template.pageSize}</s-table-cell>
                      <s-table-cell>
                        <s-badge
                          tone={
                            template.state === "published" ? "success" : "info"
                          }
                        >
                          {template.state}
                        </s-badge>
                      </s-table-cell>
                      <s-table-cell>{template.revision}</s-table-cell>
                      <s-table-cell>
                        <div className="piqae-actions">
                          {immutable ? (
                            <Form method="post">
                              <input
                                type="hidden"
                                name="intent"
                                value="customize"
                              />
                              <input
                                type="hidden"
                                name="templateId"
                                value={template.id}
                              />
                              <button
                                className="piqae-link-button"
                                type="submit"
                              >
                                Customize
                              </button>
                            </Form>
                          ) : (
                            <s-link href={`/app/templates/${template.id}`}>
                              Edit
                            </s-link>
                          )}
                          <a
                            download={`${template.name}.piqae-template.json`}
                            href={`data:application/json;charset=utf-8,${encodeURIComponent(JSON.stringify(template))}`}
                          >
                            Export
                          </a>
                        </div>
                      </s-table-cell>
                    </s-table-row>
                  );
                })}
              </s-table-body>
            </s-table>
          ) : (
            <s-box padding="large-400">
              <s-stack direction="block" gap="small" alignItems="center">
                <s-heading>
                  {data.templates.length
                    ? "No templates match your search"
                    : "No templates yet"}
                </s-heading>
                <s-text color="subdued">
                  {data.templates.length
                    ? "Try a different name, type, page size, or status."
                    : "Create a template or import a portable template document."}
                </s-text>
              </s-stack>
            </s-box>
          )}
        </s-stack>
      </s-section>
      <s-section heading="Import template">
        <details>
          <summary>Import portable template JSON</summary>
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
      </s-section>
    </s-page>
  );
}
