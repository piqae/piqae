import type { LoaderFunctionArgs } from "react-router";
import { useEffect } from "react";

const SHOP_DOMAIN = /^[a-z0-9][a-z0-9-]*[a-z0-9][.]myshopify[.]com$/;

export function loader({ request }: LoaderFunctionArgs) {
  const shop = new URL(request.url).searchParams.get("shop") ?? "";
  return { shop: SHOP_DOMAIN.test(shop) ? shop : "your Shopify store" };
}

export default function ConnectComplete() {
  useEffect(() => {
    window.opener?.postMessage(
      { type: "piqae:node-connected" },
      window.location.origin,
    );
    const close = window.setTimeout(() => window.close(), 800);
    return () => window.clearTimeout(close);
  }, []);

  return (
    <main className="piqae-connect-complete">
      <h1>Printer computer connected</h1>
      <p>
        Piqae is now connected to this store. Return to Shopify to choose a
        printer. You can close this tab.
      </p>
      <button type="button" onClick={() => window.close()}>
        Close this tab
      </button>
    </main>
  );
}
