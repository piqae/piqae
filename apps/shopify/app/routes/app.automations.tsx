import type { LoaderFunctionArgs } from "react-router";
import { redirect } from "react-router";
import shopify from "../shopify.server";
export async function loader({ request }: LoaderFunctionArgs) {
  await shopify.authenticate.admin(request);
  return redirect("/app/templates", 302);
}
export default function Automations() {
  return null;
}
