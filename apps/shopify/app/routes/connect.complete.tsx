import type { LoaderFunctionArgs } from "react-router";
import { redirect } from "react-router";
import shopify from "../shopify.server";

const SETTINGS_PATH = "/app/printers";

export async function loader({ request }: LoaderFunctionArgs) {
  const { session } = await shopify.authenticate.admin(request);
  const requestedShop = new URL(request.url).searchParams.get("shop");
  if (requestedShop && requestedShop !== session.shop) {
    throw new Response("The connection belongs to a different Shopify store.", {
      status: 403,
    });
  }

  // Keep the destination fixed and derive the only query value from the
  // authenticated session. App Bridge needs the shop context after this
  // top-level, non-Shopify connection flow returns to the embedded app.
  return redirect(`${SETTINGS_PATH}?shop=${encodeURIComponent(session.shop)}`);
}
