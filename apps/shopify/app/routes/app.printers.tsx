import type { ActionFunctionArgs, LoaderFunctionArgs } from "react-router";
import {
  Form,
  useActionData,
  useFetcher,
  useLoaderData,
  useRevalidator,
} from "react-router";
import { useEffect, useMemo, useState } from "react";
import shopify from "../shopify.server";
import { parseSettings, workflows } from "../core/workflows.server";
import { syncTemplateIndex } from "../core/template-index.server";
import { createProductionServices } from "../services.server";

export async function loader({ request }: LoaderFunctionArgs) {
  const { session } = await shopify.authenticate.admin(request);
  const services = createProductionServices();
  try {
    const link = await services.managedAccounts.ensure(session.shop);
    const client = services.managedAccounts.client(link);
    const [nodes, printers] = await Promise.all([
      client.nodes.list(),
      client.printers.list(),
    ]);
    return {
      settings: await workflows().getSettings(session.shop),
      connected: true,
      runtime: services.runtime.mode,
      nodes,
      printers: printers.data,
      setupError: "",
    };
  } catch {
    return {
      settings: await workflows().getSettings(session.shop),
      connected: false,
      runtime: services.runtime.mode,
      nodes: [],
      printers: [],
      setupError:
        "Your managed printing workspace is still being prepared. Retry shortly.",
    };
  }
}
export async function action({ request }: ActionFunctionArgs) {
  const { session, admin } = await shopify.authenticate.admin(request);
  try {
    const form = await request.formData();
    if (form.get("intent") === "connect-node") {
      const services = createProductionServices();
      const link = await services.managedAccounts.ensure(session.shop);
      const connection = await services.managedAccounts
        .client(link)
        .connectSessions.create({
          name: `${session.shop} · Piqae Order Printing`,
          return_url: `${process.env.SHOPIFY_APP_URL}/connect/complete?shop=${encodeURIComponent(session.shop)}`,
          expires_in_seconds: 600,
        });
      return { ok: true, error: "", connection };
    }
    if (form.get("intent") === "set-default-printer") {
      const printerId = String(form.get("printerId") ?? "").trim();
      if (!printerId)
        throw new Error("Choose a printer before making it the default.");
      const current = await workflows().getSettings(session.shop);
      await workflows().saveSettings(session.shop, {
        ...current,
        defaultPrinterId: printerId,
      });
      await syncTemplateIndex(admin, workflows(), session.shop);
      return { ok: true, error: "", connection: null };
    }
    const settings = parseSettings(form);
    await workflows().saveSettings(session.shop, settings);
    await syncTemplateIndex(admin, workflows(), session.shop);
    return { ok: true, error: "", connection: null };
  } catch (error) {
    return Response.json(
      {
        ok: false,
        error:
          error instanceof Error
            ? error.message
            : "Settings could not be saved",
      },
      { status: 400 },
    );
  }
}
type SearchNode = { id: string; name: string; platform: string };
type SearchPrinter = {
  id: string;
  name: string;
  agent_id: string;
  state: string;
};

export function printerAvailability(state: string) {
  if (state === "online" || state === "busy") return "available";
  if (state === "offline") return "offline";
  return "attention";
}

export function filterPrinterInventory<T extends SearchPrinter>(
  printers: T[],
  nodes: Map<string, SearchNode>,
  query: string,
  status: string,
) {
  const needle = query.trim().toLowerCase();
  return printers.filter((printer) => {
    const node = nodes.get(printer.agent_id);
    return (
      (status === "all" || printerAvailability(printer.state) === status) &&
      (!needle ||
        [printer.name, printer.id, node?.name, node?.platform]
          .filter(Boolean)
          .some((value) => String(value).toLowerCase().includes(needle)))
    );
  });
}

