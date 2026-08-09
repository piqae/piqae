import type { ActionFunctionArgs, LoaderFunctionArgs } from "react-router";
import { Form, useActionData, useLoaderData } from "react-router";
import shopify from "../shopify.server";
import { parseSettings, workflows } from "../core/workflows.server";
import { syncTemplateIndex } from "../core/template-index.server";
import { createProductionServices } from "../services.server";

export async function loader({ request }: LoaderFunctionArgs) {
  const { session } = await shopify.authenticate.admin(request);
  const services = createProductionServices();
  return {
    settings: await workflows().getSettings(session.shop),
    connected: Boolean(await services.repository.get(session.shop)),
    runtime: services.runtime.mode,
  };
}
export async function action({ request }: ActionFunctionArgs) {
  const { session, admin } = await shopify.authenticate.admin(request);
  try {
    const form = await request.formData();
    if (form.get("intent") === "link-piqae") {
      await createProductionServices().accountLinker.linkExisting(
        session.shop,
        String(form.get("credential") ?? ""),
      );
      await syncTemplateIndex(admin, workflows(), session.shop);
      return { ok: true, error: "", linked: true };
    }
    const settings = parseSettings(form);
    await workflows().saveSettings(session.shop, settings);
    await syncTemplateIndex(admin, workflows(), session.shop);
    return { ok: true, error: "", linked: false };
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
  const { settings, connected, runtime } = useLoaderData<typeof loader>();
  const result = useActionData<typeof action>();
  return (
    <s-page heading="Settings">
      <s-section heading="Piqae connection">
        <s-stack direction="block" gap="base">
          <s-banner tone={connected ? "success" : "warning"}>
            {connected
              ? `Connected to the ${runtime} Piqae environment.`
              : `No account is connected to the ${runtime} Piqae environment.`}
          </s-banner>
          {!connected ? (
            <Form method="post">
              <input type="hidden" name="intent" value="link-piqae" />
              <label>
                Piqae API key
                <input
                  className="piqae-input"
                  type="password"
                  name="credential"
                  minLength={16}
                  maxLength={4096}
                  autoComplete="off"
                  required
                />
              </label>
              <s-paragraph>
                The key is verified against Piqae, encrypted for this shop, and
                never returned to Shopify Admin. Linking also publishes the
                default invoice so preview works immediately.
              </s-paragraph>
              <s-button type="submit" variant="primary">
                Connect Piqae
              </s-button>
            </Form>
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
