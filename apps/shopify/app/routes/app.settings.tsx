import type { ActionFunctionArgs, LoaderFunctionArgs } from "react-router";
import { Form, useActionData, useLoaderData } from "react-router";
import shopify from "../shopify.server";
import { parseSettings, workflows } from "../core/workflows.server";

export async function loader({ request }: LoaderFunctionArgs) {
  const { session } = await shopify.authenticate.admin(request);
  return { settings: await workflows().getSettings(session.shop) };
}
export async function action({ request }: ActionFunctionArgs) {
  const { session } = await shopify.authenticate.admin(request);
  try {
    const settings = parseSettings(await request.formData());
    await workflows().saveSettings(session.shop, settings);
    return { ok: true, error: "" };
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
  const { settings } = useLoaderData<typeof loader>();
  const result = useActionData<typeof action>();
  return (
    <s-page heading="Settings">
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
