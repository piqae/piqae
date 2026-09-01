import type { ActionFunctionArgs, LoaderFunctionArgs } from "react-router";
import {
  Form,
  redirect,
  useActionData,
  useFetcher,
  useLoaderData,
  useRevalidator,
} from "react-router";
import { useEffect, useMemo, useRef, useState } from "react";
import {
  openPreparedPiqaeConnection,
  preparePiqaeConnectionWindow,
} from "../components/node-connection-ui";
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
import { createProductionServices } from "../services.server";
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
  const services = createProductionServices();
  const [templates, nodeState, settings] = await Promise.all([
    seedStarterTemplates(workflows(), session.shop).then(() =>
      workflows().listTemplates(session.shop),
    ),
    services.managedAccounts
      .ensure(session.shop)
      .then((link) => services.managedAccounts.client(link).nodes.list())
      .then((nodes) => ({ hasNodes: nodes.length > 0, nodeError: "" }))
      .catch(() => ({
        hasNodes: false,
        nodeError: "Your Piqae workspace is still being prepared.",
      })),
    workflows().getSettings(session.shop),
  ]);
  return {
    templates: templates.filter(isActiveTemplate),
    defaultTemplateId: settings.defaultTemplateId,
    ...nodeState,
  };
}
export async function action({ request }: ActionFunctionArgs) {
  const { session, admin } = await shopify.authenticate.admin(request);
  const form = await request.formData();
  try {
    if (form.get("intent") === "connect-node") {
      const services = createProductionServices();
      const link = await services.managedAccounts.ensure(session.shop);
      const connection = await services.managedAccounts
        .client(link)
        .connectSessions.create({
          name: `${session.shop} · Piqae Order Printing`,
          return_url: `${process.env.SHOPIFY_APP_URL}/connect/complete`,
          expires_in_seconds: 600,
        });
      return { ok: true, error: "", connection };
    }
    if (form.get("intent") === "set-quick-print") {
      const templateId = bounded(form, "templateId", 200, true);
      const template = await workflows().getTemplate(session.shop, templateId);
      if (!template?.published || template.state !== "published")
        throw new Error(
          "Publish this document before using it for quick print.",
        );
      await workflows().updateSettings(session.shop, {
        defaultTemplateId: templateId,
      });
      let warning = "";
      try {
        await syncTemplateIndex(admin, workflows(), session.shop);
      } catch {
        warning =
          "Quick print was saved, but the Shopify shortcut may take a moment to refresh.";
      }
      return {
        ok: true,
        error: "",
        warning,
        defaultTemplateId: templateId,
      };
    }
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
  const connector = useFetcher<typeof action>();
  const quickPrint = useFetcher<typeof action>();
  const revalidator = useRevalidator();
  const connectionWindow = useRef<Window | null>(null);
  const openedConnectionUrl = useRef("");
  const connectionStarted = useRef(false);
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
  useEffect(() => {
    const connection = connector.data?.connection;
    if (!connection?.connect_url) return;
    if (openedConnectionUrl.current === connection.connect_url) return;
    openedConnectionUrl.current = connection.connect_url;
    if (
      !openPreparedPiqaeConnection(
        connectionWindow.current,
        connection.connect_url,
      )
    )
      window.open(connection.connect_url, "_blank", "noopener,noreferrer");
  }, [connector.data]);
  useEffect(() => {
    const refreshAfterConnection = () => {
      if (connectionStarted.current) revalidator.revalidate();
    };
    window.addEventListener("focus", refreshAfterConnection);
    return () => window.removeEventListener("focus", refreshAfterConnection);
  }, [revalidator]);
  return (
    <s-page heading="Templates">
      <s-button
        slot="primary-action"
        href="/app/templates/new"
        variant="primary"
      >
        Create template
      </s-button>
      {!data.hasNodes ? (
        <div className="piqae-template-node-banner">
          <span className="piqae-connection-mark" aria-hidden="true">
            <svg viewBox="0 0 24 24">
              <path d="M7 8V4h10v4M7 17H5a2 2 0 0 1-2-2v-5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2v5a2 2 0 0 1-2 2h-2M7 14h10v7H7z" />
            </svg>
          </span>
          <div>
            <strong>Connect a Piqae node</strong>
            <span>
              Add the device beside your printer to enable direct printing.
            </span>
            {connector.data?.error || data.nodeError ? (
              <small>{connector.data?.error || data.nodeError}</small>
            ) : null}
          </div>
          <connector.Form
            method="post"
            onSubmit={() => {
              connectionStarted.current = true;
              connectionWindow.current = preparePiqaeConnectionWindow();
            }}
          >
            <input type="hidden" name="intent" value="connect-node" />
            <s-button
              type="submit"
              variant="primary"
              disabled={connector.state !== "idle"}
            >
              {connector.state === "idle" ? "Connect node" : "Opening…"}
            </s-button>
          </connector.Form>
          <a
            className="piqae-download-link"
            href="https://piqae.com/downloads"
            target="_top"
          >
            Download Piqae
          </a>
        </div>
      ) : null}
      <s-section padding="none" accessibilityLabel="Templates table">
        {result?.error ? (
          <s-banner tone="critical">{result.error}</s-banner>
        ) : null}
        {quickPrint.data?.error ? (
          <s-banner tone="critical">{quickPrint.data.error}</s-banner>
        ) : null}
        {quickPrint.data?.warning ? (
          <s-banner tone="warning">{quickPrint.data.warning}</s-banner>
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
                      <s-stack direction="inline" gap="small">
                        <s-badge
                          tone={
                            template.state === "published" ? "success" : "info"
                          }
                        >
                          {template.state}
                        </s-badge>
                        {data.defaultTemplateId === template.id ? (
                          <s-badge tone="info">Quick print</s-badge>
                        ) : null}
                      </s-stack>
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
                        {template.state === "published" &&
                        data.defaultTemplateId !== template.id ? (
                          <quickPrint.Form method="post">
                            <input
                              type="hidden"
                              name="intent"
                              value="set-quick-print"
                            />
                            <input
                              type="hidden"
                              name="templateId"
                              value={template.id}
                            />
                            <s-button
                              type="submit"
                              variant="tertiary"
                              disabled={quickPrint.state !== "idle"}
                            >
                              Use for quick print
                            </s-button>
                          </quickPrint.Form>
                        ) : null}
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
