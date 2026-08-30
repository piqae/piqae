import { createHash } from "node:crypto";
import type { PiqaeClient } from "@piqae/sdk";

import type { ShopRepository } from "./model";
import {
  fetchOrders,
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

type DraftPreviewRenderInput = {
  specification: PrintPacket;
  input: ReturnType<typeof shopifyDocumentInput>;
  expires_in_seconds: number;
};

// Structurally mirrors the additive generated PrintPacketPreviewRender type.
// Keeping this at the adapter seam lets the Shopify commit compile before the
// SDK generator commit is integrated without weakening runtime purpose checks.
type DraftPreviewRender = {
  id: string;
  purpose: "preview";
  state:
    | "registered"
    | "rendering"
    | "completed"
    | "failed_terminal"
    | "expiring"
    | "expired";
  failure_code?: string | null;
  expires_at: string;
  created_at: string;
  updated_at: string;
};

type DraftPreviewRenders =
  DraftPreviewRenderClient["printPackets"]["renders"] & {
    createPreviewDraft?: (
      input: DraftPreviewRenderInput,
      idempotencyKey: string,
    ) => Promise<DraftPreviewRender>;
    retrievePreviewDraft?: (id: string) => Promise<DraftPreviewRender>;
    downloadPreviewDraft?: (id: string) => Promise<Response>;
  };

function draftPreviewRenders(client: DraftPreviewRenderClient) {
  return client.printPackets.renders as DraftPreviewRenders;
}

/**
 * Keep the draft-preview SDK seam in one place while the additive SDK method
 * lands. Production fails closed if the connected control plane is older.
 */
export function createPreviewDraftRender(
  client: DraftPreviewRenderClient,
  input: DraftPreviewRenderInput,
  idempotencyKey: string,
): Promise<DraftPreviewRender> {
  const renders = draftPreviewRenders(client);
  if (typeof renders.createPreviewDraft !== "function")
    throw new Error("Piqae draft PDF previews are not available yet");
  return renders.createPreviewDraft(input, idempotencyKey);
}

export function retrievePreviewDraftRender(
  client: DraftPreviewRenderClient,
  id: string,
): Promise<DraftPreviewRender> {
  const retrieve = draftPreviewRenders(client).retrievePreviewDraft;
  if (typeof retrieve !== "function")
    throw new Error("Piqae draft PDF previews are not available yet");
  return retrieve(id);
}

export function downloadPreviewDraftArtifact(
  client: DraftPreviewRenderClient,
  id: string,
): Promise<Response> {
  const download = draftPreviewRenders(client).downloadPreviewDraft;
  if (typeof download !== "function")
    throw new Error("Piqae draft PDF previews are not available yet");
  return download(id);
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
  const [order] = await fetchOrders(
    input.admin,
    [input.latestOrder.id],
    parseShopifyDataBindings(input.metafieldAllowlist),
  );
  if (!order || order.id !== input.latestOrder.id)
    throw new Error("The latest Shopify order is no longer available");
  input.signal?.throwIfAborted();
  const renderInput = shopifyDocumentInput(input.shop, [order]);
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
