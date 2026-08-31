import type { ActionFunctionArgs, LoaderFunctionArgs } from "react-router";
import {
  Form,
  useActionData,
  useFetcher,
  useLoaderData,
  useRevalidator,
} from "react-router";
import { useEffect, useMemo, useRef, useState } from "react";
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

type ConnectionStage = "idle" | "preparing" | "opened" | "blocked" | "failed";

export function preparePiqaeConnectionWindow(
  openWindow: typeof window.open = window.open.bind(window),
) {
  const connectionWindow = openWindow(
    "",
    "piqae-node-connection",
    "popup,width=560,height=720",
  );
  if (!connectionWindow) return null;
  try {
    connectionWindow.opener = null;
    connectionWindow.document.title = "Opening Piqae…";
    const status = connectionWindow.document.createElement("p");
    status.textContent = "Preparing your secure Piqae connection…";
    status.style.cssText =
      "margin:25vh auto;max-width:24rem;padding:2rem;color:#202223;font:600 18px system-ui;text-align:center";
    connectionWindow.document.body.replaceChildren(status);
  } catch {
    // The reserved window may already be navigating. The handoff can still use it.
  }
  return connectionWindow;
}

export function openPreparedPiqaeConnection(
  connectionWindow: Window | null,
  connectUrl: string,
) {
  if (!connectionWindow || connectionWindow.closed) return false;
  try {
    const url = new URL(connectUrl);
    if (url.protocol !== "https:" || url.hostname !== "app.piqae.com")
      return false;
    connectionWindow.location.replace(url.toString());
    return true;
  } catch {
    return false;
  }
}

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
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [connectionStage, setConnectionStage] =
    useState<ConnectionStage>("idle");
  const connectionWindow = useRef<Window | null>(null);
  const openedConnectionUrl = useRef("");
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
  const installer = connection?.downloads?.find(
    (download) => download.platform === detectedPlatform,
  );
  const platformName =
    detectedPlatform === "macos"
      ? "macOS"
      : detectedPlatform === "windows"
        ? "Windows"
        : "Linux";
  const installerUrl =
    installer?.url ??
    `https://app.piqae.com/downloads?platform=${detectedPlatform}`;

  const beginConnection = () => {
    openedConnectionUrl.current = "";
    connectionWindow.current?.close();
    connectionWindow.current = preparePiqaeConnectionWindow();
    setConnectionStage("preparing");
  };

  useEffect(() => {
    const connectUrl = connection?.connect_url;
    if (!connectUrl || openedConnectionUrl.current === connectUrl) return;
    openedConnectionUrl.current = connectUrl;
    const opened = openPreparedPiqaeConnection(
      connectionWindow.current,
      connectUrl,
    );
    connectionWindow.current = null;
    setConnectionStage(opened ? "opened" : "blocked");
  }, [connection]);

  useEffect(() => {
    if (connector.state === "idle" && connector.data && !connector.data.ok) {
      connectionWindow.current?.close();
      connectionWindow.current = null;
      setConnectionStage("failed");
    }
  }, [connector.data, connector.state]);

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
            <div className="piqae-connection-intro">
              <div className="piqae-connection-mark" aria-hidden="true">
                <svg viewBox="0 0 24 24" role="presentation">
                  <path d="M7 8V3h10v5M7 18H4a2 2 0 0 1-2-2v-5a3 3 0 0 1 3-3h14a3 3 0 0 1 3 3v5a2 2 0 0 1-2 2h-3M7 14h10v7H7z" />
                  <path d="M18 11h1" />
                </svg>
              </div>
              <div>
                <s-heading>Connect the computer beside your printer</s-heading>
                <s-paragraph>
                  Piqae will open in a separate window and ask which installed
                  printers this store may use. No Piqae account or API key is
                  needed.
                </s-paragraph>
              </div>
            </div>
          )}
          {!hasNodes ? (
            <div className="piqae-connection-panel">
              <div
                className={`piqae-connection-status piqae-connection-status--${connectionStage}`}
                role="status"
                aria-live="polite"
              >
                <span className="piqae-connection-status-dot" />
                <div>
                  <strong>
                    {connectionStage === "preparing"
                      ? "Preparing a secure connection…"
                      : connectionStage === "opened"
                        ? "Piqae opened"
                        : connectionStage === "blocked"
                          ? "Connection window was blocked"
                          : connectionStage === "failed"
                            ? "Connection could not be prepared"
                            : "Ready to connect"}
                  </strong>
                  <span>
                    {connectionStage === "opened"
                      ? "Choose the printers to share, then return here."
                      : connectionStage === "blocked"
                        ? "Open the secure invitation below to continue."
                        : connectionStage === "failed"
                          ? connector.data?.error || "Try the connection again."
                          : "A private, one-time invitation will open in a new window."}
                  </span>
                </div>
              </div>
              <connector.Form method="post" onSubmit={beginConnection}>
                <input type="hidden" name="intent" value="connect-node" />
                <s-button
                  type="submit"
                  variant="primary"
                  disabled={!connected || connector.state !== "idle"}
                >
                  {connector.state === "idle"
                    ? connectionStage === "failed"
                      ? "Try connecting again"
                      : "Connect this computer"
                    : "Opening Piqae…"}
                </s-button>
              </connector.Form>
              {connectionStage === "blocked" && connection?.connect_url ? (
                <a
                  className="piqae-connection-retry"
                  href={connection.connect_url}
                  target="_blank"
                  rel="noreferrer"
                  onClick={() => setConnectionStage("opened")}
                >
                  Open Piqae connection
                </a>
              ) : null}
              <div className="piqae-connection-download">
                <span>Don’t have Piqae installed?</span>
                <a href={installerUrl} target="_top">
                  Download Piqae for {platformName}
                </a>
              </div>
              <div
                className="piqae-connection-flow"
                aria-label="Connection steps"
              >
                <span>
                  <b>1</b> Open Piqae
                </span>
                <span>
                  <b>2</b> Choose printers
                </span>
                <span>
                  <b>3</b> Connected
                </span>
              </div>
              <small className="piqae-connection-security">
                Secure invitations can be used once and expire after 10 minutes.
              </small>
            </div>
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
            <connector.Form method="post" onSubmit={beginConnection}>
              <input type="hidden" name="intent" value="connect-node" />
              <s-button type="submit" disabled={connector.state !== "idle"}>
                {connector.state === "idle"
                  ? "Connect another computer"
                  : "Preparing connection…"}
              </s-button>
            </connector.Form>
            {connectionStage === "blocked" && connection?.connect_url ? (
              <s-paragraph>
                Your browser blocked the connection window.{" "}
                <a
                  href={connection.connect_url}
                  target="_blank"
                  rel="noreferrer"
                  onClick={() => setConnectionStage("opened")}
                >
                  Open the secure invitation
                </a>
                .
              </s-paragraph>
            ) : connectionStage === "preparing" ? (
              <s-paragraph>Preparing the secure connection…</s-paragraph>
            ) : connectionStage === "opened" ? (
              <s-paragraph>
                Piqae opened. Finish choosing printers in that window.
              </s-paragraph>
            ) : null}
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
