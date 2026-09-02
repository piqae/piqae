import type { ActionFunctionArgs, LoaderFunctionArgs } from "react-router";
import { Form, useLoaderData } from "react-router";
import shopify from "../shopify.server";
import { workflows } from "../core/workflows.server";
import {
  hostedPricingUrl,
  MANAGED_PLANS,
} from "../core/shopify-app-pricing.server";
const DISPLAY_PLANS = ["starter", "growth", "scale"] as const;
const PLAN_PRESENTATION = {
  starter: {
    title: "Starter",
    allowance: "500 documents / month",
    description: "For stores establishing a dependable print workflow.",
  },
  growth: {
    title: "Growth",
    allowance: "5,000 documents / month",
    description: "For busy fulfillment teams printing every day.",
  },
  scale: {
    title: "Scale",
    allowance: "High-volume allowance",
    description: "For high-throughput operations with larger print volumes.",
  },
} as const;
export function planUsagePercentage(used: number, limit: number) {
  if (!Number.isFinite(used) || !Number.isFinite(limit) || limit <= 0) return 0;
  return Math.min(100, Math.max(0, (used / limit) * 100));
}
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
  const usage = planUsagePercentage(billing.used, billing.limit);
  return (
    <s-page heading="Plan">
      {confirmed ? (
        <s-banner tone="success">
          Shopify confirmed and activated your plan.
        </s-banner>
      ) : null}
      <div className="piqae-plan-overview">
        <div className="piqae-plan-overview-copy">
          <span className="piqae-eyebrow">Current plan</span>
          <s-heading>{billing.plan} plan</s-heading>
          <s-paragraph>
            {billing.used.toLocaleString()} of {billing.limit.toLocaleString()}{" "}
            documents rendered this month
          </s-paragraph>
          <div
            className="piqae-plan-progress"
            role="progressbar"
            aria-label="Monthly document usage"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={Math.round(usage)}
          >
            <span style={{ width: String(usage) + "%" }} />
          </div>
        </div>
        <s-badge tone={billing.status === "active" ? "success" : "info"}>
          {billing.status}
        </s-badge>
      </div>
      <s-section heading="Choose the capacity that fits your store">
        <div className="piqae-plan-grid">
          {DISPLAY_PLANS.map((plan) => {
            const detail = PLAN_PRESENTATION[plan];
            const current = billing.plan === plan;
            return (
              <div
                className={
                  "piqae-plan-card" +
                  (current ? " piqae-plan-card--current" : "")
                }
                key={plan}
              >
                <div className="piqae-plan-card-heading">
                  <s-heading>{detail.title}</s-heading>
                  {current ? <s-badge tone="info">Current</s-badge> : null}
                </div>
                <strong className="piqae-plan-allowance">
                  {detail.allowance}
                </strong>
                <s-paragraph>{detail.description}</s-paragraph>
                <span className="piqae-plan-price-note">
                  Local price and currency shown in Shopify
                </span>
                <ul>
                  <li>Managed Piqae workspace included</li>
                  <li>Direct printing and PDF fallback</li>
                  <li>Shopify document templates</li>
                </ul>
                {current ? (
                  <s-button disabled>Current plan</s-button>
                ) : (
                  <Form method="post">
                    <input type="hidden" name="plan" value={plan} />
                    <s-button type="submit" variant="primary">
                      View in Shopify
                    </s-button>
                  </Form>
                )}
              </div>
            );
          })}
        </div>
      </s-section>
      <div className="piqae-plan-note">
        <strong>Billing stays in Shopify</strong>
        <span>
          Shopify shows the current price and currency before you approve a
          change. No separate Piqae subscription is required.
        </span>
      </div>
    </s-page>
  );
}
