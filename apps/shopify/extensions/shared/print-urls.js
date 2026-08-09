const DOCUMENTS = new Set(["invoice", "packing_slip", "receipt"]);

function safeDocuments(documents) {
  return [...new Set(documents.filter((document) => DOCUMENTS.has(document)))];
}

export function buildAdminPrintUrl({ orderIds, documents, templateId }) {
  const selectedDocuments = safeDocuments(documents);
  if (orderIds.length === 0 || selectedDocuments.length === 0) return null;

  const params = new URLSearchParams({
    orderIds: orderIds.join(","),
    documents: selectedDocuments.join(","),
    format: "pdf",
  });
  if (templateId) params.set("templateId", templateId);
  return `/api/print/admin?${params.toString()}`;
}

export async function authorizedJson(url, options = {}) {
  const token = await shopify.auth.idToken();
  const headers = new Headers(options.headers);
  headers.set("authorization", `Bearer ${token}`);
  headers.set("accept", "application/json");
  const response = await fetch(url, {
    ...options,
    headers,
  });
  const body = await response.json().catch(() => ({}));
  if (!response.ok)
    throw new Error(body.error || `Request failed (${response.status})`);
  return body;
}

export function buildDraftPrintUrl({ draftOrderIds }) {
  if (draftOrderIds.length === 0) return null;
  const params = new URLSearchParams({
    draftOrderIds: [...new Set(draftOrderIds)].join(","),
  });
  return `/api/print/admin-drafts?${params.toString()}`;
}

export function buildPosPrintUrl({ orderId, format }) {
  if (!Number.isSafeInteger(orderId) || orderId <= 0) return null;
  if (format !== "html" && format !== "pdf") return null;
  const params = new URLSearchParams({
    orderId: String(orderId),
    document: "receipt",
    format,
  });
  return `/api/print/pos?${params.toString()}`;
}

export async function printPosReceipt({ printing, orderId, printer }) {
  const html = buildPosPrintUrl({ orderId, format: "html" });
  if (!html) throw new Error("A valid POS order is required");
  if (!printer?.id || !printer.connected)
    throw new Error("Select a connected receipt printer");

  // Never fall back after this call. A rejection can be an uncertain delivery,
  // so opening another print path automatically could produce duplicates.
  await printing.print(html, { printer });
  return { mode: "receipt-printer", printer };
}
