import { createHash } from "node:crypto";
import {
  PiqaeClient,
  type PrintPacketRender,
  type PrintPacketRenderCost,
} from "@piqae/sdk";
import type { ShopLink, ShopRepository } from "./model";
import { normalizeShopDomain } from "./model";
import type { CredentialVault } from "./credentials.server";
import {
  fetchDraftOrders,
  fetchOrders,
  parseShopifyDataBindings,
  shopifyDocumentInput,
  type AdminGraphql,
} from "./orders.server";
import { workflows, type WorkflowRepository } from "./workflows.server";
import { parseTemplateEnvelope } from "./template-model";
import { ACCOUNT_DEFAULT_DOCUMENT_ID } from "./admin-print-options.server";
import type { DownloadTokenVault } from "./download-token.server";

export type PrintResult =
  | { mode: "direct"; renderId: string; jobId: string }
  | { mode: "download"; renderId: string; downloadUrl: string };
type Client = Pick<PiqaeClient, "printPackets">;

export class ShopifyPrintingService {
  constructor(
    private readonly shops: ShopRepository,
    private readonly vault: CredentialVault,
    private readonly clientFactory: (token: string) => Client,
    private readonly appUrl: string,
    private readonly previewTokens?: Pick<DownloadTokenVault, "issuePreview">,
    private readonly workflow: WorkflowRepository = workflows(),
    private readonly managedClientFactory?: (link: ShopLink) => Client,
  ) {}

