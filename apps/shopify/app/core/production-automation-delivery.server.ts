import { createHash } from "node:crypto";
import type { ShopifyPrintingService } from "./printing.server";
import type { AdminGraphql } from "./orders.server";
import type { AutomationDelivery } from "./webhook-worker.server";
import type { CloudflareEmailClient } from "./cloudflare-email.server";
import { EmailDeliveryError } from "./cloudflare-email.server";

export interface RenderArtifactLoader {
  load(shop: string, renderId: string): Promise<Uint8Array>;
}
export class ProductionAutomationDelivery implements AutomationDelivery {
  constructor(
    private readonly printing: ShopifyPrintingService,
    private readonly email: CloudflareEmailClient | undefined,
    private readonly artifacts: RenderArtifactLoader,
  ) {}
  async print(input: {
    shop: string;
    resourceId: string;
    templateId: string;
    printerId: string;
    idempotencyKey: string;
    admin: AdminGraphql;
  }) {
    const order = await resolveOrder(input.admin, input.resourceId);
    const result = await this.printing.printOrders({
      admin: input.admin,
      shop: input.shop,
      orderIds: [order.id],
      printerId: input.printerId,
      requestKey: input.idempotencyKey,
    });
    if (result.mode !== "direct")
      throw new Error("AUTOMATION_DIRECT_PRINT_NOT_ACCEPTED");
    return {
      activityId: stableUuid(input.idempotencyKey),
      orderName: order.name,
      state: "accepted" as const,
    };
  }
  async enqueueEmail(input: {
    shop: string;
    resourceId: string;
    templateId: string;
    recipient: string;
    idempotencyKey: string;
    admin: AdminGraphql;
  }) {
    if (!this.email) throw new EmailDeliveryError("email provider is not configured", false);
    const order = await resolveOrder(input.admin, input.resourceId);
    const result = await this.printing.printOrders({
      admin: input.admin,
      shop: input.shop,
      orderIds: [order.id],
      requestKey: input.idempotencyKey,
    });
    const pdf = await this.artifacts.load(input.shop, result.renderId);
    const deliveryState = await this.email.send({
      to: input.recipient,
      subject: `Document for ${order.name}`,
      html: `<p>Your document for <strong>${escapeHtml(order.name)}</strong> is attached.</p>`,
      text: `Your document for ${order.name} is attached.`,
      pdf,
      filename: `${order.name}-document.pdf`,
    });
    return {
      activityId: stableUuid(input.idempotencyKey),
      orderName: order.name,
      state:
        deliveryState === "delivered"
          ? ("reported_complete" as const)
          : ("accepted" as const),
    };
  }
}
async function resolveOrder(admin: AdminGraphql, resourceId: string) {
  const response = await admin.graphql(
    `query PiqaeAutomationOrder($id: ID!) { node(id:$id) { ... on Order { id name } ... on Refund { order { id name } } ... on Fulfillment { order { id name } } } }`,
    { variables: { id: resourceId } },
  );
  const body = (await response.json()) as any;
  if (!response.ok || body.errors?.length)
    throw new Error("AUTOMATION_RESOURCE_REFETCH_FAILED");
  const node = body.data?.node;
  const order = node?.order ?? node;
  if (!order?.id || !order?.name) throw new Error("AUTOMATION_ORDER_NOT_FOUND");
  return { id: String(order.id), name: String(order.name) };
}
function stableUuid(value: string) {
  const h = createHash("sha256")
    .update(value)
    .digest("hex")
    .slice(0, 32)
    .split("");
  h[12] = "4";
  h[16] = ((parseInt(h[16]!, 16) & 3) | 8).toString(16);
  return `${h.slice(0, 8).join("")}-${h.slice(8, 12).join("")}-${h.slice(12, 16).join("")}-${h.slice(16, 20).join("")}-${h.slice(20).join("")}`;
}
function escapeHtml(value: string) {
  return value.replace(
    /[&<>"']/g,
    (c) =>
      ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[
        c
      ]!,
  );
}
