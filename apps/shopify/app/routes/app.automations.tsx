import type { ActionFunctionArgs, LoaderFunctionArgs } from "react-router";
import { Form, useActionData, useLoaderData } from "react-router";
import shopify from "../shopify.server";
import {
  bounded,
  newWorkflowId,
  workflows,
  type AutomationRule,
} from "../core/workflows.server";
const triggers = new Set([
  "order_paid",
  "order_created",
  "fulfillment_created",
  "refund_created",
]);
const deliveries = new Set(["printer", "email"]);
export async function loader({ request }: LoaderFunctionArgs) {
  const { session } = await shopify.authenticate.admin(request);
  return {
    rules: await workflows().listAutomations(session.shop),
    templates: await workflows().listTemplates(session.shop),
  };
}
export async function action({ request }: ActionFunctionArgs) {
  const { session } = await shopify.authenticate.admin(request);
  const form = await request.formData();
  const intent = String(form.get("intent") ?? "save");
  const id = bounded(form, "id", 40) || newWorkflowId();
  if (intent === "delete") {
    await workflows().deleteAutomation(session.shop, id);
    return { ok: true, error: "" };
  }
  try {
    const trigger = bounded(form, "trigger", 40, true);
    const delivery = bounded(form, "delivery", 20, true);
    if (!triggers.has(trigger) || !deliveries.has(delivery))
      throw new Error("Unsupported automation type");
    const value: Omit<AutomationRule, "updatedAt"> = {
      id,
      name: bounded(form, "name", 200, true),
      trigger: trigger as AutomationRule["trigger"],
      delivery: delivery as AutomationRule["delivery"],
      templateId: bounded(form, "templateId", 40, true),
      destination: bounded(form, "destination", 320, true),
      enabled: form.get("enabled") === "on",
    };
    await workflows().saveAutomation(session.shop, value);
    return { ok: true, error: "" };
  } catch (error) {
    return Response.json(
      {
        ok: false,
        error:
          error instanceof Error
            ? error.message
            : "Automation could not be saved",
      },
      { status: 400 },
    );
  }
}
export default function Automations() {
  const { rules, templates } = useLoaderData<typeof loader>();
  const result = useActionData<typeof action>();
  return (
    <s-page heading="Automations">
      <s-section>
        <s-stack direction="block" gap="base">
          {result?.ok ? (
            <s-banner tone="success">Automation saved.</s-banner>
          ) : result?.error ? (
            <s-banner tone="critical">{result.error}</s-banner>
          ) : null}
          <s-banner tone="info">
            Automatic direct printing only runs when the selected node is ready.
            Failed or uncertain delivery is never silently marked complete.
          </s-banner>
          <Form method="post" className="piqae-card">
            <s-stack direction="block" gap="base">
              <s-heading>Create automation</s-heading>
              <label>
                Name
                <input
                  className="piqae-input"
                  name="name"
                  required
                  maxLength={200}
                />
              </label>
              <label>
                When
                <select className="piqae-input" name="trigger">
                  <option value="order_paid">Order is paid</option>
                  <option value="order_created">Order is created</option>
                  <option value="fulfillment_created">
                    Fulfillment is created
                  </option>
                  <option value="refund_created">Refund is created</option>
                </select>
              </label>
              <label>
                Template
                <select className="piqae-input" name="templateId" required>
                  <option value="">Select template</option>
                  {templates
                    .filter((t) => t.state === "published")
                    .map((t) => (
                      <option key={t.id} value={t.id}>
                        {t.name}
                      </option>
                    ))}
                </select>
              </label>
              <label>
                Delivery
                <select className="piqae-input" name="delivery">
                  <option value="printer">Direct printer</option>
                  <option value="email">Email attachment</option>
                </select>
              </label>
              <label>
                Printer ID or recipient email
                <input
                  className="piqae-input"
                  name="destination"
                  required
                  maxLength={320}
                />
              </label>
              <label className="piqae-check">
                <input name="enabled" type="checkbox" /> Enable immediately
              </label>
              <s-button type="submit" variant="primary">
                Create automation
              </s-button>
            </s-stack>
          </Form>
          {rules.map((rule) => (
            <div className="piqae-card" key={rule.id}>
              <div className="piqae-actions">
                <s-heading>{rule.name}</s-heading>
                <s-badge tone={rule.enabled ? "success" : "neutral"}>
                  {rule.enabled ? "Active" : "Paused"}
                </s-badge>
              </div>
              <s-paragraph>
                {rule.trigger.replaceAll("_", " ")} · {rule.delivery} ·{" "}
                {rule.destination}
              </s-paragraph>
              <Form method="post">
                <input type="hidden" name="id" value={rule.id} />
                <input type="hidden" name="intent" value="delete" />
                <s-button type="submit" tone="critical">
                  Delete
                </s-button>
              </Form>
            </div>
          ))}
        </s-stack>
      </s-section>
    </s-page>
  );
}