  private clientFor(link: ShopLink, shop: string): Client {
    if (link.entitlementMode === "shopify_child") {
      if (!this.managedClientFactory)
        throw new Error("PIQAE_MANAGED_ACCOUNT_NOT_READY");
      return this.managedClientFactory(link);
    }
    if (!link.encryptedCredential)
      throw new Error("PIQAE_ACCOUNT_CREDENTIAL_MISSING");
    return this.clientFactory(this.vault.open(link.encryptedCredential, shop));
  }
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
    const settings = await this.workflow.getSettings(shop);
    const orders = await fetchOrders(
      input.admin,
      input.orderIds,
      parseShopifyDataBindings(settings.metafieldAllowlist),
    );
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
    const client = this.clientFor(link, shop);
    const renderInput = shopifyDocumentInput(shop, orders);
    const render = await client.printPackets.renders.create(
      {
        template_revision_id: templateRevisionId,
        input: renderInput,
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
    const preview = await client.printPackets.previews.create(
      completed.id,
      { expires_in_seconds: 900 },
      `shopify-preview-${digest}`,
    );
    return {
      previewId: preview.id,
      renderId: completed.id,
      expiresAt: preview.expires_at,
      renderCost: measuredRenderCost(completed, renderInput, orders.length),
      artifactUrl: this.previewTokens
        ? `${this.appUrl}/api/public/previews/artifact?token=${encodeURIComponent(this.previewTokens.issuePreview({ shop, renderId: completed.id, previewId: preview.id }))}`
        : `${this.appUrl}/api/print/previews/${encodeURIComponent(preview.id)}/artifact?renderId=${encodeURIComponent(completed.id)}`,
    };
  }

  async approvePreview(input: {
    shop: string;
    previewId: string;
    renderId: string;
    printerId?: string;
    targetId?: string;
    targetSpecificationRevision?: string;
    requestKey: string;
    renderCost?: PrintPacketRenderCost;
  }) {
    if (
      Boolean(input.targetId) === Boolean(input.printerId) ||
      Boolean(input.targetId) !== Boolean(input.targetSpecificationRevision)
    )
      throw new Error("Choose exactly one print target or printer");
    const shop = normalizeShopDomain(input.shop);
    if (!(await this.shops.ownsRender(shop, input.renderId)))
      throw new Error("Preview not found");
    const link = await this.shops.get(shop);
    if (!link) throw new Error("Connect a Piqae account before printing");
    const client = this.clientFor(link, shop);
    const settings = await this.workflow.getSettings(shop);
    const preview = await client.printPackets.previews.retrieve(
      input.previewId,
    );
    if (preview.render_id !== input.renderId)
      throw new Error("Preview not found");
    const destination = input.targetId
      ? {
          target_id: input.targetId,
          specification_revision: input.targetSpecificationRevision!,
        }
      : { printer_id: input.printerId! };
    const approved = await client.printPackets.previews.approve(
      input.previewId,
      {
        ...destination,
        title: "Shopify order documents",
        render_policy: settings.renderExecutionPolicy,
        render_cost: input.renderCost,
      },
      input.requestKey,
    );
    return { jobId: approved.job.id, state: approved.preview.state };
  }

  async renderReadiness(input: {
    shop: string;
    renderId: string;
    printerId: string;
    renderCost?: PrintPacketRenderCost;
  }) {
    const shop = normalizeShopDomain(input.shop);
    if (!(await this.shops.ownsRender(shop, input.renderId)))
      throw new Error("Preview not found");
    const link = await this.shops.get(shop);
    if (!link) throw new Error("Connect a Piqae account before printing");
    const settings = await this.workflow.getSettings(shop);
    const client = this.clientFor(link, shop);
    return client.printPackets.renders.readiness(input.renderId, {
      printer_id: input.printerId,
      render_policy: settings.renderExecutionPolicy,
      render_cost: input.renderCost,
    });
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
    const client = this.clientFor(link, shop);
    const preview = await client.printPackets.previews.retrieve(
      input.previewId,
    );
    if (preview.render_id !== input.renderId)
      throw new Error("Preview not found");
    return client.printPackets.previews.cancel(
      input.previewId,
      input.requestKey,
    );
  }

  private async resolveTemplateRevision(
    shop: string,
    fallback: string,
    templateId?: string,
    systemTemplateKey?: "receipt",
  ) {
    if (templateId && systemTemplateKey)
      throw new Error("Select one document source");
    if (systemTemplateKey) {
      const selected = (await this.workflow.listTemplates(shop)).find(
        (candidate) => {
          if (!candidate.published) return false;
          try {
            return (
              parseTemplateEnvelope(candidate.published.source).system?.key ===
              systemTemplateKey
            );
          } catch {
            return false;
          }
        },
      );
      if (!selected)
        throw new Error("The published receipt document is unavailable");
      const revision = parseTemplateEnvelope(selected.published!.source)
        .published?.piqaeRevisionId;
      if (!revision)
        throw new Error(
          "The published receipt has no pinned Piqae revision; reconnect or publish it before printing",
        );
      return revision;
    }
    if (!templateId || templateId === ACCOUNT_DEFAULT_DOCUMENT_ID)
      return fallback;
    const selected = await this.workflow.getTemplate(shop, templateId);
    if (!selected?.published)
      throw new Error("The selected document is not published");
    const revision = parseTemplateEnvelope(selected.published.source).published
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
    targetId?: string;
    targetSpecificationRevision?: string;
    requestKey?: string;
    templateId?: string;
    systemTemplateKey?: "receipt";
    resourceType?: "orders" | "draft_orders";
  }): Promise<PrintResult> {
    if (
      (input.targetId && input.printerId) ||
      Boolean(input.targetId) !== Boolean(input.targetSpecificationRevision)
    )
      throw new Error("Target prints require one exact specification revision");
    const shop = normalizeShopDomain(input.shop);
    const link = await this.shops.get(shop);
    if (!link) throw new Error("Connect a Piqae account before printing");
    const templateRevisionId = await this.resolveTemplateRevision(
      shop,
      link.templateRevisionId,
      input.templateId,
      input.systemTemplateKey,
    );
    const settings = await this.workflow.getSettings(shop);
    const bindings = parseShopifyDataBindings(settings.metafieldAllowlist);
    const orders =
      input.resourceType === "draft_orders"
        ? await fetchDraftOrders(input.admin, input.orderIds, bindings)
        : await fetchOrders(input.admin, input.orderIds, bindings);
    const digest = createHash("sha256")
      .update(
        JSON.stringify({
          shop,
          ids: orders.map((o) => o.id),
          template: templateRevisionId,
          destination: input.targetId ?? input.printerId ?? "download",
          targetSpecificationRevision: input.targetSpecificationRevision ?? "",
          requestKey: input.requestKey ?? "",
        }),
      )
      .digest("hex");
    const client = this.clientFor(link, shop);
    const renderInput = shopifyDocumentInput(shop, orders);
    const render = await client.printPackets.renders.create(
      {
        template_revision_id: templateRevisionId,
        input: renderInput,
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
    if (input.printerId || input.targetId) {
      const settings = await this.workflow.getSettings(shop);
      const completed = await waitForRender(client, render);
      if (completed.state !== "completed")
        throw new Error(
          `document render failed: ${completed.failure_code ?? completed.state}`,
        );
      const destination = input.targetId
        ? {
            target_id: input.targetId,
            specification_revision: input.targetSpecificationRevision!,
          }
        : { printer_id: input.printerId! };
      const job = await client.printPackets.renders.print(
        completed.id,
        {
          ...destination,
          title: `Shopify orders ${orders.map((o) => o.name).join(", ")}`,
          render_policy: settings.renderExecutionPolicy,
          render_cost: measuredRenderCost(
            completed,
            renderInput,
            orders.length,
          ),
        },
        `shopify-print-${digest}-${input.targetId ?? input.printerId}`,
      );
      await workflows().recordActivity(shop, {
        id: stableActivityId(digest),
        orderName: orders.map((order) => order.name).join(", "),
        documentName: "Order document",
        destination: input.targetId ?? input.printerId!,
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

function measuredRenderCost(
  render: PrintPacketRender,
  input: Record<string, unknown>,
  documentCount: number,
): PrintPacketRenderCost | undefined {
  const pdfBytes = render.artifact_byte_length;
  const pageCount = render.page_count;
  if (
    !Number.isSafeInteger(pdfBytes) ||
    !Number.isSafeInteger(pageCount) ||
    !pdfBytes ||
    !pageCount
  )
    return undefined;
  return {
    document_count: documentCount,
    page_count: pageCount,
    pdf_bytes: pdfBytes,
    input_bytes: new TextEncoder().encode(JSON.stringify(input)).byteLength,
  };
}

export function parseRenderCost(
  value: unknown,
): PrintPacketRenderCost | undefined {
  if (value === undefined || value === null) return undefined;
  if (!value || typeof value !== "object" || Array.isArray(value))
    throw new Error("render cost is invalid");
  const candidate = value as Record<string, unknown>;
  const fields = [
    "document_count",
    "page_count",
    "pdf_bytes",
    "input_bytes",
  ] as const;
  const maxima = {
    document_count: 10_000,
    page_count: 100_000,
    pdf_bytes: 524_288_000,
    input_bytes: 52_428_800,
  } as const;
  if (
    fields.some(
      (field) =>
        !Number.isSafeInteger(candidate[field]) ||
        Number(candidate[field]) < 1 ||
        Number(candidate[field]) > maxima[field],
    )
  )
    throw new Error("render cost is invalid");
  return {
    document_count: Number(candidate.document_count),
    page_count: Number(candidate.page_count),
    pdf_bytes: Number(candidate.pdf_bytes),
    input_bytes: Number(candidate.input_bytes),
  };
}

function stableActivityId(hexDigest: string): string {
  const value = hexDigest.slice(0, 32).split("");
  value[12] = "4";
  value[16] = ((parseInt(value[16]!, 16) & 3) | 8).toString(16);
  return `${value.slice(0, 8).join("")}-${value.slice(8, 12).join("")}-${value.slice(12, 16).join("")}-${value.slice(16, 20).join("")}-${value.slice(20).join("")}`;
}

async function waitForRender(
  client: Client,
  initial: PrintPacketRender,
): Promise<PrintPacketRender> {
  let render = initial;
  for (
    let attempt = 0;
    attempt < 40 &&
    (render.state === "registered" || render.state === "rendering");
    attempt += 1
  ) {
    await new Promise((resolve) => setTimeout(resolve, 250));
    render = await client.printPackets.renders.retrieve(render.id);
  }
  if (render.state === "registered" || render.state === "rendering")
    throw new Error("document render timed out");
  return render;
}
