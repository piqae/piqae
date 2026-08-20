import type { ActionFunctionArgs, LoaderFunctionArgs } from "react-router";
import { Form, useActionData, useFetcher, useLoaderData } from "react-router";
import { useEffect, useState } from "react";
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
          return_url: `${process.env.SHOPIFY_APP_URL}/app/settings`,
          expires_in_seconds: 600,
        });
      return { ok: true, error: "", connection };
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
export default function Settings() {
  const { settings, connected, runtime, nodes, printers, setupError } =
    useLoaderData<typeof loader>();
  const result = useActionData<typeof action>();
  const connector = useFetcher<typeof action>();
  const [showInstaller, setShowInstaller] = useState(false);
  const [showAdvanced, setShowAdvanced] = useState(false);
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
  const installer = connection?.downloads.find(
    (download) => download.platform === detectedPlatform,
  );

  const hasPrinters = printers.length > 0;
  return (
    <s-page heading="Settings">
      <s-section heading={hasPrinters ? "Printers" : "Connect a printer"}>
        <s-stack direction="block" gap="base">
          {!connected ? <s-banner tone="warning">{setupError}</s-banner> : null}
          {hasPrinters ? (
            <s-paragraph>
              {printers.length} printer{printers.length === 1 ? "" : "s"}{" "}
              connected across {nodes.length} computer
              {nodes.length === 1 ? "" : "s"}.
            </s-paragraph>
          ) : (
            <s-paragraph>
              Install or open Piqae on the computer connected to your printer.
              This store manages the secure connection automatically—no Piqae
              account or API key is needed.
            </s-paragraph>
          )}
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
      <s-section heading="Documents and printing">
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
            <label className="piqae-check">
              <input
                type="checkbox"
                name="offerPdf"
                defaultChecked={settings.offerPdf}
              />{" "}
              Keep PDF download available
            </label>
            <s-button
              type="button"
              onClick={() => setShowAdvanced((value) => !value)}
            >
              {showAdvanced ? "Hide advanced settings" : "Advanced settings"}
            </s-button>
            <div hidden={!showAdvanced}>
              <s-stack direction="block" gap="base">
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
