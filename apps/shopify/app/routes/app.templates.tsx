import type { ActionFunctionArgs, LoaderFunctionArgs } from "react-router";
import { Form, redirect, useActionData, useLoaderData } from "react-router";
import { useMemo, useState } from "react";
import shopify from "../shopify.server";
import {
  bounded,
  newWorkflowId,
  workflows,
  type MerchantTemplate,
  type SaveMerchantTemplate,
} from "../core/workflows.server";
import {
  seedStarterTemplates,
  isActiveTemplate,
  syncTemplateIndex,
} from "../core/template-index.server";
import {
  parseTemplateEnvelope,
  removeSystemOwnership,
  serializeTemplateEnvelope,
} from "../core/template-model";
export const templates = [
  ["Invoice", "Orders · A4", "Published"],
  ["Packing slip", "Fulfillment · A4", "Published"],
] as const;
export function customizedSystemDraft(
  existing: MerchantTemplate,
  id: string,
): SaveMerchantTemplate {
  const envelope = parseTemplateEnvelope(existing.source);
  if (!envelope.system?.immutable)
    throw new Error("Only system documents can be customized this way");
  removeSystemOwnership(envelope);
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
  return {
    templates: (await workflows().listTemplates(session.shop)).filter(
      isActiveTemplate,
    ),
  };
}
export async function action({ request }: ActionFunctionArgs) {
  const { session, admin } = await shopify.authenticate.admin(request);
  const form = await request.formData();
  try {
    if (form.get("intent") !== "customize")
      throw new Error("Unsupported template action");
    const templateId = bounded(form, "templateId", 200, true);
    const existing = await workflows().getTemplate(session.shop, templateId);
    if (!existing) throw new Error("System document was not found");
    const saved = await workflows().saveTemplate(
      session.shop,
      customizedSystemDraft(existing, newWorkflowId()),
    );
    await syncTemplateIndex(admin, workflows(), session.shop);
    return redirect(`/app/templates/${encodeURIComponent(saved.id)}`, 303);
  } catch (error) {
    return Response.json(
      {
        ok: false,
        error:
          error instanceof Error ? error.message : "Template action failed",
      },
      { status: 400 },
    );
  }
}
export default function Templates() {
  const data = useLoaderData<typeof loader>();
  const result = useActionData<{ error: string }>();
  const [query, setQuery] = useState("");
  const [scope, setScope] = useState<"all" | "starters" | "custom">("all");
  const visibleTemplates = useMemo(() => {
    const normalized = query.trim().toLowerCase();
    const scoped = data.templates.filter((template) => {
      const starter = template.source.includes('"immutable":true');
      return scope === "all" || (scope === "starters" ? starter : !starter);
    });
    if (!normalized) return scoped;
    return scoped.filter((template) =>
      [template.name, template.kind, template.pageSize, template.state].some(
        (value) => value.toLowerCase().includes(normalized),
      ),
    );
  }, [data.templates, query, scope]);
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
        {result?.error ? (
          <s-banner tone="critical">{result.error}</s-banner>
        ) : null}
        {visibleTemplates.length ? (
          <s-table>
            <div slot="filters" className="piqae-template-filters">
              <s-search-field
                label="Search templates"
                labelAccessibilityVisibility="exclusive"
                placeholder="Search templates"
                value={query}
                onInput={(event) => setQuery(event.currentTarget.value)}
              />
              <select
                className="piqae-input"
                aria-label="Filter templates"
                value={scope}
                onChange={(event) =>
                  setScope(event.currentTarget.value as typeof scope)
                }
              >
                <option value="all">All templates</option>
                <option value="starters">Piqae starters</option>
                <option value="custom">Your templates</option>
              </select>
            </div>
            <s-table-header-row>
              <s-table-header listSlot="primary">Document</s-table-header>
              <s-table-header listSlot="inline">Type</s-table-header>
              <s-table-header listSlot="secondary">Status</s-table-header>
              <s-table-header listSlot="labeled">Actions</s-table-header>
            </s-table-header-row>
            <s-table-body>
              {visibleTemplates.map((template) => {
                const immutable = template.source.includes('"immutable":true');
                return (
                  <s-table-row key={template.id}>
                    <s-table-cell>
                      <div className="piqae-template-title-cell">
                        <div
                          className={`piqae-template-thumbnail piqae-template-${template.kind}`}
                          aria-hidden="true"
                        >
                          <span>
                            {template.kind === "packing_slip"
                              ? "PACKING SLIP"
                              : template.kind
                                  .replaceAll("_", " ")
                                  .toUpperCase()}
                          </span>
                          <i />
                          <i />
                          <i />
                        </div>
                        <div>
                          <s-link href={`/app/templates/${template.id}`}>
                            <strong>{template.name}</strong>
                          </s-link>
                          <p className="piqae-muted">
                            {immutable
                              ? "Ready-to-use Piqae starter"
                              : `Updated ${new Date(template.updatedAt).toLocaleDateString()}`}
                          </p>
                        </div>
                      </div>
                    </s-table-cell>
                    <s-table-cell>
                      {template.kind.replaceAll("_", " ")} · {template.pageSize}
                    </s-table-cell>
                    <s-table-cell>
                      <s-badge
                        tone={
                          template.state === "published" ? "success" : "info"
                        }
                      >
                        {template.state}
                      </s-badge>
                    </s-table-cell>
                    <s-table-cell>
                      <s-stack direction="inline" gap="small">
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
                            <s-button type="submit" variant="secondary">
                              Edit a copy
                            </s-button>
                          </Form>
                        ) : (
                          <>
                            <s-link href={`/app/templates/${template.id}`}>
                              Open editor
                            </s-link>
                            <s-link
                              href={`/app/templates/new?from=${encodeURIComponent(template.id)}`}
                            >
                              Duplicate
                            </s-link>
                          </>
                        )}
                        <s-link
                          download={`${template.name}.piqae-template.json`}
                          href={`data:application/json;charset=utf-8,${encodeURIComponent(JSON.stringify(template))}`}
                        >
                          Export
                        </s-link>
                      </s-stack>
                    </s-table-cell>
                  </s-table-row>
                );
              })}
            </s-table-body>
          </s-table>
        ) : (
          <s-grid gap="base" justifyItems="center" paddingBlock="large-400">
            <s-grid justifyItems="center" maxInlineSize="450px" gap="base">
              <s-stack alignItems="center">
                <s-heading>
                  {data.templates.length
                    ? "No templates match your search"
                    : "Create your first template"}
                </s-heading>
                <s-paragraph>
                  {data.templates.length
                    ? "Try a different name, type, page size, or status."
                    : "Create a document template to get started."}
                </s-paragraph>
              </s-stack>
              <s-button-group>
                <s-button href="/app/templates/new" variant="primary">
                  Create template
                </s-button>
              </s-button-group>
            </s-grid>
          </s-grid>
        )}
      </s-section>
    </s-page>
  );
}
