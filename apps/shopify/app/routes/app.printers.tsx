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
          return_url: `${process.env.SHOPIFY_APP_URL}/connect/complete`,
          expires_in_seconds: 600,
        });
      return { ok: true, error: "", connection };
    }
    if (form.get("intent") === "connection-status") {
      const connectionId = String(form.get("connectionId") ?? "").trim();
      if (!connectionId.startsWith("enr_") || connectionId.length > 64)
        throw new Error("The connection status request is invalid.");
      const services = createProductionServices();
      const link = await services.managedAccounts.ensure(session.shop);
      const connection = await services.managedAccounts
        .client(link)
        .connectSessions.retrieve(connectionId);
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

type ConnectionStage =
  | "idle"
  | "preparing"
  | "opened"
  | "connected"
  | "blocked"
  | "expired"
  | "failed";

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
  const connectionStatus = useFetcher<typeof action>();
  const connectionStatusRef = useRef(connectionStatus);
  connectionStatusRef.current = connectionStatus;
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

  useEffect(() => {
    if (connectionStage !== "opened" || !connection?.id) return;
    const poll = () => {
      if (connectionStatusRef.current.state !== "idle") return;
      const form = new FormData();
      form.set("intent", "connection-status");
      form.set("connectionId", connection.id);
      void connectionStatusRef.current.submit(form, { method: "post" });
    };
    poll();
    const timer = window.setInterval(poll, 1500);
    return () => window.clearInterval(timer);
  }, [connection?.id, connectionStage]);

  useEffect(() => {
    if (connectionStatus.data && !connectionStatus.data.ok) {
      setConnectionStage("failed");
      return;
    }
    const state = connectionStatus.data?.connection?.state;
    if (state === "connected") {
      setConnectionStage("connected");
      revalidator.revalidate();
    } else if (state === "expired") {
      setConnectionStage("expired");
    }
  }, [connectionStatus.data, revalidator]);

  const hasNodes = nodes.length > 0;
  const hasPrinters = printers.length > 0;
  const availablePrinterCount = printers.filter(
    (printer) => printerAvailability(printer.state) === "available",
  ).length;
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
      {!hasNodes ? (
        <div className="piqae-connection-panel piqae-connection-panel--standalone">
          {!connected ? <s-banner tone="warning">{setupError}</s-banner> : null}
          <div className="piqae-connection-panel-heading">
            <div className="piqae-connection-mark" aria-hidden="true">
              <svg viewBox="0 0 24 24" role="presentation">
                <path d="M7 8V3h10v5M7 18H4a2 2 0 0 1-2-2v-5a3 3 0 0 1 3-3h14a3 3 0 0 1 3 3v5a2 2 0 0 1-2 2h-3M7 14h10v7H7z" />
                <path d="M18 11h1" />
              </svg>
            </div>
            <div>
              <s-heading>Connect Piqae</s-heading>
              <s-paragraph>
                Open Piqae on this computer, choose the installed printers this
                store may use, and they’ll appear here automatically.
              </s-paragraph>
            </div>
          </div>
          <div className="piqae-connection-actions">
            <connector.Form method="post" onSubmit={beginConnection}>
              <input type="hidden" name="intent" value="connect-node" />
              <s-button
                type="submit"
                variant="primary"
                disabled={!connected || connector.state !== "idle"}
              >
                {connector.state === "idle"
                  ? connectionStage === "failed" ||
                    connectionStage === "expired"
                    ? "Try connecting again"
                    : "Connect this computer"
                  : "Opening Piqae…"}
              </s-button>
            </connector.Form>
            <div
              className={`piqae-connection-status piqae-connection-status--${connectionStage}`}
              role="status"
              aria-live="polite"
            >
              <span className="piqae-connection-status-dot" />
              <div>
                <strong>
                  {connectionStage === "preparing"
                    ? "Preparing secure connection…"
                    : connectionStage === "opened"
                      ? "Waiting for printer access…"
                      : connectionStage === "connected"
                        ? "Computer connected"
                        : connectionStage === "blocked"
                          ? "Connection window blocked"
                          : connectionStage === "expired"
                            ? "Invitation expired"
                            : connectionStage === "failed"
                              ? "Connection could not be prepared"
                              : "Ready to connect"}
                </strong>
                <span>
                  {connectionStage === "opened"
                    ? "Choose printers in Piqae. This page will update when access is confirmed."
                    : connectionStage === "connected"
                      ? "Printer access was confirmed by Piqae."
                      : connectionStage === "blocked"
                        ? "Open the secure invitation below to continue."
                        : connectionStage === "expired"
                          ? "Create a new one-time invitation to try again."
                          : connectionStage === "failed"
                            ? connector.data?.error ||
                              "Try the connection again."
                            : "No Piqae account or API key is needed."}
                </span>
              </div>
            </div>
          </div>
          {connectionStage === "blocked" && connection?.connect_url ? (
            <a
              className="piqae-connection-retry"
              href={connection.connect_url}
              target="_blank"
              rel="noreferrer"
              onClick={() => setConnectionStage("opened")}
            >
              Open secure invitation
            </a>
          ) : null}
          <div className="piqae-connection-download">
            <span>Need Piqae?</span>
            <a href={installerUrl} target="_top">
              Download for {platformName}
            </a>
            <small>One-time invitations expire after 10 minutes.</small>
          </div>
        </div>
      ) : (
        <div className="piqae-printer-overview">
          <div className="piqae-connection-mark" aria-hidden="true">
            <svg viewBox="0 0 24 24" role="presentation">
              <path d="M7 8V3h10v5M7 18H4a2 2 0 0 1-2-2v-5a3 3 0 0 1 3-3h14a3 3 0 0 1 3 3v5a2 2 0 0 1-2 2h-3M7 14h10v7H7z" />
              <path d="M18 11h1" />
            </svg>
          </div>
          <div className="piqae-printer-overview-copy">
            <s-heading>Printers connected</s-heading>
            <s-paragraph>
              Piqae is sharing live printer availability with this store.
            </s-paragraph>
          </div>
          <div
            className="piqae-printer-metrics"
            aria-label="Connection summary"
          >
            <span>
              <strong>{nodes.length}</strong> computer
              {nodes.length === 1 ? "" : "s"}
            </span>
            <span>
              <strong>{printers.length}</strong> printer
              {printers.length === 1 ? "" : "s"}
            </span>
            <span>
              <strong>{availablePrinterCount}</strong> available
            </span>
          </div>
        </div>
      )}

      {hasNodes && !hasPrinters ? (
        <s-banner tone="warning">
          Your computer is connected, but no printer inventory has reached this
          store yet. Confirm the printer is installed, keep Piqae open, then
          refresh.
        </s-banner>
      ) : null}

      {hasPrinters ? (
        <s-section
          padding="none"
          accessibilityLabel="Printers available to this store"
        >
          <s-table>
            <div slot="filters" className="piqae-printer-filters">
              <s-search-field
                label="Search printers"
                labelAccessibilityVisibility="exclusive"
                placeholder="Search printers or computers"
                value={query}
                onInput={(event) => setQuery(event.currentTarget.value)}
              />
              <select
                className="piqae-input"
                aria-label="Filter printers by status"
                value={status}
                onChange={(event) => setStatus(event.currentTarget.value)}
              >
                <option value="all">All statuses</option>
                <option value="available">Available</option>
                <option value="offline">Offline</option>
                <option value="attention">Needs attention</option>
              </select>
            </div>
            <s-table-header-row>
              <s-table-header listSlot="primary">Printer</s-table-header>
              <s-table-header listSlot="secondary">Computer</s-table-header>
              <s-table-header listSlot="inline">Status</s-table-header>
              <s-table-header listSlot="labeled">Default</s-table-header>
              <s-table-header listSlot="labeled">Last seen</s-table-header>
            </s-table-header-row>
            <s-table-body>
              {visiblePrinters.map((printer) => {
                const node = nodeById.get(printer.agent_id);
                return (
                  <s-table-row key={printer.id}>
                    <s-table-cell>
                      <div className="piqae-printer-identity">
                        <span className="piqae-printer-icon" aria-hidden="true">
                          ▣
                        </span>
                        <div>
                          <strong>{printer.name}</strong>
                          <div className="piqae-muted piqae-resource-id">
                            {printer.id}
                          </div>
                        </div>
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
                  <div className="piqae-computer-card-heading">
                    <span className="piqae-computer-icon" aria-hidden="true">
                      ▰
                    </span>
                    <div>
                      <strong>{node.name}</strong>
                      <span>{node.platform}</span>
                    </div>
                    <s-badge tone={toneFor(node.state)}>{node.state}</s-badge>
                  </div>
                  <div className="piqae-computer-meta">
                    <span>Printer access for this store</span>
                    <span>Last seen {formatSeen(node.last_seen_at)}</span>
                  </div>
                </div>
              ))}
            </div>
            <div className="piqae-connect-another">
              <span className="piqae-connect-another-mark" aria-hidden="true">
                +
              </span>
              <div>
                <strong>Add another computer</strong>
                <span>
                  Open Piqae there and choose the printers this store may use.
                </span>
                {connectionStage === "blocked" && connection?.connect_url ? (
                  <a
                    href={connection.connect_url}
                    target="_blank"
                    rel="noreferrer"
                    onClick={() => setConnectionStage("opened")}
                  >
                    Open the secure invitation
                  </a>
                ) : connectionStage === "preparing" ? (
                  <small>Preparing the secure connection…</small>
                ) : connectionStage === "opened" ? (
                  <small>Waiting for printer access in Piqae…</small>
                ) : connectionStage === "connected" ? (
                  <small>Computer connected. Refreshing printers…</small>
                ) : connectionStage === "expired" ? (
                  <small>
                    The invitation expired. Select Connect to retry.
                  </small>
                ) : connectionStage === "failed" ? (
                  <small>
                    {connectionStatus.data?.error ||
                      connector.data?.error ||
                      "The connection could not be confirmed."}
                  </small>
                ) : null}
              </div>
              <connector.Form method="post" onSubmit={beginConnection}>
                <input type="hidden" name="intent" value="connect-node" />
                <s-button type="submit" disabled={connector.state !== "idle"}>
                  {connector.state === "idle" ? "Connect" : "Opening…"}
                </s-button>
              </connector.Form>
            </div>
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
                <input
                  type="hidden"
                  name="defaultPrinterId"
                  value={settings.defaultPrinterId}
                />
                <s-paragraph>
                  Choose the default directly in the printer list above.
                </s-paragraph>
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
                <label className="piqae-field">
                  Where documents are prepared
                  <select
                    name="renderExecutionPolicy"
                    defaultValue={settings.renderExecutionPolicy}
                  >
                    <option value="automatic">Automatic (recommended)</option>
                    <option value="cloud_only">Piqae Cloud</option>
                    <option value="prefer_node">
                      This computer when compatible
                    </option>
                    <option value="require_node">Only this computer</option>
                  </select>
                </label>
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
