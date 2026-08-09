import { createHash } from "node:crypto";
import { PiqaeClient, type DocumentRender } from "@piqae/sdk";
import type { ShopRepository } from "./model";
import { normalizeShopDomain } from "./model";
import type { CredentialVault } from "./credentials.server";
import {
  fetchDraftOrders,
  fetchOrders,
  type AdminGraphql,
} from "./orders.server";
import { workflows } from "./workflows.server";

export type PrintResult =
  | { mode: "direct"; renderId: string; jobId: string }
  | { mode: "download"; renderId: string; downloadUrl: string };
type Client = Pick<PiqaeClient, "documents">;

export class ShopifyPrintingService {
  constructor(
    private readonly shops: ShopRepository,
    private readonly vault: CredentialVault,
    private readonly clientFactory: (token: string) => Client,
    private readonly appUrl: string,
  ) {}
  async printOrders(input: {
    admin: AdminGraphql;
    shop: string;
    orderIds: string[];
    printerId?: string;
    requestKey?: string;
    resourceType?: "orders" | "draft_orders";
  }): Promise<PrintResult> {
    const shop = normalizeShopDomain(input.shop);
    const link = await this.shops.get(shop);
    if (!link) throw new Error("Connect a Piqae account before printing");
    const orders =
      input.resourceType === "draft_orders"
        ? await fetchDraftOrders(input.admin, input.orderIds)
        : await fetchOrders(input.admin, input.orderIds);
    const digest = createHash("sha256")
      .update(
        JSON.stringify({
          shop,
          ids: orders.map((o) => o.id),
          template: link.templateRevisionId,
          requestKey: input.requestKey ?? "",
        }),
      )
      .digest("hex");
    const client = this.clientFactory(
      this.vault.open(link.encryptedCredential, shop),
    );
    const render = await client.documents.renders.create(
      {
        template_revision_id: link.templateRevisionId,
        input: { shop, orders },
      },
      `shopify-render-${digest}`,
    );
    const ownership =
      orders.length === 1 && input.resourceType !== "draft_orders"
        ? {
            orderGid: orders[0]!.id,
            customerGid: orders[0]!.customer?.id || undefined,
          }
        : undefined;
    await this.shops.recordRender(
      shop,
      render.id,
      `shopify-render-${digest}`,
      ownership,
    );
    if (input.printerId) {
      const completed = await waitForRender(client, render);
      if (completed.state !== "completed")
        throw new Error(
          `document render failed: ${completed.failure_code ?? completed.state}`,
        );
      const job = await client.documents.renders.print(
        completed.id,
        {
          printer_id: input.printerId,
          title: `Shopify orders ${orders.map((o) => o.name).join(", ")}`,
        },
        `shopify-print-${digest}-${input.printerId}`,
      );
      await workflows().recordActivity(shop, {
        id: stableActivityId(digest),
        orderName: orders.map((order) => order.name).join(", "),
        documentName: "Order document",
        destination: input.printerId,
        state: "accepted",
      });
      return { mode: "direct", renderId: render.id, jobId: job.id };
    }
    await workflows().recordActivity(shop, {
      id: stableActivityId(digest),
      orderName: orders.map((order) => order.name).join(", "),
      documentName: "Order document",
      destination: "PDF download",
      state: "accepted",
    });
    return {
      mode: "download",
      renderId: render.id,
      downloadUrl: `${this.appUrl}/api/renders/${encodeURIComponent(render.id)}/download`,
    };
  }
}

function stableActivityId(hexDigest: string): string {
  const value = hexDigest.slice(0, 32).split("");
  value[12] = "4";
  value[16] = ((parseInt(value[16]!, 16) & 3) | 8).toString(16);
  return `${value.slice(0, 8).join("")}-${value.slice(8, 12).join("")}-${value.slice(12, 16).join("")}-${value.slice(16, 20).join("")}-${value.slice(20).join("")}`;
}

async function waitForRender(
  client: Client,
  initial: DocumentRender,
): Promise<DocumentRender> {
  let render = initial;
  for (
    let attempt = 0;
    attempt < 40 &&
    (render.state === "registered" || render.state === "rendering");
    attempt += 1
  ) {
    await new Promise((resolve) => setTimeout(resolve, 250));
    render = await client.documents.renders.retrieve(render.id);
  }
  if (render.state === "registered" || render.state === "rendering")
    throw new Error("document render timed out");
  return render;
}
