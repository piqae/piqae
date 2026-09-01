import { createHash } from "node:crypto";
import type {
  CreatePrintPacketPreviewRender,
  PiqaeClient,
  PrintPacketPreviewRender,
} from "@piqae/sdk";

import type { ShopRepository } from "./model";
import {
  fetchOrders,
  fetchShopPrintIdentity,
  normalizeOrderGid,
  parseShopifyDataBindings,
  shopifyDocumentInput,
  type AdminGraphql,
} from "./orders.server";
import { fetchTemplateAsset } from "./template-assets.server";
import {
  validatePrintPacket,
  type ExternalAsset,
  type PrintPacket,
} from "./template-model";

const LATEST_ORDER_QUERY = `#graphql
  query PiqaeLatestPreviewOrder {
    orders(first: 1, sortKey: CREATED_AT, reverse: true) {
      nodes { id }
    }
  }
`;

export const EDITOR_PREVIEW_EXPIRES_SECONDS = 300;
const EDITOR_PREVIEW_POLL_ATTEMPTS = 40;
const EDITOR_PREVIEW_POLL_INTERVAL_MS = 250;

export type LatestOrderSummary = { id: string };

/**
 * Read only the identifier needed for an explicit recent-order preview. Buyer,
 * address, line-item and metafield data never enters the editor page payload.
 */
export async function fetchLatestOrderSummary(
  admin: AdminGraphql,
): Promise<LatestOrderSummary | null> {
  const response = await admin.graphql(LATEST_ORDER_QUERY);
  if (!response.ok) throw new Error("Shopify could not load the latest order");
  const payload = (await response.json()) as {
    data?: { orders?: { nodes?: unknown[] } };
    errors?: unknown[];
  };
  if (payload.errors?.length)
    throw new Error("Shopify rejected the latest order query");
  const value = payload.data?.orders?.nodes?.[0];
  if (!value || typeof value !== "object") return null;
  const candidate = value as Record<string, unknown>;
  const id = normalizeOrderGid(String(candidate.id ?? ""));
  return { id };
}

type DraftPreviewRenderClient = Pick<PiqaeClient, "printPackets">;

export function createPreviewDraftRender(
  client: DraftPreviewRenderClient,
  input: CreatePrintPacketPreviewRender,
  idempotencyKey: string,
): Promise<PrintPacketPreviewRender> {
  return client.printPackets.renders.createPreviewDraft(input, idempotencyKey);
}

export function retrievePreviewDraftRender(
  client: DraftPreviewRenderClient,
  id: string,
): Promise<PrintPacketPreviewRender> {
  return client.printPackets.renders.retrievePreviewDraft(id);
}

export function downloadPreviewDraftArtifact(
  client: DraftPreviewRenderClient,
  id: string,
): Promise<Response> {
  return client.printPackets.renders.downloadPreviewDraft(id);
}

export async function createEditorDraftPreview(input: {
  admin: AdminGraphql;
  shop: string;
  latestOrder: LatestOrderSummary;
  specification: PrintPacket;
  assets: ExternalAsset[];
  requestKey: string;
  metafieldAllowlist: string[];
  client: DraftPreviewRenderClient;
  renders: Pick<ShopRepository, "recordRender">;
  sleep?: (milliseconds: number) => Promise<void>;
  assetFetcher?: typeof fetchTemplateAsset;
  signal?: AbortSignal;
}): Promise<{ renderId: string }> {
  input.signal?.throwIfAborted();
  validatePrintPacket(input.specification);
  const [orders, shopIdentity] = await Promise.all([
    fetchOrders(
      input.admin,
      [input.latestOrder.id],
      parseShopifyDataBindings(input.metafieldAllowlist),
    ),
    fetchShopPrintIdentity(input.admin, input.shop),
  ]);
  const [order] = orders;
  if (!order || order.id !== input.latestOrder.id)
    throw new Error("The latest Shopify order is no longer available");
  input.signal?.throwIfAborted();
  const renderInput = shopifyDocumentInput(input.shop, [order], shopIdentity);
  const fetchAsset = input.assetFetcher ?? fetchTemplateAsset;
  await mapWithConcurrency(input.assets, 4, async (asset) => {
    input.signal?.throwIfAborted();
    const bytes = await fetchAsset(asset);
    const body = bytes.buffer.slice(
      bytes.byteOffset,
      bytes.byteOffset + bytes.byteLength,
    ) as ArrayBuffer;
    await input.client.printPackets.resources.putJpeg(asset.digest, body);
    input.signal?.throwIfAborted();
  });
  const idempotencyKey = `shopify-editor-preview-${createHash("sha256")
    .update(input.shop)
    .update("\0")
    .update(input.latestOrder.id)
    .update("\0")
    .update(input.requestKey)
    .update("\0")
    .update(JSON.stringify(input.specification))
    .digest("hex")}`;
  let render = await createPreviewDraftRender(
    input.client,
    {
      specification: input.specification,
      input: renderInput,
      expires_in_seconds: EDITOR_PREVIEW_EXPIRES_SECONDS,
    },
    idempotencyKey,
  );
  await input.renders.recordRender(input.shop, render.id, idempotencyKey, {
    orderGid: order.id,
    ...(order.customer?.id ? { customerGid: order.customer.id } : {}),
  });
  input.signal?.throwIfAborted();
  const sleep =
    input.sleep ??
    ((milliseconds) => abortableDelay(milliseconds, input.signal));
  for (
    let attempt = 0;
    attempt < EDITOR_PREVIEW_POLL_ATTEMPTS &&
    (render.state === "registered" || render.state === "rendering");
    attempt += 1
  ) {
    input.signal?.throwIfAborted();
    await sleep(EDITOR_PREVIEW_POLL_INTERVAL_MS);
    input.signal?.throwIfAborted();
    render = await retrievePreviewDraftRender(input.client, render.id);
  }
  if (render.state === "registered" || render.state === "rendering")
    throw new Error("The PDF preview timed out");
  if (render.state !== "completed")
    throw new Error(
      `The PDF preview failed: ${render.failure_code ?? render.state}`,
    );
  return { renderId: render.id };
}

function abortableDelay(milliseconds: number, signal?: AbortSignal) {
  return new Promise<void>((resolve, reject) => {
    if (signal?.aborted) {
      reject(signal.reason);
      return;
    }
    const timer = setTimeout(done, milliseconds);
    signal?.addEventListener("abort", aborted, { once: true });
    function cleanup() {
      clearTimeout(timer);
      signal?.removeEventListener("abort", aborted);
    }
    function done() {
      cleanup();
      resolve();
    }
    function aborted() {
      cleanup();
      reject(signal?.reason);
    }
  });
}

async function mapWithConcurrency<T>(
  values: T[],
  concurrency: number,
  visit: (value: T) => Promise<void>,
): Promise<void> {
  let cursor = 0;
  await Promise.all(
    Array.from({ length: Math.min(concurrency, values.length) }, async () => {
      while (cursor < values.length) {
        const value = values[cursor++];
        if (value !== undefined) await visit(value);
      }
    }),
  );
}
