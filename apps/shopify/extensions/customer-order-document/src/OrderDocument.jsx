/** @jsxImportSource preact */
import { render } from "preact";
import { useState } from "preact/hooks";

export function isTrustedDownloadUrl(value, expectedOrigin) {
  try {
    const url = new URL(value);
    return (
      url.origin === expectedOrigin &&
      url.pathname === "/api/public/documents/download" &&
      url.searchParams.has("token") &&
      !url.username &&
      !url.password &&
      !url.hash
    );
  } catch {
    return false;
  }
}

export default async () => {
  render(<OrderDocument />, document.body);
};

export function OrderDocument() {
  const [state, setState] = useState("ready");
  const [downloadUrl, setDownloadUrl] = useState("");
  const orderId = shopify.order.value?.id;

  async function prepare() {
    if (!orderId) return setState("failed");
    setState("loading");
    try {
      const sessionToken = await shopify.sessionToken.get();
      const response = await fetch(
        `/api/customer/documents?orderId=${encodeURIComponent(orderId)}`,
        {
          headers: { authorization: `Bearer ${sessionToken}` },
          signal: AbortSignal.timeout(10_000),
        },
      );
      if (!response.ok) throw new Error("document unavailable");
      const value = await response.json();
      if (
        typeof value.downloadUrl !== "string" ||
        !isTrustedDownloadUrl(value.downloadUrl, new URL(response.url).origin)
      ) {
        throw new Error("invalid download URL");
      }
      setDownloadUrl(value.downloadUrl);
      setState("ready");
    } catch {
      setState("failed");
    }
  }

  return (
    <s-section heading="Order documents">
      <s-stack direction="block" gap="base">
        <s-text>Download a PDF invoice for this order.</s-text>
        {state === "failed" && (
          <s-banner tone="critical">
            The PDF could not be prepared. Try again or contact the store.
          </s-banner>
        )}
        {downloadUrl ? (
          <s-link href={downloadUrl} target="_blank">
            Download invoice PDF
          </s-link>
        ) : (
          <s-button
            disabled={!orderId || state === "loading"}
            onClick={prepare}
          >
            {state === "loading" ? "Preparing PDF…" : "Prepare invoice PDF"}
          </s-button>
        )}
      </s-stack>
    </s-section>
  );
}
