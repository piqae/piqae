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
import { workflows, type WorkflowRepository } from "./workflows.server";
import { parseTemplateEnvelope } from "./template-model";
import type { DownloadTokenVault } from "./download-token.server";

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
    private readonly previewTokens?: Pick<DownloadTokenVault, "issuePreview">,
    private readonly workflow: WorkflowRepository = workflows(),
  ) {}
  async previewOrders(input: {
    admin: AdminGraphql;
    shop: string;
    orderIds: string[];
    templateId?: string;
    requestKey: string;
  }) {
    const shop = normalizeShopDomain(input.shop);
    const link = await this.shops.get(shop);
    if (!link) throw new Error("Connect a Piqae account before printing");
    const templateRevisionId = await this.resolveTemplateRevision(
      shop,
      link.templateRevisionId,
      input.templateId,
    );
    const orders = await fetchOrders(input.admin, input.orderIds);
    const digest = createHash("sha256")
      .update(
        JSON.stringify({
          shop,
          ids: orders.map((order) => order.id),
          templateRevisionId,
          requestKey: input.requestKey,
        }),
      )
      .digest("hex");
    const client = this.clientFactory(
      this.vault.open(link.encryptedCredential, shop),
    );
    const render = await client.documents.renders.create(
      {
        template_revision_id: templateRevisionId,
        input: { shop, orders },
      },
      `shopify-preview-render-${digest}`,
    );
    await this.shops.recordRender(
      shop,
      render.id,
      `shopify-preview-render-${digest}`,
    );
    const completed = await waitForRender(client, render);
    if (completed.state !== "completed")
      throw new Error(
        `document render failed: ${completed.failure_code ?? completed.state}`,
      );
    const preview = await client.documents.previews.create(
      completed.id,
      { expires_in_seconds: 900 },
      `shopify-preview-${digest}`,
    );
    return {
      previewId: preview.id,
      renderId: completed.id,
      expiresAt: preview.expires_at,
      artifactUrl: this.previewTokens
        ? `${this.appUrl}/api/public/previews/artifact?token=${encodeURIComponent(this.previewTokens.issuePreview({ shop, renderId: completed.id, previewId: preview.id }))}`
        : `${this.appUrl}/api/print/previews/${encodeURIComponent(preview.id)}/artifact?renderId=${encodeURIComponent(completed.id)}`,
    };
  }

  async approvePreview(input: {
    shop: string;
    previewId: string;
    renderId: string;
    printerId: string;
    requestKey: string;
  }) {
    const shop = normalizeShopDomain(input.shop);
    if (!(await this.shops.ownsRender(shop, input.renderId)))
      throw new Error("Preview not found");
    const link = await this.shops.get(shop);
    if (!link) throw new Error("Connect a Piqae account before printing");
    const client = this.clientFactory(
      this.vault.open(link.encryptedCredential, shop),
    );
    const preview = await client.documents.previews.retrieve(input.previewId);
    if (preview.render_id !== input.renderId)
      throw new Error("Preview not found");
    const approved = await client.documents.previews.approve(
      input.previewId,
      { printer_id: input.printerId, title: "Shopify order documents" },
      input.requestKey,
    );
    return { jobId: approved.job.id, state: approved.preview.state };
  }

  async cancelPreview(input: {
    shop: string;
    previewId: string;
    renderId: string;
    requestKey: string;
  }) {
    const shop = normalizeShopDomain(input.shop);
    if (!(await this.shops.ownsRender(shop, input.renderId)))
      throw new Error("Preview not found");
    const link = await this.shops.get(shop);
    if (!link) throw new Error("Preview not found");
    const client = this.clientFactory(
      this.vault.open(link.encryptedCredential, shop),
    );
    const preview = await client.documents.previews.retrieve(input.previewId);
    if (preview.render_id !== input.renderId)
      throw new Error("Preview not found");
    return client.documents.previews.cancel(input.previewId, input.requestKey);
  }

  private async resolveTemplateRevision(
    shop: string,
    fallback: string,
    templateId?: string,
  ) {
    if (!templateId) return fallback;
    const selected = await this.workflow.getTemplate(shop, templateId);
    if (!selected || selected.state !== "published")
      throw new Error("The selected document is not published");
    const revision = parseTemplateEnvelope(selected.source).published
      ?.piqaeRevisionId;
    if (!revision)
      throw new Error(
        "The selected document has no published Piqae revision; publish it again before printing",
      );
    return revision;
  }
  async printOrders(input: {
    admin: AdminGraphql;
    shop: string;
    orderIds: string[];
    printerId?: string;
    requestKey?: string;
    templateId?: string;
    resourceType?: "orders" | "draft_orders";
  }): Promise<PrintResult> {
    const shop = normalizeShopDomain(input.shop);
    const link = await this.shops.get(shop);
    if (!link) throw new Error("Connect a Piqae account before printing");
    const templateRevisionId = await this.resolveTemplateRevision(
      shop,
      link.templateRevisionId,
      input.templateId,
    );
    const orders =
      input.resourceType === "draft_orders"
        ? await fetchDraftOrders(input.admin, input.orderIds)
        : await fetchOrders(input.admin, input.orderIds);
    const digest = createHash("sha256")
      .update(
        JSON.stringify({
          shop,
          ids: orders.map((o) => o.id),
          template: templateRevisionId,
          requestKey: input.requestKey ?? "",
        }),
      )
      .digest("hex");
    const client = this.clientFactory(
      this.vault.open(link.encryptedCredential, shop),
    );
    const render = await client.documents.renders.create(
      {
        template_revision_id: templateRevisionId,
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
