import type { ActionFunctionArgs, LoaderFunctionArgs } from "react-router";
import { Form, useActionData, useLoaderData } from "react-router";
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
  return (
    <s-page heading="Settings">
      <s-section heading="Printers">
        <s-stack direction="block" gap="base">
          <s-banner tone={connected ? "success" : "warning"}>
            {connected
              ? `${nodes.length} node${nodes.length === 1 ? "" : "s"} and ${printers.length} printer${printers.length === 1 ? "" : "s"} connected.`
              : setupError}
          </s-banner>
          <s-paragraph>
            Your Piqae workspace is managed automatically by this Shopify app.
            No separate Piqae account or API key is required.
          </s-paragraph>
          <Form method="post">
            <input type="hidden" name="intent" value="connect-node" />
            <s-button type="submit" variant="primary" disabled={!connected}>
              Connect this computer
            </s-button>
          </Form>
          {result?.connection ? (
            <s-stack direction="block" gap="base">
              {result.connection.connect_url ? (
                <s-button
                  href={result.connection.connect_url}
                  variant="primary"
                >
                  Open Piqae Node
                </s-button>
              ) : null}
              {result.connection.downloads.map((download) => (
                <s-button key={download.platform} href={download.url}>
                  Download for {download.platform}
                </s-button>
              ))}
              <s-paragraph>
                This connection link expires in 10 minutes and can connect only
                to this store's isolated printing workspace.
              </s-paragraph>
            </s-stack>
          ) : null}
        </s-stack>
      </s-section>
      <s-section heading="Printing defaults">
        <Form method="post">
          <s-stack direction="block" gap="base">
            {result?.ok ? (
              <s-banner tone="success">Settings saved.</s-banner>
            ) : result?.error ? (
              <s-banner tone="critical">{result.error}</s-banner>
            ) : null}
            <label>
              Default printer ID
              <input
                className="piqae-input"
                name="defaultPrinterId"
                maxLength={200}
                defaultValue={settings.defaultPrinterId}
              />
            </label>
            <label>
              Default template ID
              <input
                className="piqae-input"
                name="defaultTemplateId"
                maxLength={200}
                defaultValue={settings.defaultTemplateId}
              />
            </label>
            <label className="piqae-check">
              <input
                type="checkbox"
                name="preferDirect"
                defaultChecked={settings.preferDirect}
              />{" "}
              Prefer direct printing when a node is ready
            </label>
            <label className="piqae-check">
              <input
                type="checkbox"
                name="offerPdf"
                defaultChecked={settings.offerPdf}
              />{" "}
              Keep PDF download available
            </label>
            <s-select
              label="Document rendering"
              name="renderExecutionPolicy"
              value={settings.renderExecutionPolicy}
            >
              <option value="automatic">Automatic (recommended)</option>
              <option value="cloud_only">Cloud only</option>
              <option value="prefer_node">Prefer node (advanced)</option>
              <option value="require_node">Require node rendering</option>
            </s-select>
            <s-paragraph>
              Automatic chooses the fastest compatible path and safely falls
              back to the exact cloud-rendered preview. Cloud only always sends
              that preview PDF. Prefer node uses a compatible ready node when
              possible, with safe PDF fallback. Require node blocks printing
              unless the selected destination reports a compatible renderer that
              can acquire every required supported resource.
            </s-paragraph>
            {settings.renderExecutionPolicy === "require_node" ? (
              <s-banner tone="warning">
                Requiring node rendering can delay or block a print while a node
                downloads supported images or other required renderer resources.
                PDF download remains available when enabled.
              </s-banner>
            ) : null}
            <label>
              Retention in days
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
              Allowed Shopify metafields{" "}
              <span className="piqae-muted">(one namespace.key per line)</span>
              <textarea
                className="piqae-code piqae-code-short"
                name="metafields"
                defaultValue={settings.metafieldAllowlist.join("\n")}
              />
            </label>
            <s-paragraph>
              Only allowlisted metafields are requested and exposed to
              templates. Secrets and protected customer data should never be
              allowlisted.
            </s-paragraph>
            <s-button type="submit" variant="primary">
              Save settings
            </s-button>
          </s-stack>
        </Form>
      </s-section>
    </s-page>
  );
}
