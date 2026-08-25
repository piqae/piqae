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

  // This destination is deliberately fixed. Shopify's authenticated app
  // loader restores embedded Admin navigation and refreshes the newly synced
  // node/printer inventory without accepting a caller-controlled redirect.
  return redirect(SETTINGS_PATH);
}
