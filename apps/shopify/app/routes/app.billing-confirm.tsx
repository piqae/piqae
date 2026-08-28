import type { ActionFunctionArgs, LoaderFunctionArgs } from "react-router";
import { Form, useLoaderData } from "react-router";
import shopify from "../shopify.server";
import { ShopifyPartnerGraphqlClient } from "../core/entitlements.server";
import {
  confirmManagedPlan,
  MANAGED_PLANS,
} from "../core/shopify-app-pricing.server";
import { createProductionServices } from "../services.server";
function required(name: string) {
  const value = process.env[name];
  if (!value) throw new Error(`${name} is required`);
  return value;
}
async function shopGid(admin: { graphql(query: string): Promise<Response> }) {
  const response = await admin.graphql(
    `query PiqaeBillingShop { shop { id } }`,
  );
  const body = (await response.json()) as any;
  if (!response.ok || body.errors?.length || !body.data?.shop?.id)
    throw new Error("SHOP_ID_LOOKUP_FAILED");
  return String(body.data.shop.id);
}
function allowedHandle(value: string | null) {
  if (!value || !(value in MANAGED_PLANS))
    throw new Response("Invalid plan return", { status: 400 });
  return value;
}
export async function loader({ request }: LoaderFunctionArgs) {
  await shopify.authenticate.admin(request);
  return {
    planHandle: allowedHandle(
      new URL(request.url).searchParams.get("plan_handle"),
    ),
  };
}
export async function action({ request }: ActionFunctionArgs) {
  const { admin, session, redirect } =
    await shopify.authenticate.admin(request);
  const planHandle = allowedHandle(
    String((await request.formData()).get("plan_handle") ?? ""),
  );
  const client = new ShopifyPartnerGraphqlClient(
    required("SHOPIFY_PARTNER_API_URL"),
    required("SHOPIFY_PARTNER_API_TOKEN"),
  );
  const plan = await confirmManagedPlan(client, {
    appId: required("SHOPIFY_PARTNER_APP_ID"),
    shopId: await shopGid(admin),
    returnedHandle: planHandle,
  });
  const services = createProductionServices();
  await services.managedAccounts.activatePlan(
    session.shop,
    plan,
    MANAGED_PLANS[plan].limit,
  );
  return redirect("/app/billing?confirmed=1");
}
export default function ConfirmPlan() {
  const { planHandle } = useLoaderData<typeof loader>();
  return (
    <s-page heading="Confirm plan">
      <s-section>
        <s-paragraph>
          Shopify returned the {planHandle} plan. Confirm to verify the live
          subscription with Shopify before activation.
        </s-paragraph>
        <Form method="post">
          <input type="hidden" name="plan_handle" value={planHandle} />
          <s-button type="submit" variant="primary">
            Confirm subscription
          </s-button>
        </Form>
      </s-section>
    </s-page>
  );
}
