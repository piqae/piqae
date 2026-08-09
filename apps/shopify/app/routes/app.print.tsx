import type { ActionFunctionArgs, LoaderFunctionArgs } from "react-router";
import { Form, useActionData, useLoaderData } from "react-router";
import { useState } from "react";
import shopify from "../shopify.server";
import { createProductionServices } from "../services.server";
import { OrderTable } from "../components/shopify-ui";

type ActionData =
  | { ok: true; result: { mode: string } }
  | { ok: false; error: string };

export function selectedOrderIds(form: FormData): string[] {
  const ids = [
    ...new Set(
      form
        .getAll("orderIds")
        .map(String)
        .map((id) => id.trim())
        .filter(Boolean),
    ),
  ];
  if (ids.length < 1 || ids.length > 50)
    throw new Error("Select between 1 and 50 orders");
  return ids;
}

export async function loader({ request }: LoaderFunctionArgs) {
  const { admin } = await shopify.authenticate.admin(request);
  const response = await admin.graphql(`#graphql
    query PiqaeRecentOrders {
      orders(first: 50, sortKey: CREATED_AT, reverse: true) {
        nodes {
          id name createdAt displayFinancialStatus displayFulfillmentStatus
          customer { displayName }
          totalPriceSet { shopMoney { amount currencyCode } }
        }
      }
    }
  `);
  if (!response.ok)
    throw new Response("Shopify orders unavailable", { status: 502 });
  const body = (await response.json()) as any;
  if (body.errors?.length)
    throw new Response("Shopify rejected the orders query", { status: 502 });
  const payment = (value: string) => {
    const label = String(value ?? "UNKNOWN")
      .replaceAll("_", " ")
      .toLowerCase();
    const tone =
      value === "PAID"
        ? "success"
        : value === "PENDING" || value === "PARTIALLY_PAID"
          ? "warning"
          : value === "VOIDED"
            ? "critical"
            : "neutral";
    return {
      label: label.replace(/^./, (c) => c.toUpperCase()),
      tone,
    } as const;
  };
  return {
    orders: (body.data?.orders?.nodes ?? []).map((order: any) => {
      const financial = payment(order.displayFinancialStatus);
      const money = order.totalPriceSet?.shopMoney;
      return {
        id: order.id,
        name: order.name,
        date: new Date(order.createdAt).toLocaleDateString("en", {
          dateStyle: "medium",
        }),
        customer: order.customer?.displayName ?? "No customer",
        total: `${money?.amount ?? "0"} ${money?.currencyCode ?? ""}`.trim(),
        status: String(order.displayFulfillmentStatus ?? "UNFULFILLED")
          .replaceAll("_", " ")
          .toLowerCase(),
        payment: financial.label,
        paymentTone: financial.tone,
      };
    }),
  };
}
export async function action({ request }: ActionFunctionArgs) {
  const { admin, session } = await shopify.authenticate.admin(request);
  const form = await request.formData();
  let ids: string[];
  try {
    ids = selectedOrderIds(form);
  } catch {
    return Response.json(
      { ok: false as const, error: "Select between 1 and 50 orders" },
      { status: 400 },
    );
  }
  const printerId = String(form.get("printerId") ?? "").trim() || undefined;
  try {
    const result = await createProductionServices().printing.printOrders({
      admin,
      shop: session.shop,
      orderIds: ids,
      printerId,
    });
    return Response.json({ ok: true as const, result });
  } catch {
    return Response.json(
      {
        ok: false as const,
        error: "The document request could not be submitted",
      },
      { status: 409 },
    );
  }
}
export default function PrintOrders() {
  const result = useActionData<typeof action>() as ActionData | undefined;
  const { orders } = useLoaderData<typeof loader>();
  const [selected, setSelected] = useState<Set<string>>(new Set());
  return (
    <s-page heading="Orders">
      <s-section>
        <Form method="post">
          <s-stack direction="block" gap="base">
            {result?.ok ? (
              <s-banner tone="success">
                Document request accepted. You can leave this page; progress
                will continue in the background.
              </s-banner>
            ) : result ? (
              <s-banner tone="critical">{result.error}</s-banner>
            ) : null}
            {orders.length ? (
              <OrderTable
                orders={orders}
                selected={selected}
                onSelectionChange={setSelected}
              />
            ) : (
              <s-banner tone="info">No orders are available.</s-banner>
            )}
            <div className="piqae-card">
              <s-stack direction="block" gap="base">
                <s-heading>
                  Print {selected.size} selected{" "}
                  {selected.size === 1 ? "order" : "orders"}
                </s-heading>
                <s-paragraph>
                  This page generates the shop's published default document as a
                  PDF. Direct destinations are offered only after a real Piqae
                  printer has been selected in an Admin or POS action.
                </s-paragraph>
                <s-checkbox
                  name="combine"
                  label="Combine documents into one print job"
                />
                <div className="piqae-actions">
                  <s-button
                    type="submit"
                    variant="primary"
                    disabled={selected.size === 0}
                  >
                    Generate PDF
                  </s-button>
                </div>
                <s-paragraph>
                  A failed or uncertain direct-print attempt never silently
                  starts a PDF or a second physical print.
                </s-paragraph>
              </s-stack>
            </div>
          </s-stack>
        </Form>
      </s-section>
    </s-page>
  );
}