export default function Printers() {
  const { settings, connected, runtime, nodes, printers, setupError } =
    useLoaderData<typeof loader>();
  const result = useActionData<typeof action>();
  const connector = useFetcher<typeof action>();
  const revalidator = useRevalidator();
  const [showInstaller, setShowInstaller] = useState(false);
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [query, setQuery] = useState("");
  const [status, setStatus] = useState("all");
  const connection = connector.data?.connection;
  const [detectedPlatform, setDetectedPlatform] = useState<
    "macos" | "windows" | "linux"
  >("macos");
  useEffect(() => {
    const value = navigator.userAgent.toLowerCase();
    setDetectedPlatform(
      value.includes("win")
        ? "windows"
        : value.includes("linux")
          ? "linux"
          : "macos",
    );
  }, []);
  useEffect(() => {
    const connected = (event: MessageEvent) => {
      if (
        event.origin === window.location.origin &&
        event.data?.type === "piqae:node-connected"
      )
        revalidator.revalidate();
    };
    window.addEventListener("message", connected);
    return () => window.removeEventListener("message", connected);
  }, [revalidator]);
  const installer = connection?.downloads.find(
    (download) => download.platform === detectedPlatform,
  );

  const hasNodes = nodes.length > 0;
  const hasPrinters = printers.length > 0;
  const nodeById = useMemo(
    () => new Map(nodes.map((node) => [node.id, node])),
    [nodes],
  );
  const visiblePrinters = useMemo(() => {
    return filterPrinterInventory(printers, nodeById, query, status);
  }, [nodeById, printers, query, status]);
  const formatSeen = (value: string) => {
    const date = new Date(value);
    return Number.isNaN(date.valueOf())
      ? "Unknown"
      : new Intl.DateTimeFormat(undefined, {
          dateStyle: "medium",
          timeStyle: "short",
        }).format(date);
  };
  const toneFor = (value: string) =>
    value === "online" || value === "connected" || value === "busy"
      ? "success"
      : value === "offline" || value === "disconnected"
        ? "neutral"
        : "warning";
  return (
    <s-page heading="Printers">
      <s-button
        slot="primary-action"
        variant="primary"
        onClick={() => revalidator.revalidate()}
        disabled={revalidator.state !== "idle"}
      >
        {revalidator.state === "idle" ? "Refresh" : "Refreshing…"}
      </s-button>
      <s-section
        heading={hasNodes ? "Connected printers" : "Connect your first printer"}
      >
        <s-stack direction="block" gap="base">
          {!connected ? <s-banner tone="warning">{setupError}</s-banner> : null}
          {hasNodes ? (
            <>
              <s-paragraph>
                {nodes.length} connected computer{nodes.length === 1 ? "" : "s"}{" "}
                · {printers.length} printer{printers.length === 1 ? "" : "s"}{" "}
                reported live for this store.
              </s-paragraph>
              {!hasPrinters ? (
                <s-banner tone="warning">
                  Your computer is connected, but no printer inventory has
                  reached this store yet. Confirm the printer is installed in
                  macOS or Windows, keep Piqae open, then refresh.
                </s-banner>
              ) : null}
            </>
          ) : (
            <s-paragraph>
              Install or open Piqae on the computer connected to your printer.
              This store manages the secure connection automatically—no Piqae
              account or API key is needed.
            </s-paragraph>
          )}
          {!hasNodes ? (
            <connector.Form method="post">
              <input type="hidden" name="intent" value="connect-node" />
              <s-button
                type="submit"
                variant="primary"
                disabled={!connected || connector.state !== "idle"}
              >
                {connector.state === "idle"
                  ? "Connect this computer"
                  : "Preparing connection…"}
              </s-button>
            </connector.Form>
          ) : null}
          {connection ? (
            <s-stack direction="block" gap="base">
              {connection.connect_url ? (
                <s-button
                  href={connection.connect_url}
                  target="_blank"
                  onClick={() => {
                    setShowInstaller(false);
                    window.setTimeout(() => setShowInstaller(true), 1800);
                  }}
                >
                  Open Piqae connection
                </s-button>
              ) : null}
              {showInstaller && installer ? (
                <s-button href={installer.url} target="_top">
                  Download Piqae for {installer.platform}
                </s-button>
              ) : null}
              <s-paragraph>
                The secure connection expires in 10 minutes and can be used
                once.
              </s-paragraph>
            </s-stack>
          ) : null}
        </s-stack>
      </s-section>

      {hasPrinters ? (
        <s-section
          padding="none"
          accessibilityLabel="Printers available to this store"
        >
          <s-table>
            <s-stack slot="filters" direction="inline" gap="base">
              <s-search-field
                label="Search printers"
                placeholder="Search printers or computers"
                value={query}
                onInput={(event) => setQuery(event.currentTarget.value)}
              />
              <s-select
                label="Status"
                value={status}
                onChange={(event) => setStatus(event.currentTarget.value)}
              >
                <option value="all">All statuses</option>
                <option value="available">Available</option>
                <option value="offline">Offline</option>
                <option value="attention">Needs attention</option>
              </s-select>
            </s-stack>
            <s-table-header-row>
              <s-table-header listSlot="primary">Printer</s-table-header>
              <s-table-header listSlot="secondary">Computer</s-table-header>
              <s-table-header listSlot="inline">Status</s-table-header>
              <s-table-header listSlot="labeled">Connection</s-table-header>
              <s-table-header listSlot="labeled">Default</s-table-header>
              <s-table-header listSlot="labeled">Last seen</s-table-header>
            </s-table-header-row>
            <s-table-body>
              {visiblePrinters.map((printer) => {
                const node = nodeById.get(printer.agent_id);
                return (
                  <s-table-row key={printer.id}>
                    <s-table-cell>
                      <strong>{printer.name}</strong>
                      <div className="piqae-muted piqae-resource-id">
                        {printer.id}
                      </div>
                    </s-table-cell>
                    <s-table-cell>
                      {node?.name ?? "Unknown computer"}
                      <div className="piqae-muted">
                        {node?.platform ?? "Platform unavailable"}
                      </div>
                    </s-table-cell>
                    <s-table-cell>
                      <s-badge tone={toneFor(printer.state)}>
                        {printerAvailability(printer.state) === "available"
                          ? "Available"
                          : printerAvailability(printer.state) === "offline"
                            ? "Offline"
                            : "Needs attention"}
                      </s-badge>
                    </s-table-cell>
                    <s-table-cell>
                      <span>This Shopify store</span>
                      <div className="piqae-muted">Managed child workspace</div>
                    </s-table-cell>
                    <s-table-cell>
                      {settings.defaultPrinterId === printer.id ? (
                        <s-badge tone="info">Default</s-badge>
                      ) : (
                        <Form method="post">
                          <input
                            type="hidden"
                            name="intent"
                            value="set-default-printer"
                          />
                          <input
                            type="hidden"
                            name="printerId"
                            value={printer.id}
                          />
                          <s-button type="submit" variant="tertiary">
                            Make default
                          </s-button>
                        </Form>
                      )}
                    </s-table-cell>
                    <s-table-cell>
                      {formatSeen(printer.updated_at)}
                    </s-table-cell>
                  </s-table-row>
                );
              })}
            </s-table-body>
          </s-table>
          {visiblePrinters.length === 0 ? (
            <div className="piqae-printer-empty">
              <s-heading>No printers match these filters</s-heading>
              <s-paragraph>
                Clear the search or choose a different status.
              </s-paragraph>
              <s-button
                onClick={() => {
                  setQuery("");
                  setStatus("all");
                }}
              >
                Clear filters
              </s-button>
            </div>
          ) : null}
        </s-section>
      ) : null}

      {hasNodes ? (
        <s-section heading="Connected computers">
          <s-stack direction="block" gap="base">
            <div className="piqae-computer-grid">
              {nodes.map((node) => (
                <div className="piqae-computer-card" key={node.id}>
                  <div>
                    <strong>{node.name}</strong>
                    <s-badge tone={toneFor(node.state)}>{node.state}</s-badge>
                  </div>
                  <s-paragraph>{node.platform}</s-paragraph>
                  <small>
                    This Shopify store · Managed child workspace · Last seen{" "}
                    {formatSeen(node.last_seen_at)}
                  </small>
                </div>
              ))}
            </div>
            <connector.Form method="post">
              <input type="hidden" name="intent" value="connect-node" />
              <s-button type="submit" disabled={connector.state !== "idle"}>
                {connector.state === "idle"
                  ? "Connect another computer"
                  : "Preparing connection…"}
              </s-button>
            </connector.Form>
          </s-stack>
        </s-section>
      ) : null}

      <s-section heading="Printing preferences">
        <Form method="post">
          <s-stack direction="block" gap="base">
            {result?.ok ? (
              <s-banner tone="success">Settings saved.</s-banner>
            ) : result?.error ? (
              <s-banner tone="critical">{result.error}</s-banner>
            ) : null}
            {hasPrinters ? (
              <>
                <s-select
                  label="Default printer"
                  name="defaultPrinterId"
                  value={settings.defaultPrinterId}
                >
                  <option value="">Ask each time</option>
                  {settings.defaultPrinterId &&
                  !printers.some(
                    (printer) => printer.id === settings.defaultPrinterId,
                  ) ? (
                    <option value={settings.defaultPrinterId} disabled>
                      Previously selected printer (unavailable)
                    </option>
                  ) : null}
                  {printers.map((printer) => (
                    <option key={printer.id} value={printer.id}>
                      {printer.name}
                      {printer.state === "online" ? "" : ` (${printer.state})`}
                    </option>
                  ))}
                </s-select>
                <label className="piqae-check">
                  <input
                    type="checkbox"
                    name="preferDirect"
                    defaultChecked={settings.preferDirect}
                  />{" "}
                  Prefer direct printing when a node is ready
                </label>
              </>
            ) : (
              <input type="hidden" name="defaultPrinterId" value="" />
            )}
            <input
              type="hidden"
              name="defaultTemplateId"
              value={settings.defaultTemplateId}
            />
            {!hasPrinters && settings.preferDirect ? (
              <input type="hidden" name="preferDirect" value="on" />
            ) : null}
            <s-button
              type="button"
              onClick={() => setShowAdvanced((value) => !value)}
            >
              {showAdvanced
                ? "Hide PDF and advanced settings"
                : "PDF and advanced settings"}
            </s-button>
            <div hidden={!showAdvanced} className="piqae-advanced-disclosure">
              <s-stack direction="block" gap="base">
                <label className="piqae-check">
                  <input
                    type="checkbox"
                    name="offerPdf"
                    defaultChecked={settings.offerPdf}
                  />{" "}
                  Keep PDF download available
                </label>
                <s-select
                  label="Where documents are prepared"
                  name="renderExecutionPolicy"
                  value={settings.renderExecutionPolicy}
                >
                  <option value="automatic">Automatic (recommended)</option>
                  <option value="cloud_only">Piqae Cloud</option>
                  <option value="prefer_node">
                    This computer when compatible
                  </option>
                  <option value="require_node">Only this computer</option>
                </s-select>
                <s-paragraph>
                  Automatic uses the fastest compatible option and always falls
                  back to the exact preview PDF when the connected app is older.
                </s-paragraph>
                <label>
                  Keep completed documents for (days)
                  <input
                    className="piqae-input"
                    type="number"
                    name="retentionDays"
                    min={1}
                    max={365}
                    defaultValue={settings.retentionDays}
                  />
                </label>
                <label>
                  Template metafields
                  <textarea
                    className="piqae-code piqae-code-short"
                    name="metafields"
                    defaultValue={settings.metafieldAllowlist.join("\n")}
                  />
                </label>
                <s-paragraph>
                  Optional. Add one field per line as namespace.key for an
                  order, or product:namespace.key / variant:namespace.key. Add a
                  final .field to expose an allowlisted referenced metaobject
                  field.
                </s-paragraph>
              </s-stack>
            </div>
            <s-button type="submit" variant="primary">
              Save settings
            </s-button>
          </s-stack>
        </Form>
      </s-section>
    </s-page>
  );
}
