import type { ActionFunctionArgs, LoaderFunctionArgs } from "react-router";
import { Form, useLoaderData } from "react-router";
import shopify from "../shopify.server";
import { workflows } from "../core/workflows.server";
import {
  hostedPricingUrl,
  MANAGED_PLANS,
} from "../core/shopify-app-pricing.server";
const DISPLAY_PLANS = ["starter", "growth", "scale"] as const;
function required(name: string) {
  const value = process.env[name];
  if (!value) throw new Error(`${name} is required`);
  return value;
}
export async function loader({ request }: LoaderFunctionArgs) {
  const { session } = await shopify.authenticate.admin(request);
  return {
    billing: await workflows().getBilling(session.shop),
    confirmed: new URL(request.url).searchParams.get("confirmed") === "1",
  };
}
export async function action({ request }: ActionFunctionArgs) {
  const { session, redirect } = await shopify.authenticate.admin(request);
  const plan = String((await request.formData()).get("plan") ?? "");
  if (!(plan in MANAGED_PLANS))
    return Response.json({ error: "Unknown plan" }, { status: 400 });
  return redirect(
    hostedPricingUrl(session.shop, required("SHOPIFY_APP_HANDLE")),
    { target: "_top" },
  );
}
export default function Billing() {
  const { billing, confirmed } = useLoaderData<typeof loader>();
  return (
    <s-page heading="Plan">
      {confirmed ? (
        <s-banner tone="success">
          Shopify confirmed and activated your plan.
        </s-banner>
      ) : null}
      <s-section>
        <s-stack direction="block" gap="base">
          <s-heading>{billing.plan} plan</s-heading>
          <s-paragraph>
            {billing.used} of {billing.limit} documents used. Status:{" "}
            {billing.status}.
          </s-paragraph>
          <div className="piqae-grid">
            {DISPLAY_PLANS.map((plan) => (
              <Form method="post" key={plan}>
                <input type="hidden" name="plan" value={plan} />
                <s-button type="submit" variant="primary">
                  Choose {plan} in Shopify
                </s-button>
              </Form>
            ))}
          </div>
          <s-paragraph>
            Paid plans are selected and approved on Shopify's hosted pricing
            page. Your isolated Piqae printing workspace is included and does
            not require a separate account or subscription.
          </s-paragraph>
        </s-stack>
      </s-section>
    </s-page>
  );
}
