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
  fetchShopPrintIdentity,
  parseShopifyDataBindings,
  shopifyDocumentInput,
  type AdminGraphql,
} from "./orders.server";
import { workflows, type WorkflowRepository } from "./workflows.server";
import { parseTemplateEnvelope } from "./template-model";
import { templateDigest } from "./template-digest.server";
import { findSystemTemplate, systemTemplateId } from "./template-index.server";
import { ACCOUNT_DEFAULT_DOCUMENT_ID } from "./admin-print-options.server";
import type { DownloadTokenVault } from "./download-token.server";
import { orderPrintSequence } from "./print-order";
import { fetchProductDocumentInput } from "./products.server";
import { DocumentRenderFailedError } from "./document-render-errors";

export type PrintResult =
  | { mode: "direct"; renderId: string; jobId: string }
  | { mode: "download"; renderId: string; downloadUrl: string };
type Client = Pick<PiqaeClient, "printPackets">;
type ResolvedPublication = {
  revisionId: string;
  designTargetId: string | null;
  designSpecificationRevision: string | null;
};

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
    const publication = await this.resolveTemplatePublication(
      shop,
      link.templateRevisionId,
      link.piqaeAccountId,
      link.piqaeLiveEnvironmentId ?? null,
      input.templateId,
    );
    const settings = await this.workflow.getSettings(shop);
    const [fetchedOrders, shopIdentity] = await Promise.all([
      fetchOrders(
        input.admin,
        input.orderIds,
        parseShopifyDataBindings(settings.metafieldAllowlist),
      ),
      fetchShopPrintIdentity(input.admin, shop),
    ]);
    const orders = orderPrintSequence(fetchedOrders, settings.printOrder);
    const renderInput = shopifyDocumentInput(shop, orders, shopIdentity);
    const digest = createHash("sha256")
      .update(
        JSON.stringify({
          shop,
          ids: orders.map((order) => order.id),
          input: renderInput,
          templateRevisionId: publication.revisionId,
          requestKey: input.requestKey,
        }),
      )
      .digest("hex");
    const client = this.clientFor(link, shop);
    const render = await client.printPackets.renders.create(
      {
        template_revision_id: publication.revisionId,
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
      throw new DocumentRenderFailedError(
        completed.failure_code ?? completed.state,
        "document",
      );
    await this.workflow.recordUsage(
      shop,
      `preview:${completed.id}`,
      orders.length,
    );
    const preview = await client.printPackets.previews.create(
      completed.id,
      { expires_in_seconds: 900 },
      `shopify-preview-${digest}`,
    );
    const previewToken = this.previewTokens?.issuePreview({
      shop,
      renderId: completed.id,
      previewId: preview.id,
    });
    return {
      previewId: preview.id,
      renderId: completed.id,
      expiresAt: preview.expires_at,
      renderCost: measuredRenderCost(completed, renderInput, orders.length),
      artifactUrl: previewToken
        ? `${this.appUrl}/api/public/previews/artifact?token=${encodeURIComponent(previewToken)}`
        : `${this.appUrl}/api/print/previews/${encodeURIComponent(preview.id)}/artifact?renderId=${encodeURIComponent(completed.id)}`,
      previewImageUrl: previewToken
        ? `${this.appUrl}/api/public/previews/image?token=${encodeURIComponent(previewToken)}`
        : null,
    };
  }

  async previewProducts(input: {
    admin: AdminGraphql;
    shop: string;
    productIds: string[];
    templateId: string;
    requestKey: string;
  }) {
    const shop = normalizeShopDomain(input.shop);
    const link = await this.shops.get(shop);
    if (!link) throw new Error("Connect a Piqae account before printing");
    const selectedTemplate = await this.workflow.getTemplate(
      shop,
      input.templateId,
    );
    if (selectedTemplate?.published?.kind !== "label")
      throw new Error("Select a published product label document");
    const publication = await this.resolveTemplatePublication(
      shop,
      link.templateRevisionId,
      link.piqaeAccountId,
      link.piqaeLiveEnvironmentId ?? null,
      input.templateId,
    );
    const productData = await fetchProductDocumentInput(
      input.admin,
      shop,
      input.productIds,
    );
    const digest = createHash("sha256")
      .update(
        JSON.stringify({
          shop,
          productIds: input.productIds,
          input: productData.input,
          templateRevisionId: publication.revisionId,
          requestKey: input.requestKey,
        }),
      )
      .digest("hex");
    const client = this.clientFor(link, shop);
    const render = await client.printPackets.renders.create(
      {
        template_revision_id: publication.revisionId,
        input: productData.input,
      },
      `shopify-product-preview-render-${digest}`,
    );
    await this.shops.recordRender(
      shop,
      render.id,
      `shopify-product-preview-render-${digest}`,
    );
    const completed = await waitForRender(client, render);
    if (completed.state !== "completed")
      throw new DocumentRenderFailedError(
        completed.failure_code ?? completed.state,
        "product label",
      );
    await this.workflow.recordUsage(
      shop,
      `product-preview:${completed.id}`,
      productData.documentCount,
    );
    const preview = await client.printPackets.previews.create(
      completed.id,
      { expires_in_seconds: 900 },
      `shopify-product-preview-${digest}`,
    );
    const previewToken = this.previewTokens?.issuePreview({
      shop,
      renderId: completed.id,
      previewId: preview.id,
    });
    return {
      previewId: preview.id,
      renderId: completed.id,
      expiresAt: preview.expires_at,
      renderCost: measuredRenderCost(
        completed,
        productData.input,
        productData.documentCount,
      ),
      artifactUrl: previewToken
        ? `${this.appUrl}/api/public/previews/artifact?token=${encodeURIComponent(previewToken)}`
        : `${this.appUrl}/api/print/previews/${encodeURIComponent(preview.id)}/artifact?renderId=${encodeURIComponent(completed.id)}`,
      previewImageUrl: previewToken
        ? `${this.appUrl}/api/public/previews/image?token=${encodeURIComponent(previewToken)}`
        : null,
    };
  }

  async approvePreview(input: {
    shop: string;
    previewId: string;
    renderId: string;
    printerId?: string;
    targetId?: string;
    targetSpecificationRevision?: string;
    templateId: string;
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
    const publication = await this.resolveTemplatePublication(
      shop,
      link.templateRevisionId,
      link.piqaeAccountId,
      link.piqaeLiveEnvironmentId ?? null,
      input.templateId,
    );
    const rendered = await client.printPackets.renders.retrieve(input.renderId);
    if (rendered.template_revision_id !== publication.revisionId)
      throw new Error("Preview does not belong to the selected publication");
    let destination:
      | { target_id: string; specification_revision: string }
      | { printer_id: string };
    if (input.targetId) {
      destination = {
        target_id: input.targetId,
        specification_revision: this.publishedTargetRevision(
          publication,
          input.targetId,
          input.targetSpecificationRevision,
        ),
      };
    } else {
      if (publication.designTargetId || publication.designSpecificationRevision)
        throw new Error(
          "This published document is pinned to a print target and cannot fall back to current printer settings",
        );
      destination = { printer_id: input.printerId! };
    }
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

  private async resolveTemplatePublication(
    shop: string,
    fallback: string,
    piqaeAccountId: string,
    piqaeEnvironmentId: string | null,
    templateId?: string,
    systemTemplateKey?: "receipt",
  ) {
    if (templateId && systemTemplateKey)
      throw new Error("Select one document source");
    if (systemTemplateKey) {
      const selected =
        (await this.workflow.getTemplate(
          shop,
          systemTemplateId(systemTemplateKey) ?? "",
        )) ??
        findSystemTemplate(
          await this.workflow.listTemplates(shop),
          systemTemplateKey,
        );
      if (!selected?.published)
        throw new Error("The published receipt document is unavailable");
      const revision = exactPublishedRevision(
        selected.published!.source,
        piqaeAccountId,
        piqaeEnvironmentId,
        "receipt",
      );
      return {
        revisionId: revision,
        designTargetId: selected.published!.designTargetId,
        designSpecificationRevision:
          selected.published!.designSpecificationRevision,
      };
    }
    if (!templateId || templateId === ACCOUNT_DEFAULT_DOCUMENT_ID)
      return {
        revisionId: fallback,
        designTargetId: null,
        designSpecificationRevision: null,
      };
    const selected = await this.workflow.getTemplate(shop, templateId);
    if (!selected?.published)
      throw new Error("The selected document is not published");
    const revision = exactPublishedRevision(
      selected.published.source,
      piqaeAccountId,
      piqaeEnvironmentId,
      "selected document",
    );
    return {
      revisionId: revision,
      designTargetId: selected.published.designTargetId,
      designSpecificationRevision:
        selected.published.designSpecificationRevision,
    };
  }

  private publishedTargetRevision(
    publication: ResolvedPublication,
    targetId: string,
    requestedRevision: string | undefined,
  ): string {
    if (
      publication.designTargetId !== targetId ||
      !publication.designSpecificationRevision
    )
      throw new Error(
        "This published document is not bound to that print target; choose the target in the editor and publish again",
      );
    if (requestedRevision !== publication.designSpecificationRevision)
      throw new Error(
        "The print target setup changed after this document was published; review it in the editor and publish again",
      );
    return publication.designSpecificationRevision;
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
    const publication = await this.resolveTemplatePublication(
      shop,
      link.templateRevisionId,
      link.piqaeAccountId,
      link.piqaeLiveEnvironmentId ?? null,
      input.templateId,
      input.systemTemplateKey,
    );
    const targetDestination = input.targetId
      ? {
          target_id: input.targetId,
          specification_revision: this.publishedTargetRevision(
            publication,
            input.targetId,
            input.targetSpecificationRevision,
          ),
        }
      : null;
    const settings = await this.workflow.getSettings(shop);
    const bindings = parseShopifyDataBindings(settings.metafieldAllowlist);
    const [fetchedOrders, shopIdentity] = await Promise.all([
      input.resourceType === "draft_orders"
        ? fetchDraftOrders(input.admin, input.orderIds, bindings)
        : fetchOrders(input.admin, input.orderIds, bindings),
      fetchShopPrintIdentity(input.admin, shop),
    ]);
    const orders = orderPrintSequence(fetchedOrders, settings.printOrder);
    const renderInput = shopifyDocumentInput(shop, orders, shopIdentity);
    const digest = createHash("sha256")
      .update(
        JSON.stringify({
          shop,
          ids: orders.map((o) => o.id),
          input: renderInput,
          template: publication.revisionId,
          destination: input.targetId ?? input.printerId ?? "download",
          targetSpecificationRevision:
            targetDestination?.specification_revision ?? "",
          requestKey: input.requestKey ?? "",
        }),
      )
      .digest("hex");
    const client = this.clientFor(link, shop);
    const render = await client.printPackets.renders.create(
      {
        template_revision_id: publication.revisionId,
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
      const destination = targetDestination ?? { printer_id: input.printerId! };
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

function exactPublishedRevision(
  source: string,
  piqaeAccountId: string,
  piqaeEnvironmentId: string | null,
  documentName: string,
): string {
  const envelope = parseTemplateEnvelope(source);
  const published = envelope.published;
  if (!published)
    throw new Error(
      `The published ${documentName} has no pinned Piqae revision; reconnect or publish it before printing`,
    );
  if (published.piqaeAccountId !== piqaeAccountId)
    throw new Error(
      `The published ${documentName} belongs to a different Piqae account; reconnect or publish it before printing`,
    );
  if (published.piqaeEnvironmentId !== piqaeEnvironmentId)
    throw new Error(
      `The published ${documentName} belongs to a different Piqae environment; reconnect or publish it before printing`,
    );
  if (
    published.canonicalDigest !==
    templateDigest(JSON.stringify(envelope.document))
  )
    throw new Error(
      `The published ${documentName} no longer matches its pinned Piqae revision; reconnect or publish it before printing`,
    );
  return published.piqaeRevisionId;
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
